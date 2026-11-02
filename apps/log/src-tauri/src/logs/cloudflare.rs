//! Cloudflare connection for sites served entirely by Cloudflare (no origin server log).
//!
//! Reads traffic from the GraphQL Analytics API (`httpRequestsAdaptiveGroups`: every plan,
//! about 31 days back, sampled estimates). Per day: (1) request counts by user agent, which
//! the local bot catalog classifies; (2) hour × path detail for the crawler user agents only.
//! The rows become one Cloudflare-format NDJSON file per day with an estimated count per row
//! (`SeobifyCount`) and go through the normal import, so every report works unchanged.
//! Crawler rows carry the client IP when the plan exposes it (Free does, checked 2026-10-31),
//! so they are verified like server logs; Cloudflare's own `verifiedBotCategory` confirms
//! crawlers Log cannot check itself. Human traffic is fetched without IPs and only reaches
//! the daily counters. The API token lives in the OS credential store.

use super::bots;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

const API: &str = "https://api.cloudflare.com/client/v4";
/// Credential-store entry holding the user's API token.
pub const SECRET: &str = "cloudflare-api-token";
/// Days fetched on the first sync (the API keeps about 31).
pub const DEFAULT_DAYS: i64 = 30;
/// Crawler user agents per detail query.
const UA_CHUNK: usize = 40;
/// Status classes queried separately when `edgeResponseStatus` is not a usable dimension:
/// (from, to, representative code).
const STATUS_CLASSES: [(u16, u16, u16); 5] = [(100, 200, 100), (200, 300, 200), (300, 400, 301), (400, 500, 404), (500, 600, 500)];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Zone {
    pub id: String,
    pub name: String,
    pub status: String,
    pub plan: Option<String>,
    pub account_id: Option<String>,
}

/// Query limits for `httpRequestsAdaptiveGroups` on this zone (from the `settings` node).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub enabled: bool,
    pub max_page: usize,
    pub not_older_than_secs: i64,
    pub max_duration_secs: i64,
    pub status_dimension: bool,
    /// Cloudflare's own crawler verification (`verifiedBotCategory`) can be grouped by.
    pub verified_dimension: bool,
    pub ip_dimension: bool,
    pub host_dimension: bool,
    pub method_dimension: bool,
    pub available_fields: Vec<String>,
}

/// One aggregated row: requests in an hour (or a day) for a path / user agent / status.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub ts: String,
    pub path: Option<String>,
    pub ua: String,
    pub status: u16,
    pub count: u64,
    pub ip: Option<String>,
    pub host: Option<String>,
    pub method: Option<String>,
    /// `Some(true)` when Cloudflare lists the request under a verified bot category.
    pub verified: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayData {
    pub day: String,
    pub requests: u64,
    pub bot_requests: u64,
    pub bot_user_agents: usize,
    /// A query still hit the page limit at one-hour granularity (rare; counts are lower bounds).
    pub truncated: bool,
    #[serde(skip)]
    pub rows: Vec<Row>,
}

fn client() -> Result<reqwest::Client, String> {
    crate::hub::http::builder().timeout(std::time::Duration::from_secs(60)).build().map_err(|e| e.to_string())
}

/// Turns a Cloudflare API envelope into a value or a readable error.
fn envelope(status: reqwest::StatusCode, body: &str) -> Result<Value, String> {
    let v: Value = serde_json::from_str(body).map_err(|_| format!("Cloudflare {status}: {}", body.chars().take(200).collect::<String>()))?;
    if v.get("success").and_then(Value::as_bool) == Some(true) {
        return Ok(v);
    }
    let errors: Vec<String> = v
        .get("errors")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|e| format!("{} ({})", e.get("message").and_then(Value::as_str).unwrap_or("error"), e.get("code").map(|c| c.to_string()).unwrap_or_default())).collect())
        .unwrap_or_default();
    Err(format!("Cloudflare {status}: {}", if errors.is_empty() { "request failed".into() } else { errors.join("; ") }))
}

