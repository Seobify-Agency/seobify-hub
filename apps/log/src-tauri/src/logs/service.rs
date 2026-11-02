//! Async entry points shared by the Tauri commands and the MCP tools (one Rust function per
//! capability, so the UI and Claude always see the same numbers).

use super::cloudflare;
use super::gaps::{self, SpiderPage};
use super::import::{self, ImportOptions, Progress};
use super::parse::Format;
use super::query::{self, Range, UrlFilter};
use super::report::{self, SpiderPart};
use super::verify::{self, RangeSet, Verifier};
use super::waste::{self, Scope};
use crate::hub::db::{self, Project};
use crate::hub::mcp::blocking;
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

fn find(conn: &rusqlite::Connection, key: &str) -> Result<Project, String> {
    db::find_project(conn, key)?.ok_or_else(|| format!("Project not found: {key} (see list_projects)"))
}

async fn project(key: &str) -> Result<Project, String> {
    let key = key.to_string();
    blocking(move || find(&db::open()?, &key)).await
}

/// Runs a read-only query against a project and a day range.
async fn with_range<F>(key: String, from: Option<String>, to: Option<String>, f: F) -> Result<Value, String>
where
    F: FnOnce(&rusqlite::Connection, &Project, &Range) -> Result<Value, String> + Send + 'static,
{
    blocking(move || {
        let conn = db::open()?;
        let p = find(&conn, &key)?;
        let range = Range::new(from.as_deref(), to.as_deref())?;
        let mut v = f(&conn, &p, &range)?;
        if let Some(obj) = v.as_object_mut() {
            obj.insert("project".into(), json!(p.name));
        }
        Ok(v)
    })
    .await
}

pub async fn import_logs<P>(key: String, paths: Vec<String>, format: Option<String>, force: bool, progress: P) -> Result<Value, String>
where
    P: FnMut(&Progress) + Send + 'static,
{
    let fmt = Format::from_name(format.as_deref().unwrap_or("auto"))?;
    if paths.is_empty() {
        return Err("Give at least one log file or folder path".into());
    }
    // Published crawler IP lists: fetched before the first import, afterwards refreshed in the
    // background at most daily so a slow or offline network never delays an import.
    let ranges_note = if blocking(|| verify::has_all_ranges(&db::open()?)).await? {
        tokio::spawn(async {
            let _ = verify::refresh_ranges(false).await;
        });
        None
    } else {
        verify::refresh_ranges(false).await.err()
    };
    blocking(move || {
        let mut conn = db::open()?;
        let p = find(&conn, &key)?;
        let settings = db::get_settings(&conn)?;
        let opts = ImportOptions { format: fmt, force, ..ImportOptions::from_settings(&settings) };
        let ranges = RangeSet::load(&conn)?;
        let salt = verify::ip_salt(&conn);
        let mut verifier = Verifier::new(ranges, salt, opts.verify_dns);
        let mut progress = progress;
        let summary = import::import_paths(&mut conn, p.id, &paths, &opts, &mut verifier, &mut progress)?;
        Ok(json!({
            "project": p.name,
            "projectId": p.id,
            "summary": summary,
            "verification": {
                "rangeListsLoaded": verifier.ranges.loaded(),
                "reverseDns": opts.verify_dns,
                "rangeRefreshError": ranges_note,
                "cloudflareEdgeIps": verifier.cloudflare_edge_ips,
                "cloudflareNote": (verifier.cloudflare_edge_ips > 0).then_some("Crawler requests came from Cloudflare edge addresses: this server is behind Cloudflare and logs Cloudflare's IP instead of the visitor's. They are counted as unverifiable. Log CF-Connecting-IP on the server, or use the Cloudflare connection, to verify them."),
            },
            "privacy": { "fakeBotIp": if opts.hash_ips { "salted-hash" } else { "masked" }, "humanTraffic": "daily counters only (no IP, no URL)" },
        }))
    })
    .await
}

