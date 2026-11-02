//! SQLite access for plans, URL sets, patterns, mappings and verification runs.
//! Tables are created by `tools::MIGRATIONS`; child tables cascade from `plans`, which
//! cascades from `projects`.

use super::model::{Candidate, MappingRow, MatchOptions, MatchResult, Pattern, Plan, PlanInput, UrlRow};
use super::norm;
use crate::hub::{self, spider::CrawledUrl};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashMap;

fn e(err: rusqlite::Error) -> String {
    err.to_string()
}

// ---------------------------------------------------------------- plans

const PLAN_COLS: &str = "id, project_id, name, old_base, new_base, staging_base, notes, options_json, created_at, updated_at";

fn row_to_plan(r: &rusqlite::Row) -> rusqlite::Result<Plan> {
    let options: String = r.get(7)?;
    Ok(Plan {
        id: r.get(0)?,
        project_id: r.get(1)?,
        name: r.get(2)?,
        old_base: r.get(3)?,
        new_base: r.get(4)?,
        staging_base: r.get(5)?,
        notes: r.get(6)?,
        options: serde_json::from_str(&options).unwrap_or_default(),
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

/// Validates and normalises a base URL (`eski.com` → `https://eski.com`).
pub fn normalize_base(input: &str, field: &str) -> Result<String, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err(format!("{field} is required (e.g. https://example.com)"));
    }
    let with_scheme = if s.contains("://") { s.to_string() } else { format!("https://{s}") };
    let u = url::Url::parse(&with_scheme).map_err(|_| format!("{field} is not a valid URL: {s}"))?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
        return Err(format!("{field} must be an http(s) URL: {s}"));
    }
    Ok(u.origin().ascii_serialization())
}

fn clean_opt(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn create_plan(conn: &Connection, project_id: i64, input: &PlanInput) -> Result<Plan, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("Plan name is required".into());
    }
    let old_base = normalize_base(&input.old_base, "old_base")?;
    let new_base = normalize_base(&input.new_base, "new_base")?;
    let staging = match clean_opt(input.staging_base.clone()) {
        Some(s) => Some(normalize_base(&s, "staging_base")?),
        None => None,
    };
    let options = serde_json::to_string(&input.options.clone().unwrap_or_default()).map_err(|e| e.to_string())?;
    let now = hub::now_iso();
    conn.execute(
        "INSERT INTO plans (project_id, name, old_base, new_base, staging_base, notes, options_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![project_id, name, old_base, new_base, staging, clean_opt(input.notes.clone()), options, now],
    )
    .map_err(|err| if err.to_string().contains("UNIQUE") { format!("A plan named \"{name}\" already exists in this project") } else { err.to_string() })?;
    let id = conn.last_insert_rowid();
    hub::db::log_activity(conn, Some(project_id), "plan", &format!("Plan created: {name}"))?;
    get_plan(conn, id)?.ok_or_else(|| "plan vanished after insert".into())
}

pub fn update_plan(conn: &Connection, id: i64, input: &PlanInput) -> Result<Plan, String> {
    let current = get_plan(conn, id)?.ok_or("Plan not found")?;
    let name = input.name.trim();
    if name.is_empty() {
        return Err("Plan name is required".into());
    }
    let staging = match clean_opt(input.staging_base.clone()) {
        Some(s) => Some(normalize_base(&s, "staging_base")?),
        None => None,
    };
    let options = input.options.clone().unwrap_or(current.options);
    conn.execute(
        "UPDATE plans SET name = ?2, old_base = ?3, new_base = ?4, staging_base = ?5, notes = ?6, options_json = ?7, updated_at = ?8 WHERE id = ?1",
        params![
            id,
            name,
            normalize_base(&input.old_base, "old_base")?,
            normalize_base(&input.new_base, "new_base")?,
            staging,
            clean_opt(input.notes.clone()),
            serde_json::to_string(&options).map_err(|e| e.to_string())?,
            hub::now_iso()
        ],
    )
    .map_err(|err| if err.to_string().contains("UNIQUE") { format!("A plan named \"{name}\" already exists in this project") } else { err.to_string() })?;
    get_plan(conn, id)?.ok_or_else(|| "Plan not found".into())
}

pub fn set_plan_options(conn: &Connection, id: i64, options: &MatchOptions) -> Result<(), String> {
    conn.execute(
        "UPDATE plans SET options_json = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, serde_json::to_string(options).map_err(|e| e.to_string())?, hub::now_iso()],
    )
    .map_err(e)?;
    Ok(())
}

pub fn touch_plan(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("UPDATE plans SET updated_at = ?2 WHERE id = ?1", params![id, hub::now_iso()]).map_err(e)?;
    Ok(())
}

