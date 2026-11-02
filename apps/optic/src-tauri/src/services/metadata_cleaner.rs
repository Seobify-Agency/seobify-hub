//! Byte-level container inspection and lossless metadata removal for PNG, JPEG and WebP.
//!
//! `inspect_image_bytes` walks the container structure (PNG chunks, JPEG segments,
//! RIFF chunks) and reports every metadata carrier it finds: EXIF/GPS IFDs, XMP packets,
//! IPTC/8BIM resources, ICC profiles, JUMBF/C2PA manifests, text chunks with generator
//! parameters, thumbnails, comments, quantization-table fingerprints and trailing bytes.
//!
//! `clean_image_bytes` re-packs the container keeping only the chunks/segments needed to
//! decode the pixels, so the decoded image is bit-identical to the input.

use crate::services::dsp_pipeline::BrandStamp;
use crate::services::image::{detect_format, read_dimensions, ImageFormat};
use flate2::read::ZlibDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub category: String,
    pub severity: Severity,
    pub location: String,
    pub name: String,
    pub detail: String,
    pub size_bytes: usize,
    pub value_preview: String,
    /// `true` when the lossless cleaner removes this item. Structural information
    /// (dimensions, codec tables) is reported but is inherent to the bitstream.
    pub removable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectReport {
    pub format: String,
    pub file_size: u64,
    pub width: u32,
    pub height: u32,
    pub findings: Vec<Finding>,
    pub critical: u32,
    pub high: u32,
    pub medium: u32,
    pub low: u32,
    pub info: u32,
    /// Number of removable findings (metadata traces).
    pub trace_count: u32,
    pub clean: bool,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanResult {
    pub data: Vec<u8>,
    pub format: ImageFormat,
    pub removed_bytes: usize,
    pub removed_items: u32,
}

const PREVIEW_LEN: usize = 96;

const GENERATOR_KEYS: &[&str] = &[
    "parameters",
    "prompt",
    "workflow",
    "comfy",
    "sd-metadata",
    "invokeai",
    "novelai",
    "dream",
    "openai",
    "dall-e",
    "midjourney",
    "firefly",
    "flux",
    "software",
    "comment",
    "description",
];

const GENERATOR_MARKERS: &[&str] = &[
    "steps:",
    "sampler:",
    "model hash:",
    "cfg scale:",
    "\"class_type\":",
    "--ar",
    "--v",
    "trainedalgorithmicmedia",
    "compositewithtrainedalgorithmicmedia",
    "gpt-image",
    "dall-e",
    "imagen",
    "gemini",
    "midjourney",
    "firefly",
    "flux",
];

const XMP_PROVENANCE_MARKERS: &[&str] = &[
    "iptc4xmpext:digitalsourcetype",
    "trainedalgorithmicmedia",
    "compositewithtrainedalgorithmicmedia",
    "firefly",
    "openai",
    "chatgpt",
    "gpt-image",
    "gemini",
    "imagen",
    "midjourney",
    "dall-e",
    "c2pa",
];

const XMP_HISTORY_MARKERS: &[&str] = &["xmpmm:history", "photoshop:", "xmp:creatortool", "xmpmm:documentid", "xmpmm:instanceid", "exif:gps"];

const JUMBF_LABELS: &[&[u8]] = &[b"c2pa", b"c2ma", b"c2as", b"c2cl"];

// ---------------------------------------------------------------------------
// Report assembly
// ---------------------------------------------------------------------------

struct Collector {
    findings: Vec<Finding>,
}

impl Collector {
    fn new() -> Self {
        Self { findings: Vec::new() }
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        category: &str,
        severity: Severity,
        location: String,
        name: String,
        detail: String,
        size_bytes: usize,
        value_preview: String,
        removable: bool,
    ) {
        self.findings.push(Finding {
            category: category.to_string(),
            severity,
            location,
            name,
            detail,
            size_bytes,
            value_preview,
            removable,
        });
    }
}

fn preview(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    for ch in text.chars() {
        if out.chars().count() >= PREVIEW_LEN {
            out.push('…');
            break;
        }
        if ch.is_control() {
            if ch == '\n' || ch == '\r' || ch == '\t' {
                out.push(' ');
            } else {
                out.push('.');
            }
        } else if ch == '\u{FFFD}' {
            out.push('.');
        } else {
            out.push(ch);
        }
    }
    out
}

fn hex_preview(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(24)
        .map(|b| format!("{:02x}", b))
        .collect::<Vec<_>>()
        .join(" ")
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.contains(needle)
}

fn matched_markers<'a>(lower_text: &str, markers: &'a [&'a str]) -> Vec<&'a str> {
    markers
        .iter()
        .copied()
        .filter(|m| contains_ci(lower_text, m))
        .collect()
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn looks_like_jumbf(payload: &[u8]) -> bool {
    let has_box = contains_bytes(payload, b"jumb") || contains_bytes(payload, b"jumd");
    let has_label = JUMBF_LABELS.iter().any(|l| contains_bytes(payload, l));
    has_box && has_label
}

fn mentions_c2pa(payload: &[u8]) -> bool {
    JUMBF_LABELS.iter().any(|l| contains_bytes(payload, l)) || contains_bytes(payload, b"jumb")
}

fn finish(format: &str, data: &[u8], width: u32, height: u32, mut findings: Vec<Finding>) -> InspectReport {
    findings.sort_by(|a, b| b.severity.cmp(&a.severity));
    let mut critical = 0;
    let mut high = 0;
    let mut medium = 0;
    let mut low = 0;
    let mut info = 0;
    let mut trace_count = 0;
    for f in &findings {
        match f.severity {
            Severity::Critical => critical += 1,
            Severity::High => high += 1,
            Severity::Medium => medium += 1,
            Severity::Low => low += 1,
            Severity::Info => info += 1,
        }
        if f.removable {
            trace_count += 1;
        }
    }
    let clean = trace_count == 0;
    let summary = if clean {
        format!("{}: no removable metadata traces", format.to_uppercase())
    } else {
        format!(
            "{}: {} trace(s) — {} critical, {} high, {} medium, {} low",
            format.to_uppercase(),
            trace_count,
            critical,
            high,
            medium,
            low
        )
    };
    InspectReport {
        format: format.to_string(),
        file_size: data.len() as u64,
        width,
        height,
        findings,
        critical,
        high,
        medium,
        low,
        info,
        trace_count,
        clean,
        summary,
    }
}

/// Inspect raw image bytes and report every metadata carrier found in the container.
pub fn inspect_image_bytes(data: &[u8]) -> InspectReport {
    match detect_format(data) {
        Ok(ImageFormat::Png) => inspect_png(data),
        Ok(ImageFormat::Jpeg) => inspect_jpeg(data),
        Ok(ImageFormat::WebP) => inspect_webp(data),
        Err(e) => {
            let mut c = Collector::new();
            c.push(
                "structure",
                Severity::Info,
                "@0x0000".to_string(),
                "unknown-format".to_string(),
                e,
                data.len(),
                hex_preview(data),
                false,
            );
            finish("unknown", data, 0, 0, c.findings)
        }
    }
}

// ---------------------------------------------------------------------------
// Shared analyzers: text, XMP, EXIF (TIFF), ICC, IPTC
// ---------------------------------------------------------------------------

fn is_brand_keyword(keyword: &str) -> bool {
    matches!(keyword.trim().to_ascii_lowercase().as_str(), "author" | "copyright")
}

fn analyze_text(c: &mut Collector, location: String, name: String, keyword: &str, text: &[u8], size: usize) {
    let text_str = String::from_utf8_lossy(text).to_string();
    let lower_text = text_str.to_lowercase();
    let lower_key = keyword.to_lowercase();
    let key_hit = GENERATOR_KEYS.iter().any(|k| lower_key.contains(k));
    let markers = matched_markers(&lower_text, GENERATOR_MARKERS);
    if is_brand_keyword(keyword) && markers.is_empty() {
        // Plain Author/Copyright attribution: the brand stamp written by this tool.
        c.push(
            "brand-stamp",
            Severity::Info,
            location,
            format!("Verified Brand Stamp ({})", name),
            format!("Attribution field '{}' (copyright/author) — not a privacy trace", keyword),
            size,
            preview(text),
            false,
        );
        return;
    }
    let (severity, category, detail) = if !markers.is_empty() {
        (
            Severity::Critical,
            "generator",
            format!(
                "Generation/software parameters detected in text (markers: {})",
                markers.join(", ")
            ),
        )
    } else if key_hit {
        (
            Severity::High,
            "generator",
            format!("Text keyword '{}' is a known software/generator metadata key", keyword),
        )
    } else {
        (
            Severity::Medium,
            "text",
            format!("Text metadata (keyword '{}')", keyword),
        )
    };
    c.push(category, severity, location, name, detail, size, preview(text), true);
}

fn analyze_xmp(c: &mut Collector, location: String, name: String, xmp: &[u8], size: usize) {
    let text = String::from_utf8_lossy(xmp).to_string();
    let lower = text.to_lowercase();
    let provenance = matched_markers(&lower, XMP_PROVENANCE_MARKERS);
    let history = matched_markers(&lower, XMP_HISTORY_MARKERS);
    let generator = matched_markers(&lower, GENERATOR_MARKERS);

    let mut details: Vec<String> = Vec::new();
    if let Some(v) = extract_xml_value(&text, "xmp:CreatorTool") {
        details.push(format!("CreatorTool={}", v));
    }
    if let Some(v) = extract_xml_value(&text, "Iptc4xmpExt:DigitalSourceType") {
        details.push(format!("DigitalSourceType={}", v));
    }
    if let Some(v) = extract_xml_value(&text, "dc:creator") {
        details.push(format!("dc:creator={}", v));
    }
    if let Some(v) = extract_xml_value(&text, "xmpMM:DocumentID") {
        details.push(format!("DocumentID={}", v));
    }
    if !provenance.is_empty() {
        details.push(format!("provenance markers: {}", provenance.join(", ")));
    }
    if !history.is_empty() {
        details.push(format!("history/software markers: {}", history.join(", ")));
    }
    if !generator.is_empty() {
        details.push(format!("generator markers: {}", generator.join(", ")));
    }

    let severity = if !provenance.is_empty() || !generator.is_empty() {
        Severity::Critical
    } else {
        Severity::High
    };
    let detail = if details.is_empty() {
        "XMP packet (document identity, software and edit history metadata)".to_string()
    } else {
        format!("XMP packet — {}", details.join("; "))
    };
    let preview_src = extract_xml_value(&text, "xmp:CreatorTool")
        .or_else(|| extract_xml_value(&text, "Iptc4xmpExt:DigitalSourceType"))
        .unwrap_or_else(|| preview(xmp));
    c.push("xmp", severity, location, name, detail, size, preview_src, true);
}

/// Extracts `name="value"` attribute form or `<name>value</name>` element form.
fn extract_xml_value(text: &str, name: &str) -> Option<String> {
    let attr = format!("{}=\"", name);
    if let Some(start) = text.find(&attr) {
        let rest = &text[start + attr.len()..];
        if let Some(end) = rest.find('"') {
            let v = rest[..end].trim();
            if !v.is_empty() {
                return Some(v.chars().take(PREVIEW_LEN).collect());
            }
        }
    }
    let open = format!("<{}>", name);
    let close = format!("</{}>", name);
    if let Some(start) = text.find(&open) {
        let rest = &text[start + open.len()..];
        if let Some(end) = rest.find(&close) {
            let inner = rest[..end].trim();
            // Values may be wrapped in rdf:Seq/rdf:li.
            let cleaned = if let Some(li) = inner.find("<rdf:li") {
                let after = &inner[li..];
                let gt = after.find('>').map(|i| i + 1).unwrap_or(0);
                let body = &after[gt..];
                let end_li = body.find("</rdf:li>").unwrap_or(body.len());
                body[..end_li].trim().to_string()
            } else {
                inner.to_string()
            };
            if !cleaned.is_empty() {
                return Some(cleaned.chars().take(PREVIEW_LEN).collect());
            }
        }
    }
    None
}

// --- EXIF / TIFF -----------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum IfdKind {
    Ifd0,
    Ifd1,
    Exif,
    Gps,
    Interop,
}

impl IfdKind {
    fn label(self) -> &'static str {
        match self {
            IfdKind::Ifd0 => "IFD0",
            IfdKind::Ifd1 => "IFD1",
            IfdKind::Exif => "ExifIFD",
            IfdKind::Gps => "GPSIFD",
            IfdKind::Interop => "InteropIFD",
        }
    }
}

struct Tiff<'a> {
    data: &'a [u8],
    le: bool,
}

#[derive(Debug, Clone)]
struct ExifEntry {
    ifd: &'static str,
    tag: u16,
    name: String,
    value: String,
    raw_len: usize,
}

