use crate::services::cwebp;
use crate::services::dsp_pipeline::SanitizeConfig;
use crate::services::metadata_cleaner::inspect_image_bytes;
use crate::services::processor::{output_file_name, process_image_file, sniff_format, unique_output_name, ProcessRequest};
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tauri_plugin_dialog::DialogExt;

#[derive(Debug, Clone, Serialize)]
pub struct ExportResult {
    #[serde(rename = "outputPath")]
    pub output_path: String,
    #[serde(rename = "fileSize")]
    pub file_size: u64,
    #[serde(rename = "findingsAfter")]
    pub findings_after: u32,
    pub clean: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SavePathResult {
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutputDirResult {
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BatchExportItem {
    pub id: String,
    #[serde(rename = "filePath")]
    pub file_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchItemResult {
    pub id: String,
    pub status: String,
    #[serde(rename = "outputPath", skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
    #[serde(rename = "fileSize", skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
    #[serde(rename = "reductionPercent", skip_serializing_if = "Option::is_none")]
    pub reduction_percent: Option<f64>,
    #[serde(rename = "encodingDurationMs", skip_serializing_if = "Option::is_none")]
    pub encoding_duration_ms: Option<u64>,
    #[serde(rename = "findingsBefore", skip_serializing_if = "Option::is_none")]
    pub findings_before: Option<u32>,
    #[serde(rename = "findingsAfter", skip_serializing_if = "Option::is_none")]
    pub findings_after: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchExportResult {
    pub results: Vec<BatchItemResult>,
    #[serde(rename = "totalCompleted")]
    pub total_completed: u32,
    #[serde(rename = "totalFailed")]
    pub total_failed: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchProgressPayload {
    #[serde(rename = "itemId")]
    pub item_id: String,
    pub status: String,
    #[serde(rename = "outputPath", skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
    #[serde(rename = "fileSize", skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
    #[serde(rename = "reductionPercent", skip_serializing_if = "Option::is_none")]
    pub reduction_percent: Option<f64>,
    #[serde(rename = "findingsAfter", skip_serializing_if = "Option::is_none")]
    pub findings_after: Option<u32>,
    #[serde(rename = "currentIndex")]
    pub current_index: u32,
    #[serde(rename = "totalItems")]
    pub total_items: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Copies the already-processed preview file to the user's chosen location and
/// re-inspects the written bytes so the UI can confirm the output is trace-free.
#[tauri::command]
pub async fn export_image(
    temp_file_path: String,
    output_path: String,
) -> Result<ExportResult, String> {
    let source = std::path::Path::new(&temp_file_path);
    if !source.exists() {
        return Err(format!("Source file not found: {}", temp_file_path));
    }

    tokio::fs::copy(&temp_file_path, &output_path)
        .await
        .map_err(|e| format!("Write error: {}", e))?;

    let written = tokio::fs::read(&output_path)
        .await
        .map_err(|e| format!("Failed to read exported file: {}", e))?;
    let report = inspect_image_bytes(&written);

    Ok(ExportResult {
        output_path,
        file_size: written.len() as u64,
        findings_after: report.trace_count,
        clean: report.clean,
    })
}

#[tauri::command]
pub async fn select_save_path(
    app: tauri::AppHandle,
    default_name: String,
    extension: Option<String>,
) -> Result<SavePathResult, String> {
    let ext = extension.unwrap_or_else(|| "webp".to_string()).to_ascii_lowercase();
    let (label, exts): (&str, Vec<&str>) = match ext.as_str() {
        "png" => ("PNG Image", vec!["png"]),
        "jpg" | "jpeg" => ("JPEG Image", vec!["jpg", "jpeg"]),
        _ => ("WebP Image", vec!["webp"]),
    };
    let result = app
        .dialog()
        .file()
        .set_file_name(&default_name)
        .add_filter(label, &exts)
        .blocking_save_file();

    Ok(SavePathResult {
        path: result.map(|p| p.to_string()),
    })
}

#[tauri::command]
pub async fn select_output_directory(
    app: tauri::AppHandle,
) -> Result<OutputDirResult, String> {
    let result = app
        .dialog()
        .file()
        .blocking_pick_folder();

    Ok(OutputDirResult {
        path: result.map(|p| p.to_string()),
    })
}

#[tauri::command]
pub async fn export_batch(
    app: tauri::AppHandle,
    items: Vec<BatchExportItem>,
    config: cwebp::EncodingConfig,
    output_dir: String,
    sanitize: Option<SanitizeConfig>,
) -> Result<BatchExportResult, String> {
    let output_path = std::path::Path::new(&output_dir);
    if !output_path.exists() {
        return Err(format!("Output directory not found: {}", output_dir));
    }
    let sanitize = sanitize.unwrap_or_default();

    let total_items = items.len() as u32;
    let mut results = Vec::new();
    let mut total_completed: u32 = 0;
    let mut total_failed: u32 = 0;
    let mut used_names = std::collections::HashSet::new();

    for (index, item) in items.iter().enumerate() {
        let src = std::path::Path::new(&item.file_path);
        let outcome = match sniff_format(src) {
            Ok(fmt) => {
                let mut out_name = output_file_name(src, fmt, sanitize.output_format, sanitize.seo_slug);
                if output_path.join(&out_name) == src {
                    out_name = out_name.replacen('.', ".clean.", 1);
                }
                let out_name = if sanitize.seo_slug {
                    unique_output_name(output_path, &out_name, &mut used_names)
                } else {
                    out_name
                };
                let out_file = output_path.join(&out_name);
                let out_str = out_file.to_string_lossy().to_string();
                process_image_file(ProcessRequest {
                    input_path: &item.file_path,
                    output_path: Some(&out_str),
                    encoding: &config,
                    sanitize: &sanitize,
                    cwebp_binary: None,
                })
                .await
            }
            Err(e) => Err(e),
        };

        match outcome {
            Ok(o) => {
                total_completed += 1;
                results.push(BatchItemResult {
                    id: item.id.clone(),
                    status: "completed".to_string(),
                    output_path: Some(o.output_path.clone()),
                    file_size: Some(o.file_size),
                    reduction_percent: Some(o.reduction_percent),
                    encoding_duration_ms: Some(o.duration_ms),
                    findings_before: Some(o.findings_before),
                    findings_after: Some(o.findings_after),
                    error: None,
                });

                let _ = app.emit("batch_progress", BatchProgressPayload {
                    item_id: item.id.clone(),
                    status: "completed".to_string(),
                    output_path: Some(o.output_path),
                    file_size: Some(o.file_size),
                    reduction_percent: Some(o.reduction_percent),
                    findings_after: Some(o.findings_after),
                    current_index: index as u32,
                    total_items,
                    error: None,
                });
            }
            Err(e) => {
                total_failed += 1;
                results.push(BatchItemResult {
                    id: item.id.clone(),
                    status: "failed".to_string(),
                    output_path: None,
                    file_size: None,
                    reduction_percent: None,
                    encoding_duration_ms: None,
                    findings_before: None,
                    findings_after: None,
                    error: Some(e.clone()),
                });

                let _ = app.emit("batch_progress", BatchProgressPayload {
                    item_id: item.id.clone(),
                    status: "failed".to_string(),
                    output_path: None,
                    file_size: None,
                    reduction_percent: None,
                    findings_after: None,
                    current_index: index as u32,
                    total_items,
                    error: Some(e),
                });
            }
        }
    }

    Ok(BatchExportResult {
        results,
        total_completed,
        total_failed,
    })
}