async fn api_get(token: &str, path: &str) -> Result<Value, String> {
    let resp = client()?.get(format!("{API}{path}")).bearer_auth(token).send().await.map_err(|e| format!("Cloudflare: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    envelope(status, &body)
}

/// Checks the token; returns `{ id, status }` (status must be `active`).
pub async fn verify(token: &str) -> Result<Value, String> {
    let v = api_get(token, "/user/tokens/verify").await?;
    let result = v.get("result").cloned().unwrap_or(Value::Null);
    match result.get("status").and_then(Value::as_str) {
        Some("active") => Ok(result),
        Some(other) => Err(format!("The Cloudflare token is {other}. Create a new one.")),
        None => Err("Cloudflare did not confirm the token".into()),
    }
}

/// Zones the token can read (needs Zone / Zone / Read).
pub async fn zones(token: &str) -> Result<Vec<Zone>, String> {
    let mut out = Vec::new();
    for page in 1..=20 {
        let v = api_get(token, &format!("/zones?per_page=50&page={page}")).await?;
        let list = v.get("result").and_then(Value::as_array).cloned().unwrap_or_default();
        for z in &list {
            out.push(Zone {
                id: z.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
                name: z.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                status: z.get("status").and_then(Value::as_str).unwrap_or_default().to_string(),
                plan: z.pointer("/plan/name").and_then(Value::as_str).map(str::to_string),
                account_id: z.pointer("/account/id").and_then(Value::as_str).map(str::to_string),
            });
        }
        let pages = v.pointer("/result_info/total_pages").and_then(Value::as_i64).unwrap_or(1);
        if page >= pages || list.is_empty() {
            break;
        }
    }
    Ok(out)
}

/// GraphQL literal for a JSON value (object keys unquoted; JSON string escapes are valid GraphQL).
pub fn gql(v: &Value) -> String {
    match v {
        Value::Object(m) => format!("{{{}}}", m.iter().map(|(k, v)| format!("{k}: {}", gql(v))).collect::<Vec<_>>().join(", ")),
        Value::Array(a) => format!("[{}]", a.iter().map(gql).collect::<Vec<_>>().join(", ")),
        other => other.to_string(),
    }
}

async fn graphql(token: &str, query: &str) -> Result<Value, String> {
    let resp = client()?
        .post(format!("{API}/graphql"))
        .bearer_auth(token)
        .json(&json!({ "query": query }))
        .send()
        .await
        .map_err(|e| format!("Cloudflare GraphQL: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&body).map_err(|_| format!("Cloudflare GraphQL {status}: {}", body.chars().take(200).collect::<String>()))?;
    if let Some(errors) = v.get("errors").and_then(Value::as_array).filter(|a| !a.is_empty()) {
        let msgs: Vec<&str> = errors.iter().filter_map(|e| e.get("message").and_then(Value::as_str)).collect();
        return Err(format!("Cloudflare GraphQL: {}", msgs.join("; ")));
    }
    Ok(v)
}

fn zone_node<'a>(v: &'a Value) -> Option<&'a Value> {
    v.pointer("/data/viewer/zones/0")
}

/// Reads the dataset limits for a zone; `status_dimension` says whether `edgeResponseStatus`
/// can be grouped by (otherwise status classes are queried one by one).
pub async fn limits(token: &str, zone_id: &str) -> Result<Limits, String> {
    let q = format!(
        "{{ viewer {{ zones(filter: {{zoneTag: {}}}) {{ settings {{ httpRequestsAdaptiveGroups {{ enabled availableFields maxPageSize maxDuration notOlderThan }} }} }} }} }}",
        gql(&json!(zone_id))
    );
    let v = graphql(token, &q).await?;
    let s = zone_node(&v).and_then(|z| z.pointer("/settings/httpRequestsAdaptiveGroups")).ok_or("Cloudflare returned no analytics settings for this zone (check Zone / Analytics / Read)")?;
    Ok(parse_limits(s))
}

pub fn parse_limits(s: &Value) -> Limits {
    let fields: Vec<&str> = s.get("availableFields").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let has = |name: &str| fields.iter().any(|f| *f == name || f.ends_with(&format!("_{name}")));
    Limits {
        enabled: s.get("enabled").and_then(Value::as_bool).unwrap_or(false),
        max_page: s.get("maxPageSize").and_then(Value::as_u64).unwrap_or(10_000).clamp(100, 10_000) as usize,
        not_older_than_secs: s.get("notOlderThan").and_then(Value::as_i64).unwrap_or(31 * 86_400),
        max_duration_secs: s.get("maxDuration").and_then(Value::as_i64).unwrap_or(86_400),
        // An empty list means the API did not say: assume the documented all-plan dimension.
        status_dimension: fields.is_empty() || has("edgeResponseStatus"),
        verified_dimension: has("verifiedBotCategory"),
        ip_dimension: has("clientIP"),
        host_dimension: has("clientRequestHTTPHost"),
        method_dimension: has("clientRequestHTTPMethodName"),
        available_fields: fields.iter().map(|f| f.to_string()).collect(),
    }
}

/// Query text for one window.
pub fn groups_query(zone_id: &str, filter: &Value, dims: &[&str], limit: usize) -> String {
    format!(
        "{{ viewer {{ zones(filter: {{zoneTag: {}}}) {{ httpRequestsAdaptiveGroups(filter: {}, limit: {limit}, orderBy: [count_DESC]) {{ count dimensions {{ {} }} }} }} }} }}",
        gql(&json!(zone_id)),
        gql(filter),
        dims.join(" ")
    )
}

/// Parses grouped rows; `fixed_status` replaces the status when it is not a dimension.
pub fn parse_rows(v: &Value, fixed_status: Option<u16>, default_ts: &str) -> Vec<Row> {
    let groups = zone_node(v).and_then(|z| z.get("httpRequestsAdaptiveGroups")).and_then(Value::as_array).cloned().unwrap_or_default();
    groups
        .iter()
        .filter_map(|g| {
            let count = g.get("count").and_then(Value::as_u64).unwrap_or(0);
            if count == 0 {
                return None;
            }
            let d = g.get("dimensions").cloned().unwrap_or(Value::Null);
            let status = fixed_status.or_else(|| d.get("edgeResponseStatus").and_then(Value::as_u64).map(|s| s as u16)).unwrap_or(200);
            Some(Row {
                ts: d.get("datetimeHour").and_then(Value::as_str).unwrap_or(default_ts).to_string(),
                path: d.get("clientRequestPath").and_then(Value::as_str).map(str::to_string),
                ua: d.get("userAgent").and_then(Value::as_str).unwrap_or_default().to_string(),
                status,
                count,
                ip: d.get("clientIP").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string),
                host: d.get("clientRequestHTTPHost").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string),
                method: d.get("clientRequestHTTPMethodName").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string),
                verified: d.get("verifiedBotCategory").and_then(Value::as_str).map(|c| !c.is_empty()),
            })
        })
        .collect()
}