pub async fn bounds(key: String) -> Result<Value, String> {
    with_range(key, None, None, |c, p, _| query::bounds(c, p.id)).await
}

pub async fn overview(key: String, from: Option<String>, to: Option<String>) -> Result<Value, String> {
    with_range(key, from, to, |c, p, r| {
        let mut v = query::overview(c, p.id, r)?;
        v["bounds"] = query::bounds(c, p.id)?;
        Ok(v)
    })
    .await
}

pub async fn bot_stats(key: String, from: Option<String>, to: Option<String>, category: Option<String>) -> Result<Value, String> {
    with_range(key, from, to, move |c, p, r| query::bot_stats(c, p.id, r, category.as_deref())).await
}

pub async fn bot_detail(key: String, bot_id: String, from: Option<String>, to: Option<String>) -> Result<Value, String> {
    with_range(key, from, to, move |c, p, r| query::bot_detail(c, p.id, &bot_id, r)).await
}

pub async fn urls(key: String, from: Option<String>, to: Option<String>, filter: UrlFilter) -> Result<Value, String> {
    with_range(key, from, to, move |c, p, r| query::urls(c, p.id, r, &filter)).await
}

pub async fn status(key: String, from: Option<String>, to: Option<String>) -> Result<Value, String> {
    with_range(key, from, to, |c, p, r| query::status_report(c, p.id, r)).await
}

pub async fn ai_activity(key: String, from: Option<String>, to: Option<String>) -> Result<Value, String> {
    with_range(key, from, to, |c, p, r| query::ai_activity(c, p.id, r)).await
}

pub async fn fake_bots(key: String, from: Option<String>, to: Option<String>, limit: i64) -> Result<Value, String> {
    with_range(key, from, to, move |c, p, r| query::fake_bots(c, p.id, r, limit)).await
}

pub async fn imports(key: String) -> Result<Value, String> {
    with_range(key, None, None, |c, p, _| query::imports(c, p.id)).await
}

pub async fn delete_import(key: String, file_id: i64) -> Result<Value, String> {
    blocking(move || {
        let mut conn = db::open()?;
        let p = find(&conn, &key)?;
        import::delete_import(&mut conn, p.id, file_id)?;
        Ok(json!({ "ok": true, "deleted": file_id }))
    })
    .await
}

/// Spider pages for a project (`None` when no Spider project is linked, or loading failed).
async fn spider_pages(p: &Project, spider_project: Option<String>, crawl_id: Option<i64>, required: bool) -> Result<Option<(Value, Vec<SpiderPage>)>, String> {
    if !required && spider_project.is_none() && p.spider_project.is_none() {
        return Ok(None);
    }
    match gaps::load_spider(p, spider_project, crawl_id).await {
        Ok(data) => {
            let domain = data.domain(p);
            let pages = gaps::index_spider(&data.urls, domain.as_deref());
            let mut meta = data.meta();
            meta["comparedPages"] = json!(pages.len());
            meta["domain"] = json!(domain);
            Ok(Some((meta, pages)))
        }
        Err(e) if required => Err(e),
        Err(_) => Ok(None),
    }
}

pub async fn crawl_waste(key: String, from: Option<String>, to: Option<String>, scope: Option<String>) -> Result<Value, String> {
    let scope = Scope::from_name(scope.as_deref())?;
    let p = project(&key).await?;
    let spider = spider_pages(&p, None, None, false).await?;
    blocking(move || {
        let conn = db::open()?;
        let range = Range::new(from.as_deref(), to.as_deref())?;
        let mut v = waste::crawl_waste(&conn, p.id, &range, scope, spider.as_ref().map(|s| s.1.as_slice()))?;
        v["project"] = json!(p.name);
        v["spider"] = spider.map(|s| s.0).unwrap_or(Value::Null);
        Ok(v)
    })
    .await
}

