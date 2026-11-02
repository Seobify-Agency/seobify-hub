//! Seobify Hub shared layer. Source of truth: `Seobify-Agency/seobify-hub` → `template/src-tauri/src/hub/`.
//! Do not fork behaviour per tool; fix bugs upstream in the hub template and copy back.

pub mod db;
pub mod http;
pub mod mcp;
pub mod paths;
pub mod report;
pub mod secrets;
pub mod spider;
pub mod text;

/// Current UTC time as RFC 3339 (seconds precision), the timestamp format used in every hub DB.
pub fn now_iso() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}
