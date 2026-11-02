//! Redirect rule generators: Apache, Nginx, IIS, Cloudflare Bulk Redirects, Netlify, Vercel,
//! Next.js, WordPress Redirection and plain CSV.
//!
//! Escaping rules (per server):
//! - Apache `Redirect` and Nginx `map $uri` compare against the *decoded* path, so sources
//!   are written decoded (Turkish characters as UTF-8) and quoted when needed;
//!   `RewriteRule`/`rewrite` sources are regex-escaped; `$request_uri` keys stay encoded.
//! - Targets are always ASCII (`norm::encode_url`): `ş` → `%C5%9F`, `$` → `%24`; in
//!   `RewriteRule` substitutions `%` is written `\%` (with `NE`) so `%2F` is not read as a
//!   back-reference.
//! - Formats without regex support (Cloudflare, Netlify, Vercel, Next.js, CSV) get the
//!   pattern rules expanded into explicit rules for the known old URLs.

use super::norm;
use serde::{Deserialize, Serialize};

/// `(format id, default file name, label)`.
pub const FORMATS: &[(&str, &str, &str)] = &[
    ("apache", ".htaccess", "Apache .htaccess"),
    ("nginx", "redirects.conf", "Nginx"),
    ("iis", "web.config", "IIS web.config"),
    ("cloudflare", "cloudflare-bulk-redirects.csv", "Cloudflare Bulk Redirects CSV"),
    ("netlify", "_redirects", "Netlify _redirects"),
    ("vercel", "vercel.json", "Vercel vercel.json"),
    ("nextjs", "next.config.js", "Next.js redirects()"),
    ("wordpress", "redirection.csv", "WordPress Redirection CSV"),
    ("csv", "redirects.csv", "CSV"),
];

