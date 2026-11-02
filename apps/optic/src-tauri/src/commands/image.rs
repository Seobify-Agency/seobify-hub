use crate::services::image as image_service;
use crate::services::metadata_cleaner::{inspect_image_bytes, InspectReport};
use base64::Engine;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct LoadImageResult {
    pub id: String,
    #[serde(rename = "filePath")]
    pub file_path: String,
    #[serde(rename = "fileName")]
    pub file_name: String,
    #[serde(rename = "fileSize")]
    pub file_size: u64,
    pub width: u32,
    pub height: u32,
    #[serde(rename = "hasAlpha")]
    pub has_alpha: bool,
    #[serde(rename = "imageData")]
    pub image_data: String,
    pub format: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

#[tauri::command]
pub async fn load_image(file_path: String) -> Result<LoadImageResult, String> {
    let path = std::path::Path::new(&file_path);
    if !path.exists() {
        return Err(format!("File not found: {}", file_path));
    }

    let data = tokio::fs::read(&file_path)
        .await
        .map_err(|e| format!("File read error: {}", e))?;

    let metadata = image_service::extract_image_metadata(&data, &file_path)?;

    let image_data = base64::engine::general_purpose::STANDARD.encode(&data);

    Ok(LoadImageResult {
        id: Uuid::new_v4().to_string(),
        file_path: metadata.file_path,
        file_name: metadata.file_name,
        file_size: metadata.file_size,
        width: metadata.width,
        height: metadata.height,
        has_alpha: metadata.has_alpha,
        image_data,
        format: metadata.format.label().to_string(),
        mime_type: metadata.mime_type,
    })
}

#[tauri::command]
pub async fn inspect_metadata(file_path: String) -> Result<InspectReport, String> {
    let path = std::path::Path::new(&file_path);
    if !path.exists() {
        return Err(format!("File not found: {}", file_path));
    }
    let data = tokio::fs::read(&file_path)
        .await
        .map_err(|e| format!("File read error: {}", e))?;
    Ok(inspect_image_bytes(&data))
}
