//! Seobify Spider comparison: orphan candidates (crawled by Googlebot, unknown to Spider),
//! indexable pages Googlebot ignored, and problem pages Googlebot keeps fetching.

use super::bots;
use super::parse;
use super::query::day_span;
use crate::hub::db::Project;
use crate::hub::spider::{self, CrawledUrl};
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

/// Static resources (excluded from page comparisons, counted as crawl-budget waste).
pub fn is_static(path: &str) -> bool {
    let p = path.split('?').next().unwrap_or(path).to_ascii_lowercase();
    let ext = match p.rsplit_once('.') {
        Some((before, ext)) if !ext.contains('/') && !before.is_empty() => ext,
        _ => return false,
    };
    matches!(ext, "css" | "js" | "mjs" | "map" | "png" | "jpg" | "jpeg" | "gif" | "svg" | "webp" | "avif" | "ico" | "bmp" | "woff" | "woff2" | "ttf" | "otf" | "eot" | "mp4" | "webm" | "mp3")
        || p.starts_with("/wp-content/") || p.starts_with("/wp-includes/") || p.starts_with("/_next/static/") || p.starts_with("/static/")
}

/// Infrastructure URLs that are never "pages".
fn is_infra(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p == "/robots.txt" || p.starts_with("/sitemap") || p == "/favicon.ico" || p.starts_with("/wp-json") || p == "/ads.txt" || p.starts_with("/.well-known/")
}

/// Comparison key: percent-decoded path + raw query (same shape as `url_daily.url`).
pub fn key_of(path: &str, query: Option<&str>) -> String {
    let p = if path.contains('%') { parse::percent_decode(path) } else { path.to_string() };
    match query.filter(|q| !q.is_empty()) {
        Some(q) => format!("{p}?{q}"),
        None => p,
    }
}

fn key_of_logged(url: &str) -> String {
    match url.split_once('?') {
        Some((p, q)) => key_of(p, Some(q)),
        None => key_of(url, None),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpiderPage {
    pub key: String,
    pub url: String,
    pub status: Option<i64>,
    pub indexable: bool,
    pub indexability: Option<String>,
    pub reason: Option<String>,
    pub canonical_differs: bool,
    pub title: Option<String>,
    pub depth: Option<i64>,
}

fn same_site(host: &str, domain: Option<&str>) -> bool {
    match domain {
        None => true,
        Some(d) => {
            let h = host.trim_start_matches("www.");
            let d = d.trim_start_matches("www.");
            h == d || h.ends_with(&format!(".{d}"))
        }
    }
}

/// Spider rows → comparison pages (only URLs of the project's domain when it is known).
pub fn index_spider(urls: &[CrawledUrl], domain: Option<&str>) -> Vec<SpiderPage> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for u in urls {
        let Ok(parsed) = url::Url::parse(&u.url) else { continue };
        let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
        if !same_site(&host, domain) {
            continue;
        }
        let key = key_of(parsed.path(), parsed.query());
        if !seen.insert(key.clone()) {
            continue;
        }
        let status = u.status_code;
        let ok_status = status.map_or(true, |s| (200..300).contains(&s));
        let indexable = ok_status && u.indexability.as_deref().map_or(true, |i| i.eq_ignore_ascii_case("indexable"));
        let canonical_differs = u.canonical.as_deref().filter(|c| !c.trim().is_empty()).map_or(false, |c| match url::Url::parse(c) {
            Ok(cu) => cu.host_str().map(|h| h.to_ascii_lowercase()) != Some(host.clone()) || key_of(cu.path(), cu.query()) != key,
            Err(_) => false,
        });
        out.push(SpiderPage {
            key,
            url: u.url.clone(),
            status,
            indexable: indexable && !canonical_differs,
            indexability: u.indexability.clone(),
            reason: u.indexability_reason.clone().or_else(|| canonical_differs.then(|| "canonicalised".to_string())),
            canonical_differs,
            title: u.title.clone(),
            depth: u.depth,
        });
    }
    out
}

struct Hit {
    hits: i64,
    last: Option<String>,
    s2: i64,
    s3: i64,
    s4: i64,
    s5: i64,
}

fn dominant(h: &Hit) -> &'static str {
    let m = [(h.s2, "2xx"), (h.s3, "3xx"), (h.s4, "4xx"), (h.s5, "5xx")];
    m.iter().max_by_key(|x| x.0).map(|x| x.1).unwrap_or("2xx")
}

