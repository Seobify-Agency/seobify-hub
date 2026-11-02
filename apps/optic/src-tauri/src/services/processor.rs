//! End-to-end file processing shared by the Tauri commands and the MCP server:
//! inspect → (container clean | DSP pipeline) → encode to the target format → verify.

use crate::services::cwebp::{self, EncodingConfig};
use crate::services::dsp_pipeline::{run_pipeline, OutputFormat, PipelineStats, SanitizeConfig, SanitizeMode};
use crate::services::image::{detect_format, read_dimensions, ImageFormat};
use crate::services::metadata_cleaner::{clean_image_bytes, embed_brand_stamp, inspect_image_bytes, InspectReport};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{DynamicImage, ImageEncoder, RgbaImage};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

/// Detected cwebp binary path, cached after the first successful detection so each
/// preview encode does not spawn `cwebp -version` again.
static CWEBP_CACHE: Mutex<Option<String>> = Mutex::new(None);

pub struct ProcessRequest<'a> {
    pub input_path: &'a str,
    /// `None` writes to a temporary preview file next to the OS temp dir.
    pub output_path: Option<&'a str>,
    pub encoding: &'a EncodingConfig,
    pub sanitize: &'a SanitizeConfig,
    /// Explicit cwebp binary; auto-detected when `None` and WebP output is requested.
    pub cwebp_binary: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessOutcome {
    pub input_path: String,
    pub output_path: String,
    pub source_format: ImageFormat,
    pub output_format: ImageFormat,
    pub mime_type: String,
    pub original_size: u64,
    pub file_size: u64,
    pub reduction_percent: f64,
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub findings_before: u32,
    pub findings_after: u32,
    pub clean: bool,
    pub inspect_before: InspectReport,
    pub inspect_after: InspectReport,
    pub pipeline: Option<PipelineStats>,
}

pub fn resolve_output_format(source: ImageFormat, wanted: OutputFormat) -> ImageFormat {
    match wanted {
        OutputFormat::Webp => ImageFormat::WebP,
        OutputFormat::Original => source,
        OutputFormat::Png => ImageFormat::Png,
        OutputFormat::Jpeg => ImageFormat::Jpeg,
    }
}

/// Reads only the magic bytes of a file to determine its container format.
pub fn sniff_format(path: &Path) -> Result<ImageFormat, String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| format!("File read error: {}", e))?;
    let mut head = [0u8; 16];
    let n = f.read(&mut head).map_err(|e| format!("File read error: {}", e))?;
    detect_format(&head[..n])
}

/// Output file name (stem + extension of the resolved target format) for an input file.
/// With `seo_slug` the stem is rewritten by [`to_seo_slug`].
pub fn output_file_name(input_path: &Path, source_format: ImageFormat, wanted: OutputFormat, seo_slug: bool) -> String {
    let stem = input_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());
    let stem = if seo_slug { to_seo_slug(&stem) } else { stem };
    format!("{}.{}", stem, resolve_output_format(source_format, wanted).extension())
}

/// Picks a file name inside `dir` that neither exists on disk nor was handed out in this
/// batch, appending `-2`, `-3`, … to the stem when needed.
pub fn unique_output_name(dir: &Path, name: &str, used: &mut std::collections::HashSet<String>) -> String {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{}", e)),
        None => (name.to_string(), String::new()),
    };
    let mut candidate = name.to_string();
    let mut n = 2;
    while used.contains(&candidate.to_lowercase()) || dir.join(&candidate).exists() {
        candidate = format!("{}-{}{}", stem, n, ext);
        n += 1;
    }
    used.insert(candidate.to_lowercase());
    candidate
}

const SLUG_FALLBACK: &str = "seobify-gorsel";

/// Tokens that never carry SEO meaning (generator/camera prefixes and month names).
const SLUG_STOP_TOKENS: &[&str] = &[
    "chatgpt", "gemini", "generated", "untitled", "adsiz", "image", "img", "dsc", "dscn", "dcim", "pxl",
    "screenshot", "ekran", "goruntusu", "whatsapp", "photo", "picture", "copy", "kopya", "final", "edited",
    "ocak", "subat", "mart", "nisan", "mayis", "haziran", "temmuz", "agustos", "eylul", "eyl", "ekim", "kasim",
    "aralik", "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "sept", "oct", "nov", "dec",
];

