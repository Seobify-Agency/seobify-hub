//! Model Context Protocol server (stdio, JSON-RPC 2.0, newline-delimited), built as the
//! `seobify-optic-mcp` and `cwebp-mcp` binaries.
//!
//! Exposes the image and document privacy toolchain to MCP clients:
//! `inspect_image`, `process_image`, `batch_process` (images) and `inspect_document`,
//! `clean_document`, `batch_documents` (DOCX/XLSX/PPTX, ODT/ODS/ODP, PDF). The same server
//! runs as `seobify-optic.exe --mcp` and as the `seobify-optic-mcp`/`cwebp-mcp` binaries.

use crate::services::cwebp::{self, EncodingConfig};
use crate::services::document_cleaner::{self, DocCleanOptions, DocInspectReport, DocMode, DocProcessOutcome, Scope};
use crate::services::dsp_pipeline::{BrandStamp, OutputFormat, SanitizeConfig, SanitizeMode};
use crate::services::image::ImageFormat;
use crate::services::metadata_cleaner::inspect_image_bytes;
use crate::services::processor::{output_file_name, process_image_file, sniff_format, to_seo_slug, unique_output_name, ProcessRequest};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

const SERVER_NAME: &str = "seobify-optic-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_PROTOCOL: &str = "2025-06-18";
const SUPPORTED_PROTOCOLS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];

/// Runs the stdio server loop until stdin closes.
pub fn run() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                write_message(&mut out, &error_response(Value::Null, -32700, &format!("Parse error: {}", e)));
                continue;
            }
        };
        if let Some(response) = runtime.block_on(handle_message(message)) {
            write_message(&mut out, &response);
        }
    }
}

