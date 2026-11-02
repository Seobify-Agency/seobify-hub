use crate::services::cwebp;

#[tauri::command]
pub async fn detect_cwebp() -> Result<cwebp::CwebpStatus, String> {
    Ok(cwebp::detect_cwebp().await)
}