pub fn get_plan(conn: &Connection, id: i64) -> Result<Option<Plan>, String> {
    conn.query_row(&format!("SELECT {PLAN_COLS} FROM plans WHERE id = ?1"), [id], row_to_plan).optional().map_err(e)
}

pub fn delete_plan(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM plans WHERE id = ?1", [id]).map_err(e)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSummary {
    #[serde(flatten)]
    pub plan: Plan,
    pub old_urls: i64,
    pub new_urls: i64,
    pub mapped: i64,
    pub approved: i64,
    pub patterns: i64,
}

pub fn list_plans(conn: &Connection, project_id: Option<i64>) -> Result<Vec<PlanSummary>, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT {PLAN_COLS} FROM plans WHERE (?1 IS NULL OR project_id = ?1) ORDER BY updated_at DESC, id DESC"))
        .map_err(e)?;
    let plans = stmt.query_map([project_id], row_to_plan).map_err(e)?.collect::<Result<Vec<_>, _>>().map_err(e)?;
    let mut out = Vec::with_capacity(plans.len());
    for plan in plans {
        let count = |sql: &str| -> Result<i64, String> { conn.query_row(sql, [plan.id], |r| r.get(0)).map_err(e) };
        out.push(PlanSummary {
            old_urls: count("SELECT COUNT(*) FROM plan_urls WHERE plan_id = ?1 AND side = 'old'")?,
            new_urls: count("SELECT COUNT(*) FROM plan_urls WHERE plan_id = ?1 AND side = 'new'")?,
            mapped: count("SELECT COUNT(*) FROM mappings WHERE plan_id = ?1")?,
            approved: count("SELECT COUNT(*) FROM mappings WHERE plan_id = ?1 AND approved = 1")?,
            patterns: count("SELECT COUNT(*) FROM patterns WHERE plan_id = ?1")?,
            plan,
        });
    }
    Ok(out)
}

/// Finds a plan by id or name. Names are unique per project; pass `project_id` when the
/// same name exists in several projects.
pub fn find_plan(conn: &Connection, key: &str, project_id: Option<i64>) -> Result<Plan, String> {
    let key = key.trim();
    if let Ok(id) = key.parse::<i64>() {
        if let Some(p) = get_plan(conn, id)? {
            return Ok(p);
        }
    }
    let mut stmt = conn
        .prepare(&format!("SELECT {PLAN_COLS} FROM plans WHERE name = ?1 COLLATE NOCASE AND (?2 IS NULL OR project_id = ?2)"))
        .map_err(e)?;
    let found = stmt.query_map(params![key, project_id], row_to_plan).map_err(e)?.collect::<Result<Vec<_>, _>>().map_err(e)?;
    match found.len() {
        0 => Err(format!("Plan not found: {key} (see list_plans)")),
        1 => Ok(found.into_iter().next().expect("one plan")),
        _ => Err(format!("Several plans are named \"{key}\"; pass the plan id or the project (see list_plans)")),
    }
}

// ---------------------------------------------------------------- URL sets

const URL_COLS: &str = "id, side, url, path, slug, title, h1, meta_description, status_code, indexability, canonical, word_count, simhash, value";

fn row_to_url(r: &rusqlite::Row) -> rusqlite::Result<UrlRow> {
    Ok(UrlRow {
        id: r.get(0)?,
        side: r.get(1)?,
        url: r.get(2)?,
        path: r.get(3)?,
        slug: r.get(4)?,
        title: r.get(5)?,
        h1: r.get(6)?,
        meta_description: r.get(7)?,
        status_code: r.get(8)?,
        indexability: r.get(9)?,
        canonical: r.get(10)?,
        word_count: r.get(11)?,
        simhash: r.get(12)?,
        value: r.get(13)?,
    })
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCounts {
    pub received: usize,
    pub inserted: usize,
    pub updated: usize,
    pub skipped_resources: usize,
    pub total: i64,
}

pub fn check_side(side: &str) -> Result<&'static str, String> {
    match side {
        "old" => Ok("old"),
        "new" => Ok("new"),
        other => Err(format!("side must be \"old\" or \"new\" (got \"{other}\")")),
    }
}

/// `true` for crawled resources that are not pages (scripts, images, JSON…).
pub fn is_resource(u: &CrawledUrl) -> bool {
    u.content_type.as_deref().map_or(false, |ct| {
        let ct = ct.to_ascii_lowercase();
        !ct.is_empty() && !ct.contains("html")
    })
}

