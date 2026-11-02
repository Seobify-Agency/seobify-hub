//! Analysis queries shared by the Tauri commands, the MCP tools and the HTML report.
//! Every function takes a project id and an inclusive day range (`YYYY-MM-DD`).

use super::bots::{self, Category};
use super::parse;
use rusqlite::{params, Connection, Params, Row};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, PartialEq)]
pub struct Range {
    pub from: String,
    pub to: String,
}

impl Range {
    pub fn all() -> Range {
        Range { from: "0000-01-01".into(), to: "9999-12-31".into() }
    }

    pub fn new(from: Option<&str>, to: Option<&str>) -> Result<Range, String> {
        let check = |s: &str| -> Result<String, String> {
            let s = s.trim();
            if parse::parse_day(s).is_none() {
                return Err(format!("Invalid date \"{s}\": use YYYY-MM-DD"));
            }
            Ok(s[..10].to_string())
        };
        let from = match from.map(str::trim).filter(|s| !s.is_empty()) {
            Some(f) => check(f)?,
            None => "0000-01-01".into(),
        };
        let to = match to.map(str::trim).filter(|s| !s.is_empty()) {
            Some(t) => check(t)?,
            None => "9999-12-31".into(),
        };
        if from > to {
            return Err(format!("\"from\" ({from}) is after \"to\" ({to})"));
        }
        Ok(Range { from, to })
    }
}

fn rows<T, P: Params, F: FnMut(&Row) -> rusqlite::Result<T>>(conn: &Connection, sql: &str, p: P, f: F) -> Result<Vec<T>, String> {
    let mut stmt = conn.prepare(sql).map_err(|e| format!("{e} — {sql}"))?;
    let out = stmt.query_map(p, f).map_err(|e| e.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string());
    out
}

fn one<T, P: Params, F: FnOnce(&Row) -> rusqlite::Result<T>>(conn: &Connection, sql: &str, p: P, f: F) -> Result<T, String> {
    conn.query_row(sql, p, f).map_err(|e| format!("{e} — {sql}"))
}

/// Every day between two `YYYY-MM-DD` strings (inclusive, capped at ~10 years).
pub fn day_span(from: &str, to: &str) -> Vec<String> {
    match (parse::parse_day(from), parse::parse_day(to)) {
        (Some(a), Some(b)) if a <= b => (a..=b.min(a + 3660)).map(|d| parse::day_string(d * 86_400)).collect(),
        _ => Vec::new(),
    }
}