/// Compares Googlebot activity of the last `days` days (ending at the newest log day).
pub fn spider_gaps(conn: &Connection, pid: i64, pages: &[SpiderPage], days: i64, limit: usize) -> Result<Value, String> {
    let g = bots::googlebot_sql_list();
    let max_day: Option<String> = conn.query_row("SELECT MAX(day) FROM bot_hits WHERE project_id = ?1", [pid], |r| r.get(0)).map_err(|e| e.to_string())?;
    let Some(end) = max_day else {
        return Err("This project has no imported logs yet. Run import_logs first.".into());
    };
    let days = days.clamp(1, 3650);
    let start = parse::parse_day(&end).map(|d| parse::day_string((d - days + 1) * 86_400)).unwrap_or_else(|| end.clone());
    let mut window: HashMap<String, Hit> = HashMap::new();
    {
        let mut stmt = conn
            .prepare(&format!("SELECT url, SUM(hits), MAX(last_ts), SUM(s2xx), SUM(s3xx), SUM(s4xx), SUM(s5xx) FROM url_daily WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 AND bot_id IN ({g}) GROUP BY url"))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![pid, start, end], |r| Ok((r.get::<_, String>(0)?, Hit { hits: r.get(1)?, last: r.get(2)?, s2: r.get(3)?, s3: r.get(4)?, s4: r.get(5)?, s5: r.get(6)? })))
            .map_err(|e| e.to_string())?;
        for row in rows.flatten() {
            let entry = window.entry(key_of_logged(&row.0)).or_insert(Hit { hits: 0, last: None, s2: 0, s3: 0, s4: 0, s5: 0 });
            entry.hits += row.1.hits;
            entry.s2 += row.1.s2;
            entry.s3 += row.1.s3;
            entry.s4 += row.1.s4;
            entry.s5 += row.1.s5;
            if row.1.last > entry.last {
                entry.last = row.1.last;
            }
        }
    }
    let mut ever: HashMap<String, String> = HashMap::new();
    {
        let mut stmt = conn.prepare(&format!("SELECT url, MAX(last_ts) FROM url_daily WHERE project_id = ?1 AND bot_id IN ({g}) GROUP BY url")).map_err(|e| e.to_string())?;
        let rows = stmt.query_map([pid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))).map_err(|e| e.to_string())?;
        for (u, last) in rows.flatten() {
            if let Some(last) = last {
                let k = key_of_logged(&u);
                let e = ever.entry(k).or_default();
                if last > *e {
                    *e = last;
                }
            }
        }
    }
    let spider_keys: HashSet<&str> = pages.iter().map(|p| p.key.as_str()).collect();
    // Parameter variants ("/page/?utm_source=x") belong to their page: compare on the path for
    // query-less Spider pages, so they are neither orphans nor hide a crawled page.
    let path_of = |k: &str| -> String { k.split('?').next().unwrap_or(k).to_string() };
    let spider_paths: HashSet<String> = pages.iter().map(|p| path_of(&p.key)).collect();
    let mut by_path: HashMap<String, Hit> = HashMap::new();
    for (k, h) in &window {
        let e = by_path.entry(path_of(k)).or_insert(Hit { hits: 0, last: None, s2: 0, s3: 0, s4: 0, s5: 0 });
        e.hits += h.hits;
        e.s2 += h.s2;
        e.s3 += h.s3;
        e.s4 += h.s4;
        e.s5 += h.s5;
        if h.last > e.last {
            e.last = h.last.clone();
        }
    }
    let mut ever_path: HashMap<String, String> = HashMap::new();
    for (k, last) in &ever {
        let e = ever_path.entry(path_of(k)).or_default();
        if *last > *e {
            *e = last.clone();
        }
    }
    let hit_of = |p: &SpiderPage| -> Option<&Hit> { if p.key.contains('?') { window.get(&p.key) } else { by_path.get(&p.key) } };
    let ever_of = |p: &SpiderPage| -> Option<&String> { if p.key.contains('?') { ever.get(&p.key) } else { ever_path.get(&p.key) } };

    let mut orphans: Vec<Value> = window
        .iter()
        .filter(|(k, _)| !spider_keys.contains(k.as_str()) && !spider_paths.contains(&path_of(k)) && !is_static(k) && !is_infra(k))
        .map(|(k, h)| json!({ "url": k, "googlebotHits": h.hits, "lastGooglebot": h.last, "status": dominant(h), "s2xx": h.s2, "s3xx": h.s3, "s4xx": h.s4, "s5xx": h.s5 }))
        .collect();
    orphans.sort_by(|a, b| b["googlebotHits"].as_i64().cmp(&a["googlebotHits"].as_i64()).then(a["url"].as_str().cmp(&b["url"].as_str())));
    let orphan_total = orphans.len();
    let orphan_hits: i64 = orphans.iter().filter_map(|o| o["googlebotHits"].as_i64()).sum();
    let orphan_2xx = orphans.iter().filter(|o| o["status"] == "2xx").count();

    let indexable: Vec<&SpiderPage> = pages.iter().filter(|p| p.indexable).collect();
    let mut uncrawled: Vec<Value> = indexable
        .iter()
        .filter(|p| hit_of(p).is_none())
        .map(|p| json!({ "url": p.url, "path": p.key, "title": p.title, "depth": p.depth, "lastGooglebot": ever_of(p) }))
        .collect();
    uncrawled.sort_by(|a, b| a["depth"].as_i64().cmp(&b["depth"].as_i64()).then(a["path"].as_str().cmp(&b["path"].as_str())));
    let uncrawled_total = uncrawled.len();

    let mut problems: Vec<Value> = pages
        .iter()
        .filter(|p| !p.indexable)
        .filter_map(|p| hit_of(p).map(|h| (p, h)))
        .map(|(p, h)| {
            json!({ "url": p.url, "path": p.key, "status": p.status, "indexability": p.indexability, "reason": p.reason, "canonicalDiffers": p.canonical_differs, "googlebotHits": h.hits, "lastGooglebot": h.last })
        })
        .collect();
    problems.sort_by(|a, b| b["googlebotHits"].as_i64().cmp(&a["googlebotHits"].as_i64()));
    let problem_total = problems.len();
    let problem_hits: i64 = problems.iter().filter_map(|o| o["googlebotHits"].as_i64()).sum();

    let crawled_indexable = indexable.len() - uncrawled_total;
    Ok(json!({
        "window": { "from": start, "to": end, "days": day_span(&start, &end).len() },
        "summary": {
            "spiderPages": pages.len(),
            "indexable": indexable.len(),
            "indexableCrawled": crawled_indexable,
            "indexableUncrawled": uncrawled_total,
            "uncrawledShare": if indexable.is_empty() { 0.0 } else { uncrawled_total as f64 / indexable.len() as f64 },
            "orphanCandidates": orphan_total,
            "orphanCandidates2xx": orphan_2xx,
            "orphanHits": orphan_hits,
            "problemPagesCrawled": problem_total,
            "problemHits": problem_hits,
            "googlebotUrls": window.len(),
        },
        "orphans": orphans.into_iter().take(limit).collect::<Vec<_>>(),
        "uncrawled": uncrawled.into_iter().take(limit).collect::<Vec<_>>(),
        "problems": problems.into_iter().take(limit).collect::<Vec<_>>(),
    }))
}