/// Upserts URLs into one side. With `replace`, the side (and its mappings) is cleared first.
/// Known metadata is kept when a later import lacks it.
pub fn insert_urls(conn: &mut Connection, plan_id: i64, side: &str, urls: &[CrawledUrl], replace: bool, include_resources: bool) -> Result<ImportCounts, String> {
    let side = check_side(side)?;
    let tx = conn.transaction().map_err(e)?;
    if replace {
        tx.execute("DELETE FROM plan_urls WHERE plan_id = ?1 AND side = ?2", params![plan_id, side]).map_err(e)?;
    }
    let mut counts = ImportCounts { received: urls.len(), ..Default::default() };
    {
        let mut exists = tx.prepare("SELECT 1 FROM plan_urls WHERE plan_id = ?1 AND side = ?2 AND url = ?3").map_err(e)?;
        let mut upsert = tx
            .prepare(
                "INSERT INTO plan_urls (plan_id, side, url, path, slug, title, h1, meta_description, status_code, indexability, canonical, word_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT(plan_id, side, url) DO UPDATE SET
                   title = COALESCE(excluded.title, title), h1 = COALESCE(excluded.h1, h1),
                   meta_description = COALESCE(excluded.meta_description, meta_description),
                   status_code = COALESCE(excluded.status_code, status_code), indexability = COALESCE(excluded.indexability, indexability),
                   canonical = COALESCE(excluded.canonical, canonical), word_count = COALESCE(excluded.word_count, word_count)",
            )
            .map_err(e)?;
        for u in urls {
            if !include_resources && is_resource(u) {
                counts.skipped_resources += 1;
                continue;
            }
            let url = u.url.trim();
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                continue;
            }
            let p = norm::parts(url);
            let clean = norm::clean_path(&p.path);
            let had = exists.exists(params![plan_id, side, url]).map_err(e)?;
            upsert
                .execute(params![
                    plan_id,
                    side,
                    url,
                    p.path,
                    norm::slug_of(&clean),
                    u.title.as_deref().map(str::trim).filter(|s| !s.is_empty()),
                    u.h1.as_deref().map(str::trim).filter(|s| !s.is_empty()),
                    u.meta_description.as_deref().map(str::trim).filter(|s| !s.is_empty()),
                    u.status_code,
                    u.indexability,
                    u.canonical,
                    u.word_count
                ])
                .map_err(e)?;
            if had {
                counts.updated += 1;
            } else {
                counts.inserted += 1;
            }
        }
    }
    counts.total = tx.query_row("SELECT COUNT(*) FROM plan_urls WHERE plan_id = ?1 AND side = ?2", params![plan_id, side], |r| r.get(0)).map_err(e)?;
    tx.execute("UPDATE plans SET updated_at = ?2 WHERE id = ?1", params![plan_id, hub::now_iso()]).map_err(e)?;
    tx.commit().map_err(e)?;
    Ok(counts)
}

pub fn load_urls(conn: &Connection, plan_id: i64, side: &str) -> Result<Vec<UrlRow>, String> {
    let mut stmt = conn.prepare(&format!("SELECT {URL_COLS} FROM plan_urls WHERE plan_id = ?1 AND side = ?2 ORDER BY id")).map_err(e)?;
    let rows = stmt.query_map(params![plan_id, side], row_to_url).map_err(e)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(e)
}

/// URLs of one side, filtered by a search text (URL or title), highest value first.
pub fn list_urls(conn: &Connection, plan_id: i64, side: &str, query: Option<&str>, limit: i64) -> Result<Vec<UrlRow>, String> {
    let q = query.map(|s| format!("%{}%", s.trim())).filter(|s| s.len() > 2);
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {URL_COLS} FROM plan_urls WHERE plan_id = ?1 AND side = ?2 AND (?3 IS NULL OR url LIKE ?3 OR path LIKE ?3 OR title LIKE ?3 OR h1 LIKE ?3)
             ORDER BY COALESCE(value, 0) DESC, id LIMIT ?4"
        ))
        .map_err(e)?;
    let rows = stmt.query_map(params![plan_id, side, q, limit], row_to_url).map_err(e)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(e)
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SideStats {
    pub total: i64,
    pub with_title: i64,
    pub status_200: i64,
    pub non_200: i64,
    pub indexable: i64,
    pub with_value: i64,
    pub with_simhash: i64,
    pub total_value: f64,
}

pub fn side_stats(conn: &Connection, plan_id: i64, side: &str) -> Result<SideStats, String> {
    conn.query_row(
        "SELECT COUNT(*), COUNT(title), SUM(status_code = 200), SUM(status_code IS NOT NULL AND status_code <> 200),
                SUM(LOWER(COALESCE(indexability, '')) = 'indexable'), COUNT(value), COUNT(simhash), COALESCE(SUM(value), 0)
         FROM plan_urls WHERE plan_id = ?1 AND side = ?2",
        params![plan_id, side],
        |r| {
            Ok(SideStats {
                total: r.get(0)?,
                with_title: r.get(1)?,
                status_200: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                non_200: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                indexable: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                with_value: r.get(5)?,
                with_simhash: r.get(6)?,
                total_value: r.get(7)?,
            })
        },
    )
    .map_err(e)
}