struct Ctx<'a> {
    token: &'a str,
    zone_id: &'a str,
    limits: &'a Limits,
    truncated: bool,
}

fn iso(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0).map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string()).unwrap_or_default()
}

/// Fetches rows for `[start, end)`, halving the window while a page comes back full.
async fn fetch_window(ctx: &mut Ctx<'_>, start: i64, end: i64, extra: &Map<String, Value>, dims: &[&str]) -> Result<Vec<Row>, String> {
    let mut out = Vec::new();
    let mut stack = vec![(start, end)];
    while let Some((s, e)) = stack.pop() {
        if e - s > ctx.limits.max_duration_secs.max(3600) {
            let mid = s + (e - s) / 2;
            stack.push((mid, e));
            stack.push((s, mid));
            continue;
        }
        let statuses: Vec<(Option<(u16, u16)>, Option<u16>)> =
            if ctx.limits.status_dimension { vec![(None, None)] } else { STATUS_CLASSES.iter().map(|(a, b, c)| (Some((*a, *b)), Some(*c))).collect() };
        let mut dims_q: Vec<&str> = dims.to_vec();
        if ctx.limits.status_dimension {
            dims_q.push("edgeResponseStatus");
        }
        let mut full = false;
        let mut window_rows = Vec::new();
        for (range, fixed) in &statuses {
            let mut f = Map::new();
            f.insert("datetime_geq".into(), json!(iso(s)));
            f.insert("datetime_lt".into(), json!(iso(e)));
            f.insert("requestSource".into(), json!("eyeball"));
            if let Some((a, b)) = range {
                f.insert("edgeResponseStatus_geq".into(), json!(a));
                f.insert("edgeResponseStatus_lt".into(), json!(b));
            }
            for (k, v) in extra {
                f.insert(k.clone(), v.clone());
            }
            let v = graphql(ctx.token, &groups_query(ctx.zone_id, &Value::Object(f), &dims_q, ctx.limits.max_page)).await?;
            let rows = parse_rows(&v, *fixed, &iso(s));
            full |= rows.len() >= ctx.limits.max_page;
            window_rows.extend(rows);
        }
        if full && e - s > 3600 {
            let mid = s + ((e - s) / 2 / 3600).max(1) * 3600;
            stack.push((mid, e));
            stack.push((s, mid));
        } else {
            ctx.truncated |= full;
            out.extend(window_rows);
        }
    }
    Ok(out)
}