struct ExifSummary {
    entries: Vec<ExifEntry>,
    total_tags: usize,
    thumbnail: Option<(usize, usize)>,
    gps: Vec<ExifEntry>,
}

impl<'a> Tiff<'a> {
    fn parse(data: &'a [u8]) -> Option<Tiff<'a>> {
        if data.len() < 8 {
            return None;
        }
        let le = match &data[0..2] {
            b"II" => true,
            b"MM" => false,
            _ => return None,
        };
        let t = Tiff { data, le };
        if t.u16(2)? != 42 {
            return None;
        }
        Some(t)
    }

    fn u16(&self, off: usize) -> Option<u16> {
        let b = self.data.get(off..off + 2)?;
        Some(if self.le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    }

    fn u32(&self, off: usize) -> Option<u32> {
        let b = self.data.get(off..off + 4)?;
        Some(if self.le {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    }

    fn type_size(t: u16) -> usize {
        match t {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 => 8,
            _ => 0,
        }
    }

    fn value_bytes(&self, entry_off: usize) -> Option<(u16, usize, &'a [u8])> {
        let ty = self.u16(entry_off + 2)?;
        let count = self.u32(entry_off + 4)? as usize;
        let size = Self::type_size(ty);
        if size == 0 {
            return None;
        }
        let total = size.checked_mul(count)?;
        let bytes = if total <= 4 {
            self.data.get(entry_off + 8..entry_off + 8 + total)?
        } else {
            let off = self.u32(entry_off + 8)? as usize;
            self.data.get(off..off.checked_add(total)?)?
        };
        Some((ty, count, bytes))
    }

    fn format_value(&self, ty: u16, count: usize, bytes: &[u8], gps_coord: bool) -> String {
        match ty {
            2 => {
                let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
                String::from_utf8_lossy(&bytes[..end]).trim().to_string()
            }
            1 | 6 | 7 => {
                if ty == 7 && bytes.len() >= 8 {
                    // UNDEFINED with a character-code prefix (UserComment).
                    let code = &bytes[..8];
                    if code.starts_with(b"ASCII") || code.starts_with(b"UNICODE") || code.iter().all(|&b| b == 0) {
                        let body = &bytes[8..];
                        let text = if code.starts_with(b"UNICODE") {
                            let units: Vec<u16> = body
                                .chunks(2)
                                .filter(|c| c.len() == 2)
                                .map(|c| if self.le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
                                .collect();
                            String::from_utf16_lossy(&units)
                        } else {
                            String::from_utf8_lossy(body).to_string()
                        };
                        return text.trim_matches(char::from(0)).trim().to_string();
                    }
                }
                if count <= 32 {
                    hex_preview(bytes)
                } else {
                    format!("{} bytes: {}", count, hex_preview(bytes))
                }
            }
            3 => {
                let vals: Vec<String> = bytes
                    .chunks(2)
                    .take(16)
                    .filter(|c| c.len() == 2)
                    .map(|c| {
                        (if self.le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).to_string()
                    })
                    .collect();
                vals.join(" ")
            }
            4 | 9 => {
                let vals: Vec<String> = bytes
                    .chunks(4)
                    .take(16)
                    .filter(|c| c.len() == 4)
                    .map(|c| {
                        let v = if self.le { u32::from_le_bytes([c[0], c[1], c[2], c[3]]) } else { u32::from_be_bytes([c[0], c[1], c[2], c[3]]) };
                        if ty == 9 { (v as i32).to_string() } else { v.to_string() }
                    })
                    .collect();
                vals.join(" ")
            }
            5 | 10 => {
                let rats: Vec<f64> = bytes
                    .chunks(8)
                    .take(16)
                    .filter(|c| c.len() == 8)
                    .map(|c| {
                        let n = if self.le { u32::from_le_bytes([c[0], c[1], c[2], c[3]]) } else { u32::from_be_bytes([c[0], c[1], c[2], c[3]]) };
                        let d = if self.le { u32::from_le_bytes([c[4], c[5], c[6], c[7]]) } else { u32::from_be_bytes([c[4], c[5], c[6], c[7]]) };
                        if ty == 10 {
                            let (n, d) = (n as i32 as f64, d as i32 as f64);
                            if d == 0.0 { 0.0 } else { n / d }
                        } else if d == 0 {
                            0.0
                        } else {
                            n as f64 / d as f64
                        }
                    })
                    .collect();
                if gps_coord && rats.len() >= 3 {
                    let dec = rats[0] + rats[1] / 60.0 + rats[2] / 3600.0;
                    format!("{}° {}' {:.3}\" ({:.6})", rats[0], rats[1], rats[2], dec)
                } else {
                    rats.iter().map(|v| format!("{:.4}", v)).collect::<Vec<_>>().join(" ")
                }
            }
            _ => hex_preview(bytes),
        }
    }

    fn walk(&self, ifd_off: usize, kind: IfdKind, out: &mut ExifSummary, visited: &mut Vec<usize>, depth: u8) {
        if depth > 4 || visited.contains(&ifd_off) || visited.len() > 16 {
            return;
        }
        visited.push(ifd_off);
        let count = match self.u16(ifd_off) {
            Some(c) => c as usize,
            None => return,
        };
        if count == 0 || count > 512 {
            return;
        }
        let mut thumb_off: Option<usize> = None;
        let mut thumb_len: Option<usize> = None;
        for i in 0..count {
            let e = ifd_off + 2 + i * 12;
            let tag = match self.u16(e) {
                Some(t) => t,
                None => break,
            };
            out.total_tags += 1;
            let (ty, cnt, bytes) = match self.value_bytes(e) {
                Some(v) => v,
                None => continue,
            };
            // Sub-IFD pointers
            match (kind, tag) {
                (IfdKind::Ifd0, 0x8769) => {
                    if let Some(off) = self.u32(e + 8) {
                        self.walk(off as usize, IfdKind::Exif, out, visited, depth + 1);
                    }
                    continue;
                }
                (IfdKind::Ifd0, 0x8825) => {
                    if let Some(off) = self.u32(e + 8) {
                        self.walk(off as usize, IfdKind::Gps, out, visited, depth + 1);
                    }
                    continue;
                }
                (IfdKind::Exif, 0xA005) => {
                    if let Some(off) = self.u32(e + 8) {
                        self.walk(off as usize, IfdKind::Interop, out, visited, depth + 1);
                    }
                    continue;
                }
                (IfdKind::Ifd1, 0x0201) => {
                    thumb_off = self.u32(e + 8).map(|v| v as usize);
                    continue;
                }
                (IfdKind::Ifd1, 0x0202) => {
                    thumb_len = self.u32(e + 8).map(|v| v as usize);
                    continue;
                }
                _ => {}
            }
            let name = tag_name(kind, tag);
            let Some(name) = name else { continue };
            let gps_coord = kind == IfdKind::Gps && (tag == 2 || tag == 4);
            let value = self.format_value(ty, cnt, bytes, gps_coord);
            let entry = ExifEntry {
                ifd: kind.label(),
                tag,
                name: name.to_string(),
                value,
                raw_len: bytes.len(),
            };
            if kind == IfdKind::Gps {
                out.gps.push(entry);
            } else {
                out.entries.push(entry);
            }
        }
        if kind == IfdKind::Ifd1 {
            if let (Some(o), Some(l)) = (thumb_off, thumb_len) {
                out.thumbnail = Some((o, l));
            }
        }
        if kind == IfdKind::Ifd0 {
            let next = ifd_off + 2 + count * 12;
            if let Some(next_ifd) = self.u32(next) {
                if next_ifd != 0 {
                    self.walk(next_ifd as usize, IfdKind::Ifd1, out, visited, depth + 1);
                }
            }
        }
    }
}

fn tag_name(kind: IfdKind, tag: u16) -> Option<&'static str> {
    match kind {
        IfdKind::Ifd0 | IfdKind::Ifd1 => match tag {
            0x010E => Some("ImageDescription"),
            0x010F => Some("Make"),
            0x0110 => Some("Model"),
            0x0112 => Some("Orientation"),
            0x0131 => Some("Software"),
            0x0132 => Some("DateTime"),
            0x013B => Some("Artist"),
            0x013C => Some("HostComputer"),
            0x8298 => Some("Copyright"),
            0x9C9B => Some("XPTitle"),
            0x9C9C => Some("XPComment"),
            0x9C9D => Some("XPAuthor"),
            0x9C9E => Some("XPKeywords"),
            0x9C9F => Some("XPSubject"),
            0xC4A5 => Some("PrintImageMatching"),
            0x02BC => Some("XMP (embedded in TIFF)"),
            0x83BB => Some("IPTC-NAA (embedded in TIFF)"),
            0x8773 => Some("ICC Profile (embedded in TIFF)"),
            _ => None,
        },
        IfdKind::Exif => match tag {
            0x9003 => Some("DateTimeOriginal"),
            0x9004 => Some("DateTimeDigitized"),
            0x9286 => Some("UserComment"),
            0x927C => Some("MakerNote"),
            0xA420 => Some("ImageUniqueID"),
            0xA430 => Some("CameraOwnerName"),
            0xA431 => Some("BodySerialNumber"),
            0xA432 => Some("LensSpecification"),
            0xA433 => Some("LensMake"),
            0xA434 => Some("LensModel"),
            0xA435 => Some("LensSerialNumber"),
            0x9010 => Some("OffsetTime"),
            0x9011 => Some("OffsetTimeOriginal"),
            0xA000 => Some("FlashpixVersion"),
            0x829A => Some("ExposureTime"),
            0x829D => Some("FNumber"),
            0x8827 => Some("ISOSpeedRatings"),
            0x920A => Some("FocalLength"),
            _ => None,
        },
        IfdKind::Gps => match tag {
            0x0000 => Some("GPSVersionID"),
            0x0001 => Some("GPSLatitudeRef"),
            0x0002 => Some("GPSLatitude"),
            0x0003 => Some("GPSLongitudeRef"),
            0x0004 => Some("GPSLongitude"),
            0x0005 => Some("GPSAltitudeRef"),
            0x0006 => Some("GPSAltitude"),
            0x0007 => Some("GPSTimeStamp"),
            0x0010 => Some("GPSImgDirection"),
            0x0012 => Some("GPSMapDatum"),
            0x001D => Some("GPSDateStamp"),
            0x001F => Some("GPSHPositioningError"),
            _ => None,
        },
        IfdKind::Interop => match tag {
            0x0001 => Some("InteropIndex"),
            _ => None,
        },
    }
}

fn exif_tag_severity(name: &str) -> Severity {
    match name {
        "BodySerialNumber" | "LensSerialNumber" | "ImageUniqueID" | "CameraOwnerName" | "Artist"
        | "Copyright" | "UserComment" | "XPAuthor" | "XPComment" | "MakerNote" | "HostComputer"
        | "XMP (embedded in TIFF)" | "IPTC-NAA (embedded in TIFF)" => Severity::High,
        "ImageDescription" | "XPTitle" | "XPKeywords" | "XPSubject" | "Software" | "Make" | "Model"
        | "LensModel" | "LensMake" | "DateTime" | "DateTimeOriginal" | "DateTimeDigitized"
        | "OffsetTime" | "OffsetTimeOriginal" | "ICC Profile (embedded in TIFF)" => Severity::Medium,
        _ => Severity::Low,
    }
}

fn analyze_exif(c: &mut Collector, location: &str, container_name: &str, tiff: &[u8], size: usize) {
    let Some(t) = Tiff::parse(tiff) else {
        c.push(
            "exif",
            Severity::High,
            location.to_string(),
            container_name.to_string(),
            "EXIF block present but TIFF header could not be parsed".to_string(),
            size,
            hex_preview(tiff),
            true,
        );
        return;
    };
    let mut summary = ExifSummary {
        entries: Vec::new(),
        total_tags: 0,
        thumbnail: None,
        gps: Vec::new(),
    };
    let mut visited = Vec::new();
    if let Some(ifd0) = t.u32(4) {
        t.walk(ifd0 as usize, IfdKind::Ifd0, &mut summary, &mut visited, 0);
    }

    let only_attribution = !summary.entries.is_empty()
        && summary.entries.len() == summary.total_tags
        && summary.gps.is_empty()
        && summary.thumbnail.is_none()
        && visited.len() == 1
        && summary.entries.iter().all(|e| e.name == "Artist" || e.name == "Copyright");
    if only_attribution {
        let value = summary
            .entries
            .iter()
            .map(|e| format!("{}: {}", e.name, e.value))
            .collect::<Vec<_>>()
            .join(" — ");
        c.push(
            "brand-stamp",
            Severity::Info,
            location.to_string(),
            format!("Verified Brand Stamp ({})", container_name),
            "Minimal EXIF carrying only Artist/Copyright attribution — not a privacy trace".to_string(),
            size,
            value,
            false,
        );
        return;
    }

    let software_hint = summary
        .entries
        .iter()
        .find(|e| e.name == "Software" || e.name == "Model")
        .map(|e| e.value.clone())
        .unwrap_or_default();
    c.push(
        "exif",
        Severity::High,
        location.to_string(),
        container_name.to_string(),
        format!(
            "EXIF/TIFF block with {} tag(s) across {} IFD(s){}{}",
            summary.total_tags,
            visited.len(),
            if summary.gps.is_empty() { "" } else { " — includes GPS IFD" },
            if summary.thumbnail.is_some() { " — includes IFD1 thumbnail" } else { "" }
        ),
        size,
        if software_hint.is_empty() { hex_preview(tiff) } else { software_hint },
        true,
    );

    for e in &summary.entries {
        let sev = exif_tag_severity(&e.name);
        c.push(
            "exif",
            sev,
            format!("{} > {}", location, e.ifd),
            format!("EXIF {} (0x{:04X})", e.name, e.tag),
            match e.name.as_str() {
                "MakerNote" => "Proprietary maker note (frequently embeds serial numbers, owner name and firmware details)".to_string(),
                "BodySerialNumber" | "LensSerialNumber" => "Hardware serial number — uniquely identifies the device".to_string(),
                "ImageUniqueID" => "Unique image identifier assigned by the capturing software".to_string(),
                "DateTimeOriginal" | "DateTime" | "DateTimeDigitized" => "Capture/modification timestamp".to_string(),
                _ => format!("{} value", e.name),
            },
            e.raw_len,
            e.value.clone(),
            true,
        );
    }

    if !summary.gps.is_empty() {
        let lat = summary.gps.iter().find(|g| g.name == "GPSLatitude").map(|g| g.value.clone());
        let lon = summary.gps.iter().find(|g| g.name == "GPSLongitude").map(|g| g.value.clone());
        let lat_ref = summary.gps.iter().find(|g| g.name == "GPSLatitudeRef").map(|g| g.value.clone()).unwrap_or_default();
        let lon_ref = summary.gps.iter().find(|g| g.name == "GPSLongitudeRef").map(|g| g.value.clone()).unwrap_or_default();
        let coords = match (lat, lon) {
            (Some(la), Some(lo)) => format!("{} {} / {} {}", la, lat_ref, lo, lon_ref),
            _ => summary
                .gps
                .iter()
                .map(|g| format!("{}={}", g.name, g.value))
                .collect::<Vec<_>>()
                .join("; "),
        };
        c.push(
            "gps",
            Severity::Critical,
            format!("{} > GPSIFD", location),
            "EXIF GPS location".to_string(),
            format!(
                "Geolocation data ({} GPS tag(s)): {}",
                summary.gps.len(),
                summary
                    .gps
                    .iter()
                    .map(|g| g.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            summary.gps.iter().map(|g| g.raw_len).sum(),
            coords,
            true,
        );
    }

    if let Some((off, len)) = summary.thumbnail {
        let valid = off.checked_add(len).map(|end| end <= tiff.len()).unwrap_or(false);
        c.push(
            "thumbnail",
            Severity::High,
            format!("{} > IFD1", location),
            "EXIF IFD1 embedded thumbnail".to_string(),
            if valid {
                format!("Embedded JPEG thumbnail ({} bytes) — may show uncropped/unedited original content", len)
            } else {
                format!("IFD1 thumbnail pointer (offset {}, length {}) out of bounds", off, len)
            },
            len,
            if valid { hex_preview(&tiff[off..off + len.min(24)]) } else { String::new() },
            true,
        );
    }
}

// --- ICC ---------------------------------------------------------------------

fn icc_description(profile: &[u8]) -> (String, String) {
    if profile.len() < 132 {
        return (String::new(), String::new());
    }
    let be32 = |o: usize| -> u32 {
        u32::from_be_bytes([profile[o], profile[o + 1], profile[o + 2], profile[o + 3]])
    };
    let profile_id: String = profile[84..100].iter().map(|b| format!("{:02x}", b)).collect();
    let device_class = String::from_utf8_lossy(&profile[12..16]).to_string();
    let color_space = String::from_utf8_lossy(&profile[16..20]).trim().to_string();
    let cmm = String::from_utf8_lossy(&profile[4..8]).trim().to_string();
    let tag_count = be32(128) as usize;
    let mut desc = String::new();
    for i in 0..tag_count.min(256) {
        let e = 132 + i * 12;
        if e + 12 > profile.len() {
            break;
        }
        if &profile[e..e + 4] == b"desc" {
            let off = be32(e + 4) as usize;
            let len = be32(e + 8) as usize;
            if let Some(tag) = profile.get(off..off.saturating_add(len).min(profile.len())) {
                if tag.len() >= 12 {
                    match &tag[0..4] {
                        b"desc" => {
                            let n = u32::from_be_bytes([tag[8], tag[9], tag[10], tag[11]]) as usize;
                            let body = &tag[12..(12 + n).min(tag.len())];
                            let end = body.iter().position(|&b| b == 0).unwrap_or(body.len());
                            desc = String::from_utf8_lossy(&body[..end]).to_string();
                        }
                        b"mluc" => {
                            if tag.len() >= 28 {
                                let rec_len = u32::from_be_bytes([tag[20], tag[21], tag[22], tag[23]]) as usize;
                                let rec_off = u32::from_be_bytes([tag[24], tag[25], tag[26], tag[27]]) as usize;
                                if let Some(s) = tag.get(rec_off..rec_off.saturating_add(rec_len).min(tag.len())) {
                                    let units: Vec<u16> = s
                                        .chunks(2)
                                        .filter(|c| c.len() == 2)
                                        .map(|c| u16::from_be_bytes([c[0], c[1]]))
                                        .collect();
                                    desc = String::from_utf16_lossy(&units).trim_matches(char::from(0)).to_string();
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            break;
        }
    }
    let info = format!(
        "class={} space={} cmm={} id={}",
        device_class.trim(),
        color_space,
        if cmm.is_empty() { "-" } else { &cmm },
        if profile_id.chars().all(|c| c == '0') { "(unset)".to_string() } else { profile_id }
    );
    (desc, info)
}

fn analyze_icc(c: &mut Collector, location: String, name: String, profile: &[u8], size: usize, extra: &str) {
    let (desc, info) = icc_description(profile);
    c.push(
        "icc",
        Severity::Medium,
        location,
        name,
        format!(
            "ICC colour profile{} ({} bytes; {}) — identifies the producing software/device colour pipeline",
            if extra.is_empty() { String::new() } else { format!(" {}", extra) },
            profile.len(),
            info
        ),
        size,
        if desc.is_empty() { hex_preview(profile) } else { desc },
        true,
    );
}

// --- IPTC / Photoshop 8BIM ----------------------------------------------------

fn iptc_dataset_name(record: u8, dataset: u8) -> Option<&'static str> {
    if record != 2 {
        return None;
    }
    match dataset {
        5 => Some("ObjectName/Title"),
        25 => Some("Keywords"),
        40 => Some("SpecialInstructions"),
        55 => Some("DateCreated"),
        60 => Some("TimeCreated"),
        80 => Some("By-line (Author)"),
        85 => Some("By-lineTitle"),
        90 => Some("City"),
        92 => Some("Sub-location"),
        95 => Some("Province/State"),
        101 => Some("Country"),
        105 => Some("Headline"),
        110 => Some("Credit"),
        115 => Some("Source"),
        116 => Some("CopyrightNotice"),
        118 => Some("Contact"),
        120 => Some("Caption/Abstract"),
        122 => Some("Writer/Editor"),
        _ => None,
    }
}

fn analyze_photoshop(c: &mut Collector, location: String, name: String, body: &[u8], size: usize) {
    // body starts right after "Photoshop 3.0\0"
    let mut pos = 0usize;
    let mut resources: Vec<String> = Vec::new();
    let mut iptc_items: Vec<(String, String)> = Vec::new();
    while pos + 12 <= body.len() {
        if &body[pos..pos + 4] != b"8BIM" {
            break;
        }
        let id = u16::from_be_bytes([body[pos + 4], body[pos + 5]]);
        let name_len = body[pos + 6] as usize;
        let mut p = pos + 7 + name_len;
        if (name_len + 1) % 2 == 1 {
            p += 1;
        }
        if p + 4 > body.len() {
            break;
        }
        let data_len = u32::from_be_bytes([body[p], body[p + 1], body[p + 2], body[p + 3]]) as usize;
        p += 4;
        let data = match body.get(p..p + data_len) {
            Some(d) => d,
            None => break,
        };
        let label = match id {
            0x0404 => "IPTC-NAA",
            0x040F => "ICC profile",
            0x0409 | 0x040C => "Thumbnail",
            0x0422 => "EXIF data",
            0x0424 => "XMP metadata",
            0x0425 => "Caption digest",
            0x0426 => "Print scale",
            0x03ED => "Resolution info",
            0x03F3 => "Print flags",
            0x0406 => "JPEG quality",
            0x0408 => "Grid/guides",
            0x0410 | 0x0411 => "Watermark/ICC untagged",
            0x0421 => "Version info",
            0x0423 => "EXIF data 3",
            0x043A..=0x043B => "Print info",
            _ => "Resource",
        };
        resources.push(format!("0x{:04X} {} ({} B)", id, label, data_len));
        if id == 0x0404 {
            let mut q = 0usize;
            while q + 5 <= data.len() {
                if data[q] != 0x1C {
                    q += 1;
                    continue;
                }
                let record = data[q + 1];
                let dataset = data[q + 2];
                let len = u16::from_be_bytes([data[q + 3], data[q + 4]]) as usize;
                if len >= 0x8000 {
                    break;
                }
                let val = data.get(q + 5..q + 5 + len).unwrap_or(&[]);
                if let Some(n) = iptc_dataset_name(record, dataset) {
                    iptc_items.push((n.to_string(), preview(val)));
                }
                q += 5 + len;
            }
        }
        pos = p + data_len + (data_len & 1);
    }
    let severity = if iptc_items.iter().any(|(n, _)| {
        matches!(n.as_str(), "By-line (Author)" | "Contact" | "Writer/Editor" | "City" | "Sub-location" | "Province/State" | "Country" | "CopyrightNotice")
    }) {
        Severity::Critical
    } else {
        Severity::High
    };
    let mut detail = format!("Photoshop 8BIM resource block(s): {}", resources.join("; "));
    if !iptc_items.is_empty() {
        detail.push_str(" — IPTC: ");
        detail.push_str(
            &iptc_items
                .iter()
                .map(|(n, v)| format!("{}={}", n, v))
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    let value_preview = iptc_items
        .first()
        .map(|(n, v)| format!("{}: {}", n, v))
        .unwrap_or_else(|| preview(body));
    c.push("iptc", severity, location, name, detail, size, value_preview, true);
}

// ---------------------------------------------------------------------------
// PNG
// ---------------------------------------------------------------------------

fn png_crc(type_bytes: &[u8], data: &[u8]) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(type_bytes);
    h.update(data);
    h.finalize()
}

const PNG_KEEP: &[&[u8; 4]] = &[
    b"IHDR", b"PLTE", b"IDAT", b"IEND", b"tRNS", b"gAMA", b"cHRM", b"sBIT", b"bKGD", b"sRGB", b"acTL",
    b"fcTL", b"fdAT",
];

const PNG_KNOWN_ANCILLARY: &[&[u8; 4]] = &[
    b"tEXt", b"zTXt", b"iTXt", b"eXIf", b"iCCP", b"tIME", b"pHYs", b"hIST", b"sPLT", b"oFFs", b"pCAL",
    b"sCAL", b"sTER", b"cICP", b"mDCv", b"cLLi", b"caBX", b"gIFg", b"gIFx", b"gIFt", b"dSIG", b"eXtr",
];

fn zlib_inflate(data: &[u8], limit: usize) -> Option<Vec<u8>> {
    let mut dec = ZlibDecoder::new(data);
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = dec.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() > limit {
            break;
        }
    }
    Some(out)
}

fn inspect_png(data: &[u8]) -> InspectReport {
    let mut c = Collector::new();
    let (mut width, mut height) = (0u32, 0u32);
    let mut offset = 8usize;
    let mut index = 0usize;
    let mut saw_iend = false;

    while offset + 8 <= data.len() {
        let length = u32::from_be_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]) as usize;
        let ty = &data[offset + 4..offset + 8];
        let ty_str = String::from_utf8_lossy(ty).to_string();
        let data_start = offset + 8;
        let Some(data_end) = data_start.checked_add(length) else { break };
        if data_end + 4 > data.len() {
            c.push(
                "structure",
                Severity::Medium,
                format!("PNG chunk #{} @0x{:04x}", index, offset),
                format!("{} (truncated)", ty_str),
                format!("Chunk declares {} bytes but file ends early", length),
                data.len() - data_start,
                String::new(),
                false,
            );
            break;
        }
        let body = &data[data_start..data_end];
        let stored_crc = u32::from_be_bytes([data[data_end], data[data_end + 1], data[data_end + 2], data[data_end + 3]]);
        let location = format!("PNG chunk #{} @0x{:04x}", index, offset);
        let chunk_size = length + 12;

        if ty != b"IDAT" && ty != b"fdAT" && png_crc(ty, body) != stored_crc {
            c.push(
                "structure",
                Severity::Medium,
                location.clone(),
                format!("{} CRC mismatch", ty_str),
                "Stored CRC32 does not match chunk contents (tampered or injected chunk)".to_string(),
                chunk_size,
                format!("stored=0x{:08x}", stored_crc),
                true,
            );
        }

        match ty {
            b"IHDR" => {
                if body.len() >= 13 {
                    width = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
                    height = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
                    c.push(
                        "structure",
                        Severity::Info,
                        location,
                        "IHDR".to_string(),
                        format!(
                            "{}×{} bit depth {} colour type {} interlace {}",
                            width, height, body[8], body[9], body[12]
                        ),
                        chunk_size,
                        String::new(),
                        false,
                    );
                }
            }
            b"PLTE" | b"IDAT" | b"IEND" | b"tRNS" | b"gAMA" | b"cHRM" | b"sBIT" | b"bKGD" | b"sRGB"
            | b"acTL" | b"fcTL" | b"fdAT" => {}
            b"tEXt" => {
                let split = body.iter().position(|&b| b == 0).unwrap_or(body.len());
                let keyword = String::from_utf8_lossy(&body[..split]).to_string();
                let text = if split < body.len() { &body[split + 1..] } else { &[][..] };
                analyze_text(&mut c, location, format!("tEXt:{}", keyword), &keyword, text, chunk_size);
            }
            b"zTXt" => {
                let split = body.iter().position(|&b| b == 0).unwrap_or(body.len());
                let keyword = String::from_utf8_lossy(&body[..split]).to_string();
                let comp = body.get(split + 2..).unwrap_or(&[]);
                let text = zlib_inflate(comp, 4 * 1024 * 1024).unwrap_or_else(|| comp.to_vec());
                analyze_text(&mut c, location, format!("zTXt:{}", keyword), &keyword, &text, chunk_size);
            }
            b"iTXt" => {
                let split = body.iter().position(|&b| b == 0).unwrap_or(body.len());
                let keyword = String::from_utf8_lossy(&body[..split]).to_string();
                let mut p = split + 1;
                let comp_flag = body.get(p).copied().unwrap_or(0);
                p += 2;
                let lang_end = body.get(p..).and_then(|s| s.iter().position(|&b| b == 0)).map(|i| p + i).unwrap_or(body.len());
                p = (lang_end + 1).min(body.len());
                let tkey_end = body.get(p..).and_then(|s| s.iter().position(|&b| b == 0)).map(|i| p + i).unwrap_or(body.len());
                p = (tkey_end + 1).min(body.len());
                let raw = &body[p..];
                let text = if comp_flag == 1 {
                    zlib_inflate(raw, 4 * 1024 * 1024).unwrap_or_else(|| raw.to_vec())
                } else {
                    raw.to_vec()
                };
                let is_xmp = keyword == "XML:com.adobe.xmp" || contains_bytes(&text, b"x:xmpmeta");
                if is_xmp {
                    analyze_xmp(&mut c, location, "iTXt:XML:com.adobe.xmp".to_string(), &text, chunk_size);
                } else {
                    analyze_text(&mut c, location, format!("iTXt:{}", keyword), &keyword, &text, chunk_size);
                }
            }
            b"eXIf" => {
                let tiff = if body.starts_with(b"Exif\0\0") { &body[6..] } else { body };
                analyze_exif(&mut c, &location, "eXIf chunk", tiff, chunk_size);
            }
            b"iCCP" => {
                let split = body.iter().position(|&b| b == 0).unwrap_or(body.len());
                let name = String::from_utf8_lossy(&body[..split]).to_string();
                let comp = body.get(split + 2..).unwrap_or(&[]);
                let profile = zlib_inflate(comp, 16 * 1024 * 1024).unwrap_or_default();
                analyze_icc(&mut c, location, format!("iCCP:{}", name), &profile, chunk_size, "");
            }
            b"tIME" => {
                let detail = if body.len() >= 7 {
                    format!(
                        "Last-modification time {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
                        u16::from_be_bytes([body[0], body[1]]),
                        body[2],
                        body[3],
                        body[4],
                        body[5],
                        body[6]
                    )
                } else {
                    "Malformed tIME chunk".to_string()
                };
                c.push("time", Severity::Medium, location, "tIME".to_string(), detail.clone(), chunk_size, detail, true);
            }
            b"pHYs" => {
                let detail = if body.len() >= 9 {
                    let x = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
                    let y = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
                    format!("Physical pixel density {}×{} px/{} (software fingerprint)", x, y, if body[8] == 1 { "m" } else { "unit" })
                } else {
                    "Malformed pHYs chunk".to_string()
                };
                c.push("physical", Severity::Low, location, "pHYs".to_string(), detail.clone(), chunk_size, String::new(), true);
            }
            b"caBX" => {
                c.push(
                    "c2pa",
                    Severity::Critical,
                    location,
                    "caBX (JUMBF / C2PA manifest)".to_string(),
                    format!(
                        "Content-credentials manifest box{} — carries signed provenance, generator identity and edit history",
                        if looks_like_jumbf(body) { " with c2pa box labels" } else { "" }
                    ),
                    chunk_size,
                    preview(body),
                    true,
                );
            }
            _ => {
                if looks_like_jumbf(body) {
                    c.push(
                        "c2pa",
                        Severity::Critical,
                        location,
                        format!("{} (JUMBF / C2PA payload)", ty_str),
                        "Chunk body contains JUMBF box structure with C2PA labels".to_string(),
                        chunk_size,
                        preview(body),
                        true,
                    );
                } else {
                    let is_critical_chunk = ty[0].is_ascii_uppercase();
                    let known = PNG_KNOWN_ANCILLARY.iter().any(|k| *k == ty);
                    c.push(
                        "unknown-chunk",
                        if is_critical_chunk { Severity::High } else { Severity::Medium },
                        location,
                        ty_str.clone(),
                        if known {
                            format!("Ancillary chunk '{}' (not required for rendering)", ty_str)
                        } else if is_critical_chunk {
                            format!("Unknown critical chunk '{}' — non-standard container extension", ty_str)
                        } else {
                            format!("Non-standard private/ancillary chunk '{}'", ty_str)
                        },
                        chunk_size,
                        preview(body),
                        true,
                    );
                }
            }
        }

        offset = data_end + 4;
        index += 1;
        if ty == b"IEND" {
            saw_iend = true;
            break;
        }
    }

    if saw_iend {
        if offset < data.len() {
            let trailing = &data[offset..];
            c.push(
                "trailing",
                Severity::High,
                format!("@0x{:04x}", offset),
                "Trailing bytes after IEND".to_string(),
                format!("{} byte(s) appended after the IEND chunk (hidden payload / polyglot data)", trailing.len()),
                trailing.len(),
                preview(trailing),
                true,
            );
        }
    } else {
        c.push(
            "structure",
            Severity::Medium,
            format!("@0x{:04x}", offset),
            "Missing IEND".to_string(),
            "PNG stream ends without an IEND chunk".to_string(),
            0,
            String::new(),
            false,
        );
    }

    finish("png", data, width, height, c.findings)
}

fn clean_png(data: &[u8]) -> Result<CleanResult, String> {
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..8]);
    let mut offset = 8usize;
    let mut removed_items = 0u32;
    let mut saw_iend = false;
    while offset + 8 <= data.len() {
        let length = u32::from_be_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]) as usize;
        let ty = &data[offset + 4..offset + 8];
        let data_start = offset + 8;
        let data_end = data_start.checked_add(length).ok_or("PNG chunk length overflow")?;
        if data_end + 4 > data.len() {
            return Err("Truncated PNG chunk".to_string());
        }
        let body = &data[data_start..data_end];
        if PNG_KEEP.iter().any(|k| *k == ty) {
            let crc = png_crc(ty, body);
            out.extend_from_slice(&(length as u32).to_be_bytes());
            out.extend_from_slice(ty);
            out.extend_from_slice(body);
            out.extend_from_slice(&crc.to_be_bytes());
        } else {
            removed_items += 1;
        }
        offset = data_end + 4;
        if ty == b"IEND" {
            saw_iend = true;
            break;
        }
    }
    if !saw_iend {
        return Err("PNG stream has no IEND chunk".to_string());
    }
    if offset < data.len() {
        removed_items += 1;
    }
    let removed_bytes = data.len().saturating_sub(out.len());
    Ok(CleanResult {
        data: out,
        format: ImageFormat::Png,
        removed_bytes,
        removed_items,
    })
}

// ---------------------------------------------------------------------------
// JPEG
// ---------------------------------------------------------------------------

struct JpegSegment {
    marker: u8,
    offset: usize,
    /// Full segment length including the 2 marker bytes and the length field.
    total_len: usize,
    payload_start: usize,
    payload_end: usize,
}

/// Walks the JPEG marker structure. Returns the segments in order plus the offset
/// right after EOI (or `None` if EOI is missing). Entropy-coded scan data following
/// SOS is attached to the SOS segment (`payload_end` covers it).
fn jpeg_segments(data: &[u8]) -> (Vec<JpegSegment>, Option<usize>) {
    let mut segs = Vec::new();
    let mut pos = 2usize;
    let mut eoi: Option<usize> = None;
    while pos + 2 <= data.len() {
        if data[pos] != 0xFF {
            // Garbage between segments: skip byte-wise.
            pos += 1;
            continue;
        }
        let marker = data[pos + 1];
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        if marker == 0xD9 {
            segs.push(JpegSegment { marker, offset: pos, total_len: 2, payload_start: pos + 2, payload_end: pos + 2 });
            eoi = Some(pos + 2);
            break;
        }
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            segs.push(JpegSegment { marker, offset: pos, total_len: 2, payload_start: pos + 2, payload_end: pos + 2 });
            pos += 2;
            continue;
        }
        if pos + 4 > data.len() {
            break;
        }
        let len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        if len < 2 {
            break;
        }
        let payload_start = pos + 4;
        let mut payload_end = (pos + 2 + len).min(data.len());
        let mut total_len = payload_end - pos;
        if marker == 0xDA {
            // Scan: entropy-coded data runs until the next non-RST, non-stuffed marker.
            let mut p = payload_end;
            while p + 1 < data.len() {
                if data[p] == 0xFF {
                    let m = data[p + 1];
                    if m != 0x00 && !(0xD0..=0xD7).contains(&m) && m != 0xFF {
                        break;
                    }
                }
                p += 1;
            }
            if p + 1 >= data.len() {
                p = data.len();
            }
            payload_end = p;
            total_len = p - pos;
        }
        segs.push(JpegSegment { marker, offset: pos, total_len, payload_start, payload_end });
        pos += total_len;
    }
    (segs, eoi)
}

fn marker_name(m: u8) -> String {
    match m {
        0xC0 => "SOF0 (baseline)".into(),
        0xC1 => "SOF1 (extended)".into(),
        0xC2 => "SOF2 (progressive)".into(),
        0xC3 => "SOF3 (lossless)".into(),
        0xC4 => "DHT".into(),
        0xC8 => "JPG".into(),
        0xCC => "DAC".into(),
        0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => format!("SOF{}", m - 0xC0),
        0xD8 => "SOI".into(),
        0xD9 => "EOI".into(),
        0xDA => "SOS".into(),
        0xDB => "DQT".into(),
        0xDC => "DNL".into(),
        0xDD => "DRI".into(),
        0xDE => "DHP".into(),
        0xDF => "EXP".into(),
        0xE0..=0xEF => format!("APP{}", m - 0xE0),
        0xF0..=0xFD => format!("JPG{}", m - 0xF0),
        0xFE => "COM".into(),
        0x01 => "TEM".into(),
        0xD0..=0xD7 => format!("RST{}", m - 0xD0),
        _ => format!("0x{:02X}", m),
    }
}

fn inspect_jpeg(data: &[u8]) -> InspectReport {
    let mut c = Collector::new();
    let (segs, eoi) = jpeg_segments(data);
    let (mut width, mut height, mut components) = (0u32, 0u32, 0u8);
    let mut icc_parts: Vec<(u8, u8, &[u8])> = Vec::new();
    let mut icc_location = String::new();
    let mut icc_size = 0usize;

    for (i, s) in segs.iter().enumerate() {
        let payload = &data[s.payload_start..s.payload_end];
        let location = format!("JPEG segment #{} {} @0x{:04x}", i, marker_name(s.marker), s.offset);
        match s.marker {
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                if payload.len() >= 6 {
                    height = u16::from_be_bytes([payload[1], payload[2]]) as u32;
                    width = u16::from_be_bytes([payload[3], payload[4]]) as u32;
                    components = payload[5];
                    c.push(
                        "structure",
                        Severity::Info,
                        location,
                        marker_name(s.marker),
                        format!("{}×{} {} component(s), {}-bit", width, height, components, payload[0]),
                        s.total_len,
                        String::new(),
                        false,
                    );
                }
            }
            0xDB => {
                let digest = Sha256::digest(payload);
                let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
                let tables = payload.first().map(|b| (b & 0x0F) as usize).unwrap_or(0);
                c.push(
                    "quantization",
                    Severity::Low,
                    location,
                    "DQT quantization table".to_string(),
                    format!(
                        "Quantization table (first id {}, {} bytes) — SHA-256 fingerprint identifies the encoder/quality preset",
                        tables,
                        payload.len()
                    ),
                    s.total_len,
                    hex,
                    false,
                );
            }
            0xE0 => {
                if payload.starts_with(b"JFIF\0") {
                    let detail = if payload.len() >= 14 {
                        format!(
                            "JFIF {}.{:02} density {}×{} unit {} thumbnail {}×{}",
                            payload[5], payload[6], u16::from_be_bytes([payload[8], payload[9]]),
                            u16::from_be_bytes([payload[10], payload[11]]), payload[7], payload[12], payload[13]
                        )
                    } else {
                        "JFIF header".to_string()
                    };
                    let has_thumb = payload.len() >= 14 && payload[12] > 0 && payload[13] > 0;
                    if has_thumb {
                        c.push("thumbnail", Severity::High, location, "APP0 JFIF embedded thumbnail".to_string(), detail, s.total_len, String::new(), true);
                    } else {
                        c.push("structure", Severity::Info, location, "APP0 JFIF".to_string(), detail, s.total_len, String::new(), false);
                    }
                } else if payload.starts_with(b"JFXX\0") {
                    c.push(
                        "thumbnail",
                        Severity::High,
                        location,
                        "APP0 JFXX thumbnail".to_string(),
                        format!("JFIF extension thumbnail (format code 0x{:02X}, {} bytes)", payload.get(5).copied().unwrap_or(0), payload.len()),
                        s.total_len,
                        hex_preview(payload),
                        true,
                    );
                } else {
                    c.push("unknown-chunk", Severity::Medium, location, "APP0 (non-JFIF)".to_string(), "Unknown APP0 payload".to_string(), s.total_len, preview(payload), true);
                }
            }
            0xE1 => {
                if payload.starts_with(b"Exif\0\0") {
                    analyze_exif(&mut c, &location, "APP1 EXIF", &payload[6..], s.total_len);
                } else if payload.starts_with(b"http://ns.adobe.com/xap/1.0/\0") {
                    analyze_xmp(&mut c, location, "APP1 XMP".to_string(), &payload[29..], s.total_len);
                } else if payload.starts_with(b"http://ns.adobe.com/xmp/extension/\0") {
                    let body = payload.get(35 + 32 + 8..).unwrap_or(&[]);
                    analyze_xmp(&mut c, location, "APP1 Extended XMP".to_string(), body, s.total_len);
                } else if contains_bytes(payload, b"x:xmpmeta") {
                    analyze_xmp(&mut c, location, "APP1 XMP (unlabelled)".to_string(), payload, s.total_len);
                } else if looks_like_jumbf(payload) {
                    c.push("c2pa", Severity::Critical, location, "APP1 JUMBF/C2PA".to_string(), "JUMBF box structure with C2PA labels inside APP1".to_string(), s.total_len, preview(payload), true);
                } else {
                    c.push("unknown-chunk", Severity::Medium, location, "APP1 (unknown)".to_string(), "Unrecognised APP1 payload".to_string(), s.total_len, preview(payload), true);
                }
            }
            0xE2 => {
                if payload.starts_with(b"ICC_PROFILE\0") && payload.len() >= 14 {
                    icc_parts.push((payload[12], payload[13], &payload[14..]));
                    if icc_location.is_empty() {
                        icc_location = location.clone();
                    }
                    icc_size += s.total_len;
                } else if payload.starts_with(b"MPF\0") {
                    c.push(
                        "thumbnail",
                        Severity::High,
                        location,
                        "APP2 MPF multi-picture".to_string(),
                        "Multi-Picture Format index — references additional embedded images (previews, depth maps, gain maps)".to_string(),
                        s.total_len,
                        hex_preview(payload),
                        true,
                    );
                } else if looks_like_jumbf(payload) {
                    c.push("c2pa", Severity::Critical, location, "APP2 JUMBF/C2PA".to_string(), "JUMBF box structure with C2PA labels".to_string(), s.total_len, preview(payload), true);
                } else {
                    c.push("unknown-chunk", Severity::Medium, location, "APP2 (unknown)".to_string(), "Unrecognised APP2 payload".to_string(), s.total_len, preview(payload), true);
                }
            }
            0xEB => {
                let is_jumbf = payload.starts_with(b"JP") || looks_like_jumbf(payload) || mentions_c2pa(payload);
                if is_jumbf {
                    let seq = if payload.len() >= 6 { Some((u16::from_be_bytes([payload[2], payload[3]]), u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]))) } else { None };
                    c.push(
                        "c2pa",
                        Severity::Critical,
                        location,
                        "APP11 JUMBF / C2PA manifest".to_string(),
                        format!(
                            "Content-credentials manifest segment{} — signed provenance, generator and edit history{}",
                            seq.map(|(ci, sq)| format!(" (CI={}, box seq {})", ci, sq)).unwrap_or_default(),
                            if looks_like_jumbf(payload) { " (c2pa box labels present)" } else { "" }
                        ),
                        s.total_len,
                        preview(payload),
                        true,
                    );
                } else {
                    c.push("unknown-chunk", Severity::Medium, location, "APP11 (unknown)".to_string(), "Unrecognised APP11 payload".to_string(), s.total_len, preview(payload), true);
                }
            }
            0xED => {
                if payload.starts_with(b"Photoshop 3.0\0") {
                    analyze_photoshop(&mut c, location, "APP13 Photoshop/IPTC".to_string(), &payload[14..], s.total_len);
                } else if payload.starts_with(b"Adobe_Photoshop2.5:") {
                    c.push("iptc", Severity::High, location, "APP13 Photoshop 2.5".to_string(), "Legacy Photoshop resource block".to_string(), s.total_len, preview(payload), true);
                } else {
                    c.push("unknown-chunk", Severity::Medium, location, "APP13 (unknown)".to_string(), "Unrecognised APP13 payload".to_string(), s.total_len, preview(payload), true);
                }
            }
            0xEE => {
                if payload.starts_with(b"Adobe") {
                    let transform = payload.get(11).copied();
                    c.push(
                        "software",
                        if components == 4 { Severity::Info } else { Severity::Low },
                        location,
                        "APP14 Adobe DCT marker".to_string(),
                        format!(
                            "Adobe encoder marker (DCTEncodeVersion {}, transform {}) — identifies Adobe-family encoders{}",
                            payload.get(5..7).map(|v| u16::from_be_bytes([v[0], v[1]])).unwrap_or(0),
                            transform.map(|t| t.to_string()).unwrap_or_else(|| "?".into()),
                            if components == 4 { "; required for CMYK/YCCK colour interpretation (kept)" } else { "" }
                        ),
                        s.total_len,
                        hex_preview(payload),
                        components != 4,
                    );
                } else {
                    c.push("unknown-chunk", Severity::Medium, location, "APP14 (unknown)".to_string(), "Unrecognised APP14 payload".to_string(), s.total_len, preview(payload), true);
                }
            }
            0xE3..=0xEA | 0xEC | 0xEF => {
                if looks_like_jumbf(payload) || mentions_c2pa(payload) {
                    c.push("c2pa", Severity::Critical, location, format!("{} JUMBF/C2PA", marker_name(s.marker)), "JUMBF box structure / C2PA labels in application segment".to_string(), s.total_len, preview(payload), true);
                } else if contains_bytes(payload, b"x:xmpmeta") {
                    analyze_xmp(&mut c, location, format!("{} XMP", marker_name(s.marker)), payload, s.total_len);
                } else {
                    let lower = String::from_utf8_lossy(payload).to_lowercase();
                    let markers = matched_markers(&lower, GENERATOR_MARKERS);
                    c.push(
                        "unknown-chunk",
                        if markers.is_empty() { Severity::Medium } else { Severity::Critical },
                        location,
                        format!("{} (application data)", marker_name(s.marker)),
                        if markers.is_empty() {
                            "Vendor application segment (camera/software private data)".to_string()
                        } else {
                            format!("Application segment contains generator markers: {}", markers.join(", "))
                        },
                        s.total_len,
                        preview(payload),
                        true,
                    );
                }
            }
            0xFE => {
                let lower = String::from_utf8_lossy(payload).to_lowercase();
                let markers = matched_markers(&lower, GENERATOR_MARKERS);
                c.push(
                    if markers.is_empty() { "comment" } else { "generator" },
                    if markers.is_empty() { Severity::Medium } else { Severity::Critical },
                    location,
                    "COM comment".to_string(),
                    if markers.is_empty() {
                        format!("Free-text comment segment ({} bytes)", payload.len())
                    } else {
                        format!("Comment contains generation parameters (markers: {})", markers.join(", "))
                    },
                    s.total_len,
                    preview(payload),
                    true,
                );
            }
            _ => {}
        }
    }

    if !icc_parts.is_empty() {
        icc_parts.sort_by_key(|p| p.0);
        let total = icc_parts[0].1;
        let mut profile = Vec::new();
        for p in &icc_parts {
            profile.extend_from_slice(p.2);
        }
        analyze_icc(
            &mut c,
            icc_location,
            "APP2 ICC_PROFILE".to_string(),
            &profile,
            icc_size,
            &format!("in {} segment(s) of {}", icc_parts.len(), total),
        );
    }

    match eoi {
        Some(end) if end < data.len() => {
            let trailing = &data[end..];
            c.push(
                "trailing",
                Severity::High,
                format!("@0x{:04x}", end),
                "Trailing bytes after EOI".to_string(),
                format!("{} byte(s) appended after the EOI marker (hidden payload / polyglot data)", trailing.len()),
                trailing.len(),
                preview(trailing),
                true,
            );
        }
        Some(_) => {}
        None => {
            c.push(
                "structure",
                Severity::Medium,
                format!("@0x{:04x}", data.len()),
                "Missing EOI".to_string(),
                "JPEG stream ends without an EOI marker".to_string(),
                0,
                String::new(),
                false,
            );
        }
    }

    finish("jpeg", data, width, height, c.findings)
}

fn clean_jpeg(data: &[u8]) -> Result<CleanResult, String> {
    let (segs, eoi) = jpeg_segments(data);
    if segs.is_empty() || eoi.is_none() {
        return Err("JPEG stream is truncated (no EOI marker)".to_string());
    }
    let mut components = 0u8;
    for s in &segs {
        if matches!(s.marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            if let Some(&n) = data.get(s.payload_start + 5) {
                components = n;
            }
        }
    }
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..2]);
    let mut removed_items = 0u32;
    for s in &segs {
        let payload = &data[s.payload_start..s.payload_end];
        let keep = match s.marker {
            0xD8 => false, // SOI already written
            0xE0 => payload.starts_with(b"JFIF\0") && !(payload.len() >= 14 && payload[12] > 0 && payload[13] > 0),
            0xEE => components == 4 && payload.starts_with(b"Adobe"),
            0xE1..=0xED | 0xEF | 0xFE => false,
            _ => true,
        };
        if keep {
            out.extend_from_slice(&data[s.offset..s.offset + s.total_len]);
        } else if s.marker != 0xD8 {
            removed_items += 1;
        }
    }
    if let Some(end) = eoi {
        if end < data.len() {
            removed_items += 1;
        }
    }
    let removed_bytes = data.len().saturating_sub(out.len());
    Ok(CleanResult {
        data: out,
        format: ImageFormat::Jpeg,
        removed_bytes,
        removed_items,
    })
}

