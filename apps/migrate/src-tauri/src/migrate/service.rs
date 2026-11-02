//! Operations shared by the Tauri commands and the MCP tools (one implementation, two doors).
//! Synchronous functions take a connection; async ones (network work) open their own.

use super::matcher::{self, MatchInput, PatternSuggestion};
use super::model::{MappingRow, MatchOptions, Plan, UrlRow};
use super::rules::{self, ExportOptions, ExportOutput, PatternRule, RuleEntry, RuleSet};
use super::store::{self, MappingFilter, MappingStats, SideStats, VerifyResultRow, VerifyRun};
use super::validate::{self, VMap, Validation};
use super::verify::{self, Job};
use super::{fetch, norm, report, Progress};
use crate::hub::{self, db, mcp::blocking, spider};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub fn plan_or_err(conn: &Connection, plan_id: i64) -> Result<Plan, String> {
    store::get_plan(conn, plan_id)?.ok_or_else(|| format!("Plan {plan_id} not found"))
}

// ---------------------------------------------------------------- overview

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub plan: Plan,
    pub old: SideStats,
    pub new: SideStats,
    pub stats: MappingStats,
    pub patterns: usize,
    pub validation: BTreeMap<String, usize>,
    pub blocking_issues: usize,
    /// Mappings with confidence ≥ 60 (or user decisions) / old URLs.
    pub match_rate: f64,
    /// Approved mappings / mappings.
    pub approval_rate: f64,
    pub last_post: Option<VerifyRun>,
    pub last_staging: Option<VerifyRun>,
}

pub fn overview(conn: &Connection, plan_id: i64) -> Result<Overview, String> {
    let plan = plan_or_err(conn, plan_id)?;
    let stats = store::mapping_stats(conn, plan_id)?;
    let old = store::side_stats(conn, plan_id, "old")?;
    let new = store::side_stats(conn, plan_id, "new")?;
    let v = validate_plan(conn, plan_id)?;
    let runs = store::list_runs(conn, plan_id)?;
    let matched: i64 = stats.by_status.iter().filter(|(s, _)| s.as_str() != "weak").map(|(_, n)| n).sum();
    Ok(Overview {
        match_rate: if old.total > 0 { matched as f64 / old.total as f64 } else { 0.0 },
        approval_rate: if stats.total > 0 { stats.approved as f64 / stats.total as f64 } else { 0.0 },
        patterns: store::list_patterns(conn, plan_id)?.len(),
        validation: v.counts,
        blocking_issues: v.blocking,
        last_post: runs.iter().find(|r| r.kind == "post").cloned(),
        last_staging: runs.iter().find(|r| r.kind == "staging").cloned(),
        plan,
        stats,
        old,
        new,
    })
}

// ---------------------------------------------------------------- import

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub side: String,
    pub source: String,
    #[serde(flatten)]
    pub counts: store::ImportCounts,
    pub stats: SideStats,
}

/// Imports the old or new URL set from any `ImportSource` (Spider crawl, file, sitemap, list).
pub async fn import_urls(plan_id: i64, side: String, source: spider::ImportSource, replace: bool, include_resources: bool) -> Result<ImportSummary, String> {
    store::check_side(&side)?;
    let result = spider::import(&source).await?;
    blocking(move || {
        let mut conn = db::open()?;
        let plan = plan_or_err(&conn, plan_id)?;
        let counts = store::insert_urls(&mut conn, plan_id, &side, &result.urls, replace, include_resources)?;
        let stats = store::side_stats(&conn, plan_id, &side)?;
        db::log_activity(&conn, Some(plan.project_id), "import", &format!("{}: {} {} URLs imported from {}", plan.name, counts.inserted + counts.updated, side, result.source))?;
        Ok(ImportSummary { side, source: result.source, counts, stats })
    })
    .await
}

const VALUE_HEADERS: &[&str] = &[
    "value", "değer", "deger", "clicks", "tıklamalar", "tiklamalar", "traffic", "organic traffic", "sessions", "oturumlar", "referring domains", "ref domains", "referring domains dofollow", "backlinks", "impressions", "gösterimler", "pageviews",
];
const URL_HEADERS: &[&str] = &["url", "address", "page", "pages", "top pages", "landing page", "adres", "sayfa", "en çok tıklanan sayfalar", "source url", "target url", "loc"];

fn parse_number(s: &str) -> Option<f64> {
    let t: String = s.trim().chars().filter(|c| !c.is_whitespace() && *c != '%').collect();
    if t.is_empty() {
        return None;
    }
    let thousands_comma = regex::Regex::new(r"^-?\d{1,3}(,\d{3})+$").expect("static regex");
    let thousands_dot = regex::Regex::new(r"^-?\d{1,3}(\.\d{3})+$").expect("static regex");
    let cleaned = if thousands_comma.is_match(&t) {
        t.replace(',', "")
    } else if thousands_dot.is_match(&t) {
        t.replace('.', "")
    } else if t.contains(',') && t.contains('.') {
        if t.rfind(',') > t.rfind('.') {
            t.replace('.', "").replace(',', ".")
        } else {
            t.replace(',', "")
        }
    } else {
        t.replace(',', ".")
    };
    cleaned.parse().ok()
}