/// First/last day with data and row counts.
pub fn bounds(conn: &Connection, pid: i64) -> Result<Value, String> {
    let (bmin, bmax, hits): (Option<String>, Option<String>, i64) = one(conn, "SELECT MIN(day), MAX(day), COUNT(*) FROM bot_hits WHERE project_id = ?1", [pid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let (hmin, hmax, human): (Option<String>, Option<String>, i64) =
        one(conn, "SELECT MIN(day), MAX(day), COALESCE(SUM(requests), 0) FROM human_daily WHERE project_id = ?1", [pid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let files: i64 = one(conn, "SELECT COUNT(*) FROM log_files WHERE project_id = ?1", [pid], |r| r.get(0))?;
    let min = [bmin, hmin].into_iter().flatten().min();
    let max = [bmax, hmax].into_iter().flatten().max();
    Ok(json!({ "minDay": min, "maxDay": max, "botHits": hits, "humanRequests": human, "files": files }))
}

/// The range clipped to the days that have data (`None` when the project is empty).
pub fn effective(conn: &Connection, pid: i64, range: &Range) -> Result<Option<Range>, String> {
    let b = bounds(conn, pid)?;
    let (Some(min), Some(max)) = (b["minDay"].as_str(), b["maxDay"].as_str()) else { return Ok(None) };
    let from = range.from.as_str().max(min).to_string();
    let to = range.to.as_str().min(max).to_string();
    Ok((from <= to).then_some(Range { from, to }))
}

fn gb() -> String {
    bots::googlebot_sql_list()
}

// ---------------------------------------------------------------- overview

pub fn overview(conn: &Connection, pid: i64, range: &Range) -> Result<Value, String> {
    let g = gb();
    let t = one(
        conn,
        &format!(
            "SELECT COUNT(*), COALESCE(SUM(verification = 'verified'), 0), COALESCE(SUM(verification = 'fake'), 0), COALESCE(SUM(verification = 'unverifiable'), 0),
                COALESCE(SUM(status BETWEEN 200 AND 299), 0), COALESCE(SUM(status BETWEEN 300 AND 399), 0), COALESCE(SUM(status BETWEEN 400 AND 499), 0), COALESCE(SUM(status >= 500), 0),
                COALESCE(SUM(bot_id IN ({g}) AND verification != 'fake'), 0), COALESCE(SUM(bot_id = 'googlebot-smartphone' AND verification != 'fake'), 0),
                COALESCE(SUM(bot_id = 'googlebot-desktop' AND verification != 'fake'), 0), COALESCE(SUM(bot_id IN ({g}) AND verification = 'fake'), 0)
             FROM bot_hits WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3"
        ),
        params![pid, range.from, range.to],
        |r| (0..12).map(|i| r.get::<_, i64>(i)).collect::<rusqlite::Result<Vec<i64>>>(),
    )?;
    let human: i64 = one(conn, "SELECT COALESCE(SUM(requests), 0) FROM human_daily WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3", params![pid, range.from, range.to], |r| r.get(0))?;
    let unique_urls: i64 = one(conn, "SELECT COUNT(DISTINCT url) FROM url_daily WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3", params![pid, range.from, range.to], |r| r.get(0))?;
    let human_status = rows(
        conn,
        "SELECT status_class, SUM(requests) FROM human_daily WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 GROUP BY status_class ORDER BY status_class",
        params![pid, range.from, range.to],
        |r| Ok(json!({ "class": r.get::<_, String>(0)?, "requests": r.get::<_, i64>(1)? })),
    )?;
    let human_devices = rows(
        conn,
        "SELECT device, SUM(requests) FROM human_daily WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 GROUP BY device ORDER BY 2 DESC",
        params![pid, range.from, range.to],
        |r| Ok(json!({ "device": r.get::<_, String>(0)?, "requests": r.get::<_, i64>(1)? })),
    )?;
    let by_category = rows(
        conn,
        "SELECT category, COUNT(*) FROM bot_hits WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 AND verification != 'fake' GROUP BY category ORDER BY 2 DESC",
        params![pid, range.from, range.to],
        |r| Ok(json!({ "category": r.get::<_, String>(0)?, "requests": r.get::<_, i64>(1)? })),
    )?;
    let daily = daily_by_category(conn, pid, range)?;
    let top_urls = rows(
        conn,
        &format!(
            "SELECT url, SUM(hits) h, SUM(CASE WHEN bot_id IN ({g}) THEN hits ELSE 0 END), SUM(s4xx) + SUM(s5xx) FROM url_daily
             WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 GROUP BY url ORDER BY h DESC LIMIT 10"
        ),
        params![pid, range.from, range.to],
        |r| Ok(json!({ "url": r.get::<_, String>(0)?, "hits": r.get::<_, i64>(1)?, "googlebotHits": r.get::<_, i64>(2)?, "errors": r.get::<_, i64>(3)? })),
    )?;
    let total = t[0];
    let ratio = |n: i64| if total > 0 { n as f64 / total as f64 } else { 0.0 };
    let gm = t[9] + t[10];
    let shown = effective(conn, pid, range)?.unwrap_or_else(|| range.clone());
    Ok(json!({
        "range": { "from": shown.from, "to": shown.to },
        "totalBotRequests": total,
        "humanRequests": human,
        "uniqueUrls": unique_urls,
        "verification": { "verified": t[1], "fake": t[2], "unverifiable": t[3], "verifiedShare": ratio(t[1]), "fakeShare": ratio(t[2]) },
        "status": { "s2xx": t[4], "s3xx": t[5], "s4xx": t[6], "s5xx": t[7] },
        "googlebot": { "requests": t[8], "smartphone": t[9], "desktop": t[10], "fakeClaims": t[11], "smartphoneShare": if gm > 0 { t[9] as f64 / gm as f64 } else { 0.0 } },
        "byCategory": by_category,
        "humanStatus": human_status,
        "humanDevices": human_devices,
        "daily": daily,
        "topUrls": top_urls,
    }))
}

/// `{ days: [...], series: { search: [...], ai_training: [...], …, fake: [...], human: [...] } }`
fn daily_by_category(conn: &Connection, pid: i64, range: &Range) -> Result<Value, String> {
    let Some(eff) = effective(conn, pid, range)? else { return Ok(json!({ "days": [], "series": {} })) };
    let days = day_span(&eff.from, &eff.to);
    let index: HashMap<&str, usize> = days.iter().enumerate().map(|(i, d)| (d.as_str(), i)).collect();
    let mut series: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    for cat in Category::BOTS {
        series.insert(cat.as_str().into(), vec![0; days.len()]);
    }
    series.insert("fake".into(), vec![0; days.len()]);
    series.insert("human".into(), vec![0; days.len()]);
    let data = rows(
        conn,
        "SELECT day, CASE WHEN verification = 'fake' THEN 'fake' ELSE category END, COUNT(*) FROM bot_hits WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 GROUP BY 1, 2",
        params![pid, eff.from, eff.to],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
    )?;
    let human = rows(
        conn,
        "SELECT day, 'human', SUM(requests) FROM human_daily WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 GROUP BY day",
        params![pid, eff.from, eff.to],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
    )?;
    for (day, key, n) in data.into_iter().chain(human) {
        if let (Some(&i), Some(s)) = (index.get(day.as_str()), series.get_mut(&key)) {
            s[i] = n;
        }
    }
    Ok(json!({ "days": days, "series": series }))
}

// ---------------------------------------------------------------- bots

pub fn bot_stats(conn: &Connection, pid: i64, range: &Range, category: Option<&str>) -> Result<Value, String> {
    let cat_filter = match category.map(str::trim).filter(|c| !c.is_empty() && *c != "all") {
        Some(c) => {
            Category::from_name(c).ok_or_else(|| format!("Unknown category \"{c}\": use search, ai_training, ai_assistant, seo_tool, social or other_bot"))?;
            format!(" AND category = '{c}'")
        }
        None => String::new(),
    };
    let list = rows(
        conn,
        &format!(
            "SELECT bot_id, COUNT(*), COUNT(DISTINCT path || IFNULL('?' || query, '')),
                SUM(status BETWEEN 200 AND 299), SUM(status BETWEEN 300 AND 399), SUM(status BETWEEN 400 AND 499), SUM(status >= 500),
                AVG(response_ms), MIN(ts), MAX(ts), SUM(verification = 'verified'), SUM(verification = 'fake'), SUM(verification = 'unverifiable'), SUM(bytes)
             FROM bot_hits WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3{cat_filter} GROUP BY bot_id ORDER BY 2 DESC"
        ),
        params![pid, range.from, range.to],
        |r| {
            let id: String = r.get(0)?;
            let def = bots::find(&id);
            Ok(json!({
                "botId": id,
                "name": def.map(|d| d.name).unwrap_or("?"),
                "category": def.map(|d| d.category.as_str()).unwrap_or("other_bot"),
                "owner": def.map(|d| d.owner).unwrap_or("—"),
                "verifiable": def.map(|d| d.family.verifiable()).unwrap_or(false),
                "requests": r.get::<_, i64>(1)?,
                "uniqueUrls": r.get::<_, i64>(2)?,
                "s2xx": r.get::<_, i64>(3)?, "s3xx": r.get::<_, i64>(4)?, "s4xx": r.get::<_, i64>(5)?, "s5xx": r.get::<_, i64>(6)?,
                "avgResponseMs": r.get::<_, Option<f64>>(7)?,
                "firstSeen": r.get::<_, Option<String>>(8)?,
                "lastSeen": r.get::<_, Option<String>>(9)?,
                "verified": r.get::<_, i64>(10)?, "fake": r.get::<_, i64>(11)?, "unverifiable": r.get::<_, i64>(12)?,
                "bytes": r.get::<_, Option<i64>>(13)?,
            }))
        },
    )?;
    Ok(json!({ "range": { "from": range.from, "to": range.to }, "total": list.len(), "bots": list }))
}

pub fn bot_detail(conn: &Connection, pid: i64, bot_id: &str, range: &Range) -> Result<Value, String> {
    let eff = effective(conn, pid, range)?.unwrap_or_else(|| range.clone());
    let days = day_span(&eff.from, &eff.to);
    let index: HashMap<&str, usize> = days.iter().enumerate().map(|(i, d)| (d.as_str(), i)).collect();
    let mut ok = vec![0i64; days.len()];
    let mut fake = vec![0i64; days.len()];
    let mut errors = vec![0i64; days.len()];
    for (day, verified, f, e) in rows(
        conn,
        "SELECT day, SUM(verification != 'fake'), SUM(verification = 'fake'), SUM(status >= 400 AND verification != 'fake') FROM bot_hits
         WHERE project_id = ?1 AND bot_id = ?2 AND day BETWEEN ?3 AND ?4 GROUP BY day",
        params![pid, bot_id, eff.from, eff.to],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)),
    )? {
        if let Some(&i) = index.get(day.as_str()) {
            ok[i] = verified;
            fake[i] = f;
            errors[i] = e;
        }
    }
    let urls = rows(
        conn,
        "SELECT path || IFNULL('?' || query, '') u, COUNT(*), SUM(status BETWEEN 200 AND 299), SUM(status BETWEEN 300 AND 399), SUM(status >= 400), MAX(ts), SUM(verification = 'fake')
         FROM bot_hits WHERE project_id = ?1 AND bot_id = ?2 AND day BETWEEN ?3 AND ?4 GROUP BY u ORDER BY 2 DESC LIMIT 200",
        params![pid, bot_id, eff.from, eff.to],
        |r| Ok(json!({ "url": r.get::<_, String>(0)?, "hits": r.get::<_, i64>(1)?, "s2xx": r.get::<_, i64>(2)?, "s3xx": r.get::<_, i64>(3)?, "errors": r.get::<_, i64>(4)?, "lastSeen": r.get::<_, String>(5)?, "fake": r.get::<_, i64>(6)? })),
    )?;
    let def = bots::find(bot_id);
    Ok(json!({ "botId": bot_id, "name": def.map(|d| d.name), "days": days, "requests": ok, "fake": fake, "errors": errors, "urls": urls }))
}

// ---------------------------------------------------------------- URLs

#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlFilter {
    /// Bot id, `googlebot` (all Googlebot crawlers) or a category name.
    pub bot: Option<String>,
    /// `2xx` | `3xx` | `4xx` | `5xx` | `error` (4xx or 5xx)
    pub status: Option<String>,
    /// First path segment, e.g. `/blog/`.
    pub dir: Option<String>,
    /// `with` | `without` query parameters.
    pub params: Option<String>,
    pub search: Option<String>,
    pub limit: Option<i64>,
}

fn bot_clause(bot: Option<&str>) -> Result<String, String> {
    Ok(match bot.map(str::trim).filter(|b| !b.is_empty() && *b != "all") {
        None => String::new(),
        Some("googlebot") => format!(" AND bot_id IN ({})", gb()),
        Some(b) => {
            if let Some(cat) = Category::from_name(b) {
                format!(" AND bot_id IN ({})", bots::category_sql_list(cat))
            } else if bots::find(b).is_some() {
                format!(" AND bot_id = '{b}'")
            } else {
                return Err(format!("Unknown bot \"{b}\": use a bot id from bot_stats, googlebot or a category"));
            }
        }
    })
}

pub fn urls(conn: &Connection, pid: i64, range: &Range, f: &UrlFilter) -> Result<Value, String> {
    let g = gb();
    let mut filter = bot_clause(f.bot.as_deref())?;
    let mut args: Vec<String> = vec![range.from.clone(), range.to.clone()];
    if let Some(dir) = f.dir.as_deref().map(str::trim).filter(|d| !d.is_empty() && *d != "all") {
        args.push(format!("{}%", dir.replace('%', "")));
        filter.push_str(&format!(" AND url LIKE ?{}", args.len() + 1));
    }
    match f.params.as_deref() {
        Some("with") => filter.push_str(" AND instr(url, '?') > 0"),
        Some("without") => filter.push_str(" AND instr(url, '?') = 0"),
        _ => {}
    }
    if let Some(q) = f.search.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        args.push(format!("%{}%", q.replace('%', "")));
        filter.push_str(&format!(" AND url LIKE ?{}", args.len() + 1));
    }
    let having = match f.status.as_deref() {
        Some("2xx") => " HAVING SUM(s2xx) > 0",
        Some("3xx") => " HAVING SUM(s3xx) > 0",
        Some("4xx") => " HAVING SUM(s4xx) > 0",
        Some("5xx") => " HAVING SUM(s5xx) > 0",
        Some("error") => " HAVING SUM(s4xx) + SUM(s5xx) > 0",
        _ => "",
    };
    let limit = f.limit.unwrap_or(500).clamp(1, 20_000);
    let eff = effective(conn, pid, range)?;
    let span_days = eff.as_ref().map(|e| day_span(&e.from, &e.to).len()).unwrap_or(1).max(1) as f64;
    let base = format!(
        "SELECT url, SUM(hits) h, SUM(CASE WHEN bot_id IN ({g}) THEN hits ELSE 0 END), MAX(CASE WHEN bot_id IN ({g}) THEN last_ts END), MIN(day), MAX(day),
            SUM(s2xx), SUM(s3xx), SUM(s4xx), SUM(s5xx), COUNT(DISTINCT bot_id)
         FROM url_daily WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3{filter} GROUP BY url{having}"
    );
    let mut sql_params: Vec<&dyn rusqlite::ToSql> = vec![&pid];
    for a in &args {
        sql_params.push(a);
    }
    let total: i64 = one(conn, &format!("SELECT COUNT(*) FROM ({base})"), sql_params.as_slice(), |r| r.get(0))?;
    let list = rows(conn, &format!("{base} ORDER BY h DESC LIMIT {limit}"), sql_params.as_slice(), |r| {
        let hits: i64 = r.get(1)?;
        Ok(json!({
            "url": r.get::<_, String>(0)?,
            "hits": hits,
            "googlebotHits": r.get::<_, i64>(2)?,
            "lastGooglebot": r.get::<_, Option<String>>(3)?,
            "firstSeen": r.get::<_, String>(4)?,
            "lastSeen": r.get::<_, String>(5)?,
            "s2xx": r.get::<_, i64>(6)?, "s3xx": r.get::<_, i64>(7)?, "s4xx": r.get::<_, i64>(8)?, "s5xx": r.get::<_, i64>(9)?,
            "bots": r.get::<_, i64>(10)?,
            "perDay": hits as f64 / span_days,
        }))
    })?;
    let dirs = rows(
        conn,
        "SELECT CASE WHEN instr(substr(url, 2), '/') > 0 THEN substr(url, 1, instr(substr(url, 2), '/') + 1) ELSE '/' END d, SUM(hits)
         FROM url_daily WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 GROUP BY d ORDER BY 2 DESC LIMIT 60",
        params![pid, range.from, range.to],
        |r| Ok(json!({ "dir": r.get::<_, String>(0)?, "hits": r.get::<_, i64>(1)? })),
    )?;
    Ok(json!({ "range": { "from": range.from, "to": range.to }, "total": total, "limit": limit, "urls": list, "dirs": dirs }))
}