pub fn clear_side(conn: &Connection, plan_id: i64, side: &str) -> Result<usize, String> {
    let side = check_side(side)?;
    let n = conn.execute("DELETE FROM plan_urls WHERE plan_id = ?1 AND side = ?2", params![plan_id, side]).map_err(e)?;
    touch_plan(conn, plan_id)?;
    Ok(n)
}

/// Sets priority values on old URLs; matches by exact URL first, then by path key.
/// Returns `(matched, unmatched)`.
pub fn set_values(conn: &mut Connection, plan: &Plan, values: &[(String, f64)]) -> Result<(usize, usize), String> {
    let olds = load_urls(conn, plan.id, "old")?;
    let by_url: HashMap<&str, i64> = olds.iter().map(|u| (u.url.as_str(), u.id)).collect();
    let by_key: HashMap<String, i64> = olds.iter().map(|u| (norm::match_key(&u.url, &plan.options), u.id)).collect();
    let tx = conn.transaction().map_err(e)?;
    let (mut matched, mut unmatched) = (0, 0);
    {
        let mut stmt = tx.prepare("UPDATE plan_urls SET value = ?2 WHERE id = ?1").map_err(e)?;
        for (url, value) in values {
            let id = by_url.get(url.trim()).copied().or_else(|| {
                let full = if url.trim().starts_with("http") { url.trim().to_string() } else { norm::resolve(url, &plan.old_base) };
                by_key.get(&norm::match_key(&full, &plan.options)).copied()
            });
            match id {
                Some(id) => {
                    stmt.execute(params![id, value]).map_err(e)?;
                    matched += 1;
                }
                None => unmatched += 1,
            }
        }
    }
    tx.commit().map_err(e)?;
    Ok((matched, unmatched))
}

pub fn update_content(conn: &Connection, url_id: i64, title: Option<&str>, h1: Option<&str>, simhash: Option<&str>, status: Option<i64>, word_count: Option<i64>) -> Result<(), String> {
    conn.execute(
        "UPDATE plan_urls SET title = COALESCE(title, ?2), h1 = COALESCE(h1, ?3), simhash = COALESCE(?4, simhash),
                status_code = COALESCE(status_code, ?5), word_count = COALESCE(word_count, ?6) WHERE id = ?1",
        params![url_id, title, h1, simhash, status, word_count],
    )
    .map_err(e)?;
    Ok(())
}

// ---------------------------------------------------------------- patterns

fn row_to_pattern(r: &rusqlite::Row) -> rusqlite::Result<Pattern> {
    Ok(Pattern { id: r.get(0)?, plan_id: r.get(1)?, position: r.get(2)?, regex: r.get(3)?, replacement: r.get(4)?, enabled: r.get::<_, i64>(5)? != 0, note: r.get(6)? })
}

pub fn list_patterns(conn: &Connection, plan_id: i64) -> Result<Vec<Pattern>, String> {
    let mut stmt = conn.prepare("SELECT id, plan_id, position, regex, replacement, enabled, note FROM patterns WHERE plan_id = ?1 ORDER BY position, id").map_err(e)?;
    let rows = stmt.query_map([plan_id], row_to_pattern).map_err(e)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(e)
}

pub fn validate_pattern(regex: &str, replacement: &str) -> Result<(), String> {
    if regex.trim().is_empty() {
        return Err("regex is required, e.g. ^/blog/\\d{4}/\\d{2}/(.+)$".into());
    }
    if replacement.trim().is_empty() {
        return Err("replacement is required, e.g. /makale/$1".into());
    }
    let re = regex::Regex::new(regex).map_err(|err| format!("Invalid regex: {err}"))?;
    let names: Vec<Option<String>> = re.capture_names().map(|n| n.map(str::to_string)).collect();
    for part in norm::parse_replacement(replacement, &names) {
        if let norm::RepPart::Group(g) = part {
            if g >= re.captures_len() {
                return Err(format!("The replacement uses ${g} but the regex has only {} capture group(s)", re.captures_len() - 1));
            }
        }
    }
    Ok(())
}