pub async fn spider_gaps(key: String, spider_project: Option<String>, crawl_id: Option<i64>, days: Option<i64>, limit: Option<i64>) -> Result<Value, String> {
    let p = project(&key).await?;
    let (meta, pages) = spider_pages(&p, spider_project, crawl_id, true).await?.ok_or("Spider data unavailable")?;
    let days = days.unwrap_or(30);
    let limit = limit.unwrap_or(200).clamp(1, 50_000) as usize;
    blocking(move || {
        let conn = db::open()?;
        let mut v = gaps::spider_gaps(&conn, p.id, &pages, days, limit)?;
        v["project"] = json!(p.name);
        v["spider"] = meta;
        Ok(v)
    })
    .await
}

pub async fn robots(key: String) -> Result<Value, String> {
    let p = project(&key).await?;
    let domain = p.domain.clone().unwrap_or_default();
    let mut v = super::robots::fetch_summary(&domain).await?;
    v["project"] = json!(p.name);
    Ok(v)
}

pub async fn generate_report(key: String, from: Option<String>, to: Option<String>, lang: Option<String>, dest: Option<String>) -> Result<Value, String> {
    let p = project(&key).await?;
    let spider = spider_pages(&p, None, None, false).await?;
    let lang = lang.unwrap_or_else(|| "tr".into());
    blocking(move || {
        let conn = db::open()?;
        let range = Range::new(from.as_deref(), to.as_deref())?;
        let part = match spider {
            Some((meta, pages)) => {
                let g = gaps::spider_gaps(&conn, p.id, &pages, 30, 200)?;
                Some(SpiderPart { meta, gaps: g, pages })
            }
            None => None,
        };
        let (r, findings) = report::build(&conn, &p, &range, &lang, part.as_ref())?;
        let path = crate::hub::report::save(&r, dest.as_deref().map(std::path::Path::new))?;
        db::log_activity(&conn, Some(p.id), "report", &format!("Report saved: {}", path.display()))?;
        Ok(json!({
            "project": p.name,
            "path": path.display().to_string(),
            "lang": lang,
            "spiderIncluded": part.is_some(),
            "findings": findings.iter().map(|f| json!({ "severity": f.severity, "code": f.code, "text": f.text(&lang) })).collect::<Vec<_>>(),
        }))
    })
    .await
}

pub async fn ranges(force: bool) -> Result<Value, String> {
    let list = verify::refresh_ranges(force).await?;
    Ok(json!({ "sources": list }))
}

pub async fn range_status() -> Result<Value, String> {
    blocking(|| {
        let conn = db::open()?;
        let cache: i64 = conn.query_row("SELECT COUNT(*) FROM ip_verifications", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        Ok(json!({ "sources": verify::range_status(&conn)?, "dnsCacheEntries": cache }))
    })
    .await
}

pub async fn clear_ip_cache() -> Result<Value, String> {
    blocking(|| {
        let conn = db::open()?;
        let n = conn.execute("DELETE FROM ip_verifications", []).map_err(|e| e.to_string())?;
        Ok(json!({ "deleted": n }))
    })
    .await
}

// ---------------------------------------------------------------- Cloudflare connection

fn cf_token() -> Result<String, String> {
    crate::hub::secrets::get(cloudflare::SECRET)?.ok_or_else(|| "Cloudflare is not connected: add an API token in the app (Imports → Cloudflare connection)".to_string())
}

fn cf_links(conn: &rusqlite::Connection) -> Result<Vec<Value>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT l.project_id, p.name, l.zone_id, l.zone_name, l.last_sync_at, l.last_error,
                    (SELECT COUNT(*) FROM cf_days d WHERE d.project_id = l.project_id),
                    (SELECT MIN(day) FROM cf_days d WHERE d.project_id = l.project_id),
                    (SELECT MAX(day) FROM cf_days d WHERE d.project_id = l.project_id),
                    (SELECT COALESCE(SUM(requests), 0) FROM cf_days d WHERE d.project_id = l.project_id),
                    (SELECT COALESCE(SUM(bot_requests), 0) FROM cf_days d WHERE d.project_id = l.project_id)
             FROM cf_links l JOIN projects p ON p.id = l.project_id ORDER BY p.name",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(json!({
                "projectId": r.get::<_, i64>(0)?, "project": r.get::<_, String>(1)?, "zoneId": r.get::<_, String>(2)?, "zoneName": r.get::<_, String>(3)?,
                "lastSyncAt": r.get::<_, Option<String>>(4)?, "lastError": r.get::<_, Option<String>>(5)?, "days": r.get::<_, i64>(6)?,
                "firstDay": r.get::<_, Option<String>>(7)?, "lastDay": r.get::<_, Option<String>>(8)?, "requests": r.get::<_, i64>(9)?, "botRequests": r.get::<_, i64>(10)?,
            }))
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