fn write_message(out: &mut impl Write, value: &Value) {
    if let Ok(text) = serde_json::to_string(value) {
        let _ = out.write_all(text.as_bytes());
        let _ = out.write_all(b"\n");
        let _ = out.flush();
    }
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn result_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

async fn handle_message(message: Value) -> Option<Value> {
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(|m| m.as_str()).unwrap_or("").to_string();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let is_notification = id.is_none();

    if method.is_empty() {
        return if is_notification { None } else { Some(error_response(id.unwrap_or(Value::Null), -32600, "Invalid Request: missing method")) };
    }

    let outcome: Result<Value, (i64, String)> = match method.as_str() {
        "initialize" => Ok(handle_initialize(&params)),
        "notifications/initialized" | "notifications/cancelled" | "notifications/progress" | "notifications/roots/list_changed" => {
            return None;
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => handle_tools_call(&params).await,
        "resources/list" => Ok(json!({ "resources": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        _ => Err((-32601, format!("Method not found: {}", method))),
    };

    if is_notification {
        return None;
    }
    let id = id.unwrap_or(Value::Null);
    Some(match outcome {
        Ok(result) => result_response(id, result),
        Err((code, msg)) => error_response(id, code, &msg),
    })
}

fn handle_initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or(DEFAULT_PROTOCOL);
    let version = if SUPPORTED_PROTOCOLS.contains(&requested) { requested } else { DEFAULT_PROTOCOL };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
        "instructions": "Image and document privacy toolchain. Images: inspect_image reports every metadata carrier (EXIF/GPS, XMP, IPTC, ICC, JUMBF/C2PA, generator parameters, trailing bytes) in PNG/JPEG/WebP files; process_image and batch_process remove them in safe (lossless), sensor_natural or studio_master (full DSP) mode and can transcode to webp/png/jpeg. Both accept max_width (Lanczos downscale before the DSP stages, never upscales); batch_process accepts an explicit files list instead of input_dir. Documents (DOCX/XLSX/PPTX, ODT/ODS/ODP/ODG, PDF): call inspect_document first to see each carrier with its severity and the mode that removes it (scope safe/strict/notes/report), then clean_document (one file) or batch_documents (a folder or a files list) in safe mode (properties, thumbnails, printer settings, rsids, local paths, embedded-image EXIF, PDF Info/XMP/ID/old revisions) or strict mode (+ comments and tracked-change authors; remove_notes also deletes PPTX speaker notes). Outputs are re-inspected; inputs are never overwritten."
    })
}

fn mode_schema() -> Value {
    json!({
        "type": "string",
        "enum": ["safe", "sensor_natural", "studio_master"],
        "description": "safe = lossless container clean (bit-identical pixels); sensor_natural = + camera sensor noise & dither; studio_master = + elastic resampling, FFT spectral conditioning, grid shift"
    })
}

fn format_schema() -> Value {
    json!({
        "type": "string",
        "enum": ["original", "webp", "png", "jpeg"],
        "default": "original",
        "description": "Output container. webp requires the cwebp binary."
    })
}

fn max_width_schema() -> Value {
    json!({
        "type": "integer",
        "minimum": 1,
        "description": "Optional maximum output width in px. Wider images are downscaled with a Lanczos filter (aspect ratio preserved) BEFORE the DSP stages, so sensor noise and spectral conditioning run at the final resolution; narrower images are never upscaled. In safe mode a downscale re-encodes the pixels, so the bit-identical guarantee only holds for images already at or below max_width. studio_master still trims 1-2 px (grid shift) after the downscale."
    })
}

fn doc_mode_schema() -> Value {
    json!({
        "type": "string",
        "enum": ["safe", "strict"],
        "description": "safe = document/custom properties, thumbnails, custom XML, printer settings, rsid/GUID fingerprints, local file paths, account IDs, embedded-image EXIF/GPS, ZIP timestamps; PDF Info, XMP, /ID, PieceInfo and earlier incremental revisions. strict = safe + comments, comment/revision author lists and tracked-change authors/dates (PDF: annotation authors). Content is never rewritten."
    })
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "inspect_image",
            "description": "Deep container inspection of a PNG, JPEG or WebP file. Returns every metadata carrier found (EXIF IFDs incl. GPS and serial numbers, XMP packets with AI-provenance markers, IPTC/8BIM, ICC profiles, JUMBF/C2PA manifests, generator parameters in text chunks/comments, thumbnails, quantization fingerprints, trailing bytes) with severity ratings.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path of the image file" }
                },
                "required": ["path"]
            }
        },
        {
            "name": "process_image",
            "description": "Sanitise one image: strips all metadata (safe), optionally applies the sensor-noise or full studio DSP pipeline, and writes the result in the requested format. Never overwrites the input. Returns before/after trace counts, file sizes and pipeline fidelity metrics.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "input_path": { "type": "string", "description": "Absolute path of the source image" },
                    "output_path": { "type": "string", "description": "Optional destination path. Defaults to <stem>.clean.<ext> next to the input." },
                    "mode": mode_schema(),
                    "deblend_corner": { "type": "boolean", "default": false, "description": "studio_master only: inverse-blend a semi-transparent overlay in the bottom-right corner" },
                    "output_format": format_schema(),
                    "quality": { "type": "integer", "minimum": 0, "maximum": 100, "default": 85, "description": "Lossy quality for webp/jpeg output" },
                    "max_width": max_width_schema(),
                    "seed": { "type": "integer", "description": "Optional seed for reproducible stochastic stages" },
                    "seo_slug": { "type": "boolean", "default": false, "description": "Name the output with an SEO slug derived from the source file name (Turkish characters folded, generator/camera prefixes, dates and counters removed)" },
                    "brand_artist": { "type": "string", "description": "Optional clean attribution written after cleaning (EXIF Artist / PNG tEXt Author)" },
                    "brand_copyright": { "type": "string", "description": "Optional clean copyright notice written after cleaning (EXIF Copyright / PNG tEXt Copyright)" }
                },
                "required": ["input_path", "mode"]
            }
        },
        {
            "name": "batch_process",
            "description": "Sanitise every png/jpg/jpeg/webp image in a directory, or an explicit list of files, with the same settings and return a summary report (per-file status, traces removed, total bytes).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "input_dir": { "type": "string", "description": "Directory containing the images. Required unless files is given; ignored when files is given." },
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Optional explicit list of absolute image paths to process instead of scanning input_dir (takes precedence over input_dir and recursive). Outputs are written flat into output_dir; a file that already lives in output_dir gets the .clean suffix." },
                    "output_dir": { "type": "string", "description": "Destination directory (created if missing). When it equals input_dir, outputs get a .clean suffix." },
                    "mode": mode_schema(),
                    "deblend_corner": { "type": "boolean", "default": false },
                    "output_format": format_schema(),
                    "quality": { "type": "integer", "minimum": 0, "maximum": 100, "default": 85 },
                    "max_width": max_width_schema(),
                    "recursive": { "type": "boolean", "default": false, "description": "Also process sub-directories (mirrored under output_dir)" },
                    "seed": { "type": "integer" },
                    "seo_slug": { "type": "boolean", "default": false, "description": "Name outputs with SEO slugs (collisions get -2, -3 suffixes)" },
                    "brand_artist": { "type": "string" },
                    "brand_copyright": { "type": "string" }
                },
                "required": ["output_dir", "mode"]
            }
        },
        {
            "name": "inspect_document",
            "description": "Inspect a DOCX/XLSX/PPTX (and macro/template variants), ODT/ODS/ODP/ODG or PDF file. Returns every metadata carrier (authors, dates, company, custom properties, comments, tracked changes, rsids, printer names, local paths, account IDs, embedded-image EXIF/GPS, thumbnails, PDF Info/XMP/ID/revisions) with severity and scope: safe/strict/notes = removed by that cleaning mode, report = reported only (macros, JavaScript, embedded files).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path of the document" },
                    "limit": { "type": "integer", "minimum": 1, "default": 100, "description": "Maximum findings to return (most severe first)" }
                },
                "required": ["path"]
            }
        },
        {
            "name": "clean_document",
            "description": "Write a cleaned copy of one document (never overwrites the input; default <stem>.clean.<ext> next to it) and re-inspect it. Returns traces before/after, what was removed and what remains. The file stays openable in Word/Excel/PowerPoint/LibreOffice/Acrobat.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "input_path": { "type": "string", "description": "Absolute path of the source document" },
                    "output_path": { "type": "string", "description": "Optional destination path" },
                    "mode": doc_mode_schema(),
                    "remove_notes": { "type": "boolean", "default": false, "description": "strict only: also delete PPTX speaker notes" },
                    "brand_artist": { "type": "string", "description": "Optional organisation written after cleaning (OOXML Company, ODF Company field, PDF Producer)" },
                    "brand_copyright": { "type": "string", "description": "Optional copyright written after cleaning (custom Copyright property / PDF Copyright)" }
                },
                "required": ["input_path", "mode"]
            }
        },
        {
            "name": "batch_documents",
            "description": "Clean every supported document in a folder, or an explicit files list, with the same settings. Outputs keep their names inside output_dir (a .clean suffix when output_dir is the source folder). Returns per-file traces before/after and totals.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "input_dir": { "type": "string", "description": "Folder to scan. Required unless files is given; ignored when files is given." },
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Optional explicit list of absolute document paths (takes precedence over input_dir)" },
                    "output_dir": { "type": "string", "description": "Destination folder (created if missing)" },
                    "mode": doc_mode_schema(),
                    "remove_notes": { "type": "boolean", "default": false },
                    "recursive": { "type": "boolean", "default": false, "description": "Also scan sub-folders (mirrored under output_dir)" },
                    "brand_artist": { "type": "string" },
                    "brand_copyright": { "type": "string" }
                },
                "required": ["output_dir", "mode"]
            }
        }
    ])
}

