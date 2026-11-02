pub mod commands;
pub mod mcp_server;
pub mod services;

use commands::detect::detect_cwebp;
use commands::document::{clean_document, inspect_document, select_document_save_path};
use commands::encode::encode_image;
use commands::filesystem::{export_batch, export_image, select_output_directory, select_save_path};
use commands::image::{inspect_metadata, load_image};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            detect_cwebp,
            load_image,
            inspect_metadata,
            encode_image,
            export_image,
            export_batch,
            select_save_path,
            select_output_directory,
            inspect_document,
            clean_document,
            select_document_save_path,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
