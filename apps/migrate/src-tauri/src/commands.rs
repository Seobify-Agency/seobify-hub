//! Tauri commands. Hub commands first; tool commands below the marker.
//! Commands are async and return `Result<T, String>`; blocking work goes through
//! `hub::mcp::blocking` so the UI thread never waits on SQLite or the network.

use crate::hub::{self, db, mcp::blocking, report, spider};
use serde_json::{json, Value};

#[tauri::command]
pub async fn app_info() -> Result<Value, String> {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    Ok(json!({
        "product": crate::PRODUCT_NAME,
        "key": crate::TOOL_KEY,
        "version": crate::VERSION,
        "dataDir": hub::paths::data_dir(),
        "database": hub::paths::db_path(),
        "reportsDir": hub::paths::reports_dir(),
        "spiderHome": hub::paths::spider_home(),
        "mcp": {
            "name": crate::MCP_SERVER_NAME,
            "command": exe,
            "args": ["--mcp"],
        }
    }))
}

#[tauri::command]
pub async fn list_projects() -> Result<Vec<db::Project>, String> {
    blocking(|| db::list_projects(&db::open()?)).await
}

#[tauri::command]
pub async fn create_project(name: String, domain: Option<String>, spider_project: Option<String>) -> Result<db::Project, String> {
    blocking(move || db::create_project(&db::open()?, &name, domain.as_deref(), spider_project.as_deref())).await
}

#[tauri::command]
pub async fn delete_project(id: i64) -> Result<(), String> {
    blocking(move || db::delete_project(&db::open()?, id)).await
}

#[tauri::command]
pub async fn recent_activity(project_id: Option<i64>, limit: Option<i64>) -> Result<Vec<db::Activity>, String> {
    blocking(move || db::recent_activity(&db::open()?, project_id, limit.unwrap_or(50))).await
}

#[tauri::command]
pub async fn get_settings() -> Result<Value, String> {
    blocking(|| db::get_settings(&db::open()?)).await
}

#[tauri::command]
pub async fn save_settings(settings: Value) -> Result<(), String> {
    blocking(move || db::save_settings(&db::open()?, &settings)).await
}

#[tauri::command]
pub async fn spider_projects() -> Result<Value, String> {
    let (projects, source) = spider::projects().await?;
    Ok(json!({ "source": source, "projects": projects }))
}

#[tauri::command]
pub async fn spider_crawls(project: String) -> Result<Value, String> {
    let (crawls, source) = spider::crawls(&project).await?;
    Ok(json!({ "source": source, "crawls": crawls }))
}

/// Imports a URL list from any source and returns the total plus a preview.
#[tauri::command]
pub async fn import_preview(source: spider::ImportSource, limit: Option<usize>) -> Result<Value, String> {
    let result = spider::import(&source).await?;
    let total = result.urls.len();
    let preview: Vec<_> = result.urls.into_iter().take(limit.unwrap_or(50)).collect();
    Ok(json!({ "source": result.source, "total": total, "preview": preview }))
}