fn tool_ok(payload: Value) -> Value {
    let text = serde_json::to_string_pretty(&payload).unwrap_or_default();
    json!({ "content": [{ "type": "text", "text": text }], "structuredContent": payload, "isError": false })
}

fn tool_err(message: String) -> Value {
    json!({ "content": [{ "type": "text", "text": message }], "isError": true })
}

async fn handle_tools_call(params: &Value) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let result = match name {
        "inspect_image" => tool_inspect_image(&args).await,
        "process_image" => tool_process_image(&args).await,
        "batch_process" => tool_batch_process(&args).await,
        "inspect_document" => tool_inspect_document(&args).await,
        "clean_document" => tool_clean_document(&args).await,
        "batch_documents" => tool_batch_documents(&args).await,
        _ => return Err((-32602, format!("Unknown tool: {}", name))),
    };
    Ok(match result {
        Ok(v) => tool_ok(v),
        Err(e) => tool_err(e),
    })
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str()).filter(|s| !s.trim().is_empty())
}

fn parse_sanitize(args: &Value) -> Result<SanitizeConfig, String> {
    let mode_str = arg_str(args, "mode").ok_or("Missing required argument: mode")?;
    let mode = SanitizeMode::parse(mode_str).ok_or_else(|| format!("Unknown mode '{}' (expected safe | sensor_natural | studio_master)", mode_str))?;
    if mode == SanitizeMode::Off {
        return Err("mode 'off' performs no sanitisation; use safe, sensor_natural or studio_master".to_string());
    }
    let output_format = match arg_str(args, "output_format") {
        Some(f) => OutputFormat::parse(f).ok_or_else(|| format!("Unknown output_format '{}'", f))?,
        None => OutputFormat::Original,
    };
    let brand_stamp = {
        let stamp = BrandStamp {
            artist: arg_str(args, "brand_artist").unwrap_or("").to_string(),
            copyright: arg_str(args, "brand_copyright").unwrap_or("").to_string(),
        };
        if stamp.is_empty() { None } else { Some(stamp) }
    };
    let max_width = match args.get("max_width") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .filter(|w| (1..=u32::MAX as u64).contains(w))
                .ok_or_else(|| format!("max_width must be a positive integer (got {})", v))? as u32,
        ),
    };
    Ok(SanitizeConfig {
        mode,
        deblend_corner: args.get("deblend_corner").and_then(|v| v.as_bool()).unwrap_or(false),
        output_format,
        seed: args.get("seed").and_then(|v| v.as_u64()),
        brand_stamp,
        seo_slug: args.get("seo_slug").and_then(|v| v.as_bool()).unwrap_or(false),
        max_width,
    })
}

fn parse_encoding(args: &Value) -> EncodingConfig {
    let quality = args.get("quality").and_then(|v| v.as_u64()).unwrap_or(85).min(100) as u8;
    EncodingConfig {
        quality,
        alpha_quality: 100,
        method: 4,
        lossless: false,
        near_lossless: None,
        metadata: "none".to_string(),
    }
}

async fn resolve_cwebp_if_needed(sanitize: &SanitizeConfig, source: Option<ImageFormat>) -> Result<Option<String>, String> {
    let needs_webp = match (sanitize.output_format, source) {
        (OutputFormat::Webp, _) => true,
        (OutputFormat::Original, Some(ImageFormat::WebP)) => sanitize.mode != SanitizeMode::Safe,
        (OutputFormat::Original, None) => true,
        _ => false,
    };
    if !needs_webp {
        return Ok(None);
    }
    let status = cwebp::detect_cwebp().await;
    if status.available {
        Ok(status.binary_path)
    } else {
        Err("WebP output requested but the cwebp binary was not found on this system".to_string())
    }
}

async fn tool_inspect_image(args: &Value) -> Result<Value, String> {
    let path = arg_str(args, "path").ok_or("Missing required argument: path")?;
    if !Path::new(path).is_file() {
        return Err(format!("File not found: {}", path));
    }
    let data = tokio::fs::read(path).await.map_err(|e| format!("File read error: {}", e))?;
    let report = inspect_image_bytes(&data);
    let mut value = serde_json::to_value(&report).map_err(|e| e.to_string())?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert("path".to_string(), json!(path));
    }
    Ok(value)
}

fn default_output_path(input: &Path, sanitize: &SanitizeConfig) -> Result<PathBuf, String> {
    let fmt = sniff_format(input)?;
    let name = output_file_name(input, fmt, sanitize.output_format, sanitize.seo_slug);
    let (stem, ext) = name.rsplit_once('.').unwrap_or((&name, ""));
    let dir = input.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."));
    if sanitize.seo_slug {
        let mut used = std::collections::HashSet::new();
        return Ok(dir.join(unique_output_name(&dir, &format!("{}.{}", stem, ext), &mut used)));
    }
    Ok(dir.join(format!("{}.clean.{}", stem, ext)))
}