pub fn format_ids() -> Vec<&'static str> {
    FORMATS.iter().map(|f| f.0).collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleEntry {
    pub mapping_id: i64,
    pub source_url: String,
    /// Absolute target (`None` for 410).
    pub target_url: Option<String>,
    /// 301 or 410.
    pub status: u16,
    pub confidence: i64,
    pub reason: String,
    /// The first matching pattern already produces this target (omitted where patterns are native).
    pub covered: bool,
    /// Matches a pattern but needs another target (or 410): must run before the pattern rules.
    pub overrides_pattern: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternRule {
    pub position: i64,
    pub regex: String,
    pub replacement: String,
}

#[derive(Debug, Clone)]
pub struct RuleSet {
    pub plan_name: String,
    pub old_base: String,
    pub new_base: String,
    pub entries: Vec<RuleEntry>,
    pub patterns: Vec<PatternRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportOptions {
    /// Keep the visitor's query string on redirects (default true).
    pub preserve_query: bool,
    /// Header row for CSV formats (default: on for csv/wordpress, off for cloudflare).
    pub include_header: Option<bool>,
    /// Comment language: `tr` or `en`.
    pub lang: String,
    /// Plan option: sources ignore query strings.
    pub ignore_query: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions { preserve_query: true, include_header: None, lang: "tr".into(), ignore_query: true }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOutput {
    pub format: String,
    pub filename: String,
    pub content: String,
    /// Explicit rules written.
    pub rules: usize,
    /// Pattern rules written natively.
    pub pattern_rules: usize,
    /// Entries left out (not expressible in this format).
    pub skipped: usize,
    pub warnings: Vec<String>,
    /// Save with a UTF-8 BOM (Excel-friendly CSV).
    pub bom: bool,
}

/// Marks entries covered or overridden by the enabled patterns.
pub fn classify(entries: &mut [RuleEntry], patterns: &[PatternRule], new_base: &str, opts: &super::model::MatchOptions) -> Result<(), String> {
    let compiled: Vec<(regex::Regex, &str)> = patterns
        .iter()
        .map(|p| regex::Regex::new(&p.regex).map(|re| (re, p.replacement.as_str())).map_err(|e| format!("Pattern #{}: {e}", p.position)))
        .collect::<Result<_, _>>()?;
    for e in entries.iter_mut() {
        let clean = norm::clean_path(&norm::parts(&e.source_url).path);
        let hit = compiled.iter().find_map(|(re, rep)| norm::apply_pattern(re, rep, &clean));
        if let Some(out) = hit {
            let produced = norm::resolve(&out, new_base);
            match &e.target_url {
                Some(t) if norm::site_key(t, opts) == norm::site_key(&produced, opts) && e.status != 410 => e.covered = true,
                _ => e.overrides_pattern = true,
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- helpers

fn tr(lang: &str, key: &str) -> &'static str {
    let en = lang == "en";
    match key {
        "rules" => if en { "explicit rules" } else { "tekil kural" },
        "patterns" => if en { "pattern rules" } else { "desen kuralı" },
        "pattern_block" => if en { "Pattern rules" } else { "Desen kuralları" },
        "before" => if en { "Single rules that must run before the pattern rules (overrides, query strings, 410)" } else { "Desenlerden önce çalışması gereken tekil kurallar (istisnalar, sorgu parametreleri, 410)" },
        "exact" => if en { "Single redirects, most specific first (Redirect matches path prefixes; RedirectMatch is exact)" } else { "Tekil yönlendirmeler, en özelden genele (Redirect yol önekiyle eşleşir; RedirectMatch tam eşleşir)" },
        "rewrite_first" => if en { "mod_rewrite rules run before mod_alias Redirect lines." } else { "mod_rewrite kuralları mod_alias Redirect satırlarından önce çalışır." },
        "http" => if en { "1) Add to the http { } block:" } else { "1) http { } bloğuna ekleyin:" },
        "server" => if en { "2) Add to the server { } block:" } else { "2) server { } bloğuna ekleyin:" },
        "bucket" => if en { "For long URLs add to http { } if nginx asks: map_hash_bucket_size 256;" } else { "Uzun URL'lerde nginx isterse http { } içine: map_hash_bucket_size 256;" },
        "expanded" => if en { "Pattern rules are not supported by this format and were expanded into explicit rules." } else { "Desen kuralları bu biçimde desteklenmediği için açık kurallara dönüştürüldü." },
        "skip_query" => if en { "skipped (query string not supported)" } else { "atlandı (sorgu parametresi desteklenmiyor)" },
        "skip_gone" => if en { "skipped (410 not supported)" } else { "atlandı (410 desteklenmiyor)" },
        "netlify_410" => if en { "410 rules serve /404.html with status 410." } else { "410 kuralları /404.html içeriğini 410 durumuyla sunar." },
        "next_slash" => if en { "Sources have no trailing slash (Next.js default). With trailingSlash: true add it back." } else { "Kaynaklar sondaki / olmadan yazıldı (Next.js varsayılanı). trailingSlash: true kullanıyorsanız geri ekleyin." },
        "iis_keys" => if en { "Map keys are listed decoded and encoded so either {URL} form matches." } else { "Eşleme anahtarları çözülmüş ve kodlanmış biçimde eklendi; {URL} hangi biçimde gelirse eşleşir." },
        _ => "",
    }
}

fn header_lines(set: &RuleSet, lang: &str, rules: usize, patterns: usize, gone: usize) -> Vec<String> {
    vec![
        format!("Seobify Migrate — {} ({} → {})", set.plan_name, set.old_base, set.new_base),
        format!("{rules} {}, {patterns} {}, {gone} × 410", tr(lang, "rules"), tr(lang, "patterns")),
    ]
}

/// Decoded source path exactly as requested (trailing slash kept).
fn src_path(e: &RuleEntry) -> String {
    norm::parts(&e.source_url).path
}

fn src_raw_path(e: &RuleEntry) -> String {
    norm::encode_path(&norm::parts(&e.source_url).path)
}

fn src_query(e: &RuleEntry, opts: &ExportOptions) -> Option<String> {
    if opts.ignore_query {
        None
    } else {
        norm::parts(&e.source_url).query
    }
}

fn same_origin(a: &str, b: &str) -> bool {
    let (pa, pb) = (norm::parts(a), norm::parts(b));
    !pa.host.is_empty() && pa.host == pb.host && pa.scheme == pb.scheme
}

/// ASCII target; root-relative when it stays on the old origin.
fn target(e: &RuleEntry, set: &RuleSet) -> String {
    let t = e.target_url.as_deref().unwrap_or_default();
    let enc = norm::encode_url(t);
    if same_origin(t, &set.old_base) {
        let p = norm::parts(&enc);
        format!("{}{}", p.raw_path, p.query.map(|q| format!("?{q}")).unwrap_or_default())
    } else {
        enc
    }
}

fn abs_target(e: &RuleEntry) -> String {
    norm::encode_url(e.target_url.as_deref().unwrap_or_default())
}

/// Origin prefix for pattern replacements when the new site is on another origin.
fn pattern_prefix(set: &RuleSet) -> String {
    let (o, n) = (norm::origin(&set.old_base), norm::origin(&set.new_base));
    if o == n {
        String::new()
    } else {
        n
    }
}

fn names_of(regex: &str) -> Vec<Option<String>> {
    regex::Regex::new(regex).map(|re| re.capture_names().map(|n| n.map(str::to_string)).collect()).unwrap_or_default()
}

/// Rewrites a replacement for a server: `group(n)` renders back-references, `lit` escapes text.
fn render_replacement(p: &PatternRule, prefix: &str, group: impl Fn(usize) -> String, lit: impl Fn(&str) -> String) -> String {
    let mut out = String::new();
    if p.replacement.starts_with('/') {
        out.push_str(&lit(prefix));
    }
    for part in norm::parse_replacement(&p.replacement, &names_of(&p.regex)) {
        match part {
            norm::RepPart::Lit(s) => out.push_str(&lit(&s)),
            norm::RepPart::Group(i) => out.push_str(&group(i)),
        }
    }
    out
}

/// Path without leading/trailing slash, regex-escaped, for `^/?…/?$` rules.
fn exact_regex(decoded: &str) -> String {
    let trimmed = decoded.trim_matches('/');
    if trimmed.is_empty() {
        "^/?$".into()
    } else {
        format!("^/?{}/?$", norm::regex_escape(trimmed))
    }
}

fn apache_quote(s: &str) -> String {
    if s.contains(' ') || s.contains('"') || s.contains('\t') {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn apache_subst(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('$', "\\$")
}

fn nginx_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn xml_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// path-to-regexp escaping used by Vercel and Next.js sources.
fn ptr_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if ":()[]{}*+?\\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn csv_line(fields: &[String]) -> String {
    let mut w = csv::WriterBuilder::new().terminator(csv::Terminator::Any(b'\n')).from_writer(Vec::new());
    let _ = w.write_record(fields);
    String::from_utf8(w.into_inner().unwrap_or_default()).unwrap_or_default()
}

fn sorted<'a>(entries: impl Iterator<Item = &'a RuleEntry>) -> Vec<&'a RuleEntry> {
    let mut v: Vec<&RuleEntry> = entries.collect();
    v.sort_by(|a, b| a.source_url.cmp(&b.source_url));
    v
}

// ---------------------------------------------------------------- formats

pub fn export(set: &RuleSet, format: &str, opts: &ExportOptions) -> Result<ExportOutput, String> {
    let filename = FORMATS.iter().find(|f| f.0 == format).map(|f| f.1).ok_or_else(|| format!("Unknown format \"{format}\" (use one of: {})", format_ids().join(", ")))?;
    let mut out = match format {
        "apache" => apache(set, opts),
        "nginx" => nginx(set, opts),
        "iis" => iis(set, opts),
        "cloudflare" => cloudflare(set, opts),
        "netlify" => netlify(set, opts),
        "vercel" => vercel_like(set, opts, false),
        "nextjs" => vercel_like(set, opts, true),
        "wordpress" => wordpress(set, opts),
        _ => plain_csv(set, opts),
    };
    out.format = format.into();
    out.filename = filename.into();
    out.bom = format == "csv";
    Ok(out)
}

fn empty_output() -> ExportOutput {
    ExportOutput { format: String::new(), filename: String::new(), content: String::new(), rules: 0, pattern_rules: 0, skipped: 0, warnings: Vec::new(), bom: false }
}

fn apache(set: &RuleSet, opts: &ExportOptions) -> ExportOutput {
    let lang = opts.lang.as_str();
    let native: Vec<&RuleEntry> = sorted(set.entries.iter().filter(|e| !e.covered));
    let mut rewrite_lines: Vec<String> = Vec::new();
    let mut redirect: Vec<(String, String)> = Vec::new();
    let qsd = |extra: bool| if extra || !opts.preserve_query { ",QSD" } else { "" };
    for e in &native {
        let path = src_path(e);
        let query = src_query(e, opts);
        // `Redirect /` would swallow the whole site (prefix match), so the root uses an exact rule.
        if query.is_none() && !e.overrides_pattern && opts.preserve_query && path != "/" {
            let line = if e.status == 410 {
                format!("Redirect 410 {}", apache_quote(&path))
            } else {
                let t = target(e, set);
                // `Redirect /a/ /b` would turn /a/x into /bx (prefix match + appended rest),
                // so a slash-ending source with a slash-less target gets an exact RedirectMatch.
                if path.ends_with('/') && !t.ends_with('/') {
                    format!("RedirectMatch 301 {} {}", apache_quote(&format!("^{}$", norm::regex_escape(&path))), t)
                } else {
                    format!("Redirect 301 {} {}", apache_quote(&path), t)
                }
            };
            redirect.push((path, line));
            continue;
        }
        if let Some(q) = &query {
            rewrite_lines.push(format!("RewriteCond %{{QUERY_STRING}} ^{}$", norm::regex_escape(q)));
        }
        let re = apache_quote(&exact_regex(&path));
        if e.status == 410 {
            rewrite_lines.push(format!("RewriteRule {re} - [G,L]"));
        } else {
            rewrite_lines.push(format!("RewriteRule {re} {} [R=301,L,NE{}]", apache_subst(&target(e, set)), qsd(query.is_some())));
        }
    }
    let prefix = pattern_prefix(set);
    let mut pattern_lines = Vec::new();
    for p in &set.patterns {
        let re = if let Some(rest) = p.regex.strip_prefix("^/") { format!("^/?{rest}") } else { p.regex.clone() };
        let rep = render_replacement(p, &prefix, |i| format!("${i}"), apache_subst);
        pattern_lines.push(format!("RewriteRule {} {} [R=301,L{}]", apache_quote(&re), rep, qsd(false)));
    }
    redirect.sort_by(|a, b| b.0.chars().count().cmp(&a.0.chars().count()).then(a.0.cmp(&b.0)));
    let gone = native.iter().filter(|e| e.status == 410).count();
    let mut lines: Vec<String> = header_lines(set, lang, native.len(), pattern_lines.len(), gone).into_iter().map(|l| format!("# {l}")).collect();
    if !rewrite_lines.is_empty() || !pattern_lines.is_empty() {
        lines.push(format!("# {}", tr(lang, "rewrite_first")));
        lines.push(String::new());
        lines.push("RewriteEngine On".into());
    }
    if !rewrite_lines.is_empty() {
        lines.push(String::new());
        lines.push(format!("# {}", tr(lang, "before")));
        lines.extend(rewrite_lines);
    }
    if !pattern_lines.is_empty() {
        lines.push(String::new());
        lines.push(format!("# {}", tr(lang, "pattern_block")));
        lines.extend(pattern_lines.iter().cloned());
    }
    if !redirect.is_empty() {
        lines.push(String::new());
        lines.push(format!("# {}", tr(lang, "exact")));
        lines.extend(redirect.into_iter().map(|(_, l)| l));
    }
    let mut o = empty_output();
    o.content = lines.join("\n") + "\n";
    o.rules = native.len();
    o.pattern_rules = set.patterns.len();
    o
}

fn nginx(set: &RuleSet, opts: &ExportOptions) -> ExportOutput {
    let lang = opts.lang.as_str();
    let native: Vec<&RuleEntry> = sorted(set.entries.iter().filter(|e| !e.covered));
    let (mut plain, mut query, mut gone) = (Vec::new(), Vec::new(), Vec::new());
    for e in &native {
        match (e.status == 410, src_query(e, opts)) {
            (true, _) => gone.push(format!("    {} 1;", nginx_str(&src_path(e)))),
            (false, Some(q)) => query.push(format!("    {} {};", nginx_str(&format!("{}?{q}", src_raw_path(e))), nginx_str(&target(e, set)))),
            (false, None) => plain.push(format!("    {} {};", nginx_str(&src_path(e)), nginx_str(&target(e, set)))),
        }
    }
    let mut lines: Vec<String> = header_lines(set, lang, native.len(), set.patterns.len(), gone.len()).into_iter().map(|l| format!("# {l}")).collect();
    lines.push(format!("# {}", tr(lang, "bucket")));
    lines.push(String::new());
    lines.push(format!("# {}", tr(lang, "http")));
    let block = |name: &str, var: &str, default: &str, rows: &[String], lines: &mut Vec<String>| {
        if rows.is_empty() {
            return;
        }
        lines.push(format!("map {name} {var} {{"));
        lines.push(format!("    default {default};"));
        lines.extend(rows.iter().cloned());
        lines.push("}".into());
        lines.push(String::new());
    };
    block("$uri", "$new_uri", "\"\"", &plain, &mut lines);
    block("$request_uri", "$new_uri_q", "\"\"", &query, &mut lines);
    block("$uri", "$gone_uri", "0", &gone, &mut lines);
    lines.push(format!("# {}", tr(lang, "server")));
    if !gone.is_empty() {
        lines.extend(["if ($gone_uri) {".to_string(), "    return 410;".into(), "}".into()]);
    }
    if !query.is_empty() {
        lines.extend(["if ($new_uri_q) {".to_string(), "    return 301 $new_uri_q;".into(), "}".into()]);
    }
    if !plain.is_empty() {
        let ret = if opts.preserve_query { "return 301 $new_uri$is_args$args;" } else { "return 301 $new_uri;" };
        lines.extend(["if ($new_uri) {".to_string(), format!("    {ret}"), "}".into()]);
    }
    if !set.patterns.is_empty() {
        lines.push(String::new());
        lines.push(format!("# {}", tr(lang, "pattern_block")));
        let prefix = pattern_prefix(set);
        for p in &set.patterns {
            let mut rep = render_replacement(p, &prefix, |i| format!("${i}"), |s| s.to_string());
            if !opts.preserve_query {
                rep.push('?');
            }
            lines.push(format!("rewrite \"{}\" {} permanent;", p.regex.replace('"', "\\\""), rep));
        }
    }
    let mut o = empty_output();
    o.content = lines.join("\n") + "\n";
    o.rules = native.len();
    o.pattern_rules = set.patterns.len();
    o
}

fn iis(set: &RuleSet, opts: &ExportOptions) -> ExportOutput {
    let lang = opts.lang.as_str();
    let native: Vec<&RuleEntry> = sorted(set.entries.iter().filter(|e| !e.covered));
    let mut seen = std::collections::HashSet::new();
    let (mut plain, mut query, mut gone) = (Vec::new(), Vec::new(), Vec::new());
    for e in &native {
        let q = src_query(e, opts);
        let value = if e.status == 410 { "410".to_string() } else { target(e, set) };
        let keys: Vec<String> = match &q {
            Some(q) => vec![format!("{}?{q}", src_raw_path(e))],
            None => {
                let (d, r) = (src_path(e), src_raw_path(e));
                if d == r {
                    vec![d]
                } else {
                    vec![d, r]
                }
            }
        };
        for k in keys {
            let bucket = if e.status == 410 { "g" } else if q.is_some() { "q" } else { "p" };
            if !seen.insert((bucket, k.clone())) {
                continue;
            }
            let row = format!("          <add key=\"{}\" value=\"{}\" />", xml_attr(&k), xml_attr(&value));
            match bucket {
                "g" => gone.push(row),
                "q" => query.push(row),
                _ => plain.push(row),
            }
        }
    }
    let head = header_lines(set, lang, native.len(), set.patterns.len(), native.iter().filter(|e| e.status == 410).count()).join(" · ").replace("--", "- -");
    let mut l: Vec<String> = vec!["<?xml version=\"1.0\" encoding=\"UTF-8\"?>".into(), format!("<!-- {head} -->"), format!("<!-- {} -->", tr(lang, "iis_keys"))];
    l.extend(["<configuration>".to_string(), "  <system.webServer>".into(), "    <rewrite>".into()]);
    if !(plain.is_empty() && query.is_empty() && gone.is_empty()) {
        l.push("      <rewriteMaps>".into());
        for (name, rows) in [("SeobifyRedirects", &plain), ("SeobifyRedirectsQuery", &query), ("SeobifyGone", &gone)] {
            if rows.is_empty() {
                continue;
            }
            l.push(format!("        <rewriteMap name=\"{name}\" defaultValue=\"\">"));
            l.extend(rows.iter().cloned());
            l.push("        </rewriteMap>".into());
        }
        l.push("      </rewriteMaps>".into());
    }
    l.push("      <rules>".into());
    let append = if opts.preserve_query { "true" } else { "false" };
    let map_rule = |l: &mut Vec<String>, name: &str, input: &str, action: String| {
        l.push(format!("        <rule name=\"{name}\" stopProcessing=\"true\">"));
        l.push("          <match url=\".*\" />".into());
        l.push("          <conditions>".into());
        l.push(format!("            <add input=\"{input}\" pattern=\"(.+)\" />"));
        l.push("          </conditions>".into());
        l.push(format!("          {action}"));
        l.push("        </rule>".into());
    };
    if !gone.is_empty() {
        map_rule(&mut l, "Seobify 410", "{SeobifyGone:{URL}}", "<action type=\"CustomResponse\" statusCode=\"410\" statusReason=\"Gone\" statusDescription=\"Gone\" />".into());
    }
    if !query.is_empty() {
        map_rule(&mut l, "Seobify redirects (query)", "{SeobifyRedirectsQuery:{UNENCODED_URL}}", "<action type=\"Redirect\" url=\"{C:1}\" appendQueryString=\"false\" redirectType=\"Permanent\" />".into());
    }
    if !plain.is_empty() {
        map_rule(&mut l, "Seobify redirects", "{SeobifyRedirects:{URL}}", format!("<action type=\"Redirect\" url=\"{{C:1}}\" appendQueryString=\"{append}\" redirectType=\"Permanent\" />"));
    }
    let prefix = pattern_prefix(set);
    for p in &set.patterns {
        let re = p.regex.strip_prefix("^/").map(|r| format!("^{r}")).or_else(|| p.regex.strip_prefix('/').map(str::to_string)).unwrap_or_else(|| p.regex.clone());
        let rep = render_replacement(p, &prefix, |i| format!("{{R:{i}}}"), |s| s.to_string());
        l.push(format!("        <rule name=\"Seobify pattern {}\" stopProcessing=\"true\">", p.position));
        l.push(format!("          <match url=\"{}\" />", xml_attr(&re)));
        l.push(format!("          <action type=\"Redirect\" url=\"{}\" appendQueryString=\"{append}\" redirectType=\"Permanent\" />", xml_attr(&rep)));
        l.push("        </rule>".into());
    }
    l.extend(["      </rules>".to_string(), "    </rewrite>".into(), "  </system.webServer>".into(), "</configuration>".into()]);
    let mut o = empty_output();
    o.content = l.join("\n") + "\n";
    o.rules = native.len();
    o.pattern_rules = set.patterns.len();
    o
}

fn cloudflare(set: &RuleSet, opts: &ExportOptions) -> ExportOutput {
    let mut o = empty_output();
    let mut rows = String::new();
    if opts.include_header.unwrap_or(false) {
        rows.push_str("source,target,status,preserve_query_string,include_subdomains,subpath_matching,preserve_path_suffix\n");
    }
    let pq = if opts.preserve_query { "TRUE" } else { "FALSE" };
    for e in sorted(set.entries.iter()) {
        if e.status == 410 {
            o.skipped += 1;
            o.warnings.push(format!("{}: {}", tr(&opts.lang, "skip_gone"), e.source_url));
            continue;
        }
        if src_query(e, opts).is_some() {
            o.skipped += 1;
            o.warnings.push(format!("{}: {}", tr(&opts.lang, "skip_query"), e.source_url));
            continue;
        }
        let host = norm::parts(&e.source_url).host;
        rows.push_str(&csv_line(&[format!("{host}{}", src_raw_path(e)), abs_target(e), "301".into(), pq.into(), "FALSE".into(), "FALSE".into(), "FALSE".into()]));
        o.rules += 1;
    }
    if !set.patterns.is_empty() {
        o.warnings.insert(0, tr(&opts.lang, "expanded").into());
    }
    o.content = rows;
    o
}

fn netlify(set: &RuleSet, opts: &ExportOptions) -> ExportOutput {
    let lang = opts.lang.as_str();
    let mut o = empty_output();
    let mut body = Vec::new();
    let mut skipped = Vec::new();
    for e in sorted(set.entries.iter()) {
        if let Some(q) = src_query(e, opts) {
            skipped.push(format!("# {}: {}?{q}", tr(lang, "skip_query"), src_raw_path(e)));
            o.warnings.push(format!("{}: {}", tr(lang, "skip_query"), e.source_url));
            o.skipped += 1;
            continue;
        }
        if e.status == 410 {
            body.push(format!("{}  /404.html  410", src_raw_path(e)));
        } else {
            body.push(format!("{}  {}  301", src_raw_path(e), target(e, set)));
        }
        o.rules += 1;
    }
    let gone = set.entries.iter().filter(|e| e.status == 410).count();
    let mut lines: Vec<String> = header_lines(set, lang, o.rules, 0, gone).into_iter().map(|l| format!("# {l}")).collect();
    if !set.patterns.is_empty() {
        lines.push(format!("# {}", tr(lang, "expanded")));
        o.warnings.insert(0, tr(lang, "expanded").into());
    }
    if gone > 0 {
        lines.push(format!("# {}", tr(lang, "netlify_410")));
    }
    lines.extend(body);
    lines.extend(skipped);
    o.content = lines.join("\n") + "\n";
    o
}

fn vercel_like(set: &RuleSet, opts: &ExportOptions, next: bool) -> ExportOutput {
    let lang = opts.lang.as_str();
    let mut o = empty_output();
    let mut items: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for e in sorted(set.entries.iter()) {
        if e.status == 410 {
            o.skipped += 1;
            o.warnings.push(format!("{}: {}", tr(lang, "skip_gone"), e.source_url));
            continue;
        }
        let mut path = src_raw_path(e);
        if next && path.len() > 1 {
            path = path.trim_end_matches('/').to_string();
        }
        let has: Vec<String> = src_query(e, opts)
            .map(|q| {
                url::form_urlencoded::parse(q.as_bytes())
                    .map(|(k, v)| {
                        if next {
                            format!("{{ type: \"query\", key: {}, value: {} }}", js_str(&k), js_str(&norm::regex_escape(&v)))
                        } else {
                            format!("{{ \"type\": \"query\", \"key\": {}, \"value\": {} }}", js_str(&k), js_str(&norm::regex_escape(&v)))
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !seen.insert((path.clone(), has.clone())) {
            o.skipped += 1;
            continue;
        }
        let source = js_str(&ptr_escape(&path));
        let dest = js_str(&target(e, set));
        let has_part = if has.is_empty() { String::new() } else if next { format!(", has: [{}]", has.join(", ")) } else { format!(", \"has\": [{}]", has.join(", ")) };
        items.push(if next {
            format!("  {{ source: {source}{has_part}, destination: {dest}, permanent: true }},")
        } else {
            format!("    {{ \"source\": {source}{has_part}, \"destination\": {dest}, \"permanent\": true }}")
        });
        o.rules += 1;
    }
    if !set.patterns.is_empty() {
        o.warnings.insert(0, tr(lang, "expanded").into());
    }
    let gone = 0;
    if next {
        let mut lines: Vec<String> = header_lines(set, lang, o.rules, 0, gone).into_iter().map(|l| format!("// {l}")).collect();
        if !set.patterns.is_empty() {
            lines.push(format!("// {}", tr(lang, "expanded")));
        }
        lines.push(format!("// {}", tr(lang, "next_slash")));
        lines.push("const redirects = [".into());
        lines.extend(items);
        lines.push("];".into());
        lines.push(String::new());
        lines.extend(["module.exports = {".to_string(), "  async redirects() {".into(), "    return redirects;".into(), "  },".into(), "};".into()]);
        o.content = lines.join("\n") + "\n";
    } else {
        let joined = items.join(",\n");
        o.content = if joined.is_empty() { "{\n  \"redirects\": []\n}\n".into() } else { format!("{{\n  \"redirects\": [\n{joined}\n  ]\n}}\n") };
    }
    o
}

fn wordpress(set: &RuleSet, opts: &ExportOptions) -> ExportOutput {
    let mut o = empty_output();
    let mut rows = String::new();
    if opts.include_header.unwrap_or(true) {
        rows.push_str(&csv_line(&["source", "target", "regex", "code", "type", "hits", "title", "status"].map(String::from)));
    }
    for e in sorted(set.entries.iter().filter(|e| !e.covered)) {
        let source = match src_query(e, opts) {
            Some(q) => format!("{}?{q}", src_path(e)),
            None => src_path(e),
        };
        let row = if e.status == 410 {
            [source, String::new(), "0".into(), "410".into(), "error".into(), "0".into(), String::new(), "active".into()]
        } else {
            [source, target(e, set), "0".into(), "301".into(), "url".into(), "0".into(), String::new(), "active".into()]
        };
        rows.push_str(&csv_line(&row));
        o.rules += 1;
    }
    let prefix = pattern_prefix(set);
    for p in &set.patterns {
        let rep = render_replacement(p, &prefix, |i| format!("${i}"), |s| s.to_string());
        rows.push_str(&csv_line(&[p.regex.clone(), rep, "1".into(), "301".into(), "url".into(), "0".into(), format!("Seobify pattern {}", p.position), "active".into()]));
        o.pattern_rules += 1;
    }
    o.content = rows;
    o
}

fn plain_csv(set: &RuleSet, opts: &ExportOptions) -> ExportOutput {
    let mut o = empty_output();
    let mut rows = String::new();
    if opts.include_header.unwrap_or(true) {
        rows.push_str("old,new,status,confidence,reason\n");
    }
    for e in sorted(set.entries.iter()) {
        let new = if e.status == 410 { String::new() } else { abs_target(e) };
        rows.push_str(&csv_line(&[e.source_url.clone(), new, e.status.to_string(), e.confidence.to_string(), e.reason.clone()]));
        o.rules += 1;
    }
    o.content = rows;
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::model::MatchOptions;

    fn entry(id: i64, src: &str, dst: Option<&str>, reason: &str) -> RuleEntry {
        RuleEntry { mapping_id: id, source_url: src.into(), target_url: dst.map(str::to_string), status: if dst.is_some() { 301 } else { 410 }, confidence: 90, reason: reason.into(), covered: false, overrides_pattern: false }
    }

    /// Fixture covering Turkish characters, spaces, `$`, `%`, a query string, 410, a pattern,
    /// a pattern-covered URL, a pattern override and a same-origin target.
    fn fixture() -> RuleSet {
        let mut entries = vec![
            entry(1, "https://eski.com/hakkimizda/", Some("https://yeni.com/kurumsal/hakkimizda"), "same_title"),
            entry(2, "https://eski.com/hizmetler/%C3%A7ay%20bah%C3%A7esi/", Some("https://yeni.com/hizmet/çay-bahçesi"), "same_slug"),
            entry(3, "https://eski.com/index.php?id=5", Some("https://yeni.com/urun/5"), "manual"),
            entry(4, "https://eski.com/eski-kampanya/", None, "manual_gone"),
            entry(5, "https://eski.com/blog/2024/05/kahve/", Some("https://yeni.com/makale/kahve/"), "pattern:1"),
            entry(6, "https://eski.com/blog/2024/05/ozel/", Some("https://yeni.com/ozel-sayfa"), "manual"),
            entry(7, "https://eski.com/fiyat-$-listesi", Some("https://yeni.com/fiyatlar?para=$"), "fuzzy"),
            entry(8, "https://eski.com/", Some("https://yeni.com/"), "same_path"),
        ];
        let patterns = vec![PatternRule { position: 1, regex: r"^/blog/\d{4}/\d{2}/(.+)$".into(), replacement: "/makale/$1".into() }];
        classify(&mut entries, &patterns, "https://yeni.com", &MatchOptions { ignore_query: false, ..Default::default() }).unwrap();
        RuleSet { plan_name: "Test planı".into(), old_base: "https://eski.com".into(), new_base: "https://yeni.com".into(), entries, patterns }
    }

    fn opts() -> ExportOptions {
        ExportOptions { ignore_query: false, ..Default::default() }
    }

    fn golden(format: &str) {
        let out = export(&fixture(), format, &opts()).unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden").join(format!("{format}.txt"));
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &out.content).unwrap();
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing golden file {} (run with UPDATE_GOLDEN=1)", path.display()));
        assert_eq!(out.content, expected.replace("\r\n", "\n"), "golden mismatch for {format}");
    }

    #[test]
    fn classify_marks_covered_and_overrides() {
        let set = fixture();
        let by_id = |id: i64| set.entries.iter().find(|e| e.mapping_id == id).unwrap();
        assert!(by_id(5).covered && !by_id(5).overrides_pattern);
        assert!(by_id(6).overrides_pattern && !by_id(6).covered);
        assert!(!by_id(1).covered && !by_id(1).overrides_pattern);
    }

    #[test]
    fn golden_apache() {
        golden("apache");
    }
    #[test]
    fn golden_nginx() {
        golden("nginx");
    }
    #[test]
    fn golden_iis() {
        golden("iis");
    }
    #[test]
    fn golden_cloudflare() {
        golden("cloudflare");
    }
    #[test]
    fn golden_netlify() {
        golden("netlify");
    }
    #[test]
    fn golden_vercel() {
        golden("vercel");
    }
    #[test]
    fn golden_nextjs() {
        golden("nextjs");
    }
    #[test]
    fn golden_wordpress() {
        golden("wordpress");
    }
    #[test]
    fn golden_csv() {
        golden("csv");
    }

    #[test]
    fn escaping_details() {
        let set = fixture();
        let apache = export(&set, "apache", &opts()).unwrap().content;
        assert!(apache.contains("RedirectMatch 301 \"^/hizmetler/çay bahçesi/$\" https://yeni.com/hizmet/%C3%A7ay-bah%C3%A7esi"), "{apache}");
        assert!(apache.contains("Redirect 301 /fiyat-$-listesi https://yeni.com/fiyatlar?para=%24"));
        assert!(apache.contains("RewriteCond %{QUERY_STRING} ^id=5$"));
        assert!(!apache.contains("/blog/2024/05/kahve"), "pattern-covered URLs are not listed");
        let nginx = export(&set, "nginx", &opts()).unwrap().content;
        assert!(nginx.contains("\"/fiyat-$-listesi\" \"https://yeni.com/fiyatlar?para=%24\";"), "{nginx}");
        assert!(nginx.contains("rewrite \"^/blog/\\d{4}/\\d{2}/(.+)$\" https://yeni.com/makale/$1 permanent;"));
        let vercel = export(&set, "vercel", &opts()).unwrap();
        let json: serde_json::Value = serde_json::from_str(&vercel.content).expect("vercel.json is valid JSON");
        assert_eq!(json["redirects"].as_array().unwrap().len(), vercel.rules);
        assert!(vercel.warnings.iter().any(|w| w.contains("410")));
        let iis = export(&set, "iis", &opts()).unwrap().content;
        assert!(iis.contains("<match url=\"^blog/\\d{4}/\\d{2}/(.+)$\" />"));
        assert!(iis.contains("key=\"/hizmetler/çay bahçesi/\"") && iis.contains("key=\"/hizmetler/%C3%A7ay%20bah%C3%A7esi/\""));
        let cf = export(&set, "cloudflare", &opts()).unwrap();
        assert_eq!(cf.skipped, 2);
        assert!(cf.content.contains("eski.com/blog/2024/05/kahve/,https://yeni.com/makale/kahve/,301,TRUE,FALSE,FALSE,FALSE"));
    }

    #[test]
    fn query_dropping_and_relative_targets() {
        let mut set = fixture();
        set.new_base = "https://eski.com".into();
        set.entries = vec![entry(1, "https://eski.com/a", Some("https://eski.com/b"), "manual")];
        set.patterns.clear();
        let o = ExportOptions { preserve_query: false, ..opts() };
        let apache = export(&set, "apache", &o).unwrap().content;
        assert!(apache.contains("RewriteRule ^/?a/?$ /b [R=301,L,NE,QSD]"), "{apache}");
        let nginx = export(&set, "nginx", &o).unwrap().content;
        assert!(nginx.contains("return 301 $new_uri;") && nginx.contains("\"/a\" \"/b\";"));
        assert!(export(&set, "csv", &o).unwrap().content.contains("https://eski.com/a,https://eski.com/b,301,90,manual"));
    }

    #[test]
    fn unknown_format_is_an_error() {
        assert!(export(&fixture(), "caddy", &opts()).unwrap_err().contains("Unknown format"));
    }
}