// ---------------------------------------------------------------------------
// WebP (RIFF)
// ---------------------------------------------------------------------------

struct RiffChunk<'a> {
    fourcc: [u8; 4],
    offset: usize,
    payload: &'a [u8],
}

fn riff_chunks(data: &[u8]) -> (Vec<RiffChunk<'_>>, usize, Option<String>) {
    let riff_size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    let declared_end = 8usize.saturating_add(riff_size);
    let end = declared_end.min(data.len());
    let mut chunks = Vec::new();
    let mut pos = 12usize;
    let mut problem = None;
    while pos + 8 <= end {
        let fourcc = [data[pos], data[pos + 1], data[pos + 2], data[pos + 3]];
        let size = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
        let payload_end = match (pos + 8).checked_add(size) {
            Some(e) if e <= data.len() => e,
            _ => {
                problem = Some(format!(
                    "Chunk '{}' @0x{:04x} declares {} bytes beyond end of file",
                    String::from_utf8_lossy(&fourcc),
                    pos,
                    size
                ));
                break;
            }
        };
        chunks.push(RiffChunk { fourcc, offset: pos, payload: &data[pos + 8..payload_end] });
        pos = payload_end + (size & 1);
    }
    (chunks, declared_end, problem)
}

fn inspect_webp(data: &[u8]) -> InspectReport {
    let mut c = Collector::new();
    let (chunks, declared_end, problem) = riff_chunks(data);
    let (mut width, mut height) = (0u32, 0u32);
    if let Ok((_, w, h, _)) = crate::services::image::read_dimensions(data) {
        width = w;
        height = h;
    }
    if let Some(p) = problem {
        c.push("structure", Severity::Medium, "RIFF".to_string(), "Truncated chunk".to_string(), p, 0, String::new(), false);
    }
    for (i, ch) in chunks.iter().enumerate() {
        let name = String::from_utf8_lossy(&ch.fourcc).to_string();
        let location = format!("RIFF chunk #{} '{}' @0x{:04x}", i, name, ch.offset);
        let size = ch.payload.len() + 8;
        match &ch.fourcc {
            b"VP8 " | b"VP8L" | b"ALPH" | b"ANIM" | b"ANMF" => {}
            b"VP8X" => {
                let flags = ch.payload.first().copied().unwrap_or(0);
                let mut set = Vec::new();
                if flags & 0x20 != 0 { set.push("ICC"); }
                if flags & 0x10 != 0 { set.push("Alpha"); }
                if flags & 0x08 != 0 { set.push("EXIF"); }
                if flags & 0x04 != 0 { set.push("XMP"); }
                if flags & 0x02 != 0 { set.push("Animation"); }
                c.push(
                    "structure",
                    Severity::Info,
                    location,
                    "VP8X extended header".to_string(),
                    format!("Feature flags 0x{:02x}: {}", flags, if set.is_empty() { "none".to_string() } else { set.join(", ") }),
                    size,
                    String::new(),
                    false,
                );
            }
            b"ICCP" => analyze_icc(&mut c, location, "ICCP chunk".to_string(), ch.payload, size, ""),
            b"EXIF" => {
                let tiff = if ch.payload.starts_with(b"Exif\0\0") { &ch.payload[6..] } else { ch.payload };
                analyze_exif(&mut c, &location, "EXIF chunk", tiff, size);
            }
            b"XMP " => analyze_xmp(&mut c, location, "XMP chunk".to_string(), ch.payload, size),
            b"C2PA" => {
                c.push(
                    "c2pa",
                    Severity::Critical,
                    location,
                    "C2PA chunk (JUMBF manifest)".to_string(),
                    "Content-credentials manifest — signed provenance, generator identity and edit history".to_string(),
                    size,
                    preview(ch.payload),
                    true,
                );
            }
            _ => {
                if looks_like_jumbf(ch.payload) || mentions_c2pa(ch.payload) {
                    c.push("c2pa", Severity::Critical, location, format!("'{}' JUMBF/C2PA payload", name), "Chunk body contains JUMBF box structure / C2PA labels".to_string(), size, preview(ch.payload), true);
                } else if contains_bytes(ch.payload, b"x:xmpmeta") {
                    analyze_xmp(&mut c, location, format!("'{}' XMP payload", name), ch.payload, size);
                } else {
                    c.push(
                        "unknown-chunk",
                        Severity::Medium,
                        location,
                        format!("'{}' chunk", name),
                        format!("Unknown RIFF chunk ({} bytes) — not part of the WebP bitstream", ch.payload.len()),
                        size,
                        preview(ch.payload),
                        true,
                    );
                }
            }
        }
    }
    if declared_end < data.len() {
        let trailing = &data[declared_end..];
        c.push(
            "trailing",
            Severity::High,
            format!("@0x{:04x}", declared_end),
            "Trailing bytes after RIFF payload".to_string(),
            format!("{} byte(s) appended beyond the declared RIFF size (hidden payload / polyglot data)", trailing.len()),
            trailing.len(),
            preview(trailing),
            true,
        );
    } else if declared_end > data.len() {
        c.push(
            "structure",
            Severity::Medium,
            "RIFF header".to_string(),
            "RIFF size mismatch".to_string(),
            format!("Header declares {} bytes but file holds {}", declared_end, data.len()),
            0,
            String::new(),
            false,
        );
    }
    finish("webp", data, width, height, c.findings)
}