async fn tool_process_image(args: &Value) -> Result<Value, String> {
    let input = arg_str(args, "input_path").ok_or("Missing required argument: input_path")?;
    let input_path = Path::new(input);
    if !input_path.is_file() {
        return Err(format!("File not found: {}", input));
    }
    let sanitize = parse_sanitize(args)?;
    let encoding = parse_encoding(args);
    let output = match arg_str(args, "output_path") {
        Some(p) => PathBuf::from(p),
        None => default_output_path(input_path, &sanitize)?,
    };
    if output == input_path {
        return Err("output_path must differ from input_path (inputs are never overwritten)".to_string());
    }
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| format!("Cannot create output directory: {}", e))?;
        }
    }
    let source_fmt = sniff_format(input_path).ok();
    let binary = resolve_cwebp_if_needed(&sanitize, source_fmt).await?;
    let output_str = output.to_string_lossy().to_string();
    let outcome = process_image_file(ProcessRequest {
        input_path: input,
        output_path: Some(&output_str),
        encoding: &encoding,
        sanitize: &sanitize,
        cwebp_binary: binary.as_deref(),
    })
    .await?;
    Ok(json!({
        "inputPath": outcome.input_path,
        "outputPath": outcome.output_path,
        "seoSlug": if sanitize.seo_slug { Some(to_seo_slug(&input_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())) } else { None },
        "brandStamp": sanitize.brand_stamp.as_ref().map(|b| json!({ "artist": b.artist, "copyright": b.copyright })),
        "mode": sanitize.mode.label(),
        "sourceFormat": outcome.source_format.label(),
        "outputFormat": outcome.output_format.label(),
        "originalSize": outcome.original_size,
        "fileSize": outcome.file_size,
        "reductionPercent": (outcome.reduction_percent * 10.0).round() / 10.0,
        "width": outcome.width,
        "height": outcome.height,
        "findingsBefore": outcome.findings_before,
        "findingsAfter": outcome.findings_after,
        "clean": outcome.clean,
        "durationMs": outcome.duration_ms,
        "pipeline": outcome.pipeline,
        "severityBefore": {
            "critical": outcome.inspect_before.critical,
            "high": outcome.inspect_before.high,
            "medium": outcome.inspect_before.medium,
            "low": outcome.inspect_before.low
        },
        "removed": outcome.inspect_before.findings.iter().filter(|f| f.removable).map(|f| json!({
            "name": f.name, "category": f.category, "severity": f.severity
        })).collect::<Vec<_>>(),
        "remaining": outcome.inspect_after.findings.iter().filter(|f| f.removable).map(|f| json!({
            "name": f.name, "category": f.category, "severity": f.severity
        })).collect::<Vec<_>>()
    }))
}

/// Files this server produced earlier (`<stem>.clean.<ext>`), skipped on re-runs so a
/// batch into the same directory never chains into `.clean.clean` outputs.
fn is_own_output(path: &Path) -> bool {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase().ends_with(".clean"))
        .unwrap_or(false)
}

fn collect_images(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("Cannot read directory {}: {}", dir.display(), e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if recursive {
                collect_images(&path, true, out)?;
            }
            continue;
        }
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if IMAGE_EXTENSIONS.contains(&ext.as_str()) && !is_own_output(&path) {
            out.push(path);
        }
    }
    Ok(())
}

async fn tool_batch_process(args: &Value) -> Result<Value, String> {
    let output_dir = PathBuf::from(arg_str(args, "output_dir").ok_or("Missing required argument: output_dir")?);
    let recursive = args.get("recursive").and_then(|v| v.as_bool()).unwrap_or(false);
    // An explicit `files` list wins over `input_dir`, which is then neither required nor scanned.
    let mut files = Vec::new();
    let input_dir = match args.get("files") {
        Some(Value::Array(list)) => {
            for (i, entry) in list.iter().enumerate() {
                let path = entry.as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| format!("files[{}] must be a non-empty path string", i))?;
                if !Path::new(path).is_absolute() {
                    return Err(format!("files[{}] must be an absolute path: {}", i, path));
                }
                files.push(PathBuf::from(path));
            }
            if files.is_empty() {
                return Err("files must contain at least one path".to_string());
            }
            None
        }
        Some(Value::Null) | None => {
            let dir = PathBuf::from(arg_str(args, "input_dir").ok_or("Missing required argument: input_dir (or files)")?);
            if !dir.is_dir() {
                return Err(format!("Input directory not found: {}", dir.display()));
            }
            collect_images(&dir, recursive, &mut files)?;
            files.sort();
            Some(dir)
        }
        Some(_) => return Err("files must be an array of absolute paths".to_string()),
    };
    let sanitize = parse_sanitize(args)?;
    let encoding = parse_encoding(args);
    tokio::fs::create_dir_all(&output_dir).await.map_err(|e| format!("Cannot create output directory: {}", e))?;
    let binary = resolve_cwebp_if_needed(&sanitize, None).await.ok().flatten();

    let mut results = Vec::new();
    let mut completed = 0u32;
    let mut failed = 0u32;
    let mut used_names = std::collections::HashSet::new();
    let mut traces_removed = 0u64;
    let mut bytes_in = 0u64;
    let mut bytes_out = 0u64;
    let started = std::time::Instant::now();

    for file in &files {
        // Scanned files mirror their sub-directory under output_dir; explicit files land flat.
        let relative = match &input_dir {
            Some(dir) => file.strip_prefix(dir).unwrap_or(file),
            None => file.file_name().map(Path::new).unwrap_or(file.as_path()),
        };
        let item = async {
            if !file.is_file() {
                return Err(format!("File not found: {}", file.display()));
            }
            let fmt = sniff_format(file)?;
            let mut name = output_file_name(file, fmt, sanitize.output_format, sanitize.seo_slug);
            let target_dir = output_dir.join(relative.parent().unwrap_or(Path::new("")));
            tokio::fs::create_dir_all(&target_dir).await.map_err(|e| format!("Cannot create {}: {}", target_dir.display(), e))?;
            // An output written next to its source gets the .clean suffix (inputs are never overwritten).
            let same_dir = std::fs::canonicalize(file.parent().unwrap_or(Path::new("."))).ok() == std::fs::canonicalize(&target_dir).ok();
            if same_dir && !sanitize.seo_slug {
                let (stem, ext) = name.rsplit_once('.').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((name.clone(), String::new()));
                name = format!("{}.clean.{}", stem, ext);
            }
            if sanitize.seo_slug {
                name = unique_output_name(&target_dir, &name, &mut used_names);
            }
            let target = target_dir.join(name);
            if target == *file {
                return Err("output would overwrite the input".to_string());
            }
            let needs_cwebp = sanitize.output_format == OutputFormat::Webp
                || (sanitize.output_format == OutputFormat::Original && fmt == ImageFormat::WebP && sanitize.mode != SanitizeMode::Safe);
            if needs_cwebp && binary.is_none() {
                return Err("cwebp binary not found (required for WebP output)".to_string());
            }
            let target_str = target.to_string_lossy().to_string();
            let input_str = file.to_string_lossy().to_string();
            process_image_file(ProcessRequest {
                input_path: &input_str,
                output_path: Some(&target_str),
                encoding: &encoding,
                sanitize: &sanitize,
                cwebp_binary: binary.as_deref(),
            })
            .await
        }
        .await;
        match item {
            Ok(o) => {
                completed += 1;
                traces_removed += o.findings_before.saturating_sub(o.findings_after) as u64;
                bytes_in += o.original_size;
                bytes_out += o.file_size;
                results.push(json!({
                    "input": file.to_string_lossy(),
                    "output": o.output_path,
                    "status": "completed",
                    "findingsBefore": o.findings_before,
                    "findingsAfter": o.findings_after,
                    "clean": o.clean,
                    "originalSize": o.original_size,
                    "fileSize": o.file_size,
                    "outputFormat": o.output_format.label(),
                    "psnr": o.pipeline.as_ref().map(|p| (p.psnr * 100.0).round() / 100.0),
                    "ssim": o.pipeline.as_ref().map(|p| (p.ssim * 10000.0).round() / 10000.0)
                }));
            }
            Err(e) => {
                failed += 1;
                results.push(json!({ "input": file.to_string_lossy(), "status": "failed", "error": e }));
            }
        }
    }

    Ok(json!({
        "inputDir": input_dir.as_ref().map(|d| d.to_string_lossy().to_string()),
        "outputDir": output_dir.to_string_lossy(),
        "mode": sanitize.mode.label(),
        "outputFormat": format!("{:?}", sanitize.output_format).to_lowercase(),
        "totalFiles": files.len(),
        "completed": completed,
        "failed": failed,
        "tracesRemoved": traces_removed,
        "bytesIn": bytes_in,
        "bytesOut": bytes_out,
        "reductionPercent": if bytes_in > 0 { ((1.0 - bytes_out as f64 / bytes_in as f64) * 1000.0).round() / 10.0 } else { 0.0 },
        "durationMs": started.elapsed().as_millis() as u64,
        "results": results
    }))
}

