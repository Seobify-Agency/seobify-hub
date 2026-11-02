//! Seobify Spider bridge + URL importers.
//!
//! Spider keeps one DuckDB file per project in `~/.seobify-spider/` and, while running,
//! serves a local API on `127.0.0.1:8000/api` (the engine holds the DuckDB file lock, so
//! the API is tried first). Without the API the bridge shells out to the `duckdb` CLI in
//! read-only mode. CSV (Spider, Screaming Frog, GSC exports), plain URL lists and XML
//! sitemaps are accepted everywhere a URL list is needed.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CrawledUrl {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexability: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexability_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h1: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta_robots: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_time_ms: Option<f64>,
}

impl CrawledUrl {
    pub fn bare(url: impl Into<String>) -> Self {
        CrawledUrl { url: url.into(), ..Default::default() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct SpiderProject {
    pub name: String,
    pub path: Option<String>,
    pub size_bytes: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct SpiderCrawl {
    pub id: i64,
    pub name: Option<String>,
    pub start_url: Option<String>,
    pub status: Option<String>,
    pub urls_done: Option<i64>,
    pub created_at: Option<String>,
    pub finished_at: Option<String>,
}

/// Where a URL list comes from (GUI import dialogs and MCP tools share this).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ImportSource {
    /// A Seobify Spider crawl (newest completed when `crawl_id` is absent).
    #[serde(rename_all = "camelCase")]
    Spider { project: String, crawl_id: Option<i64> },
    /// A local file: `.csv`, `.txt`, `.xml` / `.xml.gz` sitemap.
    File { path: String },
    /// A remote XML sitemap or sitemap index.
    Sitemap { url: String },
    /// URLs given inline.
    List { urls: Vec<String> },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub source: String,
    pub urls: Vec<CrawledUrl>,
}

pub async fn import(source: &ImportSource) -> Result<ImportResult, String> {
    match source {
        ImportSource::Spider { project, crawl_id } => {
            let (urls, id, via) = urls(project, *crawl_id).await?;
            Ok(ImportResult { source: format!("spider:{project}#{id} ({via})"), urls })
        }
        ImportSource::File { path } => {
            let p = Path::new(path);
            let urls = read_file(p).await?;
            Ok(ImportResult { source: format!("file:{}", p.display()), urls })
        }
        ImportSource::Sitemap { url } => {
            let urls = fetch_sitemap(url, 50_000).await?.into_iter().map(CrawledUrl::bare).collect();
            Ok(ImportResult { source: format!("sitemap:{url}"), urls })
        }
        ImportSource::List { urls } => Ok(ImportResult {
            source: "list".into(),
            urls: parse_text_list(&urls.join("\n")).into_iter().map(CrawledUrl::bare).collect(),
        }),
    }
}

// ---------------------------------------------------------------- Spider API / files

fn api_base() -> String {
    std::env::var("SEOBIFY_SPIDER_API").unwrap_or_else(|_| "http://127.0.0.1:8000/api".into())
}

fn api_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(600))
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap_or_default()
}

async fn api_get(path: &str) -> Result<reqwest::Response, String> {
    let resp = api_client().get(format!("{}{}", api_base(), path)).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("Spider API {} → HTTP {}", path, resp.status()));
    }
    Ok(resp)
}

/// Spider projects: `(projects, "api" | "files")`.
pub async fn projects() -> Result<(Vec<SpiderProject>, &'static str), String> {
    if let Ok(resp) = api_get("/projects").await {
        if let Ok(list) = resp.json::<Vec<SpiderProject>>().await {
            return Ok((list, "api"));
        }
    }
    Ok((file_projects(), "files"))
}