fn clean_webp(data: &[u8]) -> Result<CleanResult, String> {
    let (chunks, declared_end, problem) = riff_chunks(data);
    if let Some(p) = problem {
        return Err(p);
    }
    let mut body: Vec<u8> = Vec::with_capacity(data.len());
    let mut removed_items = 0u32;
    for ch in &chunks {
        match &ch.fourcc {
            b"VP8 " | b"VP8L" | b"ALPH" | b"ANIM" | b"ANMF" => {
                body.extend_from_slice(&ch.fourcc);
                body.extend_from_slice(&(ch.payload.len() as u32).to_le_bytes());
                body.extend_from_slice(ch.payload);
                if ch.payload.len() & 1 == 1 {
                    body.push(0);
                }
            }
            b"VP8X" => {
                let mut payload = ch.payload.to_vec();
                if let Some(flags) = payload.first_mut() {
                    *flags &= !(0x20 | 0x08 | 0x04); // clear ICC, EXIF, XMP
                }
                body.extend_from_slice(&ch.fourcc);
                body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                body.extend_from_slice(&payload);
                if payload.len() & 1 == 1 {
                    body.push(0);
                }
            }
            _ => removed_items += 1,
        }
    }
    if declared_end < data.len() {
        removed_items += 1;
    }
    if !chunks.iter().any(|c| matches!(&c.fourcc, b"VP8 " | b"VP8L" | b"ANMF")) {
        return Err("WebP container has no image bitstream chunk".to_string());
    }
    let mut out = Vec::with_capacity(body.len() + 12);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&body);
    let removed_bytes = data.len().saturating_sub(out.len());
    Ok(CleanResult {
        data: out,
        format: ImageFormat::WebP,
        removed_bytes,
        removed_items,
    })
}