pub fn add_pattern(conn: &Connection, plan_id: i64, regex: &str, replacement: &str, note: Option<&str>) -> Result<Pattern, String> {
    validate_pattern(regex, replacement)?;
    let position: i64 = conn.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM patterns WHERE plan_id = ?1", [plan_id], |r| r.get(0)).map_err(e)?;
    conn.execute(
        "INSERT INTO patterns (plan_id, position, regex, replacement, enabled, note, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6)",
        params![plan_id, position, regex.trim(), replacement.trim(), note.map(str::trim).filter(|s| !s.is_empty()), hub::now_iso()],
    )
    .map_err(e)?;
    let id = conn.last_insert_rowid();
    touch_plan(conn, plan_id)?;
    conn.query_row("SELECT id, plan_id, position, regex, replacement, enabled, note FROM patterns WHERE id = ?1", [id], row_to_pattern).map_err(e)
}

pub fn update_pattern(conn: &Connection, id: i64, regex: &str, replacement: &str, enabled: bool, note: Option<&str>) -> Result<(), String> {
    validate_pattern(regex, replacement)?;
    conn.execute(
        "UPDATE patterns SET regex = ?2, replacement = ?3, enabled = ?4, note = ?5 WHERE id = ?1",
        params![id, regex.trim(), replacement.trim(), enabled as i64, note.map(str::trim).filter(|s| !s.is_empty())],
    )
    .map_err(e)?;
    Ok(())
}

pub fn delete_pattern(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM patterns WHERE id = ?1", [id]).map_err(e)?;
    Ok(())
}

/// Moves a pattern up (`delta < 0`) or down and renumbers positions 1..n.
pub fn move_pattern(conn: &mut Connection, id: i64, delta: i64) -> Result<(), String> {
    let plan_id: i64 = conn.query_row("SELECT plan_id FROM patterns WHERE id = ?1", [id], |r| r.get(0)).map_err(e)?;
    let mut list = list_patterns(conn, plan_id)?;
    let Some(i) = list.iter().position(|p| p.id == id) else { return Ok(()) };
    let j = (i as i64 + delta).clamp(0, list.len() as i64 - 1) as usize;
    let item = list.remove(i);
    list.insert(j, item);
    let tx = conn.transaction().map_err(e)?;
    for (k, p) in list.iter().enumerate() {
        tx.execute("UPDATE patterns SET position = ?2 WHERE id = ?1", params![p.id, k as i64 + 1]).map_err(e)?;
    }
    tx.commit().map_err(e)
}

// ---------------------------------------------------------------- mappings

/// Mappings that must survive re-matching: user decisions (approved / manual).
pub fn locked_mappings(conn: &Connection, plan_id: i64) -> Result<HashMap<i64, Option<String>>, String> {
    let mut stmt = conn.prepare("SELECT old_url_id, new_url FROM mappings WHERE plan_id = ?1 AND (approved = 1 OR status = 'manual')").map_err(e)?;
    let rows = stmt.query_map([plan_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))).map_err(e)?;
    rows.collect::<Result<HashMap<_, _>, _>>().map_err(e)
}

pub fn all_mapping_targets(conn: &Connection, plan_id: i64) -> Result<HashMap<i64, Option<String>>, String> {
    let mut stmt = conn.prepare("SELECT old_url_id, new_url FROM mappings WHERE plan_id = ?1").map_err(e)?;
    let rows = stmt.query_map([plan_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))).map_err(e)?;
    rows.collect::<Result<HashMap<_, _>, _>>().map_err(e)
}

pub fn save_results(conn: &mut Connection, plan_id: i64, results: &[MatchResult]) -> Result<(), String> {
    let now = hub::now_iso();
    let tx = conn.transaction().map_err(e)?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO mappings (plan_id, old_url_id, new_url, status, confidence, reason, candidates_json, http_status, approved, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9)
                 ON CONFLICT(plan_id, old_url_id) DO UPDATE SET new_url = excluded.new_url, status = excluded.status, confidence = excluded.confidence,
                   reason = excluded.reason, candidates_json = excluded.candidates_json, http_status = excluded.http_status, approved = 0, updated_at = excluded.updated_at",
            )
            .map_err(e)?;
        for r in results {
            let cands = serde_json::to_string(&r.candidates).map_err(|err| err.to_string())?;
            stmt.execute(params![plan_id, r.old_id, r.new_url, r.status, r.confidence, r.reason, cands, r.http_status, now]).map_err(e)?;
        }
    }
    tx.execute("UPDATE plans SET updated_at = ?2 WHERE id = ?1", params![plan_id, now]).map_err(e)?;
    tx.commit().map_err(e)
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MappingFilter {
    pub status: Option<String>,
    pub min_confidence: Option<i64>,
    pub max_confidence: Option<i64>,
    /// Old path prefix (directory), e.g. `/blog`.
    pub directory: Option<String>,
    pub query: Option<String>,
    pub approved: Option<bool>,
    pub limit: Option<i64>,
}

