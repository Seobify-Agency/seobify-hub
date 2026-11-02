//! Seobify Log MCP tool set and database migrations.
//!
//! Tool tables live in `MIGRATIONS` (append only — never edit a released entry). MCP tools
//! call the same `logs::service` functions as the Tauri commands. Shared hub tools
//! (hub_info, list_projects, create_project, spider_projects, spider_urls) are added by `hub::mcp`.

use crate::hub::mcp::{self, BoxFuture, ToolSet};
use crate::logs::query::UrlFilter;
use crate::logs::service;
use serde_json::{json, Value};

/// Tool schema migrations, applied in order and tracked by `PRAGMA user_version`.
pub const MIGRATIONS: &[&str] = &[
    // 1: log analysis schema (v0.1.0).
    "CREATE TABLE IF NOT EXISTS log_files (
        id INTEGER PRIMARY KEY,
        project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        path TEXT NOT NULL,
        name TEXT NOT NULL,
        fingerprint TEXT NOT NULL,
        size_bytes INTEGER NOT NULL DEFAULT 0,
        format TEXT NOT NULL,
        lines_total INTEGER NOT NULL DEFAULT 0,
        lines_parsed INTEGER NOT NULL DEFAULT 0,
        lines_failed INTEGER NOT NULL DEFAULT 0,
        bot_hits INTEGER NOT NULL DEFAULT 0,
        human_requests INTEGER NOT NULL DEFAULT 0,
        first_ts TEXT,
        last_ts TEXT,
        imported_at TEXT NOT NULL,
        duration_ms INTEGER NOT NULL DEFAULT 0,
        sample_errors TEXT NOT NULL DEFAULT '[]',
        UNIQUE (project_id, fingerprint)
    );
    CREATE TABLE IF NOT EXISTS bot_hits (
        id INTEGER PRIMARY KEY,
        project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        file_id INTEGER NOT NULL,
        ts TEXT NOT NULL,
        day TEXT NOT NULL,
        bot_id TEXT NOT NULL,
        category TEXT NOT NULL,
        verification TEXT NOT NULL,
        ip TEXT,
        method TEXT NOT NULL,
        host TEXT,
        path TEXT NOT NULL,
        query TEXT,
        status INTEGER NOT NULL,
        bytes INTEGER,
        response_ms REAL,
        referer_host TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_bot_hits_day ON bot_hits(project_id, day);
    CREATE INDEX IF NOT EXISTS idx_bot_hits_bot ON bot_hits(project_id, bot_id, day);
    CREATE INDEX IF NOT EXISTS idx_bot_hits_path ON bot_hits(project_id, path);
    CREATE TABLE IF NOT EXISTS human_daily (
        project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        file_id INTEGER NOT NULL,
        day TEXT NOT NULL,
        status_class TEXT NOT NULL,
        device TEXT NOT NULL,
        requests INTEGER NOT NULL,
        PRIMARY KEY (project_id, day, status_class, device, file_id)
    );
    CREATE TABLE IF NOT EXISTS url_daily (
        project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        day TEXT NOT NULL,
        url TEXT NOT NULL,
        bot_id TEXT NOT NULL,
        hits INTEGER NOT NULL,
        s2xx INTEGER NOT NULL,
        s3xx INTEGER NOT NULL,
        s4xx INTEGER NOT NULL,
        s5xx INTEGER NOT NULL,
        last_ts TEXT,
        PRIMARY KEY (project_id, day, url, bot_id)
    );
    CREATE INDEX IF NOT EXISTS idx_url_daily_url ON url_daily(project_id, url);
    CREATE TABLE IF NOT EXISTS ip_verifications (
        ip_key TEXT NOT NULL,
        bot_family TEXT NOT NULL,
        verified INTEGER NOT NULL,
        method TEXT NOT NULL,
        hostname TEXT,
        checked_at TEXT NOT NULL,
        PRIMARY KEY (ip_key, bot_family)
    );
    CREATE TABLE IF NOT EXISTS ip_ranges (
        source TEXT PRIMARY KEY,
        url TEXT NOT NULL,
        fetched_at TEXT,
        prefixes INTEGER NOT NULL DEFAULT 0,
        json TEXT,
        error TEXT,
        checked_at TEXT
    );",
    // 2: Cloudflare connection (v0.1.3): zone per project and the imported analytics days.
    "CREATE TABLE IF NOT EXISTS cf_links (
        project_id INTEGER PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
        zone_id TEXT NOT NULL,
        zone_name TEXT NOT NULL,
        account_id TEXT,
        linked_at TEXT NOT NULL,
        last_sync_at TEXT,
        last_error TEXT
    );
    CREATE TABLE IF NOT EXISTS cf_days (
        project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        day TEXT NOT NULL,
        file_id INTEGER,
        requests INTEGER NOT NULL DEFAULT 0,
        bot_requests INTEGER NOT NULL DEFAULT 0,
        truncated INTEGER NOT NULL DEFAULT 0,
        fetched_at TEXT NOT NULL,
        PRIMARY KEY (project_id, day)
    );",
];