// ---------------------------------------------------------------------------
// Brand stamp (clean attribution written after cleaning)
// ---------------------------------------------------------------------------

/// Minimal little-endian TIFF with an IFD0 holding only Artist (0x013B) and/or
/// Copyright (0x8298) ASCII tags.
pub fn brand_stamp_tiff(stamp: &BrandStamp) -> Vec<u8> {
    let mut entries: Vec<(u16, Vec<u8>)> = Vec::new();
    if !stamp.artist.trim().is_empty() {
        let mut v = stamp.artist.trim().as_bytes().to_vec();
        v.push(0);
        entries.push((0x013B, v));
    }
    if !stamp.copyright.trim().is_empty() {
        let mut v = stamp.copyright.trim().as_bytes().to_vec();
        v.push(0);
        entries.push((0x8298, v));
    }
    let ifd_off = 8usize;
    let blob_base = ifd_off + 2 + entries.len() * 12 + 4;
    let mut out = Vec::with_capacity(blob_base + 64);
    out.extend_from_slice(b"II");
    out.extend_from_slice(&42u16.to_le_bytes());
    out.extend_from_slice(&(ifd_off as u32).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    let mut blob: Vec<u8> = Vec::new();
    for (tag, value) in &entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        if value.len() <= 4 {
            let mut v = value.clone();
            v.resize(4, 0);
            out.extend_from_slice(&v);
        } else {
            out.extend_from_slice(&((blob_base + blob.len()) as u32).to_le_bytes());
            blob.extend_from_slice(value);
            if blob.len() % 2 == 1 {
                blob.push(0);
            }
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&blob);
    out
}

fn stamp_png(data: &[u8], stamp: &BrandStamp) -> Result<Vec<u8>, String> {
    if data.len() < 33 || &data[12..16] != b"IHDR" {
        return Err("PNG without leading IHDR chunk".to_string());
    }
    let ihdr_len = u32::from_be_bytes([data[8], data[9], data[10], data[11]]) as usize;
    let ihdr_end = 8 + 12 + ihdr_len;
    if ihdr_end > data.len() {
        return Err("Truncated PNG IHDR".to_string());
    }
    let mut out = Vec::with_capacity(data.len() + 128);
    out.extend_from_slice(&data[..ihdr_end]);
    for (keyword, value) in [("Author", stamp.artist.trim()), ("Copyright", stamp.copyright.trim())] {
        if value.is_empty() {
            continue;
        }
        let mut body = keyword.as_bytes().to_vec();
        body.push(0);
        // tEXt is Latin-1; non-Latin-1 characters are transliterated by lossy conversion.
        body.extend(value.chars().map(|ch| if (ch as u32) < 256 { ch as u32 as u8 } else { b'?' }));
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(b"tEXt");
        out.extend_from_slice(&body);
        out.extend_from_slice(&png_crc(b"tEXt", &body).to_be_bytes());
    }
    out.extend_from_slice(&data[ihdr_end..]);
    Ok(out)
}

fn stamp_jpeg(data: &[u8], stamp: &BrandStamp) -> Result<Vec<u8>, String> {
    let (segs, _) = jpeg_segments(data);
    // Insert after an APP0 JFIF segment when present, otherwise right after SOI.
    let insert_at = segs
        .iter()
        .find(|s| s.marker == 0xE0)
        .map(|s| s.offset + s.total_len)
        .unwrap_or(2);
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend(brand_stamp_tiff(stamp));
    if payload.len() + 2 > 0xFFFF {
        return Err("Brand stamp too large for an APP1 segment".to_string());
    }
    let mut out = Vec::with_capacity(data.len() + payload.len() + 4);
    out.extend_from_slice(&data[..insert_at]);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&payload);
    out.extend_from_slice(&data[insert_at..]);
    Ok(out)
}

