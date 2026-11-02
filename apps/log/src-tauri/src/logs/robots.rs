//! robots.txt summary for AI crawler tokens (RFC 9309 group selection, simplified to an
//! allowed / partial / blocked verdict for the whole site).

use serde::Serialize;
use serde_json::{json, Value};

/// Robots tokens worth reporting, with the category they belong to.
pub const TOKENS: &[(&str, &str)] = &[
    ("GPTBot", "ai_training"),
    ("ClaudeBot", "ai_training"),
    ("anthropic-ai", "ai_training"),
    ("Google-Extended", "ai_training"),
    ("Google-CloudVertexBot", "ai_training"),
    ("Applebot-Extended", "ai_training"),
    ("CCBot", "ai_training"),
    ("Bytespider", "ai_training"),
    ("Amazonbot", "ai_training"),
    ("meta-externalagent", "ai_training"),
    ("cohere-ai", "ai_training"),
    ("Diffbot", "ai_training"),
    ("OAI-SearchBot", "ai_assistant"),
    ("ChatGPT-User", "ai_assistant"),
    ("Claude-SearchBot", "ai_assistant"),
    ("Claude-User", "ai_assistant"),
    ("PerplexityBot", "ai_assistant"),
    ("Perplexity-User", "ai_assistant"),
    ("MistralAI-User", "ai_assistant"),
    ("DuckAssistBot", "ai_assistant"),
    ("Meta-ExternalFetcher", "ai_assistant"),
    ("Googlebot", "search"),
    ("Bingbot", "search"),
];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Group {
    pub agents: Vec<String>,
    /// `(allow, path)`
    pub rules: Vec<(bool, String)>,
}

pub fn parse_robots(text: &str) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let mut current = Group::default();
    let mut in_rules = false;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once(':') else { continue };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        match key.as_str() {
            "user-agent" => {
                if in_rules {
                    groups.push(std::mem::take(&mut current));
                    in_rules = false;
                }
                current.agents.push(value.to_ascii_lowercase());
            }
            "allow" | "disallow" => {
                if current.agents.is_empty() {
                    continue;
                }
                in_rules = true;
                current.rules.push((key == "allow", value.to_string()));
            }
            _ => {}
        }
    }
    if !current.agents.is_empty() {
        groups.push(current);
    }
    groups
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TokenVerdict {
    pub token: String,
    pub category: String,
    /// `allowed` | `partial` | `blocked`
    pub verdict: String,
    /// `token` (a group names this bot) | `wildcard` (`*` group) | `none`
    pub matched: String,
    pub rules: Vec<String>,
}

pub fn evaluate(groups: &[Group], token: &str, category: &str) -> TokenVerdict {
    let t = token.to_ascii_lowercase();
    let specific: Vec<&Group> = groups.iter().filter(|g| g.agents.iter().any(|a| *a == t)).collect();
    let (chosen, matched) = if !specific.is_empty() {
        (specific, "token")
    } else {
        let wild: Vec<&Group> = groups.iter().filter(|g| g.agents.iter().any(|a| a == "*")).collect();
        if wild.is_empty() { (Vec::new(), "none") } else { (wild, "wildcard") }
    };
    let rules: Vec<(bool, String)> = chosen.iter().flat_map(|g| g.rules.iter().cloned()).filter(|(_, p)| !p.is_empty()).collect();
    let root_block = rules.iter().any(|(allow, p)| !allow && (p == "/" || p == "/*"));
    let any_allow = rules.iter().any(|(allow, p)| *allow && p != "/" && p != "/*");
    let root_allow = rules.iter().any(|(allow, p)| *allow && (p == "/" || p == "/*"));
    let any_disallow = rules.iter().any(|(allow, _)| !allow);
    let verdict = if root_block && !root_allow {
        if any_allow { "partial" } else { "blocked" }
    } else if any_disallow {
        "partial"
    } else {
        "allowed"
    };
    TokenVerdict {
        token: token.into(),
        category: category.into(),
        verdict: verdict.into(),
        matched: matched.into(),
        rules: rules.iter().map(|(a, p)| format!("{}: {p}", if *a { "Allow" } else { "Disallow" })).collect(),
    }
}

pub fn summarize(text: &str) -> Vec<TokenVerdict> {
    let groups = parse_robots(text);
    TOKENS.iter().map(|(t, c)| evaluate(&groups, t, c)).collect()
}

/// Fetches `https://<domain>/robots.txt` and summarises it for the AI tokens.
pub async fn fetch_summary(domain: &str) -> Result<Value, String> {
    let domain = crate::hub::text::normalize_domain(domain);
    if domain.is_empty() {
        return Err("The project has no domain. Set one on the Projects page to read its robots.txt.".into());
    }
    let url = format!("https://{domain}/robots.txt");
    let resp = crate::hub::http::client().get(&url).send().await.map_err(|e| format!("{url}: {e}"))?;
    let status = resp.status().as_u16();
    let final_url = resp.url().to_string();
    let text = if resp.status().is_success() { resp.text().await.map_err(|e| e.to_string())? } else { String::new() };
    // A missing robots.txt (4xx) means everything is allowed.
    let bots = summarize(&text);
    Ok(json!({ "url": url, "finalUrl": final_url, "status": status, "fetchedAt": crate::hub::now_iso(), "bytes": text.len(), "bots": bots }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROBOTS: &str = "# test\nUser-agent: *\nDisallow: /wp-admin/\nAllow: /wp-admin/admin-ajax.php\n\nUser-agent: GPTBot\nUser-agent: CCBot\nDisallow: /\n\nUser-agent: ClaudeBot\nDisallow: /\nAllow: /blog/\n\nUser-agent: PerplexityBot\nDisallow:\n\nSitemap: https://seobify.com/sitemap.xml\n";

    fn verdict(token: &str) -> String {
        summarize(ROBOTS).into_iter().find(|v| v.token == token).unwrap().verdict
    }

    #[test]
    fn groups_and_verdicts() {
        let groups = parse_robots(ROBOTS);
        assert_eq!(groups.len(), 4);
        assert_eq!(groups[1].agents, vec!["gptbot", "ccbot"]);
        assert_eq!(verdict("GPTBot"), "blocked");
        assert_eq!(verdict("CCBot"), "blocked");
        assert_eq!(verdict("ClaudeBot"), "partial");
        assert_eq!(verdict("PerplexityBot"), "allowed", "empty Disallow allows everything");
        assert_eq!(verdict("Googlebot"), "partial", "falls back to the * group");
        let v = summarize(ROBOTS).into_iter().find(|v| v.token == "Googlebot").unwrap();
        assert_eq!(v.matched, "wildcard");
        assert!(summarize("").iter().all(|v| v.verdict == "allowed" && v.matched == "none"));
    }
}
