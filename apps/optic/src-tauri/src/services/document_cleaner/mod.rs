//! Office/ODF/PDF document metadata inspection and cleaning.
//!
//! Every carrier is handled in one place for both directions: the format walkers report a
//! finding and, when cleaning in a mode that covers the finding's scope, remove it in the
//! same step. A cleaned file is re-inspected to prove what is left.
//!
//! - OOXML (DOCX/XLSX/PPTX and their macro/template variants): `ooxml`
//! - ODF (ODT/ODS/ODP/ODG and templates): `odf`
//! - PDF: `pdf` (full rewrite through lopdf, so incremental revisions are dropped)

mod odf;
mod ooxml;
mod package;
mod pdf;
pub mod xml;

#[cfg(test)]
pub mod test_support;
#[cfg(test)]
mod tests;

use crate::services::dsp_pipeline::BrandStamp;
use crate::services::metadata_cleaner::Severity;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const DOCUMENT_EXTENSIONS: &[&str] = &[
    "docx", "docm", "dotx", "dotm", "xlsx", "xlsm", "xltx", "xltm", "pptx", "pptm", "potx", "potm", "ppsx", "ppsm", "odt", "ott", "ods", "ots", "odp",
    "otp", "odg", "otg", "pdf",
];

pub fn is_document_path(path: &Path) -> bool {
    path.extension()
        .map(|e| DOCUMENT_EXTENSIONS.contains(&e.to_string_lossy().to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocMode {
    /// Metadata parts/properties, thumbnails, printer settings, rsids, path leaks, embedded image metadata.
    #[default]
    Safe,
    /// Safe + comments and tracked-change authors/dates (+ optional speaker notes).
    Strict,
}

impl DocMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "safe" => Some(DocMode::Safe),
            "strict" => Some(DocMode::Strict),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            DocMode::Safe => "safe",
            DocMode::Strict => "strict",
        }
    }
}