pub struct Tools;

fn range_props() -> Value {
    json!({
        "project": mcp::project_schema(),
        "from": { "type": "string", "description": "First day, YYYY-MM-DD (default: first day with data)." },
        "to": { "type": "string", "description": "Last day, YYYY-MM-DD (default: last day with data)." }
    })
}

fn with(mut base: Value, extra: Value) -> Value {
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    base
}

fn s(args: &Value, key: &str) -> Option<String> {
    mcp::arg_str(args, key).map(str::to_string)
}

fn project_key(args: &Value) -> Result<String, String> {
    args.get("project")
        .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
        .filter(|k| !k.trim().is_empty() && k != "null")
        .ok_or_else(|| "Missing required argument: project (id or name; see list_projects)".to_string())
}

/// Keeps MCP payloads small: truncates the named arrays to `limit` rows.
fn cap(mut v: Value, keys: &[&str], limit: usize) -> Value {
    for k in keys {
        if let Some(Value::Array(a)) = v.get_mut(*k) {
            a.truncate(limit);
        }
    }
    v
}

impl ToolSet for Tools {
    fn instructions(&self) -> String {
        "Seobify Log analyses web server access logs locally to show how search engines and AI crawlers really crawl a site. \
Workflow: create_project (or list_projects) → import_logs with file or folder paths (combined/common, nginx/caddy JSON, Cloudflare Logpush, IIS, CloudFront, ALB; .gz ok; format auto-detected; duplicates are skipped unless force=true) → log_overview for totals, categories and verification → bot_stats, top_urls, status_anomalies, crawl_waste, ai_bot_activity and fake_bots for detail → spider_gaps to compare Googlebot activity with a Seobify Spider crawl (orphan candidates, uncrawled indexable pages, crawled problem pages) → generate_report for a branded HTML report. \
Crawler identity is verified with published IP ranges and reverse DNS; human visitors are kept only as IP-free daily counters (GDPR). Dates are YYYY-MM-DD and default to the full imported period. \
Sites served entirely by Cloudflare have no server log: once the user has added a Cloudflare API token in the app, cloudflare_status lists the zones and cloudflare_sync(project, zone) imports the last 30 days of Cloudflare analytics (sampled estimates; crawler IPs are verified like server logs when the plan exposes them, human traffic stays IP-free) and later only new days."
            .into()
    }