/// Writes text chosen through a save dialog (CSV, HTML, rules files).
#[tauri::command]
pub async fn write_text_file(path: String, contents: String, bom: Option<bool>) -> Result<(), String> {
    blocking(move || {
        let mut bytes = Vec::with_capacity(contents.len() + 3);
        if bom.unwrap_or(false) {
            bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        bytes.extend_from_slice(contents.as_bytes());
        std::fs::write(&path, bytes).map_err(|e| format!("{path}: {e}"))
    })
    .await
}

/// Renders a report described by the UI (title + blocks) and saves it; returns the path.
#[tauri::command]
pub async fn save_report(title: String, subtitle: Option<String>, lang: String, blocks: Value, dest: Option<String>) -> Result<String, String> {
    blocking(move || {
        let blocks: Vec<report::Block> = serde_json::from_value(blocks).map_err(|e| format!("blocks: {e}"))?;
        let mut r = report::Report::new(title, &lang);
        r.subtitle = subtitle;
        r.blocks = blocks;
        report::save(&r, dest.as_deref().map(std::path::Path::new)).map(|p| p.display().to_string())
    })
    .await
}

// ---------------------------------------------------------------- tool commands
//
// Every command below calls the same `migrate::service` / `migrate::store` function as the
// matching MCP tool. Long jobs report progress through the `migrate-progress` event.

use crate::migrate::matcher::PatternSuggestion;
use crate::migrate::model::{MappingRow, MatchOptions, Pattern, Plan, PlanInput, UrlRow};
use crate::migrate::rules::{self, ExportOptions, ExportOutput};
use crate::migrate::service::{self, Comparison, FetchSummary, ImportSummary, MatchSummary, Overview, PatternPreview, PatternWithCounts, ReportPaths, ValuesSummary, VerifySummary};
use crate::migrate::store::{self, MappingFilter, PlanSummary, VerifyResultRow, VerifyRun};
use crate::migrate::validate::Validation;
use tauri::{AppHandle, Emitter};

fn progress(app: AppHandle, task: &'static str) -> crate::migrate::Progress {
    std::sync::Arc::new(move |done, total| {
        let _ = app.emit("migrate-progress", json!({ "task": task, "done": done, "total": total }));
    })
}

#[tauri::command]
pub async fn list_plans(project_id: Option<i64>) -> Result<Vec<PlanSummary>, String> {
    blocking(move || store::list_plans(&db::open()?, project_id)).await
}

#[tauri::command]
pub async fn create_plan(project_id: i64, input: PlanInput) -> Result<Plan, String> {
    blocking(move || store::create_plan(&db::open()?, project_id, &input)).await
}

#[tauri::command]
pub async fn update_plan(id: i64, input: PlanInput) -> Result<Plan, String> {
    blocking(move || store::update_plan(&db::open()?, id, &input)).await
}

#[tauri::command]
pub async fn set_plan_options(plan_id: i64, options: MatchOptions) -> Result<Plan, String> {
    blocking(move || service::set_options(&db::open()?, plan_id, &options)).await
}

#[tauri::command]
pub async fn delete_plan(id: i64) -> Result<(), String> {
    blocking(move || store::delete_plan(&db::open()?, id)).await
}

#[tauri::command]
pub async fn plan_overview(plan_id: i64) -> Result<Overview, String> {
    blocking(move || service::overview(&db::open()?, plan_id)).await
}

#[tauri::command]
pub async fn import_urls(plan_id: i64, side: String, source: spider::ImportSource, replace: Option<bool>, include_resources: Option<bool>) -> Result<ImportSummary, String> {
    service::import_urls(plan_id, side, source, replace.unwrap_or(false), include_resources.unwrap_or(false)).await
}

#[tauri::command]
pub async fn list_urls(plan_id: i64, side: String, query: Option<String>, limit: Option<i64>) -> Result<Vec<UrlRow>, String> {
    blocking(move || store::list_urls(&db::open()?, plan_id, &side, query.as_deref(), limit.unwrap_or(500))).await
}

#[tauri::command]
pub async fn clear_side(plan_id: i64, side: String) -> Result<usize, String> {
    blocking(move || store::clear_side(&db::open()?, plan_id, &side)).await
}

#[tauri::command]
pub async fn import_values(plan_id: i64, path: String) -> Result<ValuesSummary, String> {
    blocking(move || service::import_values(&mut db::open()?, plan_id, std::path::Path::new(&path))).await
}

#[tauri::command]
pub async fn fetch_content(app: AppHandle, plan_id: i64, side: Option<String>, only_missing: Option<bool>, limit: Option<usize>, delay_ms: Option<u64>) -> Result<FetchSummary, String> {
    service::fetch_content(plan_id, side, only_missing.unwrap_or(true), limit.unwrap_or(2000), delay_ms.unwrap_or(150), Some(progress(app, "fetch"))).await
}

#[tauri::command]
pub async fn auto_match(plan_id: i64, rematch: Option<bool>) -> Result<MatchSummary, String> {
    blocking(move || service::auto_match(&mut db::open()?, plan_id, rematch.unwrap_or(true))).await
}

#[tauri::command]
pub async fn list_mappings(plan_id: i64, filter: Option<MappingFilter>) -> Result<Value, String> {
    blocking(move || {
        let (rows, total) = service::list_mappings(&db::open()?, plan_id, &filter.unwrap_or_default())?;
        Ok(json!({ "rows": rows, "total": total }))
    })
    .await
}

/// `mapping_id` identifies the row; `status`: manual (with new_url), gone, keep, approve, unapprove.
#[tauri::command]
pub async fn set_mapping(plan_id: i64, mapping_id: i64, new_url: Option<String>, status: Option<String>) -> Result<MappingRow, String> {
    blocking(move || service::set_mapping(&db::open()?, plan_id, &format!("#{mapping_id}"), new_url.as_deref(), status.as_deref())).await
}

#[tauri::command]
pub async fn approve_mappings(plan_id: i64, ids: Vec<i64>, min_confidence: Option<i64>) -> Result<usize, String> {
    blocking(move || service::approve(&db::open()?, plan_id, &ids, min_confidence)).await
}

#[tauri::command]
pub async fn search_new_urls(plan_id: i64, query: String, limit: Option<i64>) -> Result<Vec<UrlRow>, String> {
    blocking(move || service::search_new(&db::open()?, plan_id, &query, limit.unwrap_or(20))).await
}

#[tauri::command]
pub async fn list_patterns(plan_id: i64) -> Result<Vec<PatternWithCounts>, String> {
    blocking(move || service::list_patterns(&db::open()?, plan_id)).await
}

#[tauri::command]
pub async fn add_pattern(plan_id: i64, regex: String, replacement: String, note: Option<String>) -> Result<Pattern, String> {
    blocking(move || store::add_pattern(&db::open()?, plan_id, &regex, &replacement, note.as_deref())).await
}

#[tauri::command]
pub async fn update_pattern(id: i64, regex: String, replacement: String, enabled: bool, note: Option<String>) -> Result<(), String> {
    blocking(move || store::update_pattern(&db::open()?, id, &regex, &replacement, enabled, note.as_deref())).await
}

#[tauri::command]
pub async fn delete_pattern(id: i64) -> Result<(), String> {
    blocking(move || store::delete_pattern(&db::open()?, id)).await
}

#[tauri::command]
pub async fn move_pattern(id: i64, delta: i64) -> Result<(), String> {
    blocking(move || store::move_pattern(&mut db::open()?, id, delta)).await
}

#[tauri::command]
pub async fn preview_pattern(plan_id: i64, regex: String, replacement: String) -> Result<PatternPreview, String> {
    blocking(move || service::preview_pattern(&db::open()?, plan_id, &regex, &replacement, 25)).await
}

#[tauri::command]
pub async fn suggest_patterns(plan_id: i64) -> Result<Vec<PatternSuggestion>, String> {
    blocking(move || service::suggest_patterns(&db::open()?, plan_id)).await
}

#[tauri::command]
pub async fn validate_plan(plan_id: i64) -> Result<Validation, String> {
    blocking(move || service::validate_plan(&db::open()?, plan_id)).await
}

#[tauri::command]
pub async fn collapse_chains(plan_id: i64) -> Result<usize, String> {
    blocking(move || service::collapse_chains(&db::open()?, plan_id)).await
}

#[tauri::command]
pub async fn export_formats() -> Result<Value, String> {
    Ok(json!(rules::FORMATS.iter().map(|(id, file, label)| json!({ "id": id, "filename": file, "label": label })).collect::<Vec<_>>()))
}

#[tauri::command]
pub async fn export_rules(plan_id: i64, format: String, options: Option<ExportOptions>, include_auto: Option<bool>) -> Result<ExportOutput, String> {
    blocking(move || service::export(&db::open()?, plan_id, &format, options.unwrap_or_default(), include_auto.unwrap_or(true))).await
}

/// Exports and writes the rules file; returns the written path.
#[tauri::command]
pub async fn save_export(plan_id: i64, format: String, options: Option<ExportOptions>, include_auto: Option<bool>, path: String) -> Result<String, String> {
    blocking(move || {
        let conn = db::open()?;
        let out = service::export(&conn, plan_id, &format, options.unwrap_or_default(), include_auto.unwrap_or(true))?;
        service::write_export(&out, std::path::Path::new(&path))?;
        let plan = service::plan_or_err(&conn, plan_id)?;
        db::log_activity(&conn, Some(plan.project_id), "export", &format!("{}: {} rules exported ({format})", plan.name, out.rules + out.pattern_rules))?;
        Ok(path)
    })
    .await
}

#[tauri::command]
pub async fn default_export_path(plan_id: i64, format: String) -> Result<String, String> {
    blocking(move || {
        let conn = db::open()?;
        let plan = service::plan_or_err(&conn, plan_id)?;
        let file = rules::FORMATS.iter().find(|f| f.0 == format).map(|f| f.1).ok_or("Unknown format")?;
        Ok(service::default_export_path(&conn, &plan, file)?.display().to_string())
    })
    .await
}

#[tauri::command]
pub async fn verify_plan(app: AppHandle, plan_id: i64, kind: String, limit: Option<usize>, delay_ms: Option<u64>, include_auto: Option<bool>) -> Result<VerifySummary, String> {
    service::verify(plan_id, kind, limit, delay_ms.unwrap_or(100), include_auto.unwrap_or(true), Some(progress(app, "verify"))).await
}

#[tauri::command]
pub async fn list_runs(plan_id: i64) -> Result<Vec<VerifyRun>, String> {
    blocking(move || store::list_runs(&db::open()?, plan_id)).await
}

#[tauri::command]
pub async fn run_results(run_id: i64) -> Result<Vec<VerifyResultRow>, String> {
    blocking(move || store::run_results(&db::open()?, run_id)).await
}

#[tauri::command]
pub async fn compare_runs(previous: i64, current: i64) -> Result<Comparison, String> {
    blocking(move || service::compare_runs(&db::open()?, previous, current)).await
}

#[tauri::command]
pub async fn generate_report(plan_id: i64, lang: String, dest: Option<String>) -> Result<ReportPaths, String> {
    blocking(move || service::generate_report(&db::open()?, plan_id, &lang, dest.as_deref().map(std::path::Path::new))).await
}