/// Reads a priority CSV (URL + clicks / referring domains / value). Values of repeated URLs add up.
pub fn parse_values_csv(text: &str) -> Result<Vec<(String, f64)>, String> {
    let text = text.trim_start_matches('\u{feff}');
    let first = text.lines().next().unwrap_or_default();
    let delimiter = [b',', b';', b'\t'].into_iter().max_by_key(|d| first.bytes().filter(|b| b == d).count()).unwrap_or(b',');
    let mut reader = csv::ReaderBuilder::new().delimiter(delimiter).flexible(true).from_reader(text.as_bytes());
    let headers: Vec<String> = reader.headers().map_err(|e| format!("CSV header: {e}"))?.iter().map(|h| h.trim().to_lowercase().replace(['_', '-'], " ")).collect();
    let url_col = headers.iter().position(|h| URL_HEADERS.contains(&h.as_str())).ok_or_else(|| format!("The CSV has no URL column (expected url, page, address…). Found: {}", headers.join(", ")))?;
    let mut value_col = headers.iter().position(|h| VALUE_HEADERS.contains(&h.as_str()));
    let records: Vec<csv::StringRecord> = reader.records().collect::<Result<_, _>>().map_err(|e| format!("CSV row: {e}"))?;
    if value_col.is_none() {
        value_col = (0..headers.len()).filter(|&c| c != url_col).find(|&c| records.iter().take(20).all(|r| r.get(c).map_or(true, |v| v.trim().is_empty() || parse_number(v).is_some())));
    }
    let value_col = value_col.ok_or("The CSV has no numeric value column (clicks, referring domains, value…)")?;
    let mut sums: HashMap<String, f64> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for r in &records {
        let (Some(u), Some(v)) = (r.get(url_col), r.get(value_col)) else { continue };
        let u = u.trim();
        let Some(v) = parse_number(v) else { continue };
        if u.is_empty() {
            continue;
        }
        if !sums.contains_key(u) {
            order.push(u.to_string());
        }
        *sums.entry(u.to_string()).or_default() += v;
    }
    Ok(order.into_iter().map(|u| {
        let v = sums[&u];
        (u, v)
    })
    .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValuesSummary {
    pub rows: usize,
    pub matched: usize,
    pub unmatched: usize,
}

pub fn import_values(conn: &mut Connection, plan_id: i64, path: &Path) -> Result<ValuesSummary, String> {
    let plan = plan_or_err(conn, plan_id)?;
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let values = parse_values_csv(&String::from_utf8_lossy(&bytes))?;
    let (matched, unmatched) = store::set_values(conn, &plan, &values)?;
    db::log_activity(conn, Some(plan.project_id), "values", &format!("{}: priority values set on {matched} URLs", plan.name))?;
    Ok(ValuesSummary { rows: values.len(), matched, unmatched })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchSummary {
    pub requested: usize,
    pub updated: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

/// Fetches pages to fill title/H1 and compute simhash (`only_missing`: URLs without title or H1).
pub async fn fetch_content(plan_id: i64, side: Option<String>, only_missing: bool, limit: usize, delay_ms: u64, progress: Option<Progress>) -> Result<FetchSummary, String> {
    let items: Vec<(i64, String)> = blocking(move || {
        let conn = db::open()?;
        let mut items = Vec::new();
        for s in ["old", "new"] {
            if side.as_deref().map_or(false, |x| x != s) {
                continue;
            }
            for u in store::load_urls(&conn, plan_id, s)? {
                if !only_missing || u.title.is_none() || u.h1.is_none() || u.simhash.is_none() {
                    items.push((u.id, u.url));
                }
            }
        }
        items.truncate(limit);
        Ok(items)
    })
    .await?;
    let requested = items.len();
    let fetched = fetch::fetch_many(items, delay_ms, progress).await;
    blocking(move || {
        let conn = db::open()?;
        let (mut updated, mut failed, mut errors) = (0, 0, Vec::new());
        for (id, f) in fetched {
            if let Some(e) = &f.error {
                failed += 1;
                if errors.len() < 20 {
                    errors.push(e.clone());
                }
                if f.status.is_none() {
                    continue;
                }
            }
            store::update_content(&conn, id, f.title.as_deref(), f.h1.as_deref(), f.simhash.as_deref(), f.status, f.word_count)?;
            updated += 1;
        }
        Ok(FetchSummary { requested, updated, failed, errors })
    })
    .await
}

// ---------------------------------------------------------------- matching

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchSummary {
    pub processed: usize,
    pub kept_decisions: usize,
    pub elapsed_ms: u128,
    pub stats: MappingStats,
    pub match_rate: f64,
    pub auto_rate: f64,
}

/// Runs the matching engine. `rematch = false` only maps URLs without a mapping;
/// `rematch = true` recomputes everything except user decisions (approved / manual).
pub fn auto_match(conn: &mut Connection, plan_id: i64, rematch: bool) -> Result<MatchSummary, String> {
    let start = std::time::Instant::now();
    let plan = plan_or_err(conn, plan_id)?;
    let old = store::load_urls(conn, plan_id, "old")?;
    let new = store::load_urls(conn, plan_id, "new")?;
    if old.is_empty() {
        return Err("The old URL set is empty: import it first (import_urls with side \"old\")".into());
    }
    if new.is_empty() {
        return Err("The new URL set is empty: import it first (import_urls with side \"new\")".into());
    }
    let patterns = store::list_patterns(conn, plan_id)?;
    let fixed = if rematch { store::locked_mappings(conn, plan_id)? } else { store::all_mapping_targets(conn, plan_id)? };
    let input = MatchInput { old: &old, new: &new, patterns: &patterns, options: &plan.options, new_base: &plan.new_base, fixed: &fixed };
    let results = matcher::run(&input)?;
    store::save_results(conn, plan_id, &results)?;
    let stats = store::mapping_stats(conn, plan_id)?;
    let total = old.len() as f64;
    let matched: i64 = stats.by_status.iter().filter(|(s, _)| s.as_str() != "weak").map(|(_, n)| n).sum();
    let auto = stats.by_status.get("auto").copied().unwrap_or(0) + stats.by_status.get("keep").copied().unwrap_or(0);
    db::log_activity(conn, Some(plan.project_id), "match", &format!("{}: {} URLs matched", plan.name, results.len()))?;
    Ok(MatchSummary { processed: results.len(), kept_decisions: fixed.len(), elapsed_ms: start.elapsed().as_millis(), match_rate: matched as f64 / total, auto_rate: auto as f64 / total, stats })
}

pub fn list_mappings(conn: &Connection, plan_id: i64, filter: &MappingFilter) -> Result<(Vec<MappingRow>, i64), String> {
    store::list_mappings(conn, plan_id, filter)
}

/// User decision on one old URL (`old_ref`: mapping id, URL or path). `status`: manual,
/// gone, keep, approve, unapprove; with only `new_url` it is a manual mapping.
pub fn set_mapping(conn: &Connection, plan_id: i64, old_ref: &str, new_url: Option<&str>, status: Option<&str>) -> Result<MappingRow, String> {
    let plan = plan_or_err(conn, plan_id)?;
    let old_id = match old_ref.trim().strip_prefix('#').and_then(|s| s.parse::<i64>().ok()) {
        Some(mapping_id) => store::get_mapping(conn, mapping_id)?.map(|m| m.old_url_id).ok_or("Mapping not found")?,
        None => store::find_old_url(conn, &plan, old_ref)?,
    };
    let action = match (status.map(str::trim).filter(|s| !s.is_empty()), new_url.map(str::trim).filter(|s| !s.is_empty())) {
        (Some(s), _) => s.to_string(),
        (None, Some(_)) => "manual".into(),
        (None, None) => return Err("Pass new_url (manual mapping) or status (gone, keep, approve, unapprove)".into()),
    };
    let action = if action == "auto" || action == "review" || action == "weak" { "approve".to_string() } else { action };
    let id = store::apply_decision(conn, &plan, old_id, &action, new_url)?;
    store::get_mapping(conn, id)?.ok_or_else(|| "mapping not found".into())
}

pub fn approve(conn: &Connection, plan_id: i64, ids: &[i64], min_confidence: Option<i64>) -> Result<usize, String> {
    plan_or_err(conn, plan_id)?;
    store::approve_bulk(conn, plan_id, ids, min_confidence)
}

pub fn search_new(conn: &Connection, plan_id: i64, query: &str, limit: i64) -> Result<Vec<UrlRow>, String> {
    store::list_urls(conn, plan_id, "new", Some(query), limit)
}

// ---------------------------------------------------------------- patterns

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternPreview {
    pub matched: usize,
    pub targets_found: usize,
    pub samples: Vec<PreviewRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRow {
    pub old_url: String,
    pub target: String,
    pub exists: bool,
}

pub fn preview_pattern(conn: &Connection, plan_id: i64, regex: &str, replacement: &str, sample_limit: usize) -> Result<PatternPreview, String> {
    store::validate_pattern(regex, replacement)?;
    let plan = plan_or_err(conn, plan_id)?;
    let re = regex::Regex::new(regex).map_err(|e| e.to_string())?;
    let new_keys: HashSet<String> = store::load_urls(conn, plan_id, "new")?.iter().map(|u| norm::match_key(&u.url, &plan.options)).collect();
    let mut p = PatternPreview { matched: 0, targets_found: 0, samples: Vec::new() };
    for u in store::load_urls(conn, plan_id, "old")? {
        let clean = norm::clean_path(&norm::parts(&u.url).path);
        if let Some(out) = norm::apply_pattern(&re, replacement, &clean) {
            let target = norm::resolve(&out, &plan.new_base);
            let exists = new_keys.contains(&norm::match_key(&target, &plan.options));
            p.matched += 1;
            p.targets_found += exists as usize;
            if p.samples.len() < sample_limit {
                p.samples.push(PreviewRow { old_url: u.url, target, exists });
            }
        }
    }
    Ok(p)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternWithCounts {
    #[serde(flatten)]
    pub pattern: super::model::Pattern,
    pub matched: usize,
    pub targets_found: usize,
}

pub fn list_patterns(conn: &Connection, plan_id: i64) -> Result<Vec<PatternWithCounts>, String> {
    store::list_patterns(conn, plan_id)?
        .into_iter()
        .map(|p| {
            let prev = preview_pattern(conn, plan_id, &p.regex, &p.replacement, 0).unwrap_or(PatternPreview { matched: 0, targets_found: 0, samples: Vec::new() });
            Ok(PatternWithCounts { pattern: p, matched: prev.matched, targets_found: prev.targets_found })
        })
        .collect()
}

pub fn suggest_patterns(conn: &Connection, plan_id: i64) -> Result<Vec<PatternSuggestion>, String> {
    let (rows, _) = store::list_mappings(conn, plan_id, &MappingFilter::default())?;
    let pairs: Vec<(String, String)> = rows
        .iter()
        .filter(|m| m.confidence >= 60 && !m.reason.starts_with("pattern") && !matches!(m.reason.as_str(), "same_path" | "same_url" | "parent" | "homepage"))
        .filter_map(|m| m.new_url.clone().map(|n| (m.old_url.clone(), n)))
        .collect();
    let olds: Vec<String> = rows.iter().map(|m| m.old_url.clone()).collect();
    let existing = store::list_patterns(conn, plan_id)?;
    Ok(matcher::suggest_patterns(&pairs, &olds, &existing, 3))
}

// ---------------------------------------------------------------- validation

fn vmaps(conn: &Connection, plan_id: i64) -> Result<Vec<VMap>, String> {
    let (rows, _) = store::list_mappings(conn, plan_id, &MappingFilter::default())?;
    Ok(rows.into_iter().map(|m| VMap { id: m.id, source: m.old_url, target: m.new_url, status: m.status }).collect())
}

pub fn validate_plan(conn: &Connection, plan_id: i64) -> Result<Validation, String> {
    let plan = plan_or_err(conn, plan_id)?;
    let news: Vec<(String, Option<i64>)> = store::load_urls(conn, plan_id, "new")?.into_iter().map(|u| (u.url, u.status_code)).collect();
    Ok(validate::validate(&vmaps(conn, plan_id)?, &news, &plan.options))
}

/// Rewrites A→B→C as A→C; returns the number of mappings changed.
pub fn collapse_chains(conn: &Connection, plan_id: i64) -> Result<usize, String> {
    let plan = plan_or_err(conn, plan_id)?;
    let fixes = validate::collapse(&vmaps(conn, plan_id)?, &plan.options);
    for (id, target) in &fixes {
        store::set_target(conn, *id, target, "chain_collapsed")?;
    }
    if !fixes.is_empty() {
        db::log_activity(conn, Some(plan.project_id), "validate", &format!("{}: {} chains collapsed", plan.name, fixes.len()))?;
    }
    Ok(fixes.len())
}

// ---------------------------------------------------------------- export

/// Approved mappings (plus unapproved `auto` ones when `include_auto`) and enabled patterns.
pub fn rule_set(conn: &Connection, plan: &Plan, include_auto: bool) -> Result<(RuleSet, usize), String> {
    let (rows, _) = store::list_mappings(conn, plan.id, &MappingFilter::default())?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut duplicates = 0;
    let mut entries: Vec<RuleEntry> = Vec::new();
    for m in rows {
        let chosen = m.approved || (include_auto && m.status == "auto");
        if !chosen || m.status == "keep" {
            continue;
        }
        let gone = m.status == "gone";
        let target = if gone { None } else { m.new_url.clone() };
        if !gone && target.as_deref().map_or(true, |t| norm::same_url(&m.old_url, t)) {
            continue;
        }
        let p = norm::parts(&m.old_url);
        let key = if plan.options.ignore_query { p.path.clone() } else { format!("{}?{}", p.path, norm::sorted_query(p.query.as_deref()).unwrap_or_default()) };
        if !seen.insert(format!("{}{key}", p.host)) {
            duplicates += 1;
            continue;
        }
        entries.push(RuleEntry { mapping_id: m.id, source_url: m.old_url, target_url: target, status: if gone { 410 } else { 301 }, confidence: m.confidence, reason: m.reason, covered: false, overrides_pattern: false });
    }
    let patterns: Vec<PatternRule> = store::list_patterns(conn, plan.id)?
        .into_iter()
        .filter(|p| p.enabled)
        .map(|p| PatternRule { position: p.position, regex: p.regex, replacement: p.replacement })
        .collect();
    rules::classify(&mut entries, &patterns, &plan.new_base, &plan.options)?;
    Ok((RuleSet { plan_name: plan.name.clone(), old_base: plan.old_base.clone(), new_base: plan.new_base.clone(), entries, patterns }, duplicates))
}

pub fn export(conn: &Connection, plan_id: i64, format: &str, mut opts: ExportOptions, include_auto: bool) -> Result<ExportOutput, String> {
    let plan = plan_or_err(conn, plan_id)?;
    opts.ignore_query = plan.options.ignore_query;
    let (set, duplicates) = rule_set(conn, &plan, include_auto)?;
    let mut out = rules::export(&set, format, &opts)?;
    if duplicates > 0 {
        out.warnings.push(if opts.lang == "en" {
            format!("{duplicates} old URLs share a source path with another rule (query strings are ignored) and were left out")
        } else {
            format!("{duplicates} eski URL başka bir kuralla aynı kaynak yolu paylaştığı için (sorgu parametreleri yok sayılıyor) atlandı")
        });
    }
    Ok(out)
}

fn plan_slug(plan: &Plan) -> String {
    let slug = hub::text::slugify(&plan.name);
    if slug.is_empty() { format!("plan-{}", plan.id) } else { slug }
}

/// Plan names are unique only within a project, so default file locations include this too.
fn project_slug(conn: &Connection, plan: &Plan) -> Result<String, String> {
    let slug = db::get_project(conn, plan.project_id)?.map(|p| hub::text::slugify(&p.name)).unwrap_or_default();
    Ok(if slug.is_empty() { format!("project-{}", plan.project_id) } else { slug })
}

/// Default export location: `<reports>/exports/<project-slug>/<plan-slug>/<file name>`.
pub fn default_export_path(conn: &Connection, plan: &Plan, filename: &str) -> Result<PathBuf, String> {
    Ok(hub::paths::reports_dir().join("exports").join(project_slug(conn, plan)?).join(plan_slug(plan)).join(filename))
}

/// Default report location: `<reports>/<report-title>-<project-slug>-<plan-slug>-<timestamp>.html`.
pub fn default_report_path(conn: &Connection, plan: &Plan, lang: &str) -> Result<PathBuf, String> {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let name = format!("{}-{}-{}-{stamp}.html", hub::text::slugify(&report::title(lang)), project_slug(conn, plan)?, plan_slug(plan));
    Ok(hub::paths::reports_dir().join(name))
}

pub fn write_export(out: &ExportOutput, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut bytes = Vec::with_capacity(out.content.len() + 3);
    if out.bom {
        bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    bytes.extend_from_slice(out.content.as_bytes());
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

// ---------------------------------------------------------------- verification

pub fn check_kind(kind: &str) -> Result<&'static str, String> {
    match kind {
        "post" | "" => Ok("post"),
        "staging" => Ok("staging"),
        other => Err(format!("kind must be \"post\" or \"staging\" (got \"{other}\")")),
    }
}

/// Verification jobs: post = every rule's old URL; staging = distinct targets on the staging host.
pub fn verify_jobs(conn: &Connection, plan: &Plan, kind: &str, limit: Option<usize>, include_auto: bool) -> Result<Vec<Job>, String> {
    let (set, _) = rule_set(conn, plan, include_auto)?;
    let values: HashMap<i64, f64> = store::list_mappings(conn, plan.id, &MappingFilter::default())?.0.into_iter().map(|m| (m.id, m.value.unwrap_or(0.0))).collect();
    let mut entries = set.entries;
    entries.sort_by(|a, b| values.get(&b.mapping_id).unwrap_or(&0.0).partial_cmp(values.get(&a.mapping_id).unwrap_or(&0.0)).unwrap_or(std::cmp::Ordering::Equal).then(a.source_url.cmp(&b.source_url)));
    let mut jobs: Vec<Job> = Vec::new();
    if kind == "post" {
        for e in entries {
            jobs.push(Job { mapping_ids: vec![e.mapping_id], check_url: e.source_url.clone(), source_url: e.source_url, expect_gone: e.status == 410, expected_url: e.target_url });
        }
    } else {
        let staging = plan.staging_base.as_deref().ok_or("This plan has no staging base URL; set one in the plan settings (staging_base)")?;
        let mut index: HashMap<String, usize> = HashMap::new();
        for e in entries.into_iter().filter(|e| e.status == 301) {
            let Some(t) = e.target_url.clone() else { continue };
            let Some(check) = verify::rebase(&t, &plan.new_base, staging) else { continue };
            match index.get(&check) {
                Some(&i) => jobs[i].mapping_ids.push(e.mapping_id),
                None => {
                    index.insert(check.clone(), jobs.len());
                    jobs.push(Job { mapping_ids: vec![e.mapping_id], source_url: t, expected_url: None, check_url: check, expect_gone: false });
                }
            }
        }
    }
    if let Some(l) = limit {
        jobs.truncate(l);
    }
    Ok(jobs)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub previous_run: VerifyRun,
    pub current_run: VerifyRun,
    pub fixed: Vec<VerifyResultRow>,
    pub broken: Vec<VerifyResultRow>,
    pub still_failing: usize,
    pub new_checks: usize,
}

pub fn compare_runs(conn: &Connection, previous: i64, current: i64) -> Result<Comparison, String> {
    let a = store::get_run(conn, previous)?.ok_or("Previous run not found")?;
    let b = store::get_run(conn, current)?.ok_or("Run not found")?;
    let before: HashMap<String, String> = store::run_results(conn, previous)?.into_iter().map(|r| (r.checked_url, r.outcome)).collect();
    let mut c = Comparison { previous_run: a, current_run: b, fixed: Vec::new(), broken: Vec::new(), still_failing: 0, new_checks: 0 };
    for r in store::run_results(conn, current)? {
        match before.get(&r.checked_url) {
            None => c.new_checks += 1,
            Some(old) if old != "ok" && r.outcome == "ok" => c.fixed.push(r),
            Some(old) if old == "ok" && r.outcome != "ok" => c.broken.push(r),
            Some(old) if old != "ok" && r.outcome != "ok" => c.still_failing += 1,
            _ => {}
        }
    }
    Ok(c)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifySummary {
    pub run: VerifyRun,
    pub success_rate: f64,
    pub by_outcome: BTreeMap<String, usize>,
    pub results: Vec<VerifyResultRow>,
    pub comparison: Option<Comparison>,
    pub elapsed_ms: u128,
}

/// Runs a verification and stores it; compares with the previous run of the same kind.
pub async fn verify(plan_id: i64, kind: String, limit: Option<usize>, delay_ms: u64, include_auto: bool, progress: Option<Progress>) -> Result<VerifySummary, String> {
    let kind = check_kind(&kind)?.to_string();
    let start = std::time::Instant::now();
    let k2 = kind.clone();
    let (jobs, run_id, previous) = blocking(move || {
        let conn = db::open()?;
        let plan = plan_or_err(&conn, plan_id)?;
        let jobs = verify_jobs(&conn, &plan, &k2, limit, include_auto)?;
        if jobs.is_empty() {
            return Err("Nothing to verify: approve mappings (or keep auto mappings included) first".to_string());
        }
        let previous = store::list_runs(&conn, plan_id)?.into_iter().find(|r| r.kind == k2).map(|r| r.id);
        let run_id = store::create_run(&conn, plan_id, &k2)?;
        Ok((jobs, run_id, previous))
    })
    .await?;
    let results = verify::run(jobs, kind == "staging", verify::MAX_CONCURRENCY, delay_ms, progress).await;
    let rows: Vec<VerifyResultRow> = results
        .into_iter()
        .map(|r| {
            let last = r.trace.hops.last().cloned();
            VerifyResultRow {
                mapping_id: r.job.mapping_ids.first().copied(),
                source_url: r.job.source_url,
                expected_url: r.job.expected_url,
                checked_url: r.job.check_url,
                outcome: r.outcome,
                final_status: last.as_ref().map(|h| h.status as i64),
                final_url: last.map(|h| h.url),
                error: r.trace.error,
                hops: r.trace.hops,
            }
        })
        .collect();
    blocking(move || {
        let mut conn = db::open()?;
        let plan = plan_or_err(&conn, plan_id)?;
        let run = store::finish_run(&mut conn, run_id, &rows)?;
        let mut by_outcome: BTreeMap<String, usize> = verify::OUTCOMES.iter().map(|o| (o.to_string(), 0)).collect();
        for r in &rows {
            *by_outcome.entry(r.outcome.clone()).or_default() += 1;
        }
        let comparison = match previous {
            Some(p) => Some(compare_runs(&conn, p, run_id)?),
            None => None,
        };
        db::log_activity(&conn, Some(plan.project_id), "verify", &format!("{}: {} verification — {}/{} OK", plan.name, kind, run.ok, run.total))?;
        let mut results = rows;
        results.sort_by_key(|r| r.outcome == "ok");
        Ok(VerifySummary { success_rate: if run.total > 0 { run.ok as f64 / run.total as f64 } else { 0.0 }, run, by_outcome, results, comparison, elapsed_ms: start.elapsed().as_millis() })
    })
    .await
}

// ---------------------------------------------------------------- report

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportPaths {
    pub html: String,
    pub csv: String,
}

pub fn generate_report(conn: &Connection, plan_id: i64, lang: &str, dest: Option<&Path>) -> Result<ReportPaths, String> {
    let ov = overview(conn, plan_id)?;
    let validation = validate_plan(conn, plan_id)?;
    let runs = store::list_runs(conn, plan_id)?;
    let run = runs.first();
    let results = match run {
        Some(r) => store::run_results(conn, r.id)?,
        None => Vec::new(),
    };
    let input = report::ReportInput { plan: &ov.plan, stats: &ov.stats, old: &ov.old, new: &ov.new, patterns: ov.patterns, validation: &validation, run, results: &results };
    let r = report::build(&input, lang);
    let dest = match dest {
        Some(p) => p.to_path_buf(),
        None => default_report_path(conn, &ov.plan, lang)?,
    };
    let html = hub::report::save(&r, Some(&dest))?;
    let (rows, _) = store::list_mappings(conn, plan_id, &MappingFilter::default())?;
    let csv = html.with_extension("csv");
    hub::report::write_csv(&csv, report::CSV_HEADERS, &report::csv_rows(&rows))?;
    db::log_activity(conn, Some(ov.plan.project_id), "report", &format!("{}: report saved", ov.plan.name))?;
    Ok(ReportPaths { html: html.display().to_string(), csv: csv.display().to_string() })
}

/// Plan options update (case sensitivity, query handling, 410 for unvalued URLs).
pub fn set_options(conn: &Connection, plan_id: i64, options: &MatchOptions) -> Result<Plan, String> {
    store::set_plan_options(conn, plan_id, options)?;
    plan_or_err(conn, plan_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::spider::CrawledUrl;
    use crate::migrate::model::PlanInput;

    fn crawled(url: &str, title: Option<&str>, status: Option<i64>) -> CrawledUrl {
        CrawledUrl { url: url.into(), title: title.map(str::to_string), status_code: status, ..Default::default() }
    }

    fn setup() -> (Connection, Plan) {
        let mut conn = db::open_memory().unwrap();
        let project = db::create_project(&conn, "Test", Some("eski.com"), None).unwrap();
        let plan = store::create_plan(
            &conn,
            project.id,
            &PlanInput { name: "WP → Astro".into(), old_base: "eski.com".into(), new_base: "https://yeni.com/".into(), staging_base: Some("staging.yeni.com".into()), ..Default::default() },
        )
        .unwrap();
        let old = vec![
            crawled("https://eski.com/", Some("Ana Sayfa | Eski"), Some(200)),
            crawled("https://eski.com/hakkimizda/", Some("Hakkımızda | Eski"), Some(200)),
            crawled("https://eski.com/blog/2024/05/kahve-rehberi/", Some("Kahve Rehberi | Eski"), Some(200)),
            crawled("https://eski.com/blog/2024/06/cay-demleme/", Some("Çay Demleme | Eski"), Some(200)),
            crawled("https://eski.com/urunler/eski-urun/", None, Some(200)),
            CrawledUrl { content_type: Some("application/javascript".into()), ..crawled("https://eski.com/app.js", None, Some(200)) },
        ];
        let new = vec![
            crawled("https://yeni.com/", Some("Yeni Ana Sayfa | Yeni"), Some(200)),
            crawled("https://yeni.com/kurumsal/hakkimizda/", Some("Hakkımızda | Yeni"), Some(200)),
            crawled("https://yeni.com/makale/kahve-rehberi/", Some("Kahve Rehberi | Yeni"), Some(200)),
            crawled("https://yeni.com/makale/cay-demleme/", Some("Çay Demleme | Yeni"), Some(404)),
            crawled("https://yeni.com/urunler/", Some("Ürünler | Yeni"), Some(200)),
        ];
        let c = store::insert_urls(&mut conn, plan.id, "old", &old, false, false).unwrap();
        assert_eq!((c.inserted, c.skipped_resources), (5, 1));
        store::insert_urls(&mut conn, plan.id, "new", &new, false, false).unwrap();
        (conn, plan)
    }

    #[test]
    fn plan_bases_are_normalised() {
        let (_, plan) = setup();
        assert_eq!(plan.old_base, "https://eski.com");
        assert_eq!(plan.new_base, "https://yeni.com");
        assert_eq!(plan.staging_base.as_deref(), Some("https://staging.yeni.com"));
    }

    #[test]
    fn end_to_end_match_validate_export_and_decisions() {
        let (mut conn, plan) = setup();
        store::add_pattern(&conn, plan.id, r"^/blog/\d{4}/\d{2}/(.+)$", "/makale/$1", None).unwrap();
        let s = auto_match(&mut conn, plan.id, true).unwrap();
        assert_eq!(s.processed, 5);
        let (rows, total) = list_mappings(&conn, plan.id, &MappingFilter::default()).unwrap();
        assert_eq!(total, 5);
        let by_old = |u: &str| rows.iter().find(|m| m.old_url == u).unwrap().clone();
        assert_eq!(by_old("https://eski.com/blog/2024/05/kahve-rehberi/").reason, "pattern:1");
        assert_eq!(by_old("https://eski.com/hakkimizda/").reason, "same_slug");
        assert_eq!(by_old("https://eski.com/urunler/eski-urun/").reason, "parent");
        assert_eq!(by_old("https://eski.com/").reason, "same_path");

        // Validation sees the 404 target; the pattern rule covers both blog URLs in nginx.
        let v = validate_plan(&conn, plan.id).unwrap();
        assert_eq!(v.counts["target_not_200"], 1);
        let out = export(&conn, plan.id, "nginx", ExportOptions::default(), true).unwrap();
        assert!(out.content.contains("rewrite \"^/blog/\\d{4}/\\d{2}/(.+)$\" https://yeni.com/makale/$1 permanent;"), "{}", out.content);
        assert!(!out.content.contains("kahve-rehberi"));
        assert!(out.content.contains("\"/hakkimizda/\" \"https://yeni.com/kurumsal/hakkimizda/\";"));
        assert!(!out.content.contains("eski-urun"), "weak fallbacks are not exported until approved");

        // Decisions survive re-matching.
        let m = set_mapping(&conn, plan.id, "/urunler/eski-urun/", None, Some("gone")).unwrap();
        assert_eq!((m.status.as_str(), m.http_status, m.approved), ("gone", 410, true));
        let m = set_mapping(&conn, plan.id, "https://eski.com/hakkimizda/", Some("/hakkinda"), None).unwrap();
        assert_eq!((m.status.as_str(), m.new_url.as_deref()), ("manual", Some("https://yeni.com/hakkinda")));
        auto_match(&mut conn, plan.id, true).unwrap();
        let (rows, _) = list_mappings(&conn, plan.id, &MappingFilter { status: Some("gone".into()), ..Default::default() }).unwrap();
        assert_eq!(rows.len(), 1);
        let out = export(&conn, plan.id, "apache", ExportOptions::default(), true).unwrap();
        assert!(out.content.contains("Redirect 410 /urunler/eski-urun/"), "{}", out.content);
        assert!(out.content.contains("RedirectMatch 301 ^/hakkimizda/$ https://yeni.com/hakkinda"), "{}", out.content);

        // Bulk approval and filters.
        let n = approve(&conn, plan.id, &[], Some(90)).unwrap();
        assert!(n >= 2);
        let (approved, _) = list_mappings(&conn, plan.id, &MappingFilter { approved: Some(true), ..Default::default() }).unwrap();
        assert!(approved.len() >= 4);
        let (blog, _) = list_mappings(&conn, plan.id, &MappingFilter { directory: Some("/blog".into()), ..Default::default() }).unwrap();
        assert_eq!(blog.len(), 2);

        // Overview numbers.
        let ov = overview(&conn, plan.id).unwrap();
        assert_eq!(ov.old.total, 5);
        assert_eq!(ov.patterns, 1);
        assert!(ov.match_rate > 0.7);
    }

    #[test]
    fn chains_are_collapsed() {
        let (mut conn, plan) = setup();
        auto_match(&mut conn, plan.id, true).unwrap();
        // Same-site restructure: point /hakkimizda/ at another old URL that is itself redirected.
        set_mapping(&conn, plan.id, "/hakkimizda/", Some("https://eski.com/blog/2024/05/kahve-rehberi/"), None).unwrap();
        set_mapping(&conn, plan.id, "/blog/2024/05/kahve-rehberi/", Some("https://yeni.com/makale/kahve-rehberi/"), None).unwrap();
        let v = validate_plan(&conn, plan.id).unwrap();
        assert_eq!(v.counts["chain"], 1);
        assert_eq!(collapse_chains(&conn, plan.id).unwrap(), 1);
        let m = set_mapping(&conn, plan.id, "/hakkimizda/", None, Some("approve")).unwrap();
        assert_eq!(m.new_url.as_deref(), Some("https://yeni.com/makale/kahve-rehberi/"));
        assert_eq!(m.reason, "chain_collapsed");
        assert_eq!(validate_plan(&conn, plan.id).unwrap().counts["chain"], 0);
    }

    #[test]
    fn verify_jobs_for_post_and_staging() {
        let (mut conn, plan) = setup();
        auto_match(&mut conn, plan.id, true).unwrap();
        approve(&conn, plan.id, &[], Some(0)).unwrap();
        let post = verify_jobs(&conn, &plan, "post", None, true).unwrap();
        assert_eq!(post.len(), 5);
        let staging = verify_jobs(&conn, &plan, "staging", Some(10), true).unwrap();
        assert!(staging.iter().all(|j| j.check_url.starts_with("https://staging.yeni.com/")));
        assert!(staging.len() <= post.len());
        assert!(check_kind("later").is_err());
    }

    #[test]
    fn priority_values_csv() {
        let rows = parse_values_csv("\u{feff}Sayfa;Tıklamalar;Gösterimler\nhttps://eski.com/hakkimizda/;1.234;9\n/blog/2024/05/kahve-rehberi/;12;3\nhttps://eski.com/hakkimizda/;6;1\n").unwrap();
        assert_eq!(rows[0], ("https://eski.com/hakkimizda/".to_string(), 1240.0));
        let (mut conn, plan) = setup();
        let (matched, unmatched) = store::set_values(&mut conn, &plan, &rows).unwrap();
        assert_eq!((matched, unmatched), (2, 0));
        assert_eq!(parse_number("1,5"), Some(1.5));
        assert_eq!(parse_number("12,345"), Some(12345.0));
        assert!(parse_values_csv("a,b\n1,2\n").is_err());
    }

    #[test]
    fn report_is_generated() {
        let (mut conn, plan) = setup();
        auto_match(&mut conn, plan.id, true).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let paths = generate_report(&conn, plan.id, "tr", Some(&dir.path().join("rapor.html"))).unwrap();
        let html = std::fs::read_to_string(&paths.html).unwrap();
        assert!(html.contains("Taşıma raporu") && html.contains("Eşleme istatistikleri") && html.contains("Öneriler"));
        let csv = std::fs::read(&paths.csv).unwrap();
        assert_eq!(&csv[..3], &[0xEF, 0xBB, 0xBF]);
    }

    #[test]
    fn same_plan_name_in_two_projects_gets_separate_files() {
        let conn = db::open_memory().unwrap();
        let plans: Vec<Plan> = ["site-a.example", "site-b.example"]
            .iter()
            .map(|name| {
                let project = db::create_project(&conn, name, Some(name), None).unwrap();
                let input = PlanInput { name: "Eski adres düzeltmesi".into(), old_base: format!("https://{name}"), new_base: format!("https://{name}"), ..Default::default() };
                store::create_plan(&conn, project.id, &input).unwrap()
            })
            .collect();
        let exports: Vec<PathBuf> = plans.iter().map(|p| default_export_path(&conn, p, "redirects.conf").unwrap()).collect();
        assert!(exports[0].ends_with("exports/site-a-example/eski-adres-duzeltmesi/redirects.conf"));
        assert!(exports[1].ends_with("exports/site-b-example/eski-adres-duzeltmesi/redirects.conf"));
        let reports: Vec<String> = plans.iter().map(|p| default_report_path(&conn, p, "tr").unwrap().file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert!(reports[0].starts_with("tasima-raporu-site-a-example-eski-adres-duzeltmesi-"), "{}", reports[0]);
        assert!(reports[1].starts_with("tasima-raporu-site-b-example-eski-adres-duzeltmesi-"), "{}", reports[1]);
        assert!(reports[0].ends_with(".html"));
    }
}