fn fold_char(ch: char) -> Option<&'static str> {
    Some(match ch {
        'ş' | 'Ş' => "s",
        'ı' | 'I' | 'İ' | 'î' | 'Î' | 'ï' | 'í' | 'ì' => "i",
        'ğ' | 'Ğ' => "g",
        'ü' | 'Ü' | 'û' | 'Û' | 'ú' | 'ù' => "u",
        'ö' | 'Ö' | 'ô' | 'ó' | 'ò' | 'ø' => "o",
        'ç' | 'Ç' => "c",
        'â' | 'Â' | 'á' | 'à' | 'ä' | 'ã' | 'å' => "a",
        'é' | 'è' | 'ê' | 'ë' | 'É' => "e",
        'ñ' => "n",
        'ß' => "ss",
        _ => return None,
    })
}

/// Removes `(1)`, `( 12 )`-style duplicate counters.
fn strip_paren_counters(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '(' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            let digits_start = j;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            let had_digits = j > digits_start;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            if had_digits && j < chars.len() && chars[j] == ')' {
                i = j + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Turns a human/camera/AI file name into an SEO-friendly slug:
/// Turkish and Latin diacritics are folded, generator and camera prefixes
/// (`ChatGPT Image`, `Gemini Generated`, `IMG_`, `Untitled`…), dates/times and
/// parenthesised counters are dropped, and the rest is kebab-cased.
pub fn to_seo_slug(input: &str) -> String {
    let mut name = input.trim();
    if let Some((stem, ext)) = name.rsplit_once('.') {
        if (2..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) && !stem.is_empty() {
            name = stem;
        }
    }
    let name = strip_paren_counters(name);
    let mut folded = String::with_capacity(name.len());
    for ch in name.chars() {
        match fold_char(ch) {
            Some(rep) => folded.push_str(rep),
            None => folded.extend(ch.to_lowercase()),
        }
    }
    let raw: Vec<String> = folded
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect();
    let is_num = |t: &str| t.chars().all(|c| c.is_ascii_digit());
    let is_month = |t: &str| SLUG_STOP_TOKENS[22..].contains(&t);
    let mut keep: Vec<String> = Vec::with_capacity(raw.len());
    for (i, tok) in raw.iter().enumerate() {
        if SLUG_STOP_TOKENS.contains(&tok.as_str()) {
            continue;
        }
        // Canva/Figma defaults: "Untitled design" / "Adsız tasarım".
        if (tok == "design" || tok == "tasarim") && i > 0 && (raw[i - 1] == "untitled" || raw[i - 1] == "adsiz") {
            continue;
        }
        if is_num(tok) {
            let long = tok.len() >= 4;
            let prev_numeric = i > 0 && (is_num(&raw[i - 1]) || is_month(&raw[i - 1]));
            let next_numeric = i + 1 < raw.len() && (is_num(&raw[i + 1]) || is_month(&raw[i + 1]));
            if long || prev_numeric || next_numeric {
                continue;
            }
        }
        keep.push(tok.clone());
    }
    let mut slug = keep.join("-");
    if slug.len() > 80 {
        let cut = slug[..80].rfind('-').unwrap_or(80);
        slug.truncate(cut);
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        SLUG_FALLBACK.to_string()
    } else {
        slug
    }
}

pub fn temp_output_path(format: ImageFormat) -> PathBuf {
    std::env::temp_dir().join(format!("cwebp-preview-{}.{}", uuid::Uuid::new_v4(), format.extension()))
}

fn decode(data: &[u8]) -> Result<RgbaImage, String> {
    let img = image::load_from_memory(data).map_err(|e| format!("Decode error: {}", e))?;
    Ok(img.to_rgba8())
}

/// Encodes pixels to PNG or JPEG with the `image` crate (standard quantisation tables).
pub fn encode_pixels(img: &RgbaImage, format: ImageFormat, encoding: &EncodingConfig) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    match format {
        ImageFormat::Png => {
            let has_alpha = img.pixels().any(|p| p[3] != 255);
            let enc = PngEncoder::new_with_quality(&mut buf, CompressionType::Default, FilterType::Adaptive);
            if has_alpha {
                enc.write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgba8)
            } else {
                let rgb = DynamicImage::ImageRgba8(img.clone()).to_rgb8();
                enc.write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
            }
            .map_err(|e| format!("PNG encode error: {}", e))?;
        }
        ImageFormat::Jpeg => {
            // JPEG has no alpha: composite over white.
            let mut rgb = image::RgbImage::new(img.width(), img.height());
            for (src, dst) in img.pixels().zip(rgb.pixels_mut()) {
                let a = src[3] as f32 / 255.0;
                for c in 0..3 {
                    dst[c] = (src[c] as f32 * a + 255.0 * (1.0 - a)).round().clamp(0.0, 255.0) as u8;
                }
            }
            let quality = if encoding.lossless { 100 } else { encoding.quality.clamp(1, 100) };
            let enc = JpegEncoder::new_with_quality(&mut buf, quality);
            enc.write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
                .map_err(|e| format!("JPEG encode error: {}", e))?;
        }
        ImageFormat::WebP => return Err("WebP output is produced by cwebp".to_string()),
    }
    Ok(buf)
}