    fn tools(&self) -> Vec<Value> {
        vec![
            mcp::tool("cloudflare_status", "Cloudflare connection: whether an API token is stored (masked), the zones it can read, and which projects are linked with their synced day range and last error. The token itself is added by the user in the app.", json!({}), &[]),
            mcp::tool(
                "cloudflare_sync",
                "Imports Cloudflare analytics for a project: on the first run the last `days` (default 30, max 31), afterwards missing days plus the two newest. Pass `zone` (name or id) to link the project to a Cloudflare zone first. Hits are sampled estimates; crawler IPs are verified when the plan exposes them.",
                json!({
                    "project": mcp::project_schema(),
                    "zone": { "type": "string", "description": "Cloudflare zone name (e.g. example.com) or id; links the project to it." },
                    "days": { "type": "integer", "description": "Days back on the first sync (1-31, default 30)." },
                    "refresh": { "type": "boolean", "description": "Re-fetch every day in the window (replaces those imports)." }
                }),
                &["project"],
            ),
            mcp::tool(
                "import_logs",
                "Imports server access log files or folders (recursively; .gz streamed) into a project. Returns lines, parsed/failed counts, bot hits, human requests and duration per file. Files imported before are reported as duplicates unless force=true.",
                json!({
                    "project": mcp::project_schema(),
                    "paths": { "type": "array", "items": { "type": "string" }, "description": "Absolute file or folder paths." },
                    "format": { "type": "string", "enum": ["auto", "combined", "common", "json", "cloudflare", "iis", "cloudfront", "alb"], "description": "Log format (default auto-detect per file)." },
                    "force": { "type": "boolean", "description": "Re-import files that were imported before (replaces their data)." }
                }),
                &["project", "paths"],
            ),
            mcp::tool("log_overview", "Totals for a period: bot requests by category, verified/fake/unverifiable shares, status code mix, unique URLs, Googlebot smartphone/desktop split, human request counter and the 10 most crawled URLs.", range_props(), &["project"]),
            mcp::tool(
                "bot_stats",
                "Per-bot statistics: requests, unique URLs, 2xx/3xx/4xx/5xx, average response time, first/last seen and verification counts.",
                with(range_props(), json!({ "category": { "type": "string", "enum": ["all", "search", "ai_training", "ai_assistant", "seo_tool", "social", "other_bot"] } })),
                &["project"],
            ),
            mcp::tool(
                "top_urls",
                "Most crawled URLs (fake bots excluded) with total and Googlebot hits, last Googlebot visit, status classes and crawl frequency per day. Filter by bot (bot id, 'googlebot' or a category) and status (2xx/3xx/4xx/5xx/error).",
                with(range_props(), json!({
                    "bot": { "type": "string", "description": "Bot id (see bot_stats), 'googlebot' or a category name." },
                    "status": { "type": "string", "enum": ["2xx", "3xx", "4xx", "5xx", "error"] },
                    "dir": { "type": "string", "description": "Only URLs under this first path segment, e.g. /blog/." },
                    "limit": { "type": "integer", "description": "Max rows (default 50)." }
                })),
                &["project"],
            ),
            mcp::tool(
                "crawl_waste",
                "Crawl-budget waste for Googlebot (or all search bots / all bots): parameterised URLs grouped by parameter name, redirects, 4xx, 5xx, static resources and — when the project is linked to a Spider project — non-indexable or canonicalised pages, each with share and example URLs.",
                with(range_props(), json!({ "scope": { "type": "string", "enum": ["googlebot", "search", "all"], "description": "Which crawlers count (default googlebot)." } })),
                &["project"],
            ),
            mcp::tool(
                "spider_gaps",
                "Compares Googlebot activity in the last N days with a Seobify Spider crawl: orphan candidates (crawled by Googlebot, absent from the crawl), indexable pages Googlebot never crawled, and noindex/3xx/4xx pages Googlebot still crawls.",
                json!({
                    "project": mcp::project_schema(),
                    "spider_project": { "type": "string", "description": "Spider project name (default: the project's linked Spider project, else 'default')." },
                    "crawl_id": { "type": "integer", "description": "Spider crawl id (default: newest completed crawl of the project's domain)." },
                    "days": { "type": "integer", "description": "Look-back window ending at the newest log day (default 30)." },
                    "limit": { "type": "integer", "description": "Max rows per list (default 100)." }
                }),
                &["project"],
            ),
            mcp::tool("ai_bot_activity", "AI training vs AI search/assistant crawler requests over time, per-bot totals with their most fetched directories, verification counts and the most fetched URLs.", range_props(), &["project"]),
            mcp::tool(
                "fake_bots",
                "Requests whose user agent claims a verifiable crawler (Googlebot, Bingbot, GPTBot, …) but failed IP-range / reverse-DNS verification: masked or hashed source, requests, sample URLs, first/last seen, plus fake share per claimed bot.",
                with(range_props(), json!({ "limit": { "type": "integer", "description": "Max groups (default 50)." } })),
                &["project"],
            ),
            mcp::tool("status_anomalies", "4xx/5xx totals and daily timeline (per bot), error URLs with status codes, and spike days where the daily count exceeded the trailing 28-day mean by more than 3σ.", range_props(), &["project"]),
            mcp::tool(
                "generate_report",
                "Builds a branded HTML report (summary, daily chart, findings critical→low, top URLs, error URLs, crawl waste, AI bots, fake bots, Spider comparison when linked) and returns the saved file path plus the findings.",
                with(range_props(), json!({ "lang": { "type": "string", "enum": ["tr", "en"], "description": "Report language (default tr)." } })),
                &["project"],
            ),
        ]
    }