// ---------------------------------------------------------------- Spider data (cached per crawl)

#[derive(Clone)]
pub struct SpiderData {
    pub project: String,
    pub crawl_id: i64,
    pub source: &'static str,
    pub start_url: Option<String>,
    pub urls: Arc<Vec<CrawledUrl>>,
}

fn cache() -> &'static Mutex<HashMap<(String, i64), (Arc<Vec<CrawledUrl>>, &'static str, Option<String>)>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, i64), (Arc<Vec<CrawledUrl>>, &'static str, Option<String>)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Loads a Spider crawl for a project. Without `crawl_id`, the newest completed crawl whose
/// start URL matches the project's domain is used (falls back to the newest completed crawl).
pub async fn load_spider(project: &Project, spider_project: Option<String>, crawl_id: Option<i64>) -> Result<SpiderData, String> {
    let sp = spider_project.or_else(|| project.spider_project.clone()).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "default".into());
    let (crawls, _) = spider::crawls(&sp).await?;
    let id = match crawl_id {
        Some(id) => id,
        None => {
            let domain = project.domain.clone();
            let matching = crawls
                .iter()
                .filter(|c| c.status.as_deref() == Some("completed"))
                .filter(|c| match (&domain, &c.start_url) {
                    (Some(d), Some(u)) => crate::hub::text::normalize_domain(u) == *d,
                    _ => false,
                })
                .map(|c| c.id)
                .max();
            matching.or_else(|| spider::pick_crawl(&crawls)).ok_or_else(|| format!("Spider project \"{sp}\" has no crawls"))?
        }
    };
    let start_url = crawls.iter().find(|c| c.id == id).and_then(|c| c.start_url.clone());
    if let Some((urls, source, su)) = cache().lock().ok().and_then(|c| c.get(&(sp.clone(), id)).cloned()) {
        return Ok(SpiderData { project: sp, crawl_id: id, source, start_url: su.or(start_url), urls });
    }
    let (urls, id, source) = spider::urls(&sp, Some(id)).await?;
    let urls = Arc::new(urls);
    if let Ok(mut c) = cache().lock() {
        c.insert((sp.clone(), id), (Arc::clone(&urls), source, start_url.clone()));
    }
    Ok(SpiderData { project: sp, crawl_id: id, source, start_url, urls })
}

