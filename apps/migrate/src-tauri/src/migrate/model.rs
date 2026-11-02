//! Data types shared by the store, the matching engine, the GUI and the MCP tools.

use serde::{Deserialize, Serialize};

/// Mapping states: `auto` (≥ 90), `review` (60–89), `weak` (< 60), `manual` (user chose),
/// `gone` (410) and `keep` (same URL, no redirect needed).
pub const STATUSES: &[&str] = &["auto", "review", "weak", "manual", "gone", "keep"];

/// Status for an engine confidence score.
pub fn status_for(confidence: i64) -> &'static str {
    if confidence >= 90 {
        "auto"
    } else if confidence >= 60 {
        "review"
    } else {
        "weak"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct MatchOptions {
    /// Compare paths case-insensitively (`/Hakkimizda` = `/hakkimizda`).
    pub case_insensitive: bool,
    /// Ignore query strings when comparing URLs and building rules.
    pub ignore_query: bool,
    /// Unmatched URLs without a priority value get 410 instead of a parent/homepage fallback.
    pub gone_without_value: bool,
    /// Minimum fuzzy score (0–100) for a candidate to be proposed.
    pub fuzzy_min: i64,
}

impl Default for MatchOptions {
    fn default() -> Self {
        MatchOptions { case_insensitive: true, ignore_query: true, gone_without_value: false, fuzzy_min: 35 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub old_base: String,
    pub new_base: String,
    pub staging_base: Option<String>,
    pub notes: Option<String>,
    pub options: MatchOptions,
    pub created_at: String,
    pub updated_at: String,
}

/// Fields the user edits when creating or updating a plan.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanInput {
    pub name: String,
    pub old_base: String,
    pub new_base: String,
    #[serde(default)]
    pub staging_base: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub options: Option<MatchOptions>,
}

/// One URL of the old or the new set.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UrlRow {
    pub id: i64,
    pub side: String,
    pub url: String,
    pub path: String,
    pub slug: String,
    pub title: Option<String>,
    pub h1: Option<String>,
    pub meta_description: Option<String>,
    pub status_code: Option<i64>,
    pub indexability: Option<String>,
    pub canonical: Option<String>,
    pub word_count: Option<i64>,
    /// 64-bit content fingerprint as 16 hex digits.
    pub simhash: Option<String>,
    /// Priority value (clicks, referring domains…).
    pub value: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pattern {
    pub id: i64,
    pub plan_id: i64,
    pub position: i64,
    pub regex: String,
    pub replacement: String,
    pub enabled: bool,
    pub note: Option<String>,
}

/// Per-signal similarity (0–1) behind a candidate score.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Breakdown {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub simhash: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub url: String,
    pub score: i64,
    pub reason: String,
    #[serde(default)]
    pub breakdown: Breakdown,
}

/// Engine output for one old URL.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchResult {
    pub old_id: i64,
    pub new_url: Option<String>,
    pub status: String,
    pub confidence: i64,
    pub reason: String,
    pub candidates: Vec<Candidate>,
    pub http_status: i64,
}

/// A mapping joined with its old URL and (when known) the target in the new set.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MappingRow {
    pub id: i64,
    pub old_url_id: i64,
    pub old_url: String,
    pub old_title: Option<String>,
    pub old_status: Option<i64>,
    pub value: Option<f64>,
    pub new_url: Option<String>,
    pub new_title: Option<String>,
    pub new_status: Option<i64>,
    pub status: String,
    pub confidence: i64,
    pub reason: String,
    pub approved: bool,
    pub http_status: i64,
    pub candidates: Vec<Candidate>,
    pub updated_at: String,
}