/// Which cleaning step removes a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Safe,
    Strict,
    /// Strict mode with `remove_notes`.
    Notes,
    /// Reported only (active content, embedded objects, brand stamp, structure).
    Report,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocFinding {
    pub category: String,
    pub severity: Severity,
    pub location: String,
    pub name: String,
    pub detail: String,
    pub size_bytes: usize,
    pub value_preview: String,
    /// `true` when some cleaning mode removes this item (it counts as a trace).
    pub removable: bool,
    pub scope: Scope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocInspectReport {
    /// docx | xlsx | pptx | odt | ods | odp | odg | pdf
    pub format: String,
    /// word | spreadsheet | presentation | drawing | pdf
    pub family: String,
    pub file_size: u64,
    pub findings: Vec<DocFinding>,
    pub critical: u32,
    pub high: u32,
    pub medium: u32,
    pub low: u32,
    pub info: u32,
    /// Removable findings (traces) in any mode.
    pub trace_count: u32,
    /// Traces removed by safe mode.
    pub safe_count: u32,
    /// Additional traces only strict mode removes.
    pub strict_count: u32,
    /// Speaker notes (strict + remove_notes).
    pub notes_count: u32,
    pub clean: bool,
    pub summary: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocCleanOptions {
    #[serde(default)]
    pub mode: DocMode,
    /// Strict mode only: also delete PPTX speaker notes.
    #[serde(default)]
    pub remove_notes: bool,
    /// Optional clean attribution written after cleaning (OOXML Company + custom Copyright,
    /// ODF user-defined Company/Copyright, PDF Producer/Copyright). Off by default.
    #[serde(default)]
    pub brand_stamp: Option<BrandStamp>,
}

impl DocCleanOptions {
    fn stamp(&self) -> Option<&BrandStamp> {
        self.brand_stamp.as_ref().filter(|s| !s.is_empty())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocCleanResult {
    pub data: Vec<u8>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocProcessOutcome {
    pub input_path: String,
    pub output_path: String,
    pub format: String,
    pub mode: String,
    pub original_size: u64,
    pub file_size: u64,
    pub traces_before: u32,
    pub traces_after: u32,
    pub traces_removed: u32,
    pub clean: bool,
    /// Preview of the brand stamp found in the output, when one was requested.
    pub stamp_verified: Option<String>,
    pub warnings: Vec<String>,
    pub duration_ms: u64,
    pub before: DocInspectReport,
    pub after: DocInspectReport,
}

// ---------------------------------------------------------------------------
// Shared walker context
// ---------------------------------------------------------------------------

pub(crate) struct Ctx<'a> {
    pub findings: Vec<DocFinding>,
    pub opts: Option<&'a DocCleanOptions>,
    /// Stamp the inspector should recognise as deliberate attribution.
    pub known_stamp: Option<&'a BrandStamp>,
    pub warnings: Vec<String>,
}

impl<'a> Ctx<'a> {
    fn new(opts: Option<&'a DocCleanOptions>, known_stamp: Option<&'a BrandStamp>) -> Self {
        Self { findings: Vec::new(), opts, known_stamp, warnings: Vec::new() }
    }

    pub fn cleaning(&self) -> bool {
        self.opts.is_some()
    }

    pub fn applies(&self, scope: Scope) -> bool {
        match (self.opts, scope) {
            (None, _) | (_, Scope::Report) => false,
            (Some(_), Scope::Safe) => true,
            (Some(o), Scope::Strict) => o.mode == DocMode::Strict,
            (Some(o), Scope::Notes) => o.mode == DocMode::Strict && o.remove_notes,
        }
    }

    pub fn stamp(&self) -> Option<&'a BrandStamp> {
        self.opts.and_then(|o| o.stamp())
    }

    /// `true` when `value` is part of the stamp being written or recognised.
    pub fn is_stamp_value(&self, value: &str) -> bool {
        let v = value.trim();
        !v.is_empty()
            && self
                .known_stamp
                .map(|s| s.artist.trim() == v || s.copyright.trim() == v)
                .unwrap_or(false)
    }

    /// Records a finding; returns whether the current run removes it.
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        scope: Scope,
        category: &str,
        severity: Severity,
        location: impl Into<String>,
        name: impl Into<String>,
        detail: impl Into<String>,
        size_bytes: usize,
        value_preview: impl AsRef<str>,
    ) -> bool {
        self.findings.push(DocFinding {
            category: category.to_string(),
            severity,
            location: location.into(),
            name: name.into(),
            detail: detail.into(),
            size_bytes,
            value_preview: preview(value_preview.as_ref()),
            removable: scope != Scope::Report,
            scope,
        });
        self.applies(scope)
    }

    pub fn stamp_finding(&mut self, location: impl Into<String>, name: &str, value: &str) {
        self.add(Scope::Report, "brand-stamp", Severity::Info, location, name, "Verified brand stamp written by Seobify Optic (deliberate attribution, not a trace)", value.len(), value);
    }
}

const PREVIEW_LEN: usize = 96;

pub(crate) fn preview(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if out.chars().count() >= PREVIEW_LEN {
            out.push('…');
            break;
        }
        out.push(if ch.is_control() { ' ' } else { ch });
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Distinct non-empty names, joined for previews.
pub(crate) fn join_names(names: &[String]) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for n in names {
        let n = n.trim();
        if !n.is_empty() && !seen.contains(&n) {
            seen.push(n);
        }
    }
    seen.join(", ")
}

// ---------------------------------------------------------------------------
// Local file path leaks
// ---------------------------------------------------------------------------

fn is_path_terminator(c: char) -> bool {
    matches!(c, '"' | '\'' | '<' | '>' | '|' | ';' | '*' | '?' | '\n' | '\r' | '\t' | '\0')
}

/// Byte spans of absolute local paths inside `s`: drive paths (`C:\…`), UNC shares
/// (`\\server\…`), `file:` URLs and macOS/Linux home directories.
pub(crate) fn local_path_spans(s: &str) -> Vec<(usize, usize)> {
    let bytes = s.as_bytes();
    let lower = s.to_ascii_lowercase();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let prev_alnum = i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'.' || bytes[i - 1] == b'/');
        let rest = &lower.as_bytes()[i..];
        let start = if !prev_alnum && rest.starts_with(b"file:/") {
            true
        } else if !prev_alnum && bytes[i].is_ascii_alphabetic() && bytes.get(i + 1) == Some(&b':') && matches!(bytes.get(i + 2), Some(b'\\') | Some(b'/')) {
            true
        } else if !prev_alnum && rest.starts_with(b"\\\\") && bytes.get(i + 2).map(|b| b.is_ascii_alphanumeric()).unwrap_or(false) {
            true
        } else {
            !prev_alnum && (rest.starts_with(b"/users/") || rest.starts_with(b"/home/"))
        };
        if start {
            let end = s[i..].char_indices().find(|(_, c)| is_path_terminator(*c)).map(|(o, _)| i + o).unwrap_or(s.len());
            let end = i + s[i..end].trim_end().len();
            if end > i + 3 {
                spans.push((i, end));
            }
            i = end.max(i + 1);
            continue;
        }
        i += 1;
    }
    spans
}