/// Connection state: masked token, readable zones and linked projects.
pub async fn cloudflare_status() -> Result<Value, String> {
    let token = blocking(|| crate::hub::secrets::get(cloudflare::SECRET)).await?;
    let links = blocking(|| cf_links(&db::open()?)).await?;
    let Some(token) = token else { return Ok(json!({ "connected": false, "links": links })) };
    let (zones, error) = match cloudflare::zones(&token).await {
        Ok(z) => (z, None),
        Err(e) => (Vec::new(), Some(e)),
    };
    Ok(json!({ "connected": true, "masked": crate::hub::secrets::mask(&token), "zones": zones, "error": error, "links": links }))
}

/// Verifies and stores the user's API token (never returned afterwards).
pub async fn cloudflare_set_token(token: String) -> Result<Value, String> {
    let token = token.trim().to_string();
    if token.len() < 20 || token.contains(char::is_whitespace) {
        return Err("This does not look like a Cloudflare API token".into());
    }
    cloudflare::verify(&token).await?;
    let zones = cloudflare::zones(&token).await.map_err(|e| format!("{e}. The token needs Zone / Zone / Read."))?;
    let first = zones.first().ok_or("The token works but sees no site. Give it access to your zones (Zone resources: All zones).")?;
    cloudflare::limits(&token, &first.id).await.map_err(|e| format!("{e}. The token needs Zone / Analytics / Read."))?;
    blocking(move || crate::hub::secrets::set(cloudflare::SECRET, &token)).await?;
    cloudflare_status().await
}

pub async fn cloudflare_delete_token() -> Result<Value, String> {
    blocking(|| crate::hub::secrets::delete(cloudflare::SECRET)).await?;
    cloudflare_status().await
}

