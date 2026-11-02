use super::filesystem::SavePathResult;
use crate::services::document_cleaner::{self, DocCleanOptions, DocInspectReport, DocProcessOutcome};
use std::path::{Path, PathBuf};
use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub async fn inspect_document(file_path: String) -> Result<DocInspectReport, String> {
    tokio::task::spawn_blocking(move || {
        let data = std::fs::read(&file_path).map_err(|e| format!("File read error: {}", e))?;
        document_cleaner::inspect_document_bytes(&data, None)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Writes a cleaned copy. Without `output_path` it goes next to the input as `<stem>.clean.<ext>`;
/// with `output_dir` it goes into that folder (batch export).
#[tauri::command]
pub async fn clean_document(
    input_path: String,
    output_path: Option<String>,
    output_dir: Option<String>,
    options: DocCleanOptions,
) -> Result<DocProcessOutcome, String> {
    tokio::task::spawn_blocking(move || {
        let input = PathBuf::from(&input_path);
        let output = match (output_path, output_dir) {
            (Some(p), _) => PathBuf::from(p),
            (None, Some(dir)) => document_cleaner::batch_output_path(&input, Path::new(&dir), &mut Default::default()),
            (None, None) => document_cleaner::default_output_path(&input),
        };
        document_cleaner::process_document_file(&input, &output, &options)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn select_document_save_path(
    app: tauri::AppHandle,
    default_name: String,
    extension: String,
) -> Result<SavePathResult, String> {
    let ext = extension.to_ascii_lowercase();
    let result = app
        .dialog()
        .file()
        .set_file_name(&default_name)
        .add_filter(ext.to_uppercase(), &[ext.as_str()])
        .blocking_save_file();
    Ok(SavePathResult {
        path: result.map(|p| p.to_string()),
    })
}