const MAPPING_SELECT: &str = "SELECT m.id, m.old_url_id, o.url, o.title, o.status_code, o.value, m.new_url, n.title, n.status_code,
       m.status, m.confidence, m.reason, m.approved, m.http_status, m.candidates_json, m.updated_at
  FROM mappings m JOIN plan_urls o ON o.id = m.old_url_id
  LEFT JOIN plan_urls n ON n.plan_id = m.plan_id AND n.side = 'new' AND n.url = m.new_url";

fn row_to_mapping(r: &rusqlite::Row) -> rusqlite::Result<MappingRow> {
    let cands: String = r.get(14)?;
    Ok(MappingRow {
        id: r.get(0)?,
        old_url_id: r.get(1)?,
        old_url: r.get(2)?,
        old_title: r.get(3)?,
        old_status: r.get(4)?,
        value: r.get(5)?,
        new_url: r.get(6)?,
        new_title: r.get(7)?,
        new_status: r.get(8)?,
        status: r.get(9)?,
        confidence: r.get(10)?,
        reason: r.get(11)?,
        approved: r.get::<_, i64>(12)? != 0,
        http_status: r.get(13)?,
        candidates: serde_json::from_str::<Vec<Candidate>>(&cands).unwrap_or_default(),
        updated_at: r.get(15)?,
    })
}

/// Filtered mappings (highest value first, then lowest confidence) and the unfiltered total.
pub fn list_mappings(conn: &Connection, plan_id: i64, f: &MappingFilter) -> Result<(Vec<MappingRow>, i64), String> {
    let dir = f.directory.as_deref().map(str::trim).filter(|d| !d.is_empty() && *d != "/").map(|d| format!("{}/%", d.trim_end_matches('/')));
    let q = f.query.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| format!("%{s}%"));
    let sql = format!(
        "{MAPPING_SELECT}
         WHERE m.plan_id = ?1 AND (?2 IS NULL OR m.status = ?2) AND (?3 IS NULL OR m.confidence >= ?3) AND (?4 IS NULL OR m.confidence <= ?4)
           AND (?5 IS NULL OR o.path LIKE ?5 OR o.path = SUBSTR(?5, 1, LENGTH(?5) - 2))
           AND (?6 IS NULL OR o.url LIKE ?6 OR o.title LIKE ?6 OR m.new_url LIKE ?6 OR n.title LIKE ?6)
           AND (?7 IS NULL OR m.approved = ?7)
         ORDER BY COALESCE(o.value, 0) DESC, m.confidence ASC, o.url LIMIT ?8"
    );
    let mut stmt = conn.prepare(&sql).map_err(e)?;
    let rows = stmt
        .query_map(
            params![plan_id, f.status.as_deref().filter(|s| !s.is_empty() && *s != "all"), f.min_confidence, f.max_confidence, dir, q, f.approved.map(|b| b as i64), f.limit.unwrap_or(100_000)],
            row_to_mapping,
        )
        .map_err(e)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(e)?;
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM mappings WHERE plan_id = ?1", [plan_id], |r| r.get(0)).map_err(e)?;
    Ok((rows, total))
}

pub fn get_mapping(conn: &Connection, id: i64) -> Result<Option<MappingRow>, String> {
    conn.query_row(&format!("{MAPPING_SELECT} WHERE m.id = ?1"), [id], row_to_mapping).optional().map_err(e)
}

/// Resolves an old URL reference (mapping id, old URL id, full URL or path) to the old URL id.
pub fn find_old_url(conn: &Connection, plan: &Plan, reference: &str) -> Result<i64, String> {
    let r = reference.trim();
    if r.is_empty() {
        return Err("old_url is required".into());
    }
    if let Some(id) = conn.query_row("SELECT id FROM plan_urls WHERE plan_id = ?1 AND side = 'old' AND url = ?2", params![plan.id, r], |row| row.get::<_, i64>(0)).optional().map_err(e)? {
        return Ok(id);
    }
    let full = if r.starts_with("http") { r.to_string() } else { norm::resolve(r, &plan.old_base) };
    let key = norm::match_key(&full, &plan.options);
    let olds = load_urls(conn, plan.id, "old")?;
    olds.iter()
        .find(|u| norm::match_key(&u.url, &plan.options) == key)
        .map(|u| u.id)
        .ok_or_else(|| format!("Old URL not found in plan \"{}\": {r} (see list_mappings)", plan.name))
}

pub fn mapping_id_for_old(conn: &Connection, plan_id: i64, old_url_id: i64) -> Result<Option<i64>, String> {
    conn.query_row("SELECT id FROM mappings WHERE plan_id = ?1 AND old_url_id = ?2", params![plan_id, old_url_id], |r| r.get(0)).optional().map_err(e)
}