async fn resolve_cwebp(explicit: Option<&str>) -> Result<String, String> {
    if let Some(b) = explicit {
        return Ok(b.to_string());
    }
    if let Some(cached) = CWEBP_CACHE.lock().ok().and_then(|c| c.clone()) {
        if Path::new(&cached).exists() {
            return Ok(cached);
        }
    }
    let status = cwebp::detect_cwebp().await;
    if !status.available {
        return Err("cwebp not available".to_string());
    }
    let path = status.binary_path.unwrap_or_default();
    if let Ok(mut c) = CWEBP_CACHE.lock() {
        *c = Some(path.clone());
    }
    Ok(path)
}

/// Runs a CPU-heavy closure on the blocking thread pool so preview encodes never stall
/// the async runtime that serves the other IPC commands.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("Worker thread failed: {}", e))?
}

async fn cwebp_to_bytes(binary: &str, encoding: &EncodingConfig, input_path: &str) -> Result<Vec<u8>, String> {
    let out = temp_output_path(ImageFormat::WebP);
    let out_str = out.to_string_lossy().to_string();
    let result = cwebp::execute_cwebp(binary, encoding, input_path, &out_str).await;
    let bytes = match result {
        Ok(()) => tokio::fs::read(&out).await.map_err(|e| format!("Failed to read cwebp output: {}", e)),
        Err(e) => Err(e),
    };
    let _ = tokio::fs::remove_file(&out).await;
    bytes
}

async fn pixels_to_webp(binary: &str, encoding: &EncodingConfig, img: RgbaImage) -> Result<Vec<u8>, String> {
    let enc = encoding.clone();
    let png = blocking(move || encode_pixels(&img, ImageFormat::Png, &enc)).await?;
    let tmp = std::env::temp_dir().join(format!("cwebp-src-{}.png", uuid::Uuid::new_v4()));
    tokio::fs::write(&tmp, &png).await.map_err(|e| format!("Temp write error: {}", e))?;
    let result = cwebp_to_bytes(binary, encoding, &tmp.to_string_lossy()).await;
    let _ = tokio::fs::remove_file(&tmp).await;
    result
}

fn strip(bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    Ok(clean_image_bytes(&bytes)?.data)
}