impl SpiderData {
    /// Domain used to filter Spider URLs: the project's domain, else the crawl's start URL host.
    pub fn domain(&self, project: &Project) -> Option<String> {
        project.domain.clone().or_else(|| self.start_url.as_deref().map(crate::hub::text::normalize_domain)).filter(|d| !d.is_empty())
    }

    pub fn meta(&self) -> Value {
        json!({ "spiderProject": self.project, "crawlId": self.crawl_id, "source": self.source, "startUrl": self.start_url, "spiderUrls": self.urls.len() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::import::tests::{line, setup, verifier, GOOGLE_M};
    use crate::logs::import::{import_paths, ImportOptions};
    use std::io::Write;

    fn cu(url: &str, status: i64, indexability: &str, canonical: Option<&str>) -> CrawledUrl {
        CrawledUrl { url: url.into(), status_code: Some(status), indexability: Some(indexability.into()), canonical: canonical.map(str::to_string), depth: Some(1), ..Default::default() }
    }

    #[test]
    fn static_and_keys() {
        assert!(is_static("/wp-content/themes/a/style.css?ver=1"));
        assert!(is_static("/img/logo.webp"));
        assert!(!is_static("/blog/yazi/"));
        assert!(!is_static("/v1.2/sayfa"));
        assert_eq!(key_of("/hizmet/%C5%9Fey/", Some("a=1")), "/hizmet/şey/?a=1");
        let pages = index_spider(
            &[
                cu("https://seobify.com/", 200, "indexable", Some("https://seobify.com/")),
                cu("https://seobify.com/a/", 200, "indexable", Some("https://seobify.com/b/")),
                cu("https://other.com/x/", 200, "indexable", None),
                cu("https://www.seobify.com/c/", 200, "non-indexable", None),
            ],
            Some("seobify.com"),
        );
        assert_eq!(pages.len(), 3, "foreign hosts are dropped, www is the same site");
        assert!(pages[0].indexable && !pages[1].indexable && pages[1].canonical_differs);
        assert_eq!(pages[1].reason.as_deref(), Some("canonicalised"));
    }

    #[test]
    fn gaps_against_a_crawl() {
        let (mut conn, pid, dir) = setup();
        let path = dir.path().join("a.log");
        let mut f = std::fs::File::create(&path).unwrap();
        for d in 1..=10u32 {
            f.write_all(line("66.249.64.5", d, "/", 200, GOOGLE_M).as_bytes()).unwrap();
            f.write_all(line("66.249.64.5", d, "/noindex-sayfa/", 200, GOOGLE_M).as_bytes()).unwrap();
            f.write_all(line("66.249.64.5", d, "/eski-sayfa/", 404, GOOGLE_M).as_bytes()).unwrap();
            f.write_all(line("66.249.64.5", d, "/yetim/", 200, GOOGLE_M).as_bytes()).unwrap();
            f.write_all(line("66.249.64.5", d, "/style.css", 200, GOOGLE_M).as_bytes()).unwrap();
            f.write_all(line("66.249.64.5", d, "/?utm_source=x", 200, GOOGLE_M).as_bytes()).unwrap();
        }
        drop(f);
        import_paths(&mut conn, pid, &[path.display().to_string()], &ImportOptions::default(), &mut verifier(), &mut |_| {}).unwrap();
        let pages = index_spider(
            &[
                cu("https://seobify.com/", 200, "indexable", None),
                cu("https://seobify.com/hic-taranmayan/", 200, "indexable", None),
                cu("https://seobify.com/noindex-sayfa/", 200, "non-indexable", None),
            ],
            Some("seobify.com"),
        );
        let r = spider_gaps(&conn, pid, &pages, 30, 100).unwrap();
        assert_eq!(r["summary"]["indexable"], 2);
        assert_eq!(r["summary"]["indexableUncrawled"], 1);
        assert_eq!(r["uncrawled"][0]["path"], "/hic-taranmayan/");
        assert_eq!(r["summary"]["orphanCandidates"], 2, "/yetim/ and /eski-sayfa/ (static excluded)");
        assert_eq!(r["summary"]["orphanCandidates2xx"], 1);
        assert_eq!(r["problems"][0]["path"], "/noindex-sayfa/");
        assert_eq!(r["problems"][0]["googlebotHits"], 10);
        assert_eq!(r["summary"]["uncrawledShare"], 0.5);
    }
}