fn stamp_webp(data: &[u8], stamp: &BrandStamp) -> Result<Vec<u8>, String> {
    let (chunks, _, problem) = riff_chunks(data);
    if let Some(p) = problem {
        return Err(p);
    }
    let (_, width, height, has_alpha) = read_dimensions(data)?;
    let has_anim = chunks.iter().any(|c| &c.fourcc == b"ANIM");
    let mut body: Vec<u8> = Vec::with_capacity(data.len() + 128);
    let has_vp8x = chunks.iter().any(|c| &c.fourcc == b"VP8X");
    if !has_vp8x {
        let mut flags = 0x08u8;
        if has_alpha {
            flags |= 0x10;
        }
        if has_anim {
            flags |= 0x02;
        }
        let w1 = width.saturating_sub(1);
        let h1 = height.saturating_sub(1);
        let vp8x = [
            flags,
            0,
            0,
            0,
            (w1 & 0xFF) as u8,
            ((w1 >> 8) & 0xFF) as u8,
            ((w1 >> 16) & 0xFF) as u8,
            (h1 & 0xFF) as u8,
            ((h1 >> 8) & 0xFF) as u8,
            ((h1 >> 16) & 0xFF) as u8,
        ];
        body.extend_from_slice(b"VP8X");
        body.extend_from_slice(&10u32.to_le_bytes());
        body.extend_from_slice(&vp8x);
    }
    for ch in &chunks {
        let mut payload = ch.payload.to_vec();
        if &ch.fourcc == b"VP8X" {
            if let Some(flags) = payload.first_mut() {
                *flags |= 0x08;
            }
        }
        if &ch.fourcc == b"EXIF" {
            continue; // replaced by the stamp below
        }
        body.extend_from_slice(&ch.fourcc);
        body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        body.extend_from_slice(&payload);
        if payload.len() & 1 == 1 {
            body.push(0);
        }
    }
    let exif = brand_stamp_tiff(stamp);
    body.extend_from_slice(b"EXIF");
    body.extend_from_slice(&(exif.len() as u32).to_le_bytes());
    body.extend_from_slice(&exif);
    if exif.len() & 1 == 1 {
        body.push(0);
    }
    let mut out = Vec::with_capacity(body.len() + 12);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&body);
    Ok(out)
}