/// Processes one file according to the sanitize configuration and writes the result.
pub async fn process_image_file(req: ProcessRequest<'_>) -> Result<ProcessOutcome, String> {
    let start = Instant::now();
    if !Path::new(req.input_path).exists() {
        return Err(format!("Source file not found: {}", req.input_path));
    }
    let data = tokio::fs::read(req.input_path)
        .await
        .map_err(|e| format!("File read error: {}", e))?;
    let source_format = detect_format(&data)?;
    let inspect_before = inspect_image_bytes(&data);
    let target = resolve_output_format(source_format, req.sanitize.output_format);
    let output_path = match req.output_path {
        Some(p) => PathBuf::from(p),
        None => temp_output_path(target),
    };
    if Path::new(req.input_path) == output_path {
        return Err("Output path must differ from the input path".to_string());
    }

    let mut pipeline: Option<PipelineStats> = None;
    let mode = req.sanitize.mode;
    // A downscale re-encodes the pixels, so safe mode leaves the lossless container path
    // only when the image is actually wider than max_width.
    let downscale = match req.sanitize.max_width {
        Some(mw) => read_dimensions(&data).map(|(_, w, _, _)| w > mw).unwrap_or(false),
        None => false,
    };

    let encoding = req.encoding.clone();
    let bytes: Vec<u8> = match mode {
        SanitizeMode::Off => {
            if target == ImageFormat::WebP {
                let binary = resolve_cwebp(req.cwebp_binary).await?;
                cwebp_to_bytes(&binary, req.encoding, req.input_path).await?
            } else if target == source_format {
                data.clone()
            } else {
                let d = data.clone();
                blocking(move || encode_pixels(&decode(&d)?, target, &encoding)).await?
            }
        }
        SanitizeMode::Safe if !downscale => {
            if target == source_format {
                let d = data.clone();
                blocking(move || strip(d)).await?
            } else if target == ImageFormat::WebP {
                let binary = resolve_cwebp(req.cwebp_binary).await?;
                let webp = cwebp_to_bytes(&binary, req.encoding, req.input_path).await?;
                blocking(move || strip(webp)).await?
            } else {
                let d = data.clone();
                blocking(move || strip(encode_pixels(&decode(&d)?, target, &encoding)?)).await?
            }
        }
        SanitizeMode::Safe | SanitizeMode::SensorNatural | SanitizeMode::StudioMaster => {
            let d = data.clone();
            let cfg = req.sanitize.clone();
            let (processed, stats) = blocking(move || run_pipeline(&decode(&d)?, &cfg)).await?;
            pipeline = Some(stats);
            if target == ImageFormat::WebP {
                let binary = resolve_cwebp(req.cwebp_binary).await?;
                let webp = pixels_to_webp(&binary, req.encoding, processed).await?;
                blocking(move || strip(webp)).await?
            } else {
                blocking(move || strip(encode_pixels(&processed, target, &encoding)?)).await?
            }
        }
    };

    let bytes = match &req.sanitize.brand_stamp {
        Some(stamp) if !stamp.is_empty() => embed_brand_stamp(&bytes, stamp)?,
        _ => bytes,
    };

    tokio::fs::write(&output_path, &bytes)
        .await
        .map_err(|e| format!("Write error: {}", e))?;

    let inspect_after = {
        let b = bytes.clone();
        blocking(move || Ok(inspect_image_bytes(&b))).await?
    };
    let (width, height) = match read_dimensions(&bytes) {
        Ok((_, w, h, _)) => (w, h),
        Err(_) => (inspect_before.width, inspect_before.height),
    };
    let original_size = data.len() as u64;
    let file_size = bytes.len() as u64;
    let reduction_percent = if original_size > 0 {
        (1.0 - (file_size as f64 / original_size as f64)) * 100.0
    } else {
        0.0
    };

    Ok(ProcessOutcome {
        input_path: req.input_path.to_string(),
        output_path: output_path.to_string_lossy().to_string(),
        source_format,
        output_format: target,
        mime_type: target.mime_type().to_string(),
        original_size,
        file_size,
        reduction_percent,
        duration_ms: start.elapsed().as_millis() as u64,
        width,
        height,
        findings_before: inspect_before.trace_count,
        findings_after: inspect_after.trace_count,
        clean: inspect_after.clean,
        inspect_before,
        inspect_after,
        pipeline,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::metadata_cleaner::test_support::{dirty_jpeg, dirty_png, pixel_sha256};

    fn encoding() -> EncodingConfig {
        EncodingConfig {
            quality: 85,
            alpha_quality: 100,
            method: 4,
            lossless: false,
            near_lossless: None,
            metadata: "none".to_string(),
        }
    }

    fn write_temp(name: &str, data: &[u8]) -> PathBuf {
        let p = std::env::temp_dir().join(format!("cwebp-test-{}-{}", uuid::Uuid::new_v4(), name));
        std::fs::write(&p, data).unwrap();
        p
    }

    #[tokio::test]
    async fn safe_mode_same_format_is_lossless_and_clean() {
        let src = write_temp("dirty.png", &dirty_png());
        let before = pixel_sha256(&std::fs::read(&src).unwrap());
        let out = std::env::temp_dir().join(format!("cwebp-test-{}-out.png", uuid::Uuid::new_v4()));
        let sanitize = SanitizeConfig { mode: SanitizeMode::Safe, deblend_corner: false, output_format: OutputFormat::Original, seed: None, ..Default::default() };
        let outcome = process_image_file(ProcessRequest {
            input_path: &src.to_string_lossy(),
            output_path: Some(&out.to_string_lossy()),
            encoding: &encoding(),
            sanitize: &sanitize,
            cwebp_binary: None,
        })
        .await
        .unwrap();
        assert_eq!(outcome.output_format, ImageFormat::Png);
        assert!(outcome.findings_before >= 7);
        assert_eq!(outcome.findings_after, 0);
        assert!(outcome.clean);
        assert_eq!(pixel_sha256(&std::fs::read(&out).unwrap()), before);
        let _ = std::fs::remove_file(src);
        let _ = std::fs::remove_file(out);
    }

    #[tokio::test]
    async fn studio_master_transcodes_jpeg_to_png_clean() {
        let src = write_temp("dirty.jpg", &dirty_jpeg());
        let sanitize = SanitizeConfig { mode: SanitizeMode::StudioMaster, deblend_corner: true, output_format: OutputFormat::Png, seed: Some(7), ..Default::default() };
        let outcome = process_image_file(ProcessRequest {
            input_path: &src.to_string_lossy(),
            output_path: None,
            encoding: &encoding(),
            sanitize: &sanitize,
            cwebp_binary: None,
        })
        .await
        .unwrap();
        assert_eq!(outcome.output_format, ImageFormat::Png);
        assert!(outcome.output_path.ends_with(".png"));
        assert!(outcome.clean);
        let stats = outcome.pipeline.expect("pipeline stats");
        assert!(stats.psnr > 35.0);
        assert!(outcome.width < 64 && outcome.height < 48);
        let _ = std::fs::remove_file(src);
        let _ = std::fs::remove_file(outcome.output_path);
    }

    #[tokio::test]
    async fn off_mode_jpeg_output_uses_standard_tables_and_is_clean() {
        let src = write_temp("dirty.png", &dirty_png());
        let sanitize = SanitizeConfig { mode: SanitizeMode::Off, deblend_corner: false, output_format: OutputFormat::Jpeg, seed: None, ..Default::default() };
        let outcome = process_image_file(ProcessRequest {
            input_path: &src.to_string_lossy(),
            output_path: None,
            encoding: &encoding(),
            sanitize: &sanitize,
            cwebp_binary: None,
        })
        .await
        .unwrap();
        assert_eq!(outcome.output_format, ImageFormat::Jpeg);
        assert!(outcome.inspect_after.findings.iter().any(|f| f.category == "quantization"));
        assert!(outcome.clean);
        let _ = std::fs::remove_file(src);
        let _ = std::fs::remove_file(outcome.output_path);
    }

    #[test]
    fn seo_slug_cleans_generator_camera_and_turkish_names() {
        assert_eq!(to_seo_slug("Kurumsal SEO Danışmanlığı.webp"), "kurumsal-seo-danismanligi");
        assert_eq!(to_seo_slug("ChatGPT Image 23 Eyl 2026 23_36_44 (1).png"), "seobify-gorsel");
        assert_eq!(to_seo_slug("IMG_4821.JPG"), "seobify-gorsel");
        assert_eq!(to_seo_slug("Gemini_Generated_Image_abc123.jpeg"), "abc123");
        assert_eq!(to_seo_slug("Seobify Blog Görseli (2).png"), "seobify-blog-gorseli");
        assert_eq!(to_seo_slug("iPhone 15 Pro Max kılıf.jpg"), "iphone-15-pro-max-kilif");
        assert_eq!(to_seo_slug("Adsız tasarım (3).png"), "seobify-gorsel");
        assert_eq!(to_seo_slug("  --Özel__Çağrı--  "), "ozel-cagri");
        assert_eq!(to_seo_slug("2026-10-29 rapor.png"), "rapor");
        assert_eq!(to_seo_slug(""), "seobify-gorsel");
        let long = to_seo_slug(&"kelime ".repeat(30));
        assert!(long.len() <= 80 && !long.ends_with('-'));
    }

    #[test]
    fn output_names_use_slug_and_stay_unique() {
        let dir = std::env::temp_dir();
        let mut used = std::collections::HashSet::new();
        let a = output_file_name(Path::new("C:/x/ChatGPT Image 23 Eyl 2026 (1).png"), ImageFormat::Png, OutputFormat::Webp, true);
        let b = output_file_name(Path::new("C:/x/IMG_0001.jpg"), ImageFormat::Jpeg, OutputFormat::Webp, true);
        assert_eq!(a, "seobify-gorsel.webp");
        assert_eq!(b, "seobify-gorsel.webp");
        let ua = unique_output_name(&dir, &a, &mut used);
        let ub = unique_output_name(&dir, &b, &mut used);
        assert_eq!(ua, "seobify-gorsel.webp");
        assert_eq!(ub, "seobify-gorsel-2.webp");
        assert_eq!(output_file_name(Path::new("C:/x/Hero Görsel.PNG"), ImageFormat::Png, OutputFormat::Original, false), "Hero Görsel.png");
    }

    #[tokio::test]
    async fn brand_stamp_is_written_after_cleaning_and_output_stays_clean() {
        let src = write_temp("dirty.jpg", &dirty_jpeg());
        let sanitize = SanitizeConfig {
            mode: SanitizeMode::Safe,
            output_format: OutputFormat::Original,
            brand_stamp: Some(crate::services::dsp_pipeline::BrandStamp {
                artist: "Seobify".to_string(),
                copyright: "© 2026 Seobify — seobify.com".to_string(),
            }),
            ..Default::default()
        };
        let outcome = process_image_file(ProcessRequest {
            input_path: &src.to_string_lossy(),
            output_path: None,
            encoding: &encoding(),
            sanitize: &sanitize,
            cwebp_binary: None,
        })
        .await
        .unwrap();
        assert!(outcome.clean);
        assert_eq!(outcome.findings_after, 0);
        let stamp = outcome.inspect_after.findings.iter().find(|f| f.category == "brand-stamp").expect("stamp reported");
        assert!(stamp.value_preview.contains("Seobify"));
        assert!(!stamp.removable);
        let _ = std::fs::remove_file(src);
        let _ = std::fs::remove_file(outcome.output_path);
    }

    #[tokio::test]
    async fn webp_round_trip_when_cwebp_is_installed() {
        let status = cwebp::detect_cwebp().await;
        if !status.available {
            eprintln!("cwebp not installed — skipping WebP round-trip test");
            return;
        }
        let src = write_temp("dirty.jpg", &dirty_jpeg());
        let sanitize = SanitizeConfig { mode: SanitizeMode::SensorNatural, deblend_corner: false, output_format: OutputFormat::Webp, seed: Some(3), ..Default::default() };
        let outcome = process_image_file(ProcessRequest {
            input_path: &src.to_string_lossy(),
            output_path: None,
            encoding: &encoding(),
            sanitize: &sanitize,
            cwebp_binary: None,
        })
        .await
        .unwrap();
        assert_eq!(outcome.output_format, ImageFormat::WebP);
        assert!(outcome.clean, "{:?}", outcome.inspect_after.summary);
        assert_eq!((outcome.width, outcome.height), (64, 48));
        let _ = std::fs::remove_file(src);
        let _ = std::fs::remove_file(outcome.output_path);
    }
}