/// Links a project to a zone (id or name). Changing the zone forgets the old day list;
/// already imported days stay until deleted on the Imports page.
pub async fn cloudflare_link(key: String, zone: String) -> Result<Value, String> {
    let token = blocking(cf_token).await?;
    let wanted = zone.trim().to_string();
    let z = cloudflare::zones(&token)
        .await?
        .into_iter()
        .find(|z| z.id == wanted || z.name.eq_ignore_ascii_case(&wanted))
        .ok_or_else(|| format!("Zone not found for this token: {zone}"))?;
    blocking(move || {
        let conn = db::open()?;
        let p = find(&conn, &key)?;
        let old: Option<String> = conn.query_row("SELECT zone_id FROM cf_links WHERE project_id = ?1", [p.id], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
        if old.as_deref().is_some_and(|o| o != z.id) {
            conn.execute("DELETE FROM cf_days WHERE project_id = ?1", [p.id]).map_err(|e| e.to_string())?;
        }
        conn.execute(
            "INSERT INTO cf_links (project_id, zone_id, zone_name, account_id, linked_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(project_id) DO UPDATE SET zone_id = excluded.zone_id, zone_name = excluded.zone_name, account_id = excluded.account_id, linked_at = excluded.linked_at, last_error = NULL",
            rusqlite::params![p.id, z.id, z.name, z.account_id, crate::hub::now_iso()],
        )
        .map_err(|e| e.to_string())?;
        db::log_activity(&conn, Some(p.id), "cloudflare", &format!("Linked to Cloudflare zone {}", z.name))?;
        Ok(json!({ "project": p.name, "zone": z }))
    })
    .await
}

/// Replaces one day's import with freshly fetched analytics.
fn store_day(project_id: i64, zone_name: &str, data: &cloudflare::DayData) -> Result<Value, String> {
    let mut conn = db::open()?;
    let old: Option<Option<i64>> = conn
        .query_row("SELECT file_id FROM cf_days WHERE project_id = ?1 AND day = ?2", rusqlite::params![project_id, data.day], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(Some(fid)) = old {
        let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM log_files WHERE id = ?1)", [fid], |r| r.get(0)).map_err(|e| e.to_string())?;
        if exists {
            import::delete_import(&mut conn, project_id, fid)?;
        }
    }
    let mut file_id = None;
    if !data.rows.is_empty() {
        let path = cloudflare::work_dir().join(format!("cloudflare-{zone_name}-{}.ndjson.gz", data.day));
        cloudflare::write_day_file(data, zone_name, &path)?;
        let settings = db::get_settings(&conn)?;
        let opts = ImportOptions { format: Some(Format::Cloudflare), force: true, ..ImportOptions::from_settings(&settings) };
        let mut verifier = Verifier::new(RangeSet::load(&conn)?, verify::ip_salt(&conn), opts.verify_dns);
        let summary = import::import_paths(&mut conn, project_id, &[path.display().to_string()], &opts, &mut verifier, &mut |_| {});
        let _ = std::fs::remove_file(&path);
        let file = summary?.files.into_iter().next().ok_or("Import produced no result")?;
        if file.status == "error" {
            return Err(file.message.unwrap_or_else(|| "Import failed".into()));
        }
        file_id = file.file_id;
    }
    conn.execute(
        "INSERT OR REPLACE INTO cf_days (project_id, day, file_id, requests, bot_requests, truncated, fetched_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![project_id, data.day, file_id, data.requests as i64, data.bot_requests as i64, data.truncated, crate::hub::now_iso()],
    )
    .map_err(|e| e.to_string())?;
    Ok(json!({ "day": data.day, "requests": data.requests, "botRequests": data.bot_requests, "botUserAgents": data.bot_user_agents, "truncated": data.truncated, "fileId": file_id }))
}

fn cf_record(project_id: i64, error: Option<String>) -> Result<(), String> {
    let conn = db::open()?;
    conn.execute("UPDATE cf_links SET last_sync_at = ?2, last_error = ?3 WHERE project_id = ?1", rusqlite::params![project_id, crate::hub::now_iso(), error]).map_err(|e| e.to_string())?;
    Ok(())
}

/// Fetches missing days (first sync: the last `days`, default 30) plus the two newest days,
/// and imports each as one file; `refresh` re-fetches every day in the window.
/// Progress: `{ project, day, index, total }`.
pub async fn cloudflare_sync<P>(key: String, days: Option<i64>, refresh: bool, mut progress: P) -> Result<Value, String>
where
    P: FnMut(&Value) + Send + 'static,
{
    let token = blocking(cf_token).await?;
    let (p, zone_id, zone_name, have) = blocking(move || {
        let conn = db::open()?;
        let p = find(&conn, &key)?;
        let (zone_id, zone_name): (String, String) = conn
            .query_row("SELECT zone_id, zone_name FROM cf_links WHERE project_id = ?1", [p.id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Project {} is not linked to a Cloudflare zone (pass zone to cloudflare_sync, or link it on the Imports page)", p.name))?;
        let mut stmt = conn
            .prepare("SELECT day, file_id IS NULL OR EXISTS(SELECT 1 FROM log_files f WHERE f.id = cf_days.file_id) FROM cf_days WHERE project_id = ?1")
            .map_err(|e| e.to_string())?;
        let have: std::collections::BTreeMap<String, bool> = stmt
            .query_map([p.id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        Ok((p, zone_id, zone_name, have))
    })
    .await?;
    let pid = p.id;
    let limits = match cloudflare::limits(&token, &zone_id).await {
        Ok(l) if l.enabled => l,
        other => {
            let e = other.err().unwrap_or_else(|| "Cloudflare analytics (httpRequestsAdaptiveGroups) is not available for this zone".into());
            let msg = e.clone();
            blocking(move || cf_record(pid, Some(msg))).await?;
            return Err(e);
        }
    };
    // Crawler IPs are verified against the published ranges: fetch them before the first day.
    if !blocking(|| verify::has_all_ranges(&db::open()?)).await? {
        let _ = verify::refresh_ranges(false).await;
    }
    let first_sync = have.is_empty();
    let have = if refresh { std::collections::BTreeMap::new() } else { have };
    let list = cloudflare::days_to_fetch(chrono::Utc::now().date_naive(), days.unwrap_or(cloudflare::DEFAULT_DAYS), limits.not_older_than_secs, &have);
    let mut results = Vec::new();
    let mut error = None;
    for (i, day) in list.iter().enumerate() {
        progress(&json!({ "project": p.name, "day": day, "index": i, "total": list.len() }));
        let data = match cloudflare::fetch_day(&token, &zone_id, &limits, day).await {
            Ok(d) => d,
            Err(e) => {
                error = Some(format!("{day}: {e}"));
                break;
            }
        };
        let zone = zone_name.clone();
        match blocking(move || store_day(pid, &zone, &data)).await {
            Ok(r) => results.push(r),
            Err(e) => {
                error = Some(format!("{day}: {e}"));
                break;
            }
        }
    }
    let requests: u64 = results.iter().filter_map(|r| r["requests"].as_u64()).sum();
    let bot_requests: u64 = results.iter().filter_map(|r| r["botRequests"].as_u64()).sum();
    let (err, fetched) = (error.clone(), results.len());
    blocking(move || {
        cf_record(pid, err)?;
        db::log_activity(&db::open()?, Some(pid), "cloudflare", &format!("Cloudflare sync: {fetched} days, {requests} requests, {bot_requests} crawler requests"))
    })
    .await?;
    Ok(json!({
        "project": p.name,
        "zone": zone_name,
        "firstSync": first_sync,
        "daysFetched": fetched,
        "requests": requests,
        "botRequests": bot_requests,
        "days": results,
        "error": error,
        "limits": limits,
        "note": if limits.ip_dimension {
            "Cloudflare analytics are sampled estimates. Crawler IPs come from Cloudflare and are verified like server logs; human traffic is counted without IPs."
        } else {
            "Cloudflare analytics are sampled estimates without client IPs on this plan: crawler hits are unverifiable unless Cloudflare verified them; human traffic is counted without IPs."
        },
    }))
}

/// Background catch-up when the app opens: every linked project not synced in the last 6 hours.
pub async fn cloudflare_auto_sync<P>(progress: P) -> Result<Value, String>
where
    P: FnMut(&Value) + Send + Clone + 'static,
{
    if blocking(|| crate::hub::secrets::get(cloudflare::SECRET)).await?.is_none() {
        return Ok(json!({ "synced": [] }));
    }
    let due: Vec<i64> = blocking(|| {
        let conn = db::open()?;
        let cutoff = (chrono::Utc::now() - chrono::Duration::hours(6)).format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let mut stmt = conn.prepare("SELECT project_id FROM cf_links WHERE last_sync_at IS NULL OR last_sync_at < ?1").map_err(|e| e.to_string())?;
        let ids = stmt.query_map([cutoff], |r| r.get(0)).map_err(|e| e.to_string())?.collect::<Result<Vec<i64>, _>>().map_err(|e| e.to_string())?;
        Ok(ids)
    })
    .await?;
    let mut synced = Vec::new();
    for id in due {
        let r = cloudflare_sync(id.to_string(), None, false, progress.clone()).await;
        synced.push(json!({ "projectId": id, "ok": r.is_ok(), "error": r.err() }));
    }
    Ok(json!({ "synced": synced }))
}
