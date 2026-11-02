use crate::services::cwebp::EncodingConfig;
use crate::services::dsp_pipeline::{PipelineStats, SanitizeConfig};
use crate::services::metadata_cleaner::InspectReport;
use crate::services::processor::{process_image_file, ProcessRequest};
use base64::Engine;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeResult {
    pub file_size: u64,
    pub reduction_percent: f64,
    pub encoding_duration_ms: u64,
    pub image_data: String,
    pub temp_file_path: String,
    pub output_format: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub findings_before: u32,
    pub findings_after: u32,
    pub clean: bool,
    pub inspect_after: InspectReport,
    pub pipeline: Option<PipelineStats>,
}

#[tauri::command]
pub async fn encode_image(
    file_path: String,
    config: EncodingConfig,
    sanitize: Option<SanitizeConfig>,
) -> Result<EncodeResult, String> {
    let sanitize = sanitize.unwrap_or_default();
    let outcome = process_image_file(ProcessRequest {
        input_path: &file_path,
        output_path: None,
        encoding: &config,
        sanitize: &sanitize,
        cwebp_binary: None,
    })
    .await?;

    let output_data = tokio::fs::read(&outcome.output_path)
        .await
        .map_err(|e| format!("Failed to read output file: {}", e))?;
    let image_data = base64::engine::general_purpose::STANDARD.encode(&output_data);

    Ok(EncodeResult {
        file_size: outcome.file_size,
        reduction_percent: outcome.reduction_percent,
        encoding_duration_ms: outcome.duration_ms,
        image_data,
        temp_file_path: outcome.output_path,
        output_format: outcome.output_format.label().to_string(),
        mime_type: outcome.mime_type,
        width: outcome.width,
        height: outcome.height,
        findings_before: outcome.findings_before,
        findings_after: outcome.findings_after,
        clean: outcome.clean,
        inspect_after: outcome.inspect_after,
        pipeline: outcome.pipeline,
    })
}