pub(crate) fn contains_local_path(s: &str) -> bool {
    !local_path_spans(s).is_empty()
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or("")
}

/// Replaces every local path in `s` with its final component (a link to `C:\Users\x\a.pdf`
/// becomes `a.pdf`, resolved next to the document).
pub(crate) fn strip_local_paths(s: &str) -> String {
    let spans = local_path_spans(s);
    if spans.is_empty() {
        return s.to_string();
    }
    let mut out = String::new();
    let mut last = 0;
    for (a, b) in spans {
        out.push_str(&s[last..a]);
        out.push_str(basename(&s[a..b]));
        last = b;
    }
    out.push_str(&s[last..]);
    out
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

enum Kind {
    Ooxml,
    Odf,
    Pdf,
}

fn detect(data: &[u8]) -> Result<Kind, String> {
    if data.len() >= 5 && data[..1024.min(data.len())].windows(5).any(|w| w == b"%PDF-") {
        return Ok(Kind::Pdf);
    }
    if data.starts_with(b"PK\x03\x04") {
        let names = package::entry_names(data)?;
        if names.iter().any(|n| n.eq_ignore_ascii_case("[Content_Types].xml")) {
            return Ok(Kind::Ooxml);
        }
        if names.iter().any(|n| n == "mimetype") {
            return Ok(Kind::Odf);
        }
        return Err("ZIP archive is neither an Office Open XML nor an OpenDocument file".to_string());
    }
    if data.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) {
        return Err("Legacy binary Office files (.doc/.xls/.ppt) are not supported; save as .docx/.xlsx/.pptx first".to_string());
    }
    Err("Unsupported document format (expected DOCX/XLSX/PPTX, ODT/ODS/ODP or PDF)".to_string())
}

fn run(data: &[u8], ctx: &mut Ctx) -> Result<(String, String, Option<Vec<u8>>), String> {
    match detect(data)? {
        Kind::Ooxml => ooxml::run(data, ctx),
        Kind::Odf => odf::run(data, ctx),
        Kind::Pdf => pdf::run(data, ctx),
    }
}

fn finish(format: String, family: String, file_size: u64, mut findings: Vec<DocFinding>) -> DocInspectReport {
    findings.sort_by(|a, b| b.severity.cmp(&a.severity));
    let (mut critical, mut high, mut medium, mut low, mut info) = (0, 0, 0, 0, 0);
    let (mut safe_count, mut strict_count, mut notes_count) = (0, 0, 0);
    for f in &findings {
        match f.severity {
            Severity::Critical => critical += 1,
            Severity::High => high += 1,
            Severity::Medium => medium += 1,
            Severity::Low => low += 1,
            Severity::Info => info += 1,
        }
        match f.scope {
            Scope::Safe => safe_count += 1,
            Scope::Strict => strict_count += 1,
            Scope::Notes => notes_count += 1,
            Scope::Report => {}
        }
    }
    let trace_count = safe_count + strict_count + notes_count;
    let summary = if trace_count == 0 {
        format!("{}: no removable metadata traces", format.to_uppercase())
    } else {
        format!(
            "{}: {} trace(s) — safe mode removes {}, strict adds {}{}",
            format.to_uppercase(),
            trace_count,
            safe_count,
            strict_count,
            if notes_count > 0 { format!(", speaker notes {}", notes_count) } else { String::new() }
        )
    };
    DocInspectReport {
        format,
        family,
        file_size,
        findings,
        critical,
        high,
        medium,
        low,
        info,
        trace_count,
        safe_count,
        strict_count,
        notes_count,
        clean: trace_count == 0,
        summary,
    }
}