/// Writes a clean Artist/Copyright attribution into an already-sanitised image:
/// PNG `tEXt` Author/Copyright, JPEG minimal EXIF APP1, WebP minimal EXIF chunk (+ VP8X flag).
pub fn embed_brand_stamp(data: &[u8], stamp: &BrandStamp) -> Result<Vec<u8>, String> {
    if stamp.is_empty() {
        return Ok(data.to_vec());
    }
    match detect_format(data)? {
        ImageFormat::Png => stamp_png(data, stamp),
        ImageFormat::Jpeg => stamp_jpeg(data, stamp),
        ImageFormat::WebP => stamp_webp(data, stamp),
    }
}

/// Lossless container clean-up: strips every metadata carrier while keeping the
/// compressed pixel payload byte-for-byte.
pub fn clean_image_bytes(data: &[u8]) -> Result<CleanResult, String> {
    match detect_format(data)? {
        ImageFormat::Png => clean_png(data),
        ImageFormat::Jpeg => clean_jpeg(data),
        ImageFormat::WebP => clean_webp(data),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_support {
    //! Builders for synthetic images carrying every metadata carrier the inspector
    //! must detect. Shared with the DSP pipeline tests.
    use super::*;

    pub fn sample_rgba(w: u32, h: u32) -> image::RgbaImage {
        image::RgbaImage::from_fn(w, h, |x, y| {
            let fx = x as f32 / w as f32;
            let fy = y as f32 / h as f32;
            let r = (120.0 + 100.0 * (fx * 6.0).sin() * (fy * 4.0).cos()) as u8;
            let g = (90.0 + 80.0 * fy + 30.0 * ((x * 7 + y * 3) % 13) as f32 / 13.0) as u8;
            let b = (200.0 - 150.0 * fx) as u8;
            image::Rgba([r, g, b, 255])
        })
    }

    pub fn png_chunk(ty: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(body.len() as u32).to_be_bytes());
        v.extend_from_slice(ty);
        v.extend_from_slice(body);
        v.extend_from_slice(&png_crc(ty, body).to_be_bytes());
        v
    }

    pub fn jumbf_payload() -> Vec<u8> {
        // Minimal JUMBF superbox: 'jumb' box containing a 'jumd' description box labelled c2pa.
        let mut desc = Vec::new();
        desc.extend_from_slice(&[0u8; 16]); // type UUID
        desc.push(0x03); // toggles
        desc.extend_from_slice(b"c2pa\0");
        let mut jumd = Vec::new();
        jumd.extend_from_slice(&((8 + desc.len()) as u32).to_be_bytes());
        jumd.extend_from_slice(b"jumd");
        jumd.extend_from_slice(&desc);
        let mut jumb = Vec::new();
        jumb.extend_from_slice(&((8 + jumd.len()) as u32).to_be_bytes());
        jumb.extend_from_slice(b"jumb");
        jumb.extend_from_slice(&jumd);
        jumb.extend_from_slice(b"c2pa.assertions c2ma c2cl");
        jumb
    }

    pub fn xmp_packet() -> Vec<u8> {
        br#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmp:CreatorTool="Adobe Firefly 3" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:Iptc4xmpExt="http://iptc.org/std/Iptc4xmpExt/2008-02-29/"><Iptc4xmpExt:DigitalSourceType>http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia</Iptc4xmpExt:DigitalSourceType><xmpMM:History xmlns:xmpMM="http://ns.adobe.com/xap/1.0/mm/"><rdf:Seq><rdf:li>saved</rdf:li></rdf:Seq></xmpMM:History></rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end="w"?>"#.to_vec()
    }

    /// Little-endian TIFF with IFD0 (Make, Model, Software, ExifIFD ptr, GPS ptr),
    /// ExifIFD (BodySerialNumber, UserComment) and GPS IFD (lat/lon).
    pub fn exif_tiff() -> Vec<u8> {
        struct Entry {
            tag: u16,
            ty: u16,
            data: Vec<u8>,
        }
        fn ifd_size(entries: &[Entry]) -> usize {
            2 + entries.len() * 12 + 4
        }
        fn write_ifd(out: &mut Vec<u8>, entries: &[Entry], ifd_off: usize, next_ifd: u32, blob: &mut Vec<u8>, blob_base: usize) {
            out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
            for e in entries {
                let unit = match e.ty { 2 | 7 | 1 => 1, 3 => 2, 4 => 4, 5 => 8, _ => 1 };
                let count = (e.data.len() / unit) as u32;
                out.extend_from_slice(&e.tag.to_le_bytes());
                out.extend_from_slice(&e.ty.to_le_bytes());
                out.extend_from_slice(&count.to_le_bytes());
                if e.data.len() <= 4 {
                    let mut v = e.data.clone();
                    v.resize(4, 0);
                    out.extend_from_slice(&v);
                } else {
                    let off = (blob_base + blob.len()) as u32;
                    out.extend_from_slice(&off.to_le_bytes());
                    blob.extend_from_slice(&e.data);
                    if blob.len() % 2 == 1 {
                        blob.push(0);
                    }
                }
            }
            let _ = ifd_off;
            out.extend_from_slice(&next_ifd.to_le_bytes());
        }
        let rat = |n: u32, d: u32| -> Vec<u8> {
            let mut v = n.to_le_bytes().to_vec();
            v.extend_from_slice(&d.to_le_bytes());
            v
        };
        let ifd0_off = 8usize;
        let ifd0_entries_count = 5usize;
        let exif_off = ifd0_off + 2 + ifd0_entries_count * 12 + 4;
        let exif_entries_count = 2usize;
        let gps_off = exif_off + 2 + exif_entries_count * 12 + 4;
        let gps_entries_count = 4usize;
        let blob_base = gps_off + 2 + gps_entries_count * 12 + 4;

        let ifd0 = vec![
            Entry { tag: 0x010F, ty: 2, data: b"TestCam Corp\0".to_vec() },
            Entry { tag: 0x0110, ty: 2, data: b"TC-9000 Pro\0".to_vec() },
            Entry { tag: 0x0131, ty: 2, data: b"Adobe Photoshop 25.0\0".to_vec() },
            Entry { tag: 0x8769, ty: 4, data: (exif_off as u32).to_le_bytes().to_vec() },
            Entry { tag: 0x8825, ty: 4, data: (gps_off as u32).to_le_bytes().to_vec() },
        ];
        let exif = vec![
            Entry { tag: 0xA431, ty: 2, data: b"SN-123456789\0".to_vec() },
            Entry { tag: 0x9286, ty: 7, data: b"ASCII\0\0\0Shot by Deniz".to_vec() },
        ];
        let mut lat = rat(41, 1);
        lat.extend(rat(1, 1));
        lat.extend(rat(2400, 100));
        let mut lon = rat(28, 1);
        lon.extend(rat(58, 1));
        lon.extend(rat(1200, 100));
        let gps = vec![
            Entry { tag: 0x0001, ty: 2, data: b"N\0".to_vec() },
            Entry { tag: 0x0002, ty: 5, data: lat },
            Entry { tag: 0x0003, ty: 2, data: b"E\0".to_vec() },
            Entry { tag: 0x0004, ty: 5, data: lon },
        ];
        assert_eq!(ifd_size(&ifd0), exif_off - ifd0_off);
        let mut out = Vec::new();
        out.extend_from_slice(b"II");
        out.extend_from_slice(&42u16.to_le_bytes());
        out.extend_from_slice(&(ifd0_off as u32).to_le_bytes());
        let mut blob = Vec::new();
        write_ifd(&mut out, &ifd0, ifd0_off, 0, &mut blob, blob_base);
        write_ifd(&mut out, &exif, exif_off, 0, &mut blob, blob_base);
        write_ifd(&mut out, &gps, gps_off, 0, &mut blob, blob_base);
        assert_eq!(out.len(), blob_base);
        out.extend_from_slice(&blob);
        out
    }

    pub fn dirty_png() -> Vec<u8> {
        let img = sample_rgba(64, 48);
        let mut cursor = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut cursor, image::ImageFormat::Png).unwrap();
        let clean = cursor.into_inner();
        // Insert chunks right after IHDR (8 + 25 bytes).
        let ihdr_end = 8 + 25;
        let mut out = clean[..ihdr_end].to_vec();
        let mut params = b"parameters\0".to_vec();
        params.extend_from_slice(b"masterpiece, portrait\nNegative prompt: blurry\nSteps: 30, Sampler: DPM++ 2M, CFG scale: 7, Seed: 42, Model hash: abcd1234");
        out.extend(png_chunk(b"tEXt", &params));
        out.extend(png_chunk(b"caBX", &jumbf_payload()));
        let mut itxt = b"XML:com.adobe.xmp\0".to_vec();
        itxt.extend_from_slice(&[0, 0]);
        itxt.extend_from_slice(b"\0\0");
        itxt.extend_from_slice(&xmp_packet());
        out.extend(png_chunk(b"iTXt", &itxt));
        let mut exif = b"Exif\0\0".to_vec();
        exif.extend(exif_tiff());
        out.extend(png_chunk(b"eXIf", &exif[6..]));
        out.extend(png_chunk(b"tIME", &[0x07, 0xE9, 3, 4, 5, 6, 7]));
        out.extend(png_chunk(b"prVt", b"private vendor chunk"));
        out.extend_from_slice(&clean[ihdr_end..]);
        out.extend_from_slice(b"TRAILING-SECRET-PAYLOAD");
        out
    }

    pub fn jpeg_segment(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, marker];
        v.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        v.extend_from_slice(payload);
        v
    }

    pub fn dirty_jpeg() -> Vec<u8> {
        let img = sample_rgba(64, 48);
        let mut cursor = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .to_rgb8()
            .write_to(&mut cursor, image::ImageFormat::Jpeg)
            .unwrap();
        let clean = cursor.into_inner();
        let mut out = clean[..2].to_vec();
        let mut exif = b"Exif\0\0".to_vec();
        exif.extend(exif_tiff());
        out.extend(jpeg_segment(0xE1, &exif));
        let mut xmp = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
        xmp.extend(xmp_packet());
        out.extend(jpeg_segment(0xE1, &xmp));
        let mut app11 = b"JP".to_vec();
        app11.extend_from_slice(&1u16.to_be_bytes());
        app11.extend_from_slice(&1u32.to_be_bytes());
        app11.extend(jumbf_payload());
        out.extend(jpeg_segment(0xEB, &app11));
        let mut ps = b"Photoshop 3.0\0".to_vec();
        let mut iptc = vec![0x1C, 2, 80];
        iptc.extend_from_slice(&(5u16).to_be_bytes());
        iptc.extend_from_slice(b"Deniz");
        iptc.extend_from_slice(&[0x1C, 2, 90]);
        iptc.extend_from_slice(&(8u16).to_be_bytes());
        iptc.extend_from_slice(b"Istanbul");
        ps.extend_from_slice(b"8BIM");
        ps.extend_from_slice(&0x0404u16.to_be_bytes());
        ps.extend_from_slice(&[0, 0]);
        ps.extend_from_slice(&(iptc.len() as u32).to_be_bytes());
        ps.extend_from_slice(&iptc);
        if iptc.len() % 2 == 1 {
            ps.push(0);
        }
        out.extend(jpeg_segment(0xED, &ps));
        out.extend(jpeg_segment(0xFE, b"Created with Midjourney --ar 16:9 --v 6"));
        out.extend_from_slice(&clean[2..]);
        out.extend_from_slice(b"TRAILING-SECRET-PAYLOAD");
        out
    }

    pub fn riff_chunk(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = fourcc.to_vec();
        v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        v.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    pub fn dirty_webp() -> Vec<u8> {
        let img = sample_rgba(64, 48);
        let mut cursor = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut cursor, image::ImageFormat::WebP).unwrap();
        let clean = cursor.into_inner();
        let (chunks, _, _) = riff_chunks(&clean);
        let bitstream: Vec<&RiffChunk> = chunks
            .iter()
            .filter(|c| matches!(&c.fourcc, b"VP8 " | b"VP8L" | b"ALPH"))
            .collect();
        assert!(!bitstream.is_empty());
        let mut body = Vec::new();
        // VP8X with ICC | EXIF | XMP flags set.
        let mut vp8x = vec![0x20 | 0x08 | 0x04, 0, 0, 0];
        vp8x.extend_from_slice(&[63, 0, 0]);
        vp8x.extend_from_slice(&[47, 0, 0]);
        body.extend(riff_chunk(b"VP8X", &vp8x));
        body.extend(riff_chunk(b"ICCP", &fake_icc()));
        for c in &bitstream {
            body.extend(riff_chunk(&c.fourcc, c.payload));
        }
        let mut exif = b"Exif\0\0".to_vec();
        exif.extend(exif_tiff());
        body.extend(riff_chunk(b"EXIF", &exif));
        body.extend(riff_chunk(b"XMP ", &xmp_packet()));
        body.extend(riff_chunk(b"C2PA", &jumbf_payload()));
        body.extend(riff_chunk(b"zzzz", b"unknown vendor"));
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
        out.extend_from_slice(b"WEBP");
        out.extend_from_slice(&body);
        out.extend_from_slice(b"TRAILING-SECRET-PAYLOAD");
        out
    }

    pub fn fake_icc() -> Vec<u8> {
        let desc_text = b"Test Display P3 Profile";
        let mut desc_tag = b"desc".to_vec();
        desc_tag.extend_from_slice(&[0, 0, 0, 0]);
        desc_tag.extend_from_slice(&((desc_text.len() + 1) as u32).to_be_bytes());
        desc_tag.extend_from_slice(desc_text);
        desc_tag.push(0);
        let header_len = 128 + 4 + 12;
        let total = header_len + desc_tag.len();
        let mut p = vec![0u8; 128];
        p[0..4].copy_from_slice(&(total as u32).to_be_bytes());
        p[4..8].copy_from_slice(b"appl");
        p[12..16].copy_from_slice(b"mntr");
        p[16..20].copy_from_slice(b"RGB ");
        p[20..24].copy_from_slice(b"XYZ ");
        for (i, b) in p[84..100].iter_mut().enumerate() {
            *b = (i * 17) as u8;
        }
        p.extend_from_slice(&1u32.to_be_bytes());
        p.extend_from_slice(b"desc");
        p.extend_from_slice(&(header_len as u32).to_be_bytes());
        p.extend_from_slice(&(desc_tag.len() as u32).to_be_bytes());
        p.extend_from_slice(&desc_tag);
        p
    }

    pub fn pixel_sha256(data: &[u8]) -> String {
        let img = image::load_from_memory(data).expect("decodable image");
        let rgba = img.to_rgba8();
        let digest = Sha256::digest(rgba.as_raw());
        digest.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    fn has(report: &InspectReport, category: &str, sev: Severity) -> bool {
        report.findings.iter().any(|f| f.category == category && f.severity == sev)
    }

    fn names(report: &InspectReport) -> Vec<String> {
        report.findings.iter().map(|f| f.name.clone()).collect()
    }

    #[test]
    fn png_inspector_catches_every_injected_carrier() {
        let data = dirty_png();
        let r = inspect_image_bytes(&data);
        assert_eq!(r.format, "png");
        assert_eq!((r.width, r.height), (64, 48));
        assert!(has(&r, "c2pa", Severity::Critical), "caBX JUMBF: {:?}", names(&r));
        assert!(has(&r, "generator", Severity::Critical), "parameters tEXt: {:?}", names(&r));
        assert!(has(&r, "xmp", Severity::Critical), "iTXt XMP: {:?}", names(&r));
        assert!(has(&r, "exif", Severity::High), "eXIf: {:?}", names(&r));
        assert!(has(&r, "gps", Severity::Critical), "GPS: {:?}", names(&r));
        assert!(has(&r, "time", Severity::Medium), "tIME: {:?}", names(&r));
        assert!(has(&r, "unknown-chunk", Severity::Medium), "prVt: {:?}", names(&r));
        assert!(has(&r, "trailing", Severity::High), "trailing: {:?}", names(&r));
        assert!(r.findings.iter().any(|f| f.name.contains("BodySerialNumber") && f.value_preview == "SN-123456789"));
        assert!(r.findings.iter().any(|f| f.category == "gps" && f.value_preview.contains("41.023333")));
        assert!(!r.clean);
        assert!(r.critical >= 4);
    }

    #[test]
    fn png_safe_clean_removes_all_traces_and_keeps_pixels() {
        let data = dirty_png();
        let before = pixel_sha256(&data);
        let cleaned = clean_image_bytes(&data).unwrap();
        assert!(cleaned.removed_items >= 7, "removed {}", cleaned.removed_items);
        let after = pixel_sha256(&cleaned.data);
        assert_eq!(before, after, "pixels must be bit-identical");
        let r = inspect_image_bytes(&cleaned.data);
        assert!(r.clean, "remaining: {:?}", names(&r));
        assert_eq!(r.trace_count, 0);
        assert!(cleaned.data.len() < data.len());
        assert!(!contains_bytes(&cleaned.data, b"TRAILING"));
        assert!(!contains_bytes(&cleaned.data, b"c2pa"));
    }

    #[test]
    fn jpeg_inspector_catches_every_injected_carrier() {
        let data = dirty_jpeg();
        let r = inspect_image_bytes(&data);
        assert_eq!(r.format, "jpeg");
        assert_eq!((r.width, r.height), (64, 48));
        assert!(has(&r, "c2pa", Severity::Critical), "APP11: {:?}", names(&r));
        assert!(has(&r, "xmp", Severity::Critical), "XMP: {:?}", names(&r));
        assert!(has(&r, "exif", Severity::High), "EXIF: {:?}", names(&r));
        assert!(has(&r, "gps", Severity::Critical), "GPS: {:?}", names(&r));
        assert!(has(&r, "iptc", Severity::Critical), "IPTC: {:?}", names(&r));
        assert!(has(&r, "generator", Severity::Critical), "COM markers: {:?}", names(&r));
        assert!(has(&r, "quantization", Severity::Low), "DQT: {:?}", names(&r));
        assert!(has(&r, "trailing", Severity::High), "trailing: {:?}", names(&r));
        assert!(r.findings.iter().any(|f| f.category == "xmp" && f.detail.contains("CreatorTool=Adobe Firefly 3")));
        assert!(r.findings.iter().any(|f| f.category == "iptc" && f.detail.contains("By-line (Author)=Deniz")));
        assert!(r.findings.iter().any(|f| f.name.contains("Make") && f.value_preview == "TestCam Corp"));
    }

    #[test]
    fn jpeg_safe_clean_removes_all_traces_and_keeps_pixels() {
        let data = dirty_jpeg();
        let before = pixel_sha256(&data);
        let cleaned = clean_image_bytes(&data).unwrap();
        let after = pixel_sha256(&cleaned.data);
        assert_eq!(before, after, "pixels must be bit-identical");
        let r = inspect_image_bytes(&cleaned.data);
        assert!(r.clean, "remaining: {:?}", names(&r));
        // DQT fingerprint is still reported (inherent to the bitstream) but is not removable.
        assert!(has(&r, "quantization", Severity::Low));
        assert!(!contains_bytes(&cleaned.data, b"TRAILING"));
        assert!(!contains_bytes(&cleaned.data, b"Firefly"));
        assert!(!contains_bytes(&cleaned.data, b"jumb"));
    }

    #[test]
    fn webp_inspector_catches_every_injected_carrier() {
        let data = dirty_webp();
        let r = inspect_image_bytes(&data);
        assert_eq!(r.format, "webp");
        assert_eq!((r.width, r.height), (64, 48));
        assert!(has(&r, "c2pa", Severity::Critical), "C2PA: {:?}", names(&r));
        assert!(has(&r, "xmp", Severity::Critical), "XMP: {:?}", names(&r));
        assert!(has(&r, "exif", Severity::High), "EXIF: {:?}", names(&r));
        assert!(has(&r, "gps", Severity::Critical), "GPS: {:?}", names(&r));
        assert!(has(&r, "icc", Severity::Medium), "ICCP: {:?}", names(&r));
        assert!(has(&r, "unknown-chunk", Severity::Medium), "zzzz: {:?}", names(&r));
        assert!(has(&r, "trailing", Severity::High), "trailing: {:?}", names(&r));
        assert!(r.findings.iter().any(|f| f.category == "icc" && f.value_preview == "Test Display P3 Profile"));
    }

    #[test]
    fn webp_safe_clean_removes_all_traces_keeps_pixels_and_fixes_vp8x() {
        let data = dirty_webp();
        let before = pixel_sha256(&data);
        let cleaned = clean_image_bytes(&data).unwrap();
        let after = pixel_sha256(&cleaned.data);
        assert_eq!(before, after, "pixels must be bit-identical");
        let r = inspect_image_bytes(&cleaned.data);
        assert!(r.clean, "remaining: {:?}", names(&r));
        let riff_size = u32::from_le_bytes([cleaned.data[4], cleaned.data[5], cleaned.data[6], cleaned.data[7]]) as usize;
        assert_eq!(riff_size + 8, cleaned.data.len(), "RIFF size must be recomputed");
        let (chunks, _, _) = riff_chunks(&cleaned.data);
        let vp8x = chunks.iter().find(|c| &c.fourcc == b"VP8X").expect("VP8X kept");
        assert_eq!(vp8x.payload[0] & (0x20 | 0x08 | 0x04), 0, "ICC/EXIF/XMP flags cleared");
        assert!(!chunks.iter().any(|c| matches!(&c.fourcc, b"EXIF" | b"XMP " | b"ICCP" | b"C2PA" | b"zzzz")));
    }

    fn stamp() -> BrandStamp {
        BrandStamp { artist: "Seobify".to_string(), copyright: "© 2026 Seobify — seobify.com".to_string() }
    }

    #[test]
    fn brand_stamp_is_embedded_recognised_and_keeps_clean_status() {
        for (label, dirty) in [("png", dirty_png()), ("jpeg", dirty_jpeg()), ("webp", dirty_webp())] {
            let cleaned = clean_image_bytes(&dirty).unwrap().data;
            let before = pixel_sha256(&cleaned);
            let stamped = embed_brand_stamp(&cleaned, &stamp()).unwrap();
            assert_eq!(pixel_sha256(&stamped), before, "{label}: pixels untouched");
            let r = inspect_image_bytes(&stamped);
            assert!(r.clean, "{label}: stamp must not count as a trace: {:?}", names(&r));
            let f = r.findings.iter().filter(|f| f.category == "brand-stamp").collect::<Vec<_>>();
            assert!(!f.is_empty(), "{label}: stamp must be reported as Verified Brand Stamp");
            assert!(f.iter().all(|f| f.severity == Severity::Info && !f.removable));
            assert!(f.iter().any(|f| f.value_preview.contains("Seobify")), "{label}: {:?}", f.iter().map(|f| &f.value_preview).collect::<Vec<_>>());
            // Cleaning again strips the stamp (it is re-applied by the processor when requested).
            assert!(inspect_image_bytes(&clean_image_bytes(&stamped).unwrap().data).findings.iter().all(|f| f.category != "brand-stamp"));
        }
    }

    #[test]
    fn webp_stamp_adds_vp8x_with_exif_flag_when_missing() {
        let img = sample_rgba(16, 16);
        let mut webp = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).to_rgb8().write_to(&mut webp, image::ImageFormat::WebP).unwrap();
        let simple = webp.into_inner();
        let (chunks, _, _) = riff_chunks(&simple);
        assert!(!chunks.iter().any(|c| &c.fourcc == b"VP8X"), "test needs a simple-format WebP");
        let stamped = embed_brand_stamp(&simple, &stamp()).unwrap();
        let (chunks, declared_end, _) = riff_chunks(&stamped);
        assert_eq!(&chunks[0].fourcc, b"VP8X");
        assert_eq!(chunks[0].payload[0] & 0x08, 0x08);
        assert!(chunks.iter().any(|c| &c.fourcc == b"EXIF"));
        assert_eq!(declared_end, stamped.len());
        assert_eq!(pixel_sha256(&stamped), pixel_sha256(&simple));
        let (_, w, h, _) = read_dimensions(&stamped).unwrap();
        assert_eq!((w, h), (16, 16));
    }

    #[test]
    fn foreign_exif_with_artist_plus_other_tags_is_still_a_trace() {
        let r = inspect_image_bytes(&dirty_jpeg());
        assert!(r.findings.iter().any(|f| f.name.contains("Make") && f.removable));
        assert!(r.findings.iter().any(|f| f.name.contains("BodySerialNumber") && f.removable));
        assert!(r.findings.iter().all(|f| f.category != "brand-stamp"));
    }

    #[test]
    fn clean_images_report_zero_traces() {
        let img = sample_rgba(16, 16);
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img.clone()).write_to(&mut png, image::ImageFormat::Png).unwrap();
        assert!(inspect_image_bytes(png.get_ref()).clean);
        let mut jpg = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img.clone()).to_rgb8().write_to(&mut jpg, image::ImageFormat::Jpeg).unwrap();
        assert!(inspect_image_bytes(jpg.get_ref()).clean);
        let mut webp = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut webp, image::ImageFormat::WebP).unwrap();
        assert!(inspect_image_bytes(webp.get_ref()).clean);
    }
}