// ---------------------------------------------------------------- status and anomalies

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Spike {
    pub day: String,
    pub count: i64,
    pub mean: f64,
    pub std_dev: f64,
    pub threshold: f64,
}

/// Days whose count exceeds the trailing-`window` mean by more than 3σ (needs ≥ 7 prior
/// days and at least 5 events, so a single error on a quiet site is not a "spike").
pub fn detect_spikes(series: &[(String, i64)], window: usize) -> Vec<Spike> {
    let mut out = Vec::new();
    for i in 0..series.len() {
        let start = i.saturating_sub(window);
        let prior = &series[start..i];
        if prior.len() < 7 {
            continue;
        }
        let n = prior.len() as f64;
        let mean = prior.iter().map(|p| p.1 as f64).sum::<f64>() / n;
        let var = prior.iter().map(|p| (p.1 as f64 - mean).powi(2)).sum::<f64>() / n;
        let sd = var.sqrt();
        let threshold = mean + 3.0 * sd;
        let count = series[i].1;
        if count >= 5 && count as f64 > threshold {
            out.push(Spike { day: series[i].0.clone(), count, mean, std_dev: sd, threshold });
        }
    }
    out
}

pub fn status_report(conn: &Connection, pid: i64, range: &Range) -> Result<Value, String> {
    let eff = effective(conn, pid, range)?;
    let days = eff.as_ref().map(|e| day_span(&e.from, &e.to)).unwrap_or_default();
    let index: HashMap<&str, usize> = days.iter().enumerate().map(|(i, d)| (d.as_str(), i)).collect();
    // Error timeline per bot (top 6 bots by errors) from the non-fake aggregates.
    let per_bot = rows(
        conn,
        "SELECT day, bot_id, SUM(s4xx), SUM(s5xx) FROM url_daily WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 AND (s4xx > 0 OR s5xx > 0) GROUP BY day, bot_id",
        params![pid, range.from, range.to],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)),
    )?;
    let mut totals: HashMap<String, i64> = HashMap::new();
    for (_, bot, a, b) in &per_bot {
        *totals.entry(bot.clone()).or_default() += a + b;
    }
    let mut top: Vec<(String, i64)> = totals.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top.truncate(6);
    let mut s4: BTreeMap<String, Vec<i64>> = top.iter().map(|(b, _)| (b.clone(), vec![0; days.len()])).collect();
    let mut s5 = s4.clone();
    let mut all4 = vec![0i64; days.len()];
    let mut all5 = vec![0i64; days.len()];
    for (day, bot, a, b) in &per_bot {
        if let Some(&i) = index.get(day.as_str()) {
            all4[i] += a;
            all5[i] += b;
            if let Some(v) = s4.get_mut(bot) {
                v[i] += a;
            }
            if let Some(v) = s5.get_mut(bot) {
                v[i] += b;
            }
        }
    }
    let error_urls = rows(
        conn,
        "SELECT path || IFNULL('?' || query, '') u, status, COUNT(*), MAX(ts), GROUP_CONCAT(DISTINCT bot_id) FROM bot_hits
         WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 AND status >= 400 AND verification != 'fake' GROUP BY u, status ORDER BY 3 DESC LIMIT 300",
        params![pid, range.from, range.to],
        |r| Ok(json!({ "url": r.get::<_, String>(0)?, "status": r.get::<_, i64>(1)?, "hits": r.get::<_, i64>(2)?, "lastSeen": r.get::<_, String>(3)?, "bots": r.get::<_, String>(4)? })),
    )?;
    // Spike detection over the full history so the 28-day baseline exists at the range start.
    let hist = rows(
        conn,
        "SELECT day, SUM(s4xx), SUM(s5xx) FROM url_daily WHERE project_id = ?1 GROUP BY day ORDER BY day",
        [pid],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)),
    )?;
    let fill = |pick: fn(&(String, i64, i64)) -> i64| -> Vec<(String, i64)> {
        let (Some(first), Some(last)) = (hist.first(), hist.last()) else { return Vec::new() };
        let map: HashMap<&str, i64> = hist.iter().map(|h| (h.0.as_str(), pick(h))).collect();
        day_span(&first.0, &last.0).into_iter().map(|d| {
            let v = map.get(d.as_str()).copied().unwrap_or(0);
            (d, v)
        }).collect()
    };
    let in_range = |s: &Spike| s.day >= range.from && s.day <= range.to;
    let spikes5: Vec<Spike> = detect_spikes(&fill(|h| h.2), 28).into_iter().filter(in_range).collect();
    let spikes4: Vec<Spike> = detect_spikes(&fill(|h| h.1), 28).into_iter().filter(in_range).collect();
    let code_mix = rows(
        conn,
        "SELECT status, COUNT(*) FROM bot_hits WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 AND status >= 300 AND verification != 'fake' GROUP BY status ORDER BY 2 DESC LIMIT 20",
        params![pid, range.from, range.to],
        |r| Ok(json!({ "status": r.get::<_, i64>(0)?, "hits": r.get::<_, i64>(1)? })),
    )?;
    Ok(json!({
        "range": { "from": range.from, "to": range.to },
        "days": days,
        "total4xx": all4.iter().sum::<i64>(),
        "total5xx": all5.iter().sum::<i64>(),
        "daily4xx": all4,
        "daily5xx": all5,
        "byBot4xx": s4,
        "byBot5xx": s5,
        "errorUrls": error_urls,
        "codes": code_mix,
        "spikes5xx": spikes5,
        "spikes4xx": spikes4,
    }))
}