/// Writes a user decision. `action`: `manual` (needs `new_url`), `gone`, `keep`, `approve`, `unapprove`.
pub fn apply_decision(conn: &Connection, plan: &Plan, old_url_id: i64, action: &str, new_url: Option<&str>) -> Result<i64, String> {
    let now = hub::now_iso();
    let old_url: String = conn.query_row("SELECT url FROM plan_urls WHERE id = ?1 AND plan_id = ?2", params![old_url_id, plan.id], |r| r.get(0)).map_err(|_| "Old URL not found".to_string())?;
    let existing = mapping_id_for_old(conn, plan.id, old_url_id)?;
    let upsert = |new_url: Option<String>, status: &str, confidence: i64, reason: &str, http: i64, approved: i64| -> Result<(), String> {
        conn.execute(
            "INSERT INTO mappings (plan_id, old_url_id, new_url, status, confidence, reason, candidates_json, http_status, approved, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, '[]', ?7, ?8, ?9)
             ON CONFLICT(plan_id, old_url_id) DO UPDATE SET new_url = excluded.new_url, status = excluded.status, confidence = excluded.confidence,
               reason = excluded.reason, http_status = excluded.http_status, approved = excluded.approved, updated_at = excluded.updated_at",
            params![plan.id, old_url_id, new_url, status, confidence, reason, http, approved, now],
        )
        .map_err(e)?;
        Ok(())
    };
    match action {
        "manual" => {
            let t = new_url.map(str::trim).filter(|s| !s.is_empty()).ok_or("new_url is required for a manual mapping")?;
            let target = norm::resolve(t, &plan.new_base);
            if norm::same_url(&old_url, &target) {
                upsert(Some(target), "keep", 100, "manual_keep", 301, 1)?;
            } else {
                upsert(Some(target), "manual", 100, "manual", 301, 1)?;
            }
        }
        "gone" => upsert(None, "gone", 100, "manual_gone", 410, 1)?,
        "keep" => upsert(Some(old_url.clone()), "keep", 100, "manual_keep", 301, 1)?,
        "approve" | "unapprove" => {
            let flag = (action == "approve") as i64;
            match existing {
                Some(id) => {
                    conn.execute("UPDATE mappings SET approved = ?2, updated_at = ?3 WHERE id = ?1", params![id, flag, now]).map_err(e)?;
                }
                None => return Err("This URL has no mapping yet; run auto_match first or set new_url".into()),
            }
        }
        other => return Err(format!("Unknown status \"{other}\" (use manual, gone, keep, approve or unapprove)")),
    }
    touch_plan(conn, plan.id)?;
    mapping_id_for_old(conn, plan.id, old_url_id)?.ok_or_else(|| "mapping not saved".into())
}

/// Approves mappings by id list and/or by minimum confidence (statuses with a target).
pub fn approve_bulk(conn: &Connection, plan_id: i64, ids: &[i64], min_confidence: Option<i64>) -> Result<usize, String> {
    let now = hub::now_iso();
    let mut n = 0;
    for id in ids {
        n += conn.execute("UPDATE mappings SET approved = 1, updated_at = ?3 WHERE id = ?1 AND plan_id = ?2 AND approved = 0", params![id, plan_id, now]).map_err(e)?;
    }
    if let Some(min) = min_confidence {
        n += conn
            .execute(
                "UPDATE mappings SET approved = 1, updated_at = ?3 WHERE plan_id = ?1 AND approved = 0 AND confidence >= ?2 AND status IN ('auto','review','weak','keep')",
                params![plan_id, min, now],
            )
            .map_err(e)?;
    }
    touch_plan(conn, plan_id)?;
    Ok(n)
}

pub fn set_target(conn: &Connection, mapping_id: i64, new_url: &str, reason: &str) -> Result<(), String> {
    conn.execute("UPDATE mappings SET new_url = ?2, reason = ?3, updated_at = ?4 WHERE id = ?1", params![mapping_id, new_url, reason, hub::now_iso()]).map_err(e)?;
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MappingStats {
    pub total: i64,
    pub by_status: HashMap<String, i64>,
    pub by_reason: HashMap<String, i64>,
    pub approved: i64,
    pub unmapped: i64,
}

pub fn mapping_stats(conn: &Connection, plan_id: i64) -> Result<MappingStats, String> {
    let mut s = MappingStats::default();
    let mut stmt = conn.prepare("SELECT status, reason, approved, COUNT(*) FROM mappings WHERE plan_id = ?1 GROUP BY status, reason, approved").map_err(e)?;
    let rows = stmt.query_map([plan_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?))).map_err(e)?;
    for row in rows {
        let (status, reason, approved, n) = row.map_err(e)?;
        s.total += n;
        *s.by_status.entry(status).or_default() += n;
        let reason = reason.split(':').next().unwrap_or_default().to_string();
        *s.by_reason.entry(reason).or_default() += n;
        if approved == 1 {
            s.approved += n;
        }
    }
    let olds: i64 = conn.query_row("SELECT COUNT(*) FROM plan_urls WHERE plan_id = ?1 AND side = 'old'", [plan_id], |r| r.get(0)).map_err(e)?;
    s.unmapped = (olds - s.total).max(0);
    Ok(s)
}

