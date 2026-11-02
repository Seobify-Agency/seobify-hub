//! Crawl-budget waste: where a crawler's requests go besides indexable 200 pages.
//! Categories may overlap (a parameter URL can also redirect); `wasteHits` counts each
//! request once.

use super::bots::{self, Category};
use super::gaps::{is_static, key_of, SpiderPage};
use super::query::Range;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scope {
    Googlebot,
    Search,
    All,
}

impl Scope {
    pub fn from_name(s: Option<&str>) -> Result<Scope, String> {
        match s.map(str::trim).unwrap_or("googlebot") {
            "" | "googlebot" => Ok(Scope::Googlebot),
            "search" => Ok(Scope::Search),
            "all" => Ok(Scope::All),
            other => Err(format!("Unknown scope \"{other}\": use googlebot, search or all")),
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Scope::Googlebot => "googlebot",
            Scope::Search => "search",
            Scope::All => "all",
        }
    }

    fn clause(&self) -> String {
        match self {
            Scope::Googlebot => format!(" AND bot_id IN ({})", bots::googlebot_sql_list()),
            Scope::Search => format!(" AND bot_id IN ({})", bots::category_sql_list(Category::Search)),
            Scope::All => String::new(),
        }
    }
}

struct UrlRow {
    url: String,
    hits: i64,
    s3: i64,
    s4: i64,
    s5: i64,
}

#[derive(Default)]
struct Bucket {
    hits: i64,
    urls: Vec<(String, i64)>,
}

impl Bucket {
    fn add(&mut self, url: &str, n: i64) {
        if n > 0 {
            self.hits += n;
            self.urls.push((url.to_string(), n));
        }
    }

    fn json(&mut self, key: &str, total: i64) -> Value {
        self.urls.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let examples: Vec<Value> = self.urls.iter().take(8).map(|(u, n)| json!({ "url": u, "hits": n })).collect();
        json!({ "key": key, "hits": self.hits, "share": if total > 0 { self.hits as f64 / total as f64 } else { 0.0 }, "urls": self.urls.len(), "examples": examples })
    }
}

/// Parameter names of a query string (`a=1&b&utm_source=x` → `a`, `b`, `utm_source`).
pub fn param_names(query: &str) -> Vec<String> {
    let mut names: Vec<String> = query.split('&').filter_map(|kv| kv.split('=').next()).map(|k| k.trim().to_ascii_lowercase()).filter(|k| !k.is_empty()).collect();
    names.sort();
    names.dedup();
    names
}

