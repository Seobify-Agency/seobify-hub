//! Seobify Migrate domain logic: plans, URL sets, the matching engine, validation, rule
//! exporters, verification and reports. Tauri commands and MCP tools both call `service`,
//! so every capability has the same behaviour in the GUI and for Claude.

pub mod fetch;
pub mod matcher;
pub mod model;
pub mod norm;
pub mod report;
pub mod rules;
pub mod service;
pub mod store;
pub mod validate;
pub mod verify;

/// Progress callback `(done, total)` used by long jobs (content fetch, verification).
pub type Progress = std::sync::Arc<dyn Fn(usize, usize) + Send + Sync>;