// ---------------------------------------------------------------- verification runs

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyRun {
    pub id: i64,
    pub plan_id: i64,
    pub kind: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub total: i64,
    pub ok: i64,
    pub failed: i64,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyResultRow {
    pub mapping_id: Option<i64>,
    pub source_url: String,
    pub expected_url: Option<String>,
    pub checked_url: String,
    pub outcome: String,
    pub hops: Vec<super::verify::Hop>,
    pub final_status: Option<i64>,
    pub final_url: Option<String>,
    pub error: Option<String>,
}

pub fn create_run(conn: &Connection, plan_id: i64, kind: &str) -> Result<i64, String> {
    conn.execute("INSERT INTO verify_runs (plan_id, kind, started_at) VALUES (?1, ?2, ?3)", params![plan_id, kind, hub::now_iso()]).map_err(e)?;
    Ok(conn.last_insert_rowid())
}

pub fn finish_run(conn: &mut Connection, run_id: i64, results: &[VerifyResultRow]) -> Result<VerifyRun, String> {
    let tx = conn.transaction().map_err(e)?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO verify_results (run_id, mapping_id, source_url, expected_url, checked_url, outcome, hops_json, final_status, final_url, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )
            .map_err(e)?;
        for r in results {
            stmt.execute(params![
                run_id,
                r.mapping_id,
                r.source_url,
                r.expected_url,
                r.checked_url,
                r.outcome,
                serde_json::to_string(&r.hops).unwrap_or_else(|_| "[]".into()),
                r.final_status,
                r.final_url,
                r.error
            ])
            .map_err(e)?;
        }
    }
    let ok = results.iter().filter(|r| r.outcome == "ok").count() as i64;
    tx.execute(
        "UPDATE verify_runs SET finished_at = ?2, total = ?3, ok = ?4, failed = ?5 WHERE id = ?1",
        params![run_id, hub::now_iso(), results.len() as i64, ok, results.len() as i64 - ok],
    )
    .map_err(e)?;
    tx.commit().map_err(e)?;
    get_run(conn, run_id)?.ok_or_else(|| "run vanished".into())
}

fn row_to_run(r: &rusqlite::Row) -> rusqlite::Result<VerifyRun> {
    Ok(VerifyRun { id: r.get(0)?, plan_id: r.get(1)?, kind: r.get(2)?, started_at: r.get(3)?, finished_at: r.get(4)?, total: r.get(5)?, ok: r.get(6)?, failed: r.get(7)? })
}

pub fn get_run(conn: &Connection, id: i64) -> Result<Option<VerifyRun>, String> {
    conn.query_row("SELECT id, plan_id, kind, started_at, finished_at, total, ok, failed FROM verify_runs WHERE id = ?1", [id], row_to_run).optional().map_err(e)
}

pub fn list_runs(conn: &Connection, plan_id: i64) -> Result<Vec<VerifyRun>, String> {
    let mut stmt = conn.prepare("SELECT id, plan_id, kind, started_at, finished_at, total, ok, failed FROM verify_runs WHERE plan_id = ?1 AND finished_at IS NOT NULL ORDER BY id DESC").map_err(e)?;
    let rows = stmt.query_map([plan_id], row_to_run).map_err(e)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(e)
}

pub fn run_results(conn: &Connection, run_id: i64) -> Result<Vec<VerifyResultRow>, String> {
    let mut stmt = conn
        .prepare("SELECT mapping_id, source_url, expected_url, checked_url, outcome, hops_json, final_status, final_url, error FROM verify_results WHERE run_id = ?1 ORDER BY outcome = 'ok', id")
        .map_err(e)?;
    let rows = stmt
        .query_map([run_id], |r| {
            let hops: String = r.get(5)?;
            Ok(VerifyResultRow {
                mapping_id: r.get(0)?,
                source_url: r.get(1)?,
                expected_url: r.get(2)?,
                checked_url: r.get(3)?,
                outcome: r.get(4)?,
                hops: serde_json::from_str(&hops).unwrap_or_default(),
                final_status: r.get(6)?,
                final_url: r.get(7)?,
                error: r.get(8)?,
            })
        })
        .map_err(e)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(e)
}