// ---------------------------------------------------------------- AI bots

pub fn ai_activity(conn: &Connection, pid: i64, range: &Range) -> Result<Value, String> {
    let eff = effective(conn, pid, range)?;
    let days = eff.as_ref().map(|e| day_span(&e.from, &e.to)).unwrap_or_default();
    let index: HashMap<&str, usize> = days.iter().enumerate().map(|(i, d)| (d.as_str(), i)).collect();
    let mut training = vec![0i64; days.len()];
    let mut assistant = vec![0i64; days.len()];
    for (day, cat, n) in rows(
        conn,
        "SELECT day, category, COUNT(*) FROM bot_hits WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 AND category IN ('ai_training', 'ai_assistant') AND verification != 'fake' GROUP BY day, category",
        params![pid, range.from, range.to],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
    )? {
        if let Some(&i) = index.get(day.as_str()) {
            if cat == "ai_training" {
                training[i] = n;
            } else {
                assistant[i] = n;
            }
        }
    }
    let mut dirs: HashMap<String, Vec<Value>> = HashMap::new();
    for (bot, dir, n) in rows(
        conn,
        "SELECT bot_id, CASE WHEN instr(substr(path, 2), '/') > 0 THEN substr(path, 1, instr(substr(path, 2), '/') + 1) ELSE '/' END d, COUNT(*) FROM bot_hits
         WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 AND category IN ('ai_training', 'ai_assistant') AND verification != 'fake' GROUP BY bot_id, d ORDER BY 3 DESC",
        params![pid, range.from, range.to],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
    )? {
        let list = dirs.entry(bot).or_default();
        if list.len() < 6 {
            list.push(json!({ "dir": dir, "hits": n }));
        }
    }
    let bots_list = rows(
        conn,
        "SELECT bot_id, category, COUNT(*), COUNT(DISTINCT path), SUM(verification = 'verified'), SUM(verification = 'unverifiable'), SUM(verification = 'fake'), MAX(ts), SUM(bytes)
         FROM bot_hits WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 AND category IN ('ai_training', 'ai_assistant') GROUP BY bot_id ORDER BY 3 DESC",
        params![pid, range.from, range.to],
        |r| {
            let id: String = r.get(0)?;
            let def = bots::find(&id);
            Ok(json!({
                "botId": id, "name": def.map(|d| d.name).unwrap_or("?"), "owner": def.map(|d| d.owner).unwrap_or("—"),
                "category": r.get::<_, String>(1)?, "requests": r.get::<_, i64>(2)?, "uniquePaths": r.get::<_, i64>(3)?,
                "verified": r.get::<_, i64>(4)?, "unverifiable": r.get::<_, i64>(5)?, "fake": r.get::<_, i64>(6)?,
                "lastSeen": r.get::<_, Option<String>>(7)?, "bytes": r.get::<_, Option<i64>>(8)?,
                "topDirs": dirs.get(&id).cloned().unwrap_or_default(),
            }))
        },
    )?;
    let top_urls = rows(
        conn,
        "SELECT path, COUNT(*), COUNT(DISTINCT bot_id) FROM bot_hits WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 AND category IN ('ai_training', 'ai_assistant') AND verification != 'fake'
         GROUP BY path ORDER BY 2 DESC LIMIT 25",
        params![pid, range.from, range.to],
        |r| Ok(json!({ "path": r.get::<_, String>(0)?, "hits": r.get::<_, i64>(1)?, "bots": r.get::<_, i64>(2)? })),
    )?;
    Ok(json!({
        "range": { "from": range.from, "to": range.to },
        "days": days,
        "training": training,
        "assistant": assistant,
        "trainingTotal": training.iter().sum::<i64>(),
        "assistantTotal": assistant.iter().sum::<i64>(),
        "bots": bots_list,
        "topUrls": top_urls,
    }))
}