/// Reports every metadata carrier in an Office/ODF/PDF document. `known_stamp` makes the
/// inspector recognise a brand stamp written by Optic instead of counting it as a trace.
pub fn inspect_document_bytes(data: &[u8], known_stamp: Option<&BrandStamp>) -> Result<DocInspectReport, String> {
    let mut ctx = Ctx::new(None, known_stamp);
    let (format, family, _) = run(data, &mut ctx)?;
    Ok(finish(format, family, data.len() as u64, ctx.findings))
}

/// Removes the carriers covered by `opts.mode` and returns the rewritten document.
pub fn clean_document_bytes(data: &[u8], opts: &DocCleanOptions) -> Result<DocCleanResult, String> {
    let mut ctx = Ctx::new(Some(opts), opts.stamp());
    if opts.remove_notes && opts.mode != DocMode::Strict {
        ctx.warnings.push("remove_notes only applies in strict mode; speaker notes were kept".to_string());
    }
    let (_, _, cleaned) = run(data, &mut ctx)?;
    let data = cleaned.ok_or("cleaning produced no output")?;
    Ok(DocCleanResult { data, warnings: ctx.warnings })
}

/// `<stem>.clean.<ext>` next to the input.
pub fn default_output_path(input: &Path) -> std::path::PathBuf {
    let stem = input.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "document".to_string());
    let ext = input.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
    let name = if ext.is_empty() { format!("{}.clean", stem) } else { format!("{}.clean.{}", stem, ext) };
    input.with_file_name(name)
}

/// Output path for a batch: `<name>` inside `out_dir`, or `<stem>.clean.<ext>` when `out_dir`
/// is the source folder; names already used in this batch get `-2`, `-3` suffixes.
pub fn batch_output_path(input: &Path, out_dir: &Path, used: &mut std::collections::HashSet<String>) -> std::path::PathBuf {
    let same_dir = std::fs::canonicalize(input.parent().unwrap_or(Path::new("."))).ok() == std::fs::canonicalize(out_dir).ok();
    let name = if same_dir {
        default_output_path(input).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    } else {
        input.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "document".to_string())
    };
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{}", e)),
        None => (name.clone(), String::new()),
    };
    let mut candidate = name.clone();
    let mut n = 2;
    while !used.insert(out_dir.join(&candidate).to_string_lossy().to_lowercase()) {
        candidate = format!("{}-{}{}", stem, n, ext);
        n += 1;
    }
    out_dir.join(candidate)
}

/// Inspects, cleans and writes `output`, then re-inspects the written bytes. The input is never overwritten.
pub fn process_document_file(input: &Path, output: &Path, opts: &DocCleanOptions) -> Result<DocProcessOutcome, String> {
    let started = std::time::Instant::now();
    if !input.is_file() {
        return Err(format!("File not found: {}", input.display()));
    }
    let same = output == input || (output.exists() && std::fs::canonicalize(output).ok() == std::fs::canonicalize(input).ok());
    if same {
        return Err("output path must differ from the input (inputs are never overwritten)".to_string());
    }
    let data = std::fs::read(input).map_err(|e| format!("File read error: {}", e))?;
    let before = inspect_document_bytes(&data, None)?;
    let cleaned = clean_document_bytes(&data, opts)?;
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create output directory: {}", e))?;
        }
    }
    std::fs::write(output, &cleaned.data).map_err(|e| format!("Write error: {}", e))?;
    let written = std::fs::read(output).map_err(|e| format!("Failed to read the written file: {}", e))?;
    let after = inspect_document_bytes(&written, opts.stamp())?;
    let stamp_verified = if opts.stamp().is_some() {
        let found: Vec<String> = after.findings.iter().filter(|f| f.category == "brand-stamp").map(|f| f.value_preview.clone()).collect();
        if found.is_empty() {
            None
        } else {
            Some(found.join(" · "))
        }
    } else {
        None
    };
    Ok(DocProcessOutcome {
        input_path: input.to_string_lossy().to_string(),
        output_path: output.to_string_lossy().to_string(),
        format: before.format.clone(),
        mode: opts.mode.label().to_string(),
        original_size: data.len() as u64,
        file_size: written.len() as u64,
        traces_before: before.trace_count,
        traces_after: after.trace_count,
        traces_removed: before.trace_count.saturating_sub(after.trace_count),
        clean: after.clean,
        stamp_verified,
        warnings: cleaned.warnings,
        duration_ms: started.elapsed().as_millis() as u64,
        before,
        after,
    })
}