// ---------------------------------------------------------------------------
// Documents
// ---------------------------------------------------------------------------

fn parse_doc_options(args: &Value) -> Result<DocCleanOptions, String> {
    let mode_str = arg_str(args, "mode").ok_or("Missing required argument: mode")?;
    let mode = DocMode::parse(mode_str).ok_or_else(|| format!("Unknown mode '{}' (expected safe | strict)", mode_str))?;
    let stamp = BrandStamp {
        artist: arg_str(args, "brand_artist").unwrap_or("").to_string(),
        copyright: arg_str(args, "brand_copyright").unwrap_or("").to_string(),
    };
    Ok(DocCleanOptions {
        mode,
        remove_notes: args.get("remove_notes").and_then(|v| v.as_bool()).unwrap_or(false),
        brand_stamp: if stamp.is_empty() { None } else { Some(stamp) },
    })
}

fn finding_rows(report: &DocInspectReport, removable_only: bool, limit: usize) -> Vec<Value> {
    report
        .findings
        .iter()
        .filter(|f| !removable_only || f.removable)
        .take(limit)
        .map(|f| json!({ "name": f.name, "category": f.category, "severity": f.severity, "scope": f.scope, "location": f.location, "value": f.value_preview }))
        .collect()
}

fn doc_outcome_json(o: &DocProcessOutcome, opts: &DocCleanOptions) -> Value {
    let applied = |s: Scope| match s {
        Scope::Safe => true,
        Scope::Strict => opts.mode == DocMode::Strict,
        Scope::Notes => opts.mode == DocMode::Strict && opts.remove_notes,
        Scope::Report => false,
    };
    json!({
        "inputPath": o.input_path,
        "outputPath": o.output_path,
        "format": o.format,
        "mode": o.mode,
        "originalSize": o.original_size,
        "fileSize": o.file_size,
        "tracesBefore": o.traces_before,
        "tracesAfter": o.traces_after,
        "tracesRemoved": o.traces_removed,
        "clean": o.clean,
        "stampVerified": o.stamp_verified,
        "warnings": o.warnings,
        "durationMs": o.duration_ms,
        "removed": o.before.findings.iter().filter(|f| f.removable && applied(f.scope)).map(|f| json!({ "name": f.name, "severity": f.severity, "location": f.location })).collect::<Vec<_>>(),
        "remaining": finding_rows(&o.after, true, 50),
        "reported": o.after.findings.iter().filter(|f| f.scope == Scope::Report && f.category != "brand-stamp").map(|f| json!({ "name": f.name, "severity": f.severity })).collect::<Vec<_>>()
    })
}