/// Day start (UTC) for `YYYY-MM-DD`.
pub fn day_start(day: &str) -> Result<i64, String> {
    chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp()).map_err(|_| format!("Bad day: {day}"))
}

/// Dimensions of the crawler detail query (optional ones only when the plan exposes them).
pub fn detail_dims(limits: &Limits) -> Vec<&'static str> {
    let mut dims = vec!["datetimeHour", "clientRequestPath", "userAgent"];
    for (on, dim) in [(limits.ip_dimension, "clientIP"), (limits.host_dimension, "clientRequestHTTPHost"), (limits.method_dimension, "clientRequestHTTPMethodName"), (limits.verified_dimension, "verifiedBotCategory")] {
        if on {
            dims.push(dim);
        }
    }
    dims
}

/// Fetches one UTC day: user-agent totals, then hour × path detail for crawler user agents.
pub async fn fetch_day(token: &str, zone_id: &str, limits: &Limits, day: &str) -> Result<DayData, String> {
    let start = day_start(day)?;
    let end = start + 86_400;
    let mut ctx = Ctx { token, zone_id, limits, truncated: false };
    let by_ua = fetch_window(&mut ctx, start, end, &Map::new(), &["userAgent"]).await?;
    let mut bot_uas: Vec<String> = by_ua.iter().filter(|r| bots::classify(&r.ua).is_some()).map(|r| r.ua.clone()).collect();
    bot_uas.sort();
    bot_uas.dedup();
    let mut data = DayData { day: day.to_string(), bot_user_agents: bot_uas.len(), ..Default::default() };
    for r in &by_ua {
        data.requests += r.count;
        if bots::classify(&r.ua).is_none() {
            // Human / unknown traffic: kept only as daily counters by the import.
            data.rows.push(Row { ts: iso(start), path: None, ip: None, verified: None, ..r.clone() });
        }
    }
    for chunk in bot_uas.chunks(UA_CHUNK) {
        let mut extra = Map::new();
        extra.insert("userAgent_in".into(), json!(chunk));
        let rows = fetch_window(&mut ctx, start, end, &extra, &detail_dims(limits)).await?;
        data.bot_requests += rows.iter().map(|r| r.count).sum::<u64>();
        data.rows.extend(rows);
    }
    data.truncated = ctx.truncated;
    Ok(data)
}

/// Writes a day's rows as gzip NDJSON in the Cloudflare Logpush field names.
pub fn write_day_file(data: &DayData, host: &str, path: &Path) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut gz = flate2::write::GzEncoder::new(std::io::BufWriter::new(file), flate2::Compression::fast());
    for r in &data.rows {
        let mut line = json!({
            "EdgeStartTimestamp": r.ts,
            "ClientIP": r.ip.as_deref().unwrap_or(""),
            "ClientRequestHost": r.host.as_deref().unwrap_or(host),
            "ClientRequestMethod": r.method.as_deref().unwrap_or("GET"),
            "ClientRequestURI": r.path.as_deref().unwrap_or("/"),
            "ClientRequestUserAgent": r.ua,
            "EdgeResponseStatus": r.status,
            "SeobifyCount": r.count,
        });
        if r.verified == Some(true) {
            line["SeobifyEdgeVerified"] = json!(true);
        }
        writeln!(gz, "{line}").map_err(|e| e.to_string())?;
    }
    gz.finish().map_err(|e| e.to_string())?.flush().map_err(|e| e.to_string())
}