// ---------------------------------------------------------------- fake bots

pub fn fake_bots(conn: &Connection, pid: i64, range: &Range, limit: i64) -> Result<Value, String> {
    let limit = limit.clamp(1, 2000);
    let groups = rows(
        conn,
        "SELECT IFNULL(ip, '—'), bot_id, COUNT(*), MIN(ts), MAX(ts), COUNT(DISTINCT path) FROM bot_hits
         WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 AND verification = 'fake' GROUP BY ip, bot_id ORDER BY 3 DESC LIMIT ?4",
        params![pid, range.from, range.to, limit],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, i64>(5)?)),
    )?;
    let mut sample = conn
        .prepare(
            "SELECT path || IFNULL('?' || query, '') u FROM bot_hits WHERE project_id = ?1 AND bot_id = ?2 AND IFNULL(ip, '—') = ?3 AND verification = 'fake' AND day BETWEEN ?4 AND ?5
             GROUP BY u ORDER BY COUNT(*) DESC LIMIT 3",
        )
        .map_err(|e| e.to_string())?;
    let mut list = Vec::new();
    for (ip, bot, hits, first, last, paths) in groups {
        let urls: Vec<String> = sample.query_map(params![pid, bot, ip, range.from, range.to], |r| r.get(0)).map_err(|e| e.to_string())?.flatten().collect();
        let def = bots::find(&bot);
        list.push(json!({ "ip": ip, "botId": bot, "claims": def.map(|d| d.name).unwrap_or("?"), "requests": hits, "firstSeen": first, "lastSeen": last, "uniquePaths": paths, "sampleUrls": urls }));
    }
    let by_bot = rows(
        conn,
        "SELECT bot_id, SUM(verification = 'fake'), COUNT(*) FROM bot_hits WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 GROUP BY bot_id HAVING SUM(verification = 'fake') > 0 ORDER BY 2 DESC",
        params![pid, range.from, range.to],
        |r| {
            let id: String = r.get(0)?;
            let fake: i64 = r.get(1)?;
            let total: i64 = r.get(2)?;
            Ok(json!({ "botId": id.clone(), "name": bots::find(&id).map(|d| d.name).unwrap_or("?"), "fake": fake, "claims": total, "fakeShare": if total > 0 { fake as f64 / total as f64 } else { 0.0 } }))
        },
    )?;
    let (fake_total, unverifiable): (i64, i64) = one(
        conn,
        "SELECT COALESCE(SUM(verification = 'fake'), 0), COALESCE(SUM(verification = 'unverifiable'), 0) FROM bot_hits WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3",
        params![pid, range.from, range.to],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let sources: i64 = one(conn, "SELECT COUNT(DISTINCT ip) FROM bot_hits WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 AND verification = 'fake'", params![pid, range.from, range.to], |r| r.get(0))?;
    Ok(json!({ "range": { "from": range.from, "to": range.to }, "fakeRequests": fake_total, "unverifiableRequests": unverifiable, "sources": sources, "byBot": by_bot, "total": list.len(), "groups": list }))
}