pub fn crawl_waste(conn: &Connection, pid: i64, range: &Range, scope: Scope, spider: Option<&[SpiderPage]>) -> Result<Value, String> {
    let sql = format!(
        "SELECT url, SUM(hits), SUM(s3xx), SUM(s4xx), SUM(s5xx) FROM url_daily WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3{} GROUP BY url",
        scope.clause()
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let list: Vec<UrlRow> = stmt
        .query_map(params![pid, range.from, range.to], |r| Ok(UrlRow { url: r.get(0)?, hits: r.get(1)?, s3: r.get(2)?, s4: r.get(3)?, s5: r.get(4)? }))
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();
    let spider_map: Option<HashMap<&str, &SpiderPage>> = spider.map(|pages| pages.iter().map(|p| (p.key.as_str(), p)).collect());
    let total: i64 = list.iter().map(|r| r.hits).sum();
    let (mut params_b, mut redirects, mut client, mut server, mut statics, mut non_indexable) = (Bucket::default(), Bucket::default(), Bucket::default(), Bucket::default(), Bucket::default(), Bucket::default());
    let mut by_param: HashMap<String, (i64, Vec<(String, i64)>)> = HashMap::new();
    let mut waste = 0i64;
    for row in &list {
        let (path, query) = match row.url.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (row.url.as_str(), None),
        };
        let mut whole = false;
        // Static files count as static even with cache-busting parameters (style.css?ver=6.6).
        if is_static(path) {
            statics.add(&row.url, row.hits);
            whole = true;
        } else if let Some(q) = query {
            params_b.add(&row.url, row.hits);
            for name in param_names(q) {
                let e = by_param.entry(name).or_default();
                e.0 += row.hits;
                e.1.push((row.url.clone(), row.hits));
            }
            whole = true;
        }
        if let Some(map) = &spider_map {
            if let Some(page) = map.get(key_of(path, query).as_str()) {
                if !page.indexable && page.status.map_or(true, |s| (200..300).contains(&s)) {
                    non_indexable.add(&row.url, row.hits);
                    whole = true;
                }
            }
        }
        redirects.add(&row.url, row.s3);
        client.add(&row.url, row.s4);
        server.add(&row.url, row.s5);
        waste += if whole { row.hits } else { row.s3 + row.s4 + row.s5 };
    }
    let mut params_list: Vec<Value> = by_param
        .into_iter()
        .map(|(name, (hits, mut urls))| {
            urls.sort_by(|a, b| b.1.cmp(&a.1));
            json!({ "param": name, "hits": hits, "share": if total > 0 { hits as f64 / total as f64 } else { 0.0 }, "urls": urls.len(), "examples": urls.iter().take(3).map(|u| u.0.clone()).collect::<Vec<_>>() })
        })
        .collect();
    params_list.sort_by(|a, b| b["hits"].as_i64().cmp(&a["hits"].as_i64()));
    params_list.truncate(30);
    let mut categories = vec![params_b.json("parameters", total), redirects.json("redirects", total), client.json("client_errors", total), server.json("server_errors", total), statics.json("static", total)];
    if spider.is_some() {
        categories.push(non_indexable.json("non_indexable", total));
    }
    Ok(json!({
        "range": { "from": range.from, "to": range.to },
        "scope": scope.as_str(),
        "totalHits": total,
        "wasteHits": waste,
        "wasteShare": if total > 0 { waste as f64 / total as f64 } else { 0.0 },
        "spiderLinked": spider.is_some(),
        "categories": categories,
        "parameters": params_list,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::spider::CrawledUrl;
    use crate::logs::gaps::index_spider;
    use crate::logs::import::tests::{line, setup, verifier, GOOGLE_M};
    use crate::logs::import::{import_paths, ImportOptions};
    use std::io::Write;

    #[test]
    fn waste_categories() {
        assert_eq!(param_names("utm_source=x&page=2&UTM_SOURCE=y&&flag"), vec!["flag", "page", "utm_source"]);
        let (mut conn, pid, dir) = setup();
        let path = dir.path().join("a.log");
        let mut f = std::fs::File::create(&path).unwrap();
        for (p, s, n) in [("/", 200, 10), ("/?utm_source=x", 200, 5), ("/eski", 301, 3), ("/yok/", 404, 1), ("/app.js", 200, 1), ("/app.css?ver=2", 200, 2), ("/noindex/", 200, 4), ("/hata/", 500, 1)] {
            for _ in 0..n {
                f.write_all(line("66.249.64.5", 3, p, s, GOOGLE_M).as_bytes()).unwrap();
            }
        }
        drop(f);
        import_paths(&mut conn, pid, &[path.display().to_string()], &ImportOptions::default(), &mut verifier(), &mut |_| {}).unwrap();
        let w = crawl_waste(&conn, pid, &Range::all(), Scope::Googlebot, None).unwrap();
        assert_eq!(w["totalHits"], 27);
        let cat = |k: &str| w["categories"].as_array().unwrap().iter().find(|c| c["key"] == k).unwrap().clone();
        assert_eq!(cat("parameters")["hits"], 5);
        assert_eq!(cat("redirects")["hits"], 3);
        assert_eq!(cat("client_errors")["hits"], 1);
        assert_eq!(cat("server_errors")["hits"], 1);
        assert_eq!(cat("static")["hits"], 3, "cache-busting parameters stay static");
        assert_eq!(w["wasteHits"], 13);
        assert_eq!(w["parameters"][0]["param"], "utm_source");
        let pages = index_spider(
            &[CrawledUrl { url: "https://seobify.com/noindex/".into(), status_code: Some(200), indexability: Some("non-indexable".into()), ..Default::default() }],
            Some("seobify.com"),
        );
        let w2 = crawl_waste(&conn, pid, &Range::all(), Scope::All, Some(&pages)).unwrap();
        assert_eq!(w2["wasteHits"], 17);
        assert!(Scope::from_name(Some("x")).is_err());
    }
}
