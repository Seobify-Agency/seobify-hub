//! Seobify Migrate — Maps old URLs to new ones, generates redirect rules and verifies them after launch.
//!
//! `hub/` is the Seobify Hub shared layer (paths, SQLite, MCP stdio server, Spider bridge,
//! HTTP client, reports, secrets). Keep it identical across hub tools; tool logic lives in
//! `tools.rs`, `commands.rs` and tool-specific modules.

pub mod commands;
pub mod hub;
pub mod migrate;
pub mod tools;

/// Short machine key (database file name, keyring service suffix).
pub const TOOL_KEY: &str = "migrate";
/// Human product name shown in the UI, reports and MCP server info.
pub const PRODUCT_NAME: &str = "Seobify Migrate";
/// Folder under `%USERPROFILE%\Seobify\` that holds this tool's data.
pub const PRODUCT_DIR: &str = "Migrate";
/// MCP server name (also the recommended key in `.mcp.json`).
pub const MCP_SERVER_NAME: &str = "seobify-migrate";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Runs the MCP stdio server (invoked as `seobify-migrate.exe --mcp`).
pub fn run_mcp() {
    hub::mcp::serve_stdio(&tools::Tools);
}

/// Runs the desktop app.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|_app| {
            // Create the data folder and apply migrations before the first command.
            hub::db::open().map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
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
            commands::list_plans,
            commands::create_plan,
            commands::update_plan,
            commands::set_plan_options,
            commands::delete_plan,
            commands::plan_overview,
            commands::import_urls,
            commands::list_urls,
            commands::clear_side,
            commands::import_values,
            commands::fetch_content,
            commands::auto_match,
            commands::list_mappings,
            commands::set_mapping,
            commands::approve_mappings,
            commands::search_new_urls,
            commands::list_patterns,
            commands::add_pattern,
            commands::update_pattern,
            commands::delete_pattern,
            commands::move_pattern,
            commands::preview_pattern,
            commands::suggest_patterns,
            commands::validate_plan,
            commands::collapse_chains,
            commands::export_formats,
            commands::export_rules,
            commands::save_export,
            commands::default_export_path,
            commands::verify_plan,
            commands::list_runs,
            commands::run_results,
            commands::compare_runs,
            commands::generate_report,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