// ---------------------------------------------------------------- imports

pub fn imports(conn: &Connection, pid: i64) -> Result<Value, String> {
    let list = rows(
        conn,
        "SELECT id, path, name, format, size_bytes, lines_total, lines_parsed, lines_failed, bot_hits, human_requests, first_ts, last_ts, imported_at, duration_ms, sample_errors
         FROM log_files WHERE project_id = ?1 ORDER BY id DESC",
        [pid],
        |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "path": r.get::<_, String>(1)?, "name": r.get::<_, String>(2)?, "format": r.get::<_, String>(3)?,
                "sizeBytes": r.get::<_, i64>(4)?, "lines": r.get::<_, i64>(5)?, "parsed": r.get::<_, i64>(6)?, "failed": r.get::<_, i64>(7)?,
                "botHits": r.get::<_, i64>(8)?, "humanRequests": r.get::<_, i64>(9)?, "firstTs": r.get::<_, Option<String>>(10)?, "lastTs": r.get::<_, Option<String>>(11)?,
                "importedAt": r.get::<_, String>(12)?, "durationMs": r.get::<_, i64>(13)?,
                "sampleErrors": serde_json::from_str::<Value>(&r.get::<_, String>(14)?).unwrap_or(Value::Array(vec![])),
            }))
        },
    )?;
    Ok(json!({ "total": list.len(), "imports": list }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::import::tests::{line, setup, verifier, CHROME, GOOGLE_M};
    use crate::logs::import::{import_paths, ImportOptions};
    use std::io::Write;

    fn seeded() -> (Connection, i64, tempfile::TempDir) {
        let (mut conn, pid, dir) = setup();
        let path = dir.path().join("a.log");
        let mut f = std::fs::File::create(&path).unwrap();
        let gpt = "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; GPTBot/1.1; +https://openai.com/gptbot";
        for d in 1..=20u32 {
            f.write_all(line("66.249.64.5", d, "/", 200, GOOGLE_M).as_bytes()).unwrap();
            f.write_all(line("66.249.64.5", d, "/blog/yazi/?utm_source=x", 200, GOOGLE_M).as_bytes()).unwrap();
            f.write_all(line("132.196.86.3", d, "/blog/yazi/", 200, gpt).as_bytes()).unwrap();
            f.write_all(line("198.51.100.7", d, "/", 200, CHROME).as_bytes()).unwrap();
        }
        for _ in 0..12 {
            f.write_all(line("66.249.64.5", 21, "/urun/", 503, GOOGLE_M).as_bytes()).unwrap();
        }
        f.write_all(line("203.0.113.5", 21, "/wp-admin/", 404, GOOGLE_M).as_bytes()).unwrap();
        drop(f);
        import_paths(&mut conn, pid, &[path.display().to_string()], &ImportOptions::default(), &mut verifier(), &mut |_| {}).unwrap();
        (conn, pid, dir)
    }

    #[test]
    fn range_validation() {
        assert_eq!(Range::new(None, None).unwrap(), Range::all());
        assert!(Range::new(Some("2024-13-1"), None).is_err());
        assert!(Range::new(Some("2024-10-10"), Some("2024-10-01")).is_err());
        assert_eq!(day_span("2024-02-27", "2024-03-01").len(), 4);
    }

    #[test]
    fn overview_and_bots() {
        let (conn, pid, _d) = seeded();
        let o = overview(&conn, pid, &Range::all()).unwrap();
        assert_eq!(o["totalBotRequests"], 73);
        assert_eq!(o["humanRequests"], 20);
        assert_eq!(o["verification"]["fake"], 1);
        assert_eq!(o["googlebot"]["requests"], 52);
        assert_eq!(o["googlebot"]["smartphone"], 52);
        assert_eq!(o["status"]["s5xx"], 12);
        assert_eq!(o["daily"]["days"].as_array().unwrap().len(), 21);
        assert_eq!(o["topUrls"][0]["hits"], 20);
        assert_eq!(o["uniqueUrls"], 4);
        let b = bot_stats(&conn, pid, &Range::all(), None).unwrap();
        assert_eq!(b["bots"][0]["botId"], "googlebot-smartphone");
        assert_eq!(b["bots"][0]["fake"], 1);
        assert!(bot_stats(&conn, pid, &Range::all(), Some("nope")).is_err());
        let ai = bot_stats(&conn, pid, &Range::all(), Some("ai_training")).unwrap();
        assert_eq!(ai["total"], 1);
        let d = bot_detail(&conn, pid, "gptbot", &Range::all()).unwrap();
        assert_eq!(d["requests"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).sum::<i64>(), 20);
    }

    #[test]
    fn url_filters() {
        let (conn, pid, _d) = seeded();
        let all = urls(&conn, pid, &Range::all(), &UrlFilter::default()).unwrap();
        assert_eq!(all["total"], 4);
        let with = urls(&conn, pid, &Range::all(), &UrlFilter { params: Some("with".into()), ..Default::default() }).unwrap();
        assert_eq!(with["total"], 1);
        let blog = urls(&conn, pid, &Range::all(), &UrlFilter { dir: Some("/blog/".into()), bot: Some("gptbot".into()), ..Default::default() }).unwrap();
        assert_eq!(blog["total"], 1);
        let errors = urls(&conn, pid, &Range::all(), &UrlFilter { status: Some("5xx".into()), ..Default::default() }).unwrap();
        assert_eq!(errors["urls"][0]["url"], "/urun/");
        assert!(urls(&conn, pid, &Range::all(), &UrlFilter { bot: Some("x' OR 1=1".into()), ..Default::default() }).is_err(), "unknown bot ids are rejected, never interpolated");
    }

    #[test]
    fn spikes_status_ai_fake_imports() {
        let flat: Vec<(String, i64)> = (0..30).map(|i| (format!("d{i:02}"), if i == 29 { 40 } else { 1 + i % 2 })).collect();
        let s = detect_spikes(&flat, 28);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].day, "d29");
        assert!(detect_spikes(&flat[..5], 28).is_empty());
        let (conn, pid, _d) = seeded();
        let st = status_report(&conn, pid, &Range::all()).unwrap();
        assert_eq!(st["total5xx"], 12);
        assert_eq!(st["spikes5xx"].as_array().unwrap().len(), 1, "day 21 5xx burst is a spike");
        let ai = ai_activity(&conn, pid, &Range::all()).unwrap();
        assert_eq!(ai["trainingTotal"], 20);
        assert_eq!(ai["bots"][0]["topDirs"][0]["dir"], "/blog/");
        let fake = fake_bots(&conn, pid, &Range::all(), 50).unwrap();
        assert_eq!(fake["groups"][0]["ip"], "203.0.113.0");
        assert_eq!(fake["groups"][0]["sampleUrls"][0], "/wp-admin/");
        let imp = imports(&conn, pid).unwrap();
        assert_eq!(imp["imports"][0]["botHits"], 73);
    }
}