pub fn file_projects() -> Vec<SpiderProject> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(super::paths::spider_home()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("seobify") {
                out.push(SpiderProject {
                    name: path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string(),
                    size_bytes: entry.metadata().ok().map(|m| m.len() as i64),
                    path: Some(path.display().to_string()),
                });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub async fn crawls(project: &str) -> Result<(Vec<SpiderCrawl>, &'static str), String> {
    let api_err = match api_get(&format!("/crawls?project={}", encode(project))).await {
        Ok(resp) => match resp.json::<Vec<SpiderCrawl>>().await {
            Ok(list) => return Ok((list, "api")),
            Err(e) => e.to_string(),
        },
        Err(e) => e,
    };
    let path = project_file(project)?;
    let sql = "SELECT id, name, start_url, status, urls_done, CAST(created_at AS VARCHAR) AS created_at, CAST(finished_at AS VARCHAR) AS finished_at FROM crawls ORDER BY id DESC";
    let rows = duckdb_json(&path, sql).await.map_err(|e| format!("{e} (Spider API: {api_err})"))?;
    let list = serde_json::from_value::<Vec<SpiderCrawl>>(Value::Array(rows)).map_err(|e| e.to_string())?;
    Ok((list, "files"))
}

/// Newest completed crawl, else newest crawl.
pub fn pick_crawl(crawls: &[SpiderCrawl]) -> Option<i64> {
    crawls
        .iter()
        .filter(|c| c.status.as_deref() == Some("completed"))
        .map(|c| c.id)
        .max()
        .or_else(|| crawls.iter().map(|c| c.id).max())
}

/// URLs of a crawl: `(urls, crawl_id, "api" | "files")`.
pub async fn urls(project: &str, crawl_id: Option<i64>) -> Result<(Vec<CrawledUrl>, i64, &'static str), String> {
    let id = match crawl_id {
        Some(id) => id,
        None => {
            let (list, _) = crawls(project).await?;
            pick_crawl(&list).ok_or_else(|| format!("Spider project \"{project}\" has no crawls"))?
        }
    };
    if let Ok(resp) = api_get(&format!("/crawl/{id}/export/urls.csv?project={}", encode(project))).await {
        if let Ok(text) = resp.text().await {
            return Ok((parse_csv(&text)?, id, "api"));
        }
    }
    let path = project_file(project)?;
    let sql = format!(
        "SELECT url, final_url, status_code, content_type, indexability, indexability_reason, title, meta_description, h1, canonical, meta_robots, word_count, depth, response_time_ms FROM urls WHERE crawl_id = {id} ORDER BY id"
    );
    let rows = duckdb_json(&path, &sql).await?;
    let list = serde_json::from_value::<Vec<SpiderRow>>(Value::Array(rows)).map_err(|e| e.to_string())?;
    Ok((list.into_iter().map(Into::into).collect(), id, "files"))
}

#[derive(Deserialize)]
struct SpiderRow {
    url: String,
    final_url: Option<String>,
    status_code: Option<i64>,
    content_type: Option<String>,
    indexability: Option<String>,
    indexability_reason: Option<String>,
    title: Option<String>,
    meta_description: Option<String>,
    h1: Option<String>,
    canonical: Option<String>,
    meta_robots: Option<String>,
    word_count: Option<i64>,
    depth: Option<i64>,
    response_time_ms: Option<f64>,
}

impl From<SpiderRow> for CrawledUrl {
    fn from(r: SpiderRow) -> Self {
        CrawledUrl {
            url: r.url,
            final_url: r.final_url,
            status_code: r.status_code,
            content_type: r.content_type,
            indexability: r.indexability,
            indexability_reason: r.indexability_reason,
            title: r.title,
            meta_description: r.meta_description,
            h1: r.h1,
            canonical: r.canonical,
            meta_robots: r.meta_robots,
            word_count: r.word_count,
            depth: r.depth,
            response_time_ms: r.response_time_ms,
        }
    }
}

fn project_file(project: &str) -> Result<std::path::PathBuf, String> {
    let path = super::paths::spider_home().join(format!("{project}.seobify"));
    if path.exists() {
        Ok(path)
    } else {
        Err(format!("Spider project file not found: {} (is Seobify Spider installed?)", path.display()))
    }
}

/// Runs a read-only query with the DuckDB CLI (`SEOBIFY_DUCKDB` or `duckdb` on PATH).
pub async fn duckdb_json(db: &Path, sql: &str) -> Result<Vec<Value>, String> {
    let db = db.to_path_buf();
    let sql = sql.to_string();
    tokio::task::spawn_blocking(move || {
        let exe = std::env::var("SEOBIFY_DUCKDB").unwrap_or_else(|_| "duckdb".into());
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("-readonly").arg("-json").arg(&db).arg("-c").arg(&sql);
        no_window(&mut cmd);
        let output = cmd.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "DuckDB CLI not found. Start Seobify Spider (its API is used first), install DuckDB (winget install DuckDB.cli) or import a CSV export.".to_string()
            } else {
                format!("duckdb: {e}")
            }
        })?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            if err.contains("already open") || err.contains("being used by another process") || err.contains("başka bir işlem") {
                return Err("The Spider project file is locked by a running Spider engine that did not answer on its API. Restart Seobify Spider or import a CSV export.".into());
            }
            return Err(format!("duckdb failed: {}", err.trim()));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let text = text.trim();
        if text.is_empty() {
            return Ok(Vec::new());
        }
        match serde_json::from_str::<Value>(text).map_err(|e| format!("duckdb JSON: {e}"))? {
            Value::Array(rows) => Ok(rows),
            other => Ok(vec![other]),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Hides the console window of child processes when the GUI runs them (Windows).
pub fn no_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

fn encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

// ---------------------------------------------------------------- files, CSV, lists, sitemaps

pub async fn read_file(path: &Path) -> Result<Vec<CrawledUrl>, String> {
    let bytes = tokio::fs::read(path).await.map_err(|e| format!("{}: {e}", path.display()))?;
    let bytes = maybe_gunzip(bytes)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_lowercase();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if name.ends_with(".xml") || name.ends_with(".xml.gz") || trimmed.starts_with("<?xml") || trimmed.starts_with("<urlset") || trimmed.starts_with("<sitemapindex") {
        let (is_index, locs) = parse_sitemap_xml(&bytes)?;
        if is_index {
            let mut all = Vec::new();
            for child in locs {
                all.extend(fetch_sitemap(&child, 50_000).await?);
            }
            return Ok(dedupe(all).into_iter().map(CrawledUrl::bare).collect());
        }
        return Ok(dedupe(locs).into_iter().map(CrawledUrl::bare).collect());
    }
    if name.ends_with(".csv") || name.ends_with(".tsv") || looks_like_csv(trimmed) {
        return parse_csv(&text);
    }
    Ok(parse_text_list(&text).into_iter().map(CrawledUrl::bare).collect())
}

fn looks_like_csv(text: &str) -> bool {
    let first = text.lines().next().unwrap_or_default().to_lowercase();
    (first.contains(',') || first.contains(';') || first.contains('\t')) && !first.starts_with("http")
}

fn maybe_gunzip(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    if bytes.len() > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        let mut out = Vec::new();
        flate2::read::MultiGzDecoder::new(&bytes[..]).read_to_end(&mut out).map_err(|e| format!("gzip: {e}"))?;
        Ok(out)
    } else {
        Ok(bytes)
    }
}

fn norm_header(h: &str) -> String {
    h.trim_start_matches('\u{feff}')
        .trim()
        .to_lowercase()
        .split(|c: char| c == '_' || c == '-' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

const URL_HEADERS: &[&str] = &["url", "address", "loc", "page", "pages", "top pages", "landing page", "adres", "sayfa", "en çok tıklanan sayfalar", "source url", "from"];

/// Parses CSV/TSV/semicolon exports with flexible headers (Spider, Screaming Frog, GSC).
pub fn parse_csv(text: &str) -> Result<Vec<CrawledUrl>, String> {
    let text = text.trim_start_matches('\u{feff}');
    let first_line = text.lines().next().unwrap_or_default();
    let delimiter = [b',', b';', b'\t']
        .into_iter()
        .max_by_key(|d| first_line.bytes().filter(|b| b == d).count())
        .unwrap_or(b',');
    let mut reader = csv::ReaderBuilder::new().delimiter(delimiter).flexible(true).from_reader(text.as_bytes());
    let headers: Vec<String> = reader.headers().map_err(|e| format!("CSV header: {e}"))?.iter().map(norm_header).collect();
    let find = |aliases: &[&str]| headers.iter().position(|h| aliases.contains(&h.as_str()));
    let url_col = find(URL_HEADERS)
        .ok_or_else(|| format!("CSV has no URL column (expected one of: url, Address, loc, page). Found: {}", headers.join(", ")))?;
    let col = |aliases: &[&str]| find(aliases);
    let c_final = col(&["final url", "redirect url", "redirect uri"]);
    let c_status = col(&["status code", "status", "durum kodu"]);
    let c_ctype = col(&["content type"]);
    let c_index = col(&["indexability", "indexable"]);
    let c_index_reason = col(&["indexability reason", "indexability status"]);
    let c_title = col(&["title", "title 1", "başlık"]);
    let c_desc = col(&["meta description", "meta description 1", "description"]);
    let c_h1 = col(&["h1", "h1 1"]);
    let c_canonical = col(&["canonical", "canonical link element 1", "canonical url"]);
    let c_robots = col(&["meta robots", "meta robots 1", "robots"]);
    let c_words = col(&["word count"]);
    let c_depth = col(&["depth", "crawl depth"]);
    let c_rt = col(&["response time ms", "response time"]);

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for record in reader.records() {
        let record = record.map_err(|e| format!("CSV row: {e}"))?;
        let get = |c: Option<usize>| c.and_then(|i| record.get(i)).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        let Some(url) = get(Some(url_col)) else { continue };
        if !(url.starts_with("http://") || url.starts_with("https://")) || !seen.insert(url.clone()) {
            continue;
        }
        out.push(CrawledUrl {
            url,
            final_url: get(c_final),
            status_code: get(c_status).and_then(|s| s.parse::<f64>().ok()).map(|f| f as i64),
            content_type: get(c_ctype),
            indexability: get(c_index),
            indexability_reason: get(c_index_reason),
            title: get(c_title),
            meta_description: get(c_desc),
            h1: get(c_h1),
            canonical: get(c_canonical),
            meta_robots: get(c_robots),
            word_count: get(c_words).and_then(|s| s.parse::<f64>().ok()).map(|f| f as i64),
            depth: get(c_depth).and_then(|s| s.parse::<f64>().ok()).map(|f| f as i64),
            response_time_ms: get(c_rt).and_then(|s| s.parse().ok()),
        });
    }
    Ok(out)
}

/// One URL per line (anything else is ignored); order kept, duplicates removed.
pub fn parse_text_list(text: &str) -> Vec<String> {
    dedupe(
        text.lines()
            .map(|l| l.trim().trim_start_matches('\u{feff}').trim_matches(|c| c == '"' || c == '\'' || c == ',').to_string())
            .filter(|l| l.starts_with("http://") || l.starts_with("https://"))
            .collect(),
    )
}

fn dedupe(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items.into_iter().filter(|s| seen.insert(s.clone())).collect()
}

/// Parses sitemap XML: `(is_sitemap_index, <loc> values)`. Only a `<loc>` whose parent is
/// `<url>` or `<sitemap>` counts, so extension URLs such as `<image:loc>` are skipped.
pub fn parse_sitemap_xml(bytes: &[u8]) -> Result<(bool, Vec<String>), String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut is_index = false;
    let mut stack: Vec<Vec<u8>> = Vec::new();
    let mut in_loc = false;
    let mut locs = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.local_name().as_ref().to_vec();
                match name.as_slice() {
                    b"sitemapindex" => is_index = true,
                    b"loc" => in_loc = matches!(stack.last().map(Vec::as_slice), Some(b"url" | b"sitemap")),
                    _ => {}
                }
                stack.push(name);
            }
            Ok(Event::End(e)) => {
                if e.local_name().as_ref() == b"loc" {
                    in_loc = false;
                }
                stack.pop();
            }
            Ok(Event::Text(t)) if in_loc => {
                if let Ok(s) = t.unescape() {
                    let s = s.trim().to_string();
                    if !s.is_empty() {
                        locs.push(s);
                    }
                }
            }
            Ok(Event::CData(t)) if in_loc => locs.push(String::from_utf8_lossy(&t).trim().to_string()),
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("sitemap XML at {}: {e}", reader.buffer_position())),
            _ => {}
        }
        buf.clear();
    }
    Ok((is_index, locs))
}