async fn tool_inspect_document(args: &Value) -> Result<Value, String> {
    let path = arg_str(args, "path").ok_or("Missing required argument: path")?.to_string();
    if !Path::new(&path).is_file() {
        return Err(format!("File not found: {}", path));
    }
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(100).max(1) as usize;
    let p = path.clone();
    let report = tokio::task::spawn_blocking(move || {
        let data = std::fs::read(&p).map_err(|e| format!("File read error: {}", e))?;
        document_cleaner::inspect_document_bytes(&data, None)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(json!({
        "path": path,
        "format": report.format,
        "family": report.family,
        "fileSize": report.file_size,
        "summary": report.summary,
        "clean": report.clean,
        "traceCount": report.trace_count,
        "safeCount": report.safe_count,
        "strictCount": report.strict_count,
        "notesCount": report.notes_count,
        "severity": { "critical": report.critical, "high": report.high, "medium": report.medium, "low": report.low, "info": report.info },
        "total": report.findings.len(),
        "findings": finding_rows(&report, false, limit)
    }))
}

async fn tool_clean_document(args: &Value) -> Result<Value, String> {
    let input = PathBuf::from(arg_str(args, "input_path").ok_or("Missing required argument: input_path")?);
    if !input.is_file() {
        return Err(format!("File not found: {}", input.display()));
    }
    if !document_cleaner::is_document_path(&input) {
        return Err(format!("Unsupported document type: {} (expected docx/xlsx/pptx, odt/ods/odp/odg or pdf)", input.display()));
    }
    let opts = parse_doc_options(args)?;
    let output = arg_str(args, "output_path").map(PathBuf::from).unwrap_or_else(|| document_cleaner::default_output_path(&input));
    let o2 = opts.clone();
    let outcome = tokio::task::spawn_blocking(move || document_cleaner::process_document_file(&input, &output, &o2)).await.map_err(|e| e.to_string())??;
    Ok(doc_outcome_json(&outcome, &opts))
}

fn collect_documents(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("Cannot read directory {}: {}", dir.display(), e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if recursive {
                collect_documents(&path, true, out)?;
            }
            continue;
        }
        if document_cleaner::is_document_path(&path) && !is_own_output(&path) {
            out.push(path);
        }
    }
    Ok(())
}

async fn tool_batch_documents(args: &Value) -> Result<Value, String> {
    let output_dir = PathBuf::from(arg_str(args, "output_dir").ok_or("Missing required argument: output_dir")?);
    let recursive = args.get("recursive").and_then(|v| v.as_bool()).unwrap_or(false);
    let mut files = Vec::new();
    let input_dir = match args.get("files") {
        Some(Value::Array(list)) => {
            for (i, entry) in list.iter().enumerate() {
                let path = entry.as_str().map(str::trim).filter(|s| !s.is_empty()).ok_or_else(|| format!("files[{}] must be a non-empty path string", i))?;
                if !Path::new(path).is_absolute() {
                    return Err(format!("files[{}] must be an absolute path: {}", i, path));
                }
                files.push(PathBuf::from(path));
            }
            if files.is_empty() {
                return Err("files must contain at least one path".to_string());
            }
            None
        }
        Some(Value::Null) | None => {
            let dir = PathBuf::from(arg_str(args, "input_dir").ok_or("Missing required argument: input_dir (or files)")?);
            if !dir.is_dir() {
                return Err(format!("Input directory not found: {}", dir.display()));
            }
            collect_documents(&dir, recursive, &mut files)?;
            files.sort();
            Some(dir)
        }
        Some(_) => return Err("files must be an array of absolute paths".to_string()),
    };
    let opts = parse_doc_options(args)?;
    std::fs::create_dir_all(&output_dir).map_err(|e| format!("Cannot create output directory: {}", e))?;
    let started = std::time::Instant::now();
    let (opts2, out2, dir2) = (opts.clone(), output_dir.clone(), input_dir.clone());
    let results = tokio::task::spawn_blocking(move || {
        let mut used = std::collections::HashSet::new();
        files
            .iter()
            .map(|file| {
                // Scanned files mirror their sub-folder under output_dir; explicit files land flat.
                let relative = match &dir2 {
                    Some(dir) => file.strip_prefix(dir).unwrap_or(file),
                    None => file.file_name().map(Path::new).unwrap_or(file.as_path()),
                };
                let target_dir = out2.join(relative.parent().unwrap_or(Path::new("")));
                let outcome = if !file.is_file() {
                    Err(format!("File not found: {}", file.display()))
                } else if !document_cleaner::is_document_path(file) {
                    Err(format!("Unsupported document type: {}", file.display()))
                } else {
                    std::fs::create_dir_all(&target_dir)
                        .map_err(|e| format!("Cannot create {}: {}", target_dir.display(), e))
                        .and_then(|_| document_cleaner::process_document_file(file, &document_cleaner::batch_output_path(file, &target_dir, &mut used), &opts2))
                };
                (file.clone(), outcome)
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| e.to_string())?;

    let (mut completed, mut failed, mut removed, mut remaining) = (0u32, 0u32, 0u64, 0u64);
    let rows: Vec<Value> = results
        .iter()
        .map(|(file, outcome)| match outcome {
            Ok(o) => {
                completed += 1;
                removed += o.traces_removed as u64;
                remaining += o.traces_after as u64;
                json!({ "input": file.to_string_lossy(), "output": o.output_path, "status": "completed", "format": o.format, "tracesBefore": o.traces_before, "tracesAfter": o.traces_after, "clean": o.clean, "warnings": o.warnings })
            }
            Err(e) => {
                failed += 1;
                json!({ "input": file.to_string_lossy(), "status": "failed", "error": e })
            }
        })
        .collect();
    Ok(json!({
        "inputDir": input_dir.as_ref().map(|d| d.to_string_lossy().to_string()),
        "outputDir": output_dir.to_string_lossy(),
        "mode": opts.mode.label(),
        "totalFiles": rows.len(),
        "completed": completed,
        "failed": failed,
        "tracesRemoved": removed,
        "tracesRemaining": remaining,
        "durationMs": started.elapsed().as_millis() as u64,
        "results": rows
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_outputs_are_skipped_on_rerun() {
        assert!(is_own_output(Path::new("C:/x/photo.clean.jpg")));
        assert!(is_own_output(Path::new("C:/x/Gün Batımı (1).CLEAN.webp")));
        assert!(!is_own_output(Path::new("C:/x/photo.jpg")));
        assert!(!is_own_output(Path::new("C:/x/clean-room.png")));
    }

    #[test]
    fn tool_definitions_declare_required_arguments() {
        let tools = tool_definitions();
        let names: Vec<&str> = tools.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["inspect_image", "process_image", "batch_process", "inspect_document", "clean_document", "batch_documents"]);
        for t in tools.as_array().unwrap() {
            assert!(t["inputSchema"]["required"].as_array().map(|r| !r.is_empty()).unwrap_or(false), "{}", t["name"]);
        }
    }

    #[test]
    fn tool_definitions_expose_max_width_and_files() {
        let tools = tool_definitions();
        let tools = tools.as_array().unwrap();
        let process = &tools[1]["inputSchema"];
        let batch = &tools[2]["inputSchema"];
        assert!(tools[0]["inputSchema"]["properties"].get("max_width").is_none());
        assert_eq!(process["properties"]["max_width"]["type"], "integer");
        assert_eq!(process["properties"]["max_width"]["minimum"], 1);
        assert_eq!(batch["properties"]["max_width"]["type"], "integer");
        assert_eq!(batch["properties"]["files"]["type"], "array");
        assert_eq!(batch["properties"]["files"]["items"]["type"], "string");
        let required: Vec<&str> = batch["required"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(required, ["output_dir", "mode"], "input_dir is optional once files can be given");
    }

    #[test]
    fn parse_sanitize_reads_max_width() {
        assert_eq!(parse_sanitize(&json!({"mode": "safe"})).unwrap().max_width, None);
        assert_eq!(parse_sanitize(&json!({"mode": "safe", "max_width": null})).unwrap().max_width, None);
        assert_eq!(parse_sanitize(&json!({"mode": "studio_master", "max_width": 1200})).unwrap().max_width, Some(1200));
        assert!(parse_sanitize(&json!({"mode": "safe", "max_width": 0})).is_err());
        assert!(parse_sanitize(&json!({"mode": "safe", "max_width": -5})).is_err());
        assert!(parse_sanitize(&json!({"mode": "safe", "max_width": "1200"})).is_err());
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cwebp-mcp-test-{}-{}", tag, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn process_image_max_width_downscales_but_never_upscales() {
        use crate::services::metadata_cleaner::test_support::{dirty_png, pixel_sha256};
        let dir = temp_dir("maxw");
        let src = dir.join("hero.png");
        std::fs::write(&src, dirty_png()).unwrap(); // 64×48 with 7+ metadata traces

        let small = tool_process_image(&json!({ "input_path": src.to_string_lossy(), "mode": "safe", "max_width": 32 })).await.unwrap();
        assert_eq!(small["width"], 32);
        assert_eq!(small["height"], 24);
        assert_eq!(small["clean"], true);
        assert_eq!(small["findingsAfter"], 0);
        assert!(small["pipeline"]["stages"][0].as_str().unwrap().starts_with("downscale(lanczos, 64×48 → 32×24)"), "{}", small["pipeline"]);
        assert!(small["outputPath"].as_str().unwrap().ends_with("hero.clean.png"));

        let out2 = dir.join("same.png");
        let same = tool_process_image(&json!({ "input_path": src.to_string_lossy(), "output_path": out2.to_string_lossy(), "mode": "safe", "max_width": 1000 })).await.unwrap();
        assert_eq!(same["width"], 64);
        assert_eq!(same["height"], 48);
        assert!(same["pipeline"].is_null(), "safe mode without a downscale stays on the lossless path");
        assert_eq!(pixel_sha256(&std::fs::read(&out2).unwrap()), pixel_sha256(&dirty_png()));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn batch_process_explicit_files_skip_input_dir_scan() {
        use crate::services::image::read_dimensions;
        use crate::services::metadata_cleaner::test_support::{dirty_jpeg, dirty_png};
        let dir = temp_dir("files");
        let a = dir.join("a.png");
        let b = dir.join("sub").join("b.jpg");
        let ignored = dir.join("ignored.png");
        std::fs::create_dir_all(b.parent().unwrap()).unwrap();
        std::fs::write(&a, dirty_png()).unwrap();
        std::fs::write(&b, dirty_jpeg()).unwrap();
        std::fs::write(&ignored, dirty_png()).unwrap();
        let missing = dir.join("missing.png");

        // files wins over input_dir: ignored.png is not scanned, outputs land flat in output_dir.
        let out_dir = dir.join("out");
        let report = tool_batch_process(&json!({
            "input_dir": dir.to_string_lossy(),
            "files": [a.to_string_lossy(), b.to_string_lossy(), missing.to_string_lossy()],
            "output_dir": out_dir.to_string_lossy(),
            "mode": "safe",
            "max_width": 32
        }))
        .await
        .unwrap();
        assert_eq!(report["totalFiles"], 3);
        assert_eq!(report["completed"], 2);
        assert_eq!(report["failed"], 1);
        assert!(report["inputDir"].is_null());
        assert!(out_dir.join("a.png").is_file());
        assert!(out_dir.join("b.jpg").is_file());
        assert!(!out_dir.join("ignored.png").exists());
        assert_eq!(report["results"][0]["status"], "completed");
        assert_eq!(report["results"][1]["status"], "completed");
        assert_eq!(report["results"][2]["status"], "failed");
        assert!(report["results"][2]["error"].as_str().unwrap().starts_with("File not found"));
        let (_, w, h, _) = read_dimensions(&std::fs::read(out_dir.join("a.png")).unwrap()).unwrap();
        assert_eq!((w, h), (32, 24));

        // Explicit file whose directory is also output_dir: .clean suffix, input untouched.
        let report = tool_batch_process(&json!({ "files": [a.to_string_lossy()], "output_dir": dir.to_string_lossy(), "mode": "safe" })).await.unwrap();
        assert_eq!(report["completed"], 1);
        assert!(dir.join("a.clean.png").is_file());
        assert_eq!(std::fs::read(&a).unwrap(), dirty_png());

        // Validation happens before any file is touched.
        let od = dir.to_string_lossy().to_string();
        assert!(tool_batch_process(&json!({ "files": ["relative.png"], "output_dir": od, "mode": "safe" })).await.is_err());
        assert!(tool_batch_process(&json!({ "files": [], "output_dir": od, "mode": "safe" })).await.is_err());
        assert!(tool_batch_process(&json!({ "files": "a.png", "output_dir": od, "mode": "safe" })).await.is_err());
        assert!(tool_batch_process(&json!({ "output_dir": od, "mode": "safe" })).await.is_err());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn parse_sanitize_rejects_off_and_reads_stamp() {
        assert!(parse_sanitize(&json!({"mode": "off"})).is_err());
        let cfg = parse_sanitize(&json!({"mode": "safe", "brand_artist": "Seobify", "seo_slug": true})).unwrap();
        assert_eq!(cfg.mode, SanitizeMode::Safe);
        assert!(cfg.seo_slug);
        assert_eq!(cfg.brand_stamp.unwrap().artist, "Seobify");
        assert!(parse_sanitize(&json!({"mode": "studio_master", "output_format": "bmp"})).is_err());
    }

    #[test]
    fn document_tool_schemas_use_safe_and_strict() {
        let tools = tool_definitions();
        let tools = tools.as_array().unwrap();
        for t in &tools[3..] {
            if t["name"] != "inspect_document" {
                assert_eq!(t["inputSchema"]["properties"]["mode"]["enum"], json!(["safe", "strict"]));
            }
        }
        let required: Vec<&str> = tools[5]["inputSchema"]["required"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(required, ["output_dir", "mode"]);
    }

    #[tokio::test]
    async fn document_tools_inspect_clean_and_batch() {
        use crate::services::document_cleaner::test_support::{docx, odt, pdf_with_update, pptx};
        let dir = temp_dir("docs");
        let a = dir.join("teklif.docx");
        let b = dir.join("sunum.pptx");
        std::fs::write(&a, docx()).unwrap();
        std::fs::write(&b, pptx()).unwrap();
        std::fs::write(dir.join("rapor.pdf"), pdf_with_update()).unwrap();
        std::fs::write(dir.join("not.odt"), odt()).unwrap();
        std::fs::write(dir.join("photo.png"), b"not a document").unwrap();

        let r = tool_inspect_document(&json!({ "path": a.to_string_lossy(), "limit": 3 })).await.unwrap();
        assert_eq!(r["format"], "docx");
        assert_eq!(r["findings"].as_array().unwrap().len(), 3);
        assert!(r["total"].as_u64().unwrap() > 20);
        assert!(r["strictCount"].as_u64().unwrap() > 0);

        let c = tool_clean_document(&json!({ "input_path": a.to_string_lossy(), "mode": "safe" })).await.unwrap();
        assert!(c["outputPath"].as_str().unwrap().ends_with("teklif.clean.docx"));
        assert_eq!(c["clean"], false, "comments stay in safe mode");
        assert!(c["remaining"].as_array().unwrap().iter().all(|f| f["scope"] == "strict"));
        assert!(c["reported"].as_array().unwrap().iter().any(|f| f["name"] == "Embedded object: oleObject1.bin"));

        let s = tool_clean_document(&json!({ "input_path": b.to_string_lossy(), "output_path": dir.join("out").join("s.pptx").to_string_lossy(), "mode": "strict", "remove_notes": true, "brand_artist": "Seobify" })).await.unwrap();
        assert_eq!(s["clean"], true);
        assert_eq!(s["tracesAfter"], 0);
        assert_eq!(s["stampVerified"], "Seobify");

        assert!(tool_clean_document(&json!({ "input_path": a.to_string_lossy(), "mode": "studio_master" })).await.unwrap_err().contains("safe | strict"));
        assert!(tool_clean_document(&json!({ "input_path": dir.join("photo.png").to_string_lossy(), "mode": "safe" })).await.is_err());

        // Folder scan: own .clean outputs and non-documents are skipped.
        let out = dir.join("batch");
        let batch = tool_batch_documents(&json!({ "input_dir": dir.to_string_lossy(), "output_dir": out.to_string_lossy(), "mode": "strict", "remove_notes": true })).await.unwrap();
        assert_eq!(batch["totalFiles"], 4, "{}", batch);
        assert_eq!(batch["completed"], 4);
        assert_eq!(batch["tracesRemaining"], 0);
        for name in ["teklif.docx", "sunum.pptx", "rapor.pdf", "not.odt"] {
            assert!(out.join(name).is_file(), "{}", name);
        }
        let listed = tool_batch_documents(&json!({ "files": [a.to_string_lossy(), dir.join("missing.docx").to_string_lossy()], "output_dir": dir.to_string_lossy(), "mode": "safe" })).await.unwrap();
        assert_eq!((listed["completed"].as_u64(), listed["failed"].as_u64()), (Some(1), Some(1)));
        assert!(dir.join("teklif.clean.docx").is_file());
        assert_eq!(std::fs::read(&a).unwrap(), docx(), "input untouched");
        let _ = std::fs::remove_dir_all(dir);
    }
}
