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
// Every tool command calls the same `logs::service` function as the matching MCP tool.

use crate::logs::{query::UrlFilter, service};
use tauri::Emitter;

fn key(project_id: i64) -> String {
    project_id.to_string()
}

/// Imports files/folders; progress is streamed as `log-import-progress` events.
#[tauri::command]
pub async fn log_import(app: tauri::AppHandle, project_id: i64, paths: Vec<String>, format: Option<String>, force: Option<bool>) -> Result<Value, String> {
    let emitter = app.clone();
    service::import_logs(key(project_id), paths, format, force.unwrap_or(false), move |p| {
        let _ = emitter.emit("log-import-progress", p);
    })
    .await
}

#[tauri::command]
pub async fn log_bounds(project_id: i64) -> Result<Value, String> {
    service::bounds(key(project_id)).await
}

#[tauri::command]
pub async fn log_overview(project_id: i64, from: Option<String>, to: Option<String>) -> Result<Value, String> {
    service::overview(key(project_id), from, to).await
}

#[tauri::command]
pub async fn log_bot_stats(project_id: i64, from: Option<String>, to: Option<String>, category: Option<String>) -> Result<Value, String> {
    service::bot_stats(key(project_id), from, to, category).await
}

#[tauri::command]
pub async fn log_bot_detail(project_id: i64, bot_id: String, from: Option<String>, to: Option<String>) -> Result<Value, String> {
    service::bot_detail(key(project_id), bot_id, from, to).await
}

#[tauri::command]
pub async fn log_urls(project_id: i64, from: Option<String>, to: Option<String>, filter: Option<UrlFilter>) -> Result<Value, String> {
    service::urls(key(project_id), from, to, filter.unwrap_or_default()).await
}

#[tauri::command]
pub async fn log_status(project_id: i64, from: Option<String>, to: Option<String>) -> Result<Value, String> {
    service::status(key(project_id), from, to).await
}

#[tauri::command]
pub async fn log_crawl_waste(project_id: i64, from: Option<String>, to: Option<String>, scope: Option<String>) -> Result<Value, String> {
    service::crawl_waste(key(project_id), from, to, scope).await
}

#[tauri::command]
pub async fn log_spider_gaps(project_id: i64, spider_project: Option<String>, crawl_id: Option<i64>, days: Option<i64>, limit: Option<i64>) -> Result<Value, String> {
    service::spider_gaps(key(project_id), spider_project, crawl_id, days, limit).await
}

#[tauri::command]
pub async fn log_ai_activity(project_id: i64, from: Option<String>, to: Option<String>) -> Result<Value, String> {
    service::ai_activity(key(project_id), from, to).await
}

#[tauri::command]
pub async fn log_robots(project_id: i64) -> Result<Value, String> {
    service::robots(key(project_id)).await
}

#[tauri::command]
pub async fn log_fake_bots(project_id: i64, from: Option<String>, to: Option<String>, limit: Option<i64>) -> Result<Value, String> {
    service::fake_bots(key(project_id), from, to, limit.unwrap_or(200)).await
}

#[tauri::command]
pub async fn log_imports(project_id: i64) -> Result<Value, String> {
    service::imports(key(project_id)).await
}

#[tauri::command]
pub async fn log_delete_import(project_id: i64, file_id: i64) -> Result<Value, String> {
    service::delete_import(key(project_id), file_id).await
}

#[tauri::command]
pub async fn log_generate_report(project_id: i64, from: Option<String>, to: Option<String>, lang: Option<String>, dest: Option<String>) -> Result<Value, String> {
    service::generate_report(key(project_id), from, to, lang, dest).await
}

#[tauri::command]
pub async fn log_range_status() -> Result<Value, String> {
    service::range_status().await
}

#[tauri::command]
pub async fn log_refresh_ranges(force: Option<bool>) -> Result<Value, String> {
    service::ranges(force.unwrap_or(true)).await
}

#[tauri::command]
pub async fn log_clear_ip_cache() -> Result<Value, String> {
    service::clear_ip_cache().await
}

#[tauri::command]
pub async fn log_cf_status() -> Result<Value, String> {
    service::cloudflare_status().await
}

#[tauri::command]
pub async fn log_cf_set_token(token: String) -> Result<Value, String> {
    service::cloudflare_set_token(token).await
}

#[tauri::command]
pub async fn log_cf_delete_token() -> Result<Value, String> {
    service::cloudflare_delete_token().await
}

#[tauri::command]
pub async fn log_cf_link(project_id: i64, zone: String) -> Result<Value, String> {
    service::cloudflare_link(key(project_id), zone).await
}

/// Imports Cloudflare analytics; progress is streamed as `log-cf-progress` events.
#[tauri::command]
pub async fn log_cf_sync(app: tauri::AppHandle, project_id: i64, days: Option<i64>) -> Result<Value, String> {
    service::cloudflare_sync(key(project_id), days, false, move |p| {
        let _ = app.emit("log-cf-progress", p);
    })
    .await
}

/// Updates a project's domain and linked Spider project (used by the Spider comparison).
#[tauri::command]
pub async fn update_project(id: i64, domain: Option<String>, spider_project: Option<String>) -> Result<db::Project, String> {
    blocking(move || {
        let conn = db::open()?;
        let mut p = db::get_project(&conn, id)?.ok_or("Project not found")?;
        p.domain = domain.map(|d| hub::text::normalize_domain(&d)).filter(|d| !d.is_empty());
        p.spider_project = spider_project.filter(|s| !s.trim().is_empty());
        db::update_project(&conn, &p)?;
        db::get_project(&conn, id)?.ok_or_else(|| "Project vanished".to_string())
    })
    .await
}