/// Folder for the temporary day files (removed after import).
pub fn work_dir() -> PathBuf {
    let dir = crate::hub::paths::data_dir().join("cloudflare");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Days to fetch: missing days in the window plus the two newest (analytics settle late).
pub fn days_to_fetch(today: chrono::NaiveDate, days_back: i64, not_older_than_secs: i64, have: &BTreeMap<String, bool>) -> Vec<String> {
    let yesterday = today - chrono::Duration::days(1);
    // Keep a day of margin inside the API's look-back so the oldest window is never refused.
    let oldest_allowed = today - chrono::Duration::days((not_older_than_secs / 86_400 - 1).max(1));
    let first = (yesterday - chrono::Duration::days(days_back.clamp(1, 31) - 1)).max(oldest_allowed);
    let mut out = Vec::new();
    let mut d = first;
    while d <= yesterday {
        let key = d.format("%Y-%m-%d").to_string();
        let recent = (yesterday - d).num_days() < 2;
        if recent || !have.get(&key).copied().unwrap_or(false) {
            out.push(key);
        }
        d += chrono::Duration::days(1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphql_literals_escape_strings() {
        let f = json!({ "datetime_geq": "2026-10-30T00:00:00Z", "userAgent_in": ["Mozilla/5.0 \"x\" \\ y"], "edgeResponseStatus_geq": 200 });
        let s = gql(&f);
        assert!(s.contains("datetime_geq: \"2026-10-30T00:00:00Z\""));
        assert!(s.contains("userAgent_in: [\"Mozilla/5.0 \\\"x\\\" \\\\ y\"]"));
        assert!(s.contains("edgeResponseStatus_geq: 200"));
        let q = groups_query("abc", &f, &["userAgent", "edgeResponseStatus"], 10000);
        assert!(q.contains("zones(filter: {zoneTag: \"abc\"})") && q.contains("limit: 10000") && q.contains("dimensions { userAgent edgeResponseStatus }"));
    }

    #[test]
    fn parses_limits_and_rows() {
        let l = parse_limits(&json!({ "enabled": true, "availableFields": ["count", "dimensions_userAgent", "dimensions_edgeResponseStatus"], "maxPageSize": 10000, "maxDuration": 2592000, "notOlderThan": 2678400 }));
        assert!(l.enabled && l.status_dimension && l.max_page == 10000 && !l.ip_dimension);
        let free = parse_limits(&json!({ "enabled": true, "availableFields": ["count", "dimensions_clientIP", "dimensions_verifiedBotCategory", "dimensions_clientRequestHTTPHost", "dimensions_edgeResponseStatus"] }));
        assert!(free.ip_dimension && free.verified_dimension && free.host_dimension && !free.method_dimension);
        assert_eq!(detail_dims(&free), vec!["datetimeHour", "clientRequestPath", "userAgent", "clientIP", "clientRequestHTTPHost", "verifiedBotCategory"]);
        assert!(!parse_limits(&json!({ "enabled": true, "availableFields": ["count", "dimensions_userAgent"] })).status_dimension);
        let v = json!({ "data": { "viewer": { "zones": [{ "httpRequestsAdaptiveGroups": [
            { "count": 12, "dimensions": { "datetimeHour": "2026-10-30T05:00:00Z", "clientRequestPath": "/blog/", "userAgent": "Googlebot/2.1", "edgeResponseStatus": 200, "clientIP": "66.249.64.5", "verifiedBotCategory": "Search Engine Crawler" } },
            { "count": 0, "dimensions": { "userAgent": "x" } },
            { "count": 3, "dimensions": { "userAgent": "Mozilla/5.0" } }
        ] }] } } });
        let rows = parse_rows(&v, None, "2026-10-30T00:00:00Z");
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].count, rows[0].status, rows[0].path.as_deref(), rows[0].ip.as_deref(), rows[0].verified), (12, 200, Some("/blog/"), Some("66.249.64.5"), Some(true)));
        assert_eq!((rows[1].ip.as_deref(), rows[1].verified), (None, None));
        assert_eq!((rows[1].ts.as_str(), rows[1].status), ("2026-10-30T00:00:00Z", 200));
        assert_eq!(parse_rows(&v, Some(404), "t")[0].status, 404);
    }

    #[test]
    fn picks_days() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        let all = days_to_fetch(today, 30, 31 * 86_400, &BTreeMap::new());
        assert_eq!((all.len(), all.first().map(String::as_str), all.last().map(String::as_str)), (30, Some("2026-10-01"), Some("2026-10-30")));
        let have: BTreeMap<String, bool> = all.iter().map(|d| (d.clone(), true)).collect();
        assert_eq!(days_to_fetch(today, 30, 31 * 86_400, &have), vec!["2026-10-29", "2026-10-30"], "only the two newest are refreshed");
        assert_eq!(days_to_fetch(today, 30, 8 * 86_400, &BTreeMap::new()).first().map(String::as_str), Some("2026-10-24"), "API look-back wins");
    }

    #[test]
    fn day_file_round_trips_through_the_cloudflare_parser() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.ndjson.gz");
        let data = DayData {
            day: "2026-10-30".into(),
            rows: vec![
                Row { ts: "2026-10-30T05:00:00Z".into(), path: Some("/blog/".into()), ua: crate::logs::import::tests::GOOGLE_M.into(), status: 200, count: 3, ip: None, host: None, method: None, verified: None },
                Row { ts: "2026-10-30T06:00:00Z".into(), path: Some("/".into()), ua: crate::logs::import::tests::GOOGLE_M.into(), status: 200, count: 2, ip: Some("66.249.64.9".into()), host: Some("www.seobify.com".into()), method: None, verified: None },
                Row { ts: "2026-10-30T07:00:00Z".into(), path: Some("/.env".into()), ua: crate::logs::import::tests::GOOGLE_M.into(), status: 404, count: 4, ip: Some("203.0.113.8".into()), host: None, method: None, verified: Some(false) },
                Row { ts: "2026-10-30T08:00:00Z".into(), path: Some("/ai/".into()), ua: "Mozilla/5.0 (compatible; ClaudeBot/1.0)".into(), status: 200, count: 1, ip: Some("198.51.100.4".into()), host: None, method: None, verified: Some(true) },
                Row { ts: "2026-10-30T00:00:00Z".into(), path: None, ua: crate::logs::import::tests::CHROME.into(), status: 404, count: 5, ip: None, host: None, method: None, verified: None },
            ],
            ..Default::default()
        };
        write_day_file(&data, "seobify.com", &path).unwrap();
        let (mut conn, pid, _d) = crate::logs::import::tests::setup();
        let opts = crate::logs::import::ImportOptions { format: Some(crate::logs::parse::Format::Cloudflare), ..Default::default() };
        let s = crate::logs::import::import_paths(&mut conn, pid, &[path.display().to_string()], &opts, &mut crate::logs::import::tests::verifier(), &mut |_| {}).unwrap();
        assert_eq!((s.imported, s.bot_hits, s.human_requests), (1, 10, 5), "SeobifyCount expands into requests");
        let v = |path: &str| -> (i64, String, String) { conn.query_row("SELECT COUNT(*), MIN(verification), MIN(host) FROM bot_hits WHERE path = ?1", [path], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap() };
        assert_eq!(v("/blog/"), (3, "unverifiable".into(), "seobify.com".into()), "no IP: unverifiable");
        assert_eq!(v("/"), (2, "verified".into(), "www.seobify.com".into()), "Google IP range");
        assert_eq!(v("/.env").1, "fake", "Googlebot user agent from a foreign IP");
        assert_eq!(v("/ai/").1, "verified", "Cloudflare-verified crawler without a published range");
        let human: i64 = conn.query_row("SELECT SUM(requests) FROM human_daily WHERE status_class = '4xx'", [], |r| r.get(0)).unwrap();
        assert_eq!(human, 5);
    }
}