/// Fetches a sitemap or sitemap index (recursively, gzip aware) up to `max_urls` page URLs.
pub async fn fetch_sitemap(url: &str, max_urls: usize) -> Result<Vec<String>, String> {
    let client = super::http::client();
    let mut queue = vec![(url.to_string(), 0u8)];
    let mut visited = HashSet::new();
    let mut pages = Vec::new();
    while let Some((sitemap, depth)) = queue.pop() {
        if !visited.insert(sitemap.clone()) || depth > 4 {
            continue;
        }
        let resp = client.get(&sitemap).send().await.map_err(|e| format!("{sitemap}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("{sitemap}: HTTP {}", resp.status()));
        }
        let bytes = maybe_gunzip(resp.bytes().await.map_err(|e| e.to_string())?.to_vec())?;
        let (is_index, locs) = parse_sitemap_xml(&bytes)?;
        if is_index {
            queue.extend(locs.into_iter().map(|l| (l, depth + 1)));
        } else {
            pages.extend(locs);
            if pages.len() >= max_urls {
                pages.truncate(max_urls);
                break;
            }
        }
    }
    Ok(dedupe(pages))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spider_csv_with_bom() {
        let csv = "\u{feff}url,final_url,status_code,indexability,title,meta_description,h1,canonical,meta_robots,word_count\nhttps://a.com/,https://a.com/,200,indexable,Home,Desc,H,https://a.com/,\"index, follow\",120\nhttps://a.com/x,,404,non-indexable,,,,,,\n";
        let rows = parse_csv(csv).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].status_code, Some(200));
        assert_eq!(rows[0].meta_robots.as_deref(), Some("index, follow"));
        assert_eq!(rows[1].title, None);
    }

    #[test]
    fn parses_screaming_frog_and_semicolon() {
        let sf = "Address;Status Code;Title 1;Indexability\nhttps://b.com/;200;B;Indexable\n";
        let rows = parse_csv(sf).unwrap();
        assert_eq!(rows[0].url, "https://b.com/");
        assert_eq!(rows[0].title.as_deref(), Some("B"));
        assert!(parse_csv("name,size\nx,1\n").is_err());
    }

    #[test]
    fn text_list_and_sitemaps() {
        assert_eq!(parse_text_list("https://a.com\n# c\nhttps://a.com\nhttp://b.com/x\n"), vec!["https://a.com", "http://b.com/x"]);
        let idx = br#"<?xml version="1.0"?><sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><sitemap><loc>https://a.com/s1.xml</loc></sitemap></sitemapindex>"#;
        assert_eq!(parse_sitemap_xml(idx).unwrap(), (true, vec!["https://a.com/s1.xml".to_string()]));
        let set = br#"<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"><url><loc>https://a.com/&amp;q</loc></url><url><loc><![CDATA[https://a.com/c]]></loc></url></urlset>"#;
        assert_eq!(parse_sitemap_xml(set).unwrap().1, vec!["https://a.com/&q", "https://a.com/c"]);
    }

    #[test]
    fn image_sitemap_returns_only_page_urls() {
        let set = br#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:image="http://www.google.com/schemas/sitemap-image/1.1" xmlns:xhtml="http://www.w3.org/1999/xhtml">
  <url>
    <loc>https://a.com/</loc>
    <xhtml:link rel="alternate" hreflang="en" href="https://a.com/en/"/>
    <image:image><image:loc>https://a.com/assets/hero.webp</image:loc></image:image>
    <image:image><image:loc>https://a.com/assets/logo.webp</image:loc></image:image>
  </url>
  <url><image:image><image:loc>https://a.com/assets/x.webp</image:loc></image:image><loc>https://a.com/urunler</loc></url>
</urlset>"#;
        assert_eq!(parse_sitemap_xml(set).unwrap(), (false, vec!["https://a.com/".to_string(), "https://a.com/urunler".to_string()]));
    }

    #[test]
    fn sitemap_index_children_are_returned() {
        let idx = br#"<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <sitemap><loc>https://a.com/pages.xml</loc><lastmod>2026-10-01</lastmod></sitemap>
  <sitemap><loc>https://a.com/posts.xml.gz</loc></sitemap>
</sitemapindex>"#;
        assert_eq!(parse_sitemap_xml(idx).unwrap(), (true, vec!["https://a.com/pages.xml".to_string(), "https://a.com/posts.xml.gz".to_string()]));
    }

    #[test]
    fn picks_newest_completed_crawl() {
        let c = |id, s: &str| SpiderCrawl { id, name: None, start_url: None, status: Some(s.into()), urls_done: None, created_at: None, finished_at: None };
        assert_eq!(pick_crawl(&[c(3, "running"), c(2, "completed"), c(1, "completed")]), Some(2));
        assert_eq!(pick_crawl(&[c(5, "failed")]), Some(5));
        assert_eq!(pick_crawl(&[]), None);
    }
}