    fn call<'a>(&'a self, name: &'a str, args: Value) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            if name == "cloudflare_status" {
                return service::cloudflare_status().await;
            }
            let key = project_key(&args)?;
            let (from, to) = (s(&args, "from"), s(&args, "to"));
            match name {
                "cloudflare_sync" => {
                    if let Some(zone) = s(&args, "zone").filter(|z| !z.trim().is_empty()) {
                        service::cloudflare_link(key.clone(), zone).await?;
                    }
                    service::cloudflare_sync(key, mcp::arg_i64(&args, "days"), mcp::arg_bool(&args, "refresh").unwrap_or(false), |_| {}).await
                }
                "import_logs" => {
                    let paths = mcp::arg_str_list(&args, "paths");
                    service::import_logs(key, paths, s(&args, "format"), mcp::arg_bool(&args, "force").unwrap_or(false), |_| {}).await
                }
                "log_overview" => service::overview(key, from, to).await,
                "bot_stats" => service::bot_stats(key, from, to, s(&args, "category")).await,
                "top_urls" => {
                    let limit = mcp::arg_i64(&args, "limit").unwrap_or(50).clamp(1, 1000);
                    let filter = UrlFilter { bot: s(&args, "bot"), status: s(&args, "status"), dir: s(&args, "dir"), limit: Some(limit), ..Default::default() };
                    let mut v = service::urls(key, from, to, filter).await?;
                    if let Some(o) = v.as_object_mut() {
                        o.remove("dirs");
                    }
                    Ok(v)
                }
                "crawl_waste" => service::crawl_waste(key, from, to, s(&args, "scope")).await,
                "spider_gaps" => {
                    let limit = mcp::arg_i64(&args, "limit").unwrap_or(100);
                    service::spider_gaps(key, s(&args, "spider_project"), mcp::arg_i64(&args, "crawl_id"), mcp::arg_i64(&args, "days"), Some(limit)).await
                }
                "ai_bot_activity" => service::ai_activity(key, from, to).await,
                "fake_bots" => service::fake_bots(key, from, to, mcp::arg_i64(&args, "limit").unwrap_or(50)).await,
                "status_anomalies" => service::status(key, from, to).await.map(|v| cap(v, &["errorUrls"], 100)),
                "generate_report" => service::generate_report(key, from, to, s(&args, "lang"), None).await,
                _ => Err(format!("Unknown tool: {name}")),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply() {
        let conn = crate::hub::db::open_memory().unwrap();
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v as usize, super::MIGRATIONS.len());
    }

    #[test]
    fn twelve_tools_with_valid_schemas() {
        let tools = Tools.tools();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            vec!["cloudflare_status", "cloudflare_sync", "import_logs", "log_overview", "bot_stats", "top_urls", "crawl_waste", "spider_gaps", "ai_bot_activity", "fake_bots", "status_anomalies", "generate_report"]
        );
        for t in &tools {
            assert_eq!(t["inputSchema"]["type"], "object");
            if t["name"] != "cloudflare_status" {
                assert!(t["inputSchema"]["required"].as_array().unwrap().iter().any(|r| r == "project"), "{}", t["name"]);
            }
            assert!(t["description"].as_str().unwrap().len() > 40);
        }
        assert!(Tools.instructions().contains("import_logs") && Tools.instructions().contains("cloudflare_sync"));
    }

    #[tokio::test]
    async fn missing_project_is_a_clear_error() {
        let err = Tools.call("log_overview", json!({})).await.unwrap_err();
        assert!(err.contains("project"));
    }
}
