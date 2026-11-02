//! Seobify Log — Local server log analyzer for search engine and AI crawler activity.
//!
//! `hub/` is the Seobify Hub shared layer (paths, SQLite, MCP stdio server, Spider bridge,
//! HTTP client, reports, secrets). Keep it identical across hub tools; tool logic lives in
//! `tools.rs`, `commands.rs` and tool-specific modules.

pub mod commands;
pub mod hub;
pub mod logs;
pub mod tools;

/// Short machine key (database file name, keyring service suffix).
pub const TOOL_KEY: &str = "log";
/// Human product name shown in the UI, reports and MCP server info.
pub const PRODUCT_NAME: &str = "Seobify Log";
/// Folder under `%USERPROFILE%\Seobify\` that holds this tool's data.
pub const PRODUCT_DIR: &str = "Log";
/// MCP server name (also the recommended key in `.mcp.json`).
pub const MCP_SERVER_NAME: &str = "seobify-log";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Runs the MCP stdio server (invoked as `seobify-log.exe --mcp`).
pub fn run_mcp() {
    hub::mcp::serve_stdio(&tools::Tools);
}

/// Runs the desktop app.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            // Create the data folder and apply migrations before the first command.
            hub::db::open().map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            // Catch up linked Cloudflare zones in the background (no-op without a token).
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tauri::Emitter;
                let progress = {
                    let h = handle.clone();
                    move |p: &serde_json::Value| {
                        let _ = h.emit("log-cf-progress", p);
                    }
                };
                if let Ok(result) = logs::service::cloudflare_auto_sync(progress).await {
                    let _ = handle.emit("log-cf-done", result);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::list_projects,
            commands::create_project,
            commands::delete_project,
            commands::recent_activity,
            commands::get_settings,
            commands::save_settings,
            commands::spider_projects,
            commands::spider_crawls,
            commands::import_preview,
            commands::write_text_file,
            commands::save_report,
            commands::update_project,
            commands::log_import,
            commands::log_bounds,
            commands::log_overview,
            commands::log_bot_stats,
            commands::log_bot_detail,
            commands::log_urls,
            commands::log_status,
            commands::log_crawl_waste,
            commands::log_spider_gaps,
            commands::log_ai_activity,
            commands::log_robots,
            commands::log_fake_bots,
            commands::log_imports,
            commands::log_delete_import,
            commands::log_generate_report,
            commands::log_range_status,
            commands::log_refresh_ranges,
            commands::log_clear_ip_cache,
            commands::log_cf_status,
            commands::log_cf_set_token,
            commands::log_cf_delete_token,
            commands::log_cf_link,
            commands::log_cf_sync,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
