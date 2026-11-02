//! Streaming import: files/folders (`.gz` decompressed on the fly) → `bot_hits` (bots only),
//! `human_daily` (IP-free day × status class × device counters) and `url_daily` aggregates.
//!
//! One transaction per file. Bot hits whose identity needs a reverse-DNS check are parked in
//! an in-memory TEMP table (raw IPs never reach the database file) and written once the
//! lookups finish, with the final verification state and the anonymised address.

use super::bots::{self, BotDef, Family};
use super::ipnet::{hash_ip, mask_ip, redact_ips};
use super::parse::{self, Format, Line, Parser};
use super::verify::{self, Verdict, Verifier};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub format: Option<Format>,
    /// Re-import files that were imported before (replaces their rows).
    pub force: bool,
    /// Store salted hashes instead of masked addresses for fake/unverifiable bots.
    pub hash_ips: bool,
    /// Keep raw unparseable lines (off by default: samples are IP-redacted).
    pub store_raw_errors: bool,
    /// Reverse-DNS verification for addresses outside the published ranges.
    pub verify_dns: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions { format: None, force: false, hash_ips: false, store_raw_errors: false, verify_dns: true }
    }
}

impl ImportOptions {
    /// Reads the GDPR / verification switches from the tool settings JSON.
    pub fn from_settings(settings: &Value) -> Self {
        ImportOptions {
            hash_ips: settings.get("ipMode").and_then(Value::as_str) == Some("hash"),
            store_raw_errors: settings.get("storeRawErrors").and_then(Value::as_bool).unwrap_or(false),
            verify_dns: settings.get("verifyDns").and_then(Value::as_bool).unwrap_or(true),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub file: String,
    pub file_index: usize,
    pub file_count: usize,
    /// `parse` | `verify` | `finalize` | `done`
    pub phase: &'static str,
    pub lines: u64,
    pub parsed: u64,
    pub failed: u64,
    pub bot_hits: u64,
    pub bytes_read: u64,
    pub bytes_total: u64,
    pub lines_per_sec: f64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileResult {
    pub path: String,
    pub name: String,
    /// `imported` | `duplicate` | `skipped` | `error`
    pub status: String,
    pub file_id: Option<i64>,
    pub format: Option<String>,
    pub lines: u64,
    pub parsed: u64,
    pub failed: u64,
    pub bot_hits: u64,
    pub human_requests: u64,
    pub dns_checked: u64,
    pub duration_ms: u64,
    /// Time spent per phase: parse+insert, reverse DNS, aggregates+commit.
    pub parse_ms: u64,
    pub dns_ms: u64,
    pub finalize_ms: u64,
    pub first_ts: Option<String>,
    pub last_ts: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub files: Vec<FileResult>,
    pub imported: usize,
    pub duplicates: usize,
    pub skipped: usize,
    pub errors: usize,
    pub lines: u64,
    pub parsed: u64,
    pub failed: u64,
    pub bot_hits: u64,
    pub human_requests: u64,
    pub duration_ms: u64,
    pub lines_per_sec: f64,
}

const SKIP_EXT: &[&str] = &["zip", "7z", "rar", "exe", "dll", "png", "jpg", "jpeg", "gif", "webp", "pdf", "db", "sqlite", "seobify", "xlsx", "docx"];

/// Expands files and folders (recursively) into a sorted, de-duplicated file list.
pub fn expand_paths(paths: &[String]) -> Result<Vec<PathBuf>, String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if p.is_dir() {
                walk(&p, out)?;
            } else if !SKIP_EXT.iter().any(|e| name.to_ascii_lowercase().ends_with(&format!(".{e}"))) {
                out.push(p);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    for raw in paths {
        let p = PathBuf::from(raw.trim());
        if p.as_os_str().is_empty() {
            continue;
        }
        if p.is_dir() {
            walk(&p, &mut out)?;
        } else if p.is_file() {
            out.push(p);
        } else {
            return Err(format!("Path not found: {}", p.display()));
        }
    }
    out.sort();
    out.dedup();
    if out.is_empty() {
        return Err("No log files found in the given paths".into());
    }
    Ok(out)
}

/// Content fingerprint: size + first and last 64 KiB (works for `.gz` without decompressing;
/// a renamed or moved copy of an imported file is still recognised).
pub fn fingerprint(path: &Path) -> Result<(String, u64), String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let size = f.metadata().map_err(|e| e.to_string())?.len();
    let chunk = 64 * 1024u64;
    let mut h = Sha256::new();
    h.update(size.to_le_bytes());
    let mut head = vec![0u8; size.min(chunk) as usize];
    f.read_exact(&mut head).map_err(|e| e.to_string())?;
    h.update(&head);
    if size > chunk {
        let start = size.saturating_sub(chunk).max(chunk);
        f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
        let mut tail = Vec::new();
        f.read_to_end(&mut tail).map_err(|e| e.to_string())?;
        h.update(&tail);
    }
    Ok((h.finalize().iter().map(|b| format!("{b:02x}")).collect(), size))
}

struct Counting<R> {
    inner: R,
    count: Arc<AtomicU64>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Opens a log file; gzip (by magic bytes) is decompressed as a stream.
pub fn open_reader(path: &Path) -> Result<(Box<dyn BufRead>, Arc<AtomicU64>, u64), String> {
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut magic = [0u8; 2];
    let gz = f.read(&mut magic).map(|n| n == 2 && magic == [0x1f, 0x8b]).unwrap_or(false);
    f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let count = Arc::new(AtomicU64::new(0));
    let counted = Counting { inner: f, count: Arc::clone(&count) };
    let reader: Box<dyn BufRead> = if gz {
        Box::new(BufReader::with_capacity(256 * 1024, flate2::read::MultiGzDecoder::new(BufReader::with_capacity(128 * 1024, counted))))
    } else {
        Box::new(BufReader::with_capacity(256 * 1024, counted))
    };
    Ok((reader, count, size))
}

/// Imports files and folders into a project.
pub fn import_paths(conn: &mut Connection, project_id: i64, paths: &[String], opts: &ImportOptions, verifier: &mut Verifier, progress: &mut dyn FnMut(&Progress)) -> Result<ImportSummary, String> {
    let started = Instant::now();
    let files = expand_paths(paths)?;
    conn.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA cache_size=-65536;").map_err(|e| e.to_string())?;
    let mut summary = ImportSummary::default();
    let count = files.len();
    for (i, path) in files.iter().enumerate() {
        let r = match import_file(conn, project_id, path, i, count, opts, verifier, progress) {
            Ok(r) => r,
            Err(e) => FileResult {
                path: path.display().to_string(),
                name: file_name(path),
                status: "error".into(),
                message: Some(e),
                ..Default::default()
            },
        };
        match r.status.as_str() {
            "imported" => summary.imported += 1,
            "duplicate" => summary.duplicates += 1,
            "skipped" => summary.skipped += 1,
            _ => summary.errors += 1,
        }
        summary.lines += r.lines;
        summary.parsed += r.parsed;
        summary.failed += r.failed;
        summary.bot_hits += r.bot_hits;
        summary.human_requests += r.human_requests;
        summary.files.push(r);
    }
    summary.duration_ms = started.elapsed().as_millis() as u64;
    summary.lines_per_sec = summary.lines as f64 / started.elapsed().as_secs_f64().max(0.001);
    Ok(summary)
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.display().to_string())
}

const BOT_COLS: &str = "project_id, file_id, ts, day, bot_id, category, verification, ip, method, host, path, query, status, bytes, response_ms, referer_host";

fn status_class(status: u16) -> u8 {
    (status / 100).clamp(1, 5) as u8
}

#[allow(clippy::too_many_arguments)]
fn import_file(conn: &mut Connection, project_id: i64, path: &Path, index: usize, count: usize, opts: &ImportOptions, verifier: &mut Verifier, progress: &mut dyn FnMut(&Progress)) -> Result<FileResult, String> {
    let started = Instant::now();
    let mut result = FileResult { path: path.display().to_string(), name: file_name(path), ..Default::default() };
    let (fp, size) = fingerprint(path)?;
    let existing: Option<i64> = conn
        .query_row("SELECT id FROM log_files WHERE project_id = ?1 AND fingerprint = ?2", params![project_id, fp], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    if let (Some(id), false) = (existing, opts.force) {
        result.status = "duplicate".into();
        result.file_id = Some(id);
        result.message = Some("This file was already imported (same size and content). Re-import with force to replace it.".into());
        return Ok(result);
    }

    let (mut reader, bytes_read, bytes_total) = open_reader(path)?;
    // Peek at the first lines for format detection.
    let mut pre: Vec<Vec<u8>> = Vec::new();
    let mut buf = Vec::with_capacity(4096);
    while pre.len() < 60 {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        pre.push(buf.clone());
    }
    let sample: Vec<String> = pre.iter().map(|b| String::from_utf8_lossy(b).into_owned()).collect();
    let sample_refs: Vec<&str> = sample.iter().map(String::as_str).collect();
    let format = match opts.format.or_else(|| parse::detect(&sample_refs)) {
        Some(f) => f,
        None => {
            result.status = "skipped".into();
            result.message = Some("Unrecognised log format (pick the format manually if this is a log file)".into());
            return Ok(result);
        }
    };
    result.format = Some(format.as_str().into());
    let mut parser = Parser::new(format);

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut rebuild_days: Option<(String, String)> = None;
    if let Some(old) = existing {
        rebuild_days = delete_file_rows(&tx, project_id, old)?;
    }
    tx.execute(
        "INSERT INTO log_files (project_id, path, name, fingerprint, size_bytes, format, imported_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![project_id, result.path, result.name, fp, size as i64, format.as_str(), crate::hub::now_iso()],
    )
    .map_err(|e| e.to_string())?;
    let file_id = tx.last_insert_rowid();
    tx.execute_batch(&format!(
        "CREATE TEMP TABLE IF NOT EXISTS pending_hits ({BOT_COLS}, family TEXT NOT NULL);
         CREATE TEMP TABLE IF NOT EXISTS ip_results (ip TEXT NOT NULL, family TEXT NOT NULL, verification TEXT NOT NULL, stored_ip TEXT, PRIMARY KEY (ip, family));
         DELETE FROM temp.pending_hits; DELETE FROM temp.ip_results;"
    ))
    .map_err(|e| e.to_string())?;

    let mut human: HashMap<(i64, u8, &'static str), u64> = HashMap::new();
    let mut pending: HashSet<(String, Family)> = HashSet::new();
    let mut samples: Vec<Value> = Vec::new();
    let mut ua_cache: HashMap<String, (Option<&'static BotDef>, &'static str)> = HashMap::new();
    let (mut min_ts, mut max_ts) = (i64::MAX, i64::MIN);
    let (mut lines, mut parsed, mut failed, mut hits, mut parked) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut cur_day_num = i64::MIN;
    let mut cur_day = String::new();
    let mut cur_ts = i64::MIN;
    let mut cur_iso = String::new();
    let mut last_emit = Instant::now();
    {
        let mut insert = tx.prepare(&format!("INSERT INTO bot_hits ({BOT_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)")).map_err(|e| e.to_string())?;
        let mut park = tx.prepare(&format!("INSERT INTO temp.pending_hits ({BOT_COLS}, family) VALUES (?1,?2,?3,?4,?5,?6,'pending',?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)")).map_err(|e| e.to_string())?;
        let mut pre_iter = pre.into_iter();
        loop {
            let line_bytes: Vec<u8>;
            let raw: &[u8] = match pre_iter.next() {
                Some(b) => {
                    line_bytes = b;
                    &line_bytes
                }
                None => {
                    buf.clear();
                    let n = reader.read_until(b'\n', &mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
                    if n == 0 {
                        break;
                    }
                    &buf
                }
            };
            let text = String::from_utf8_lossy(raw);
            let rec = match parser.parse(&text) {
                Line::Skip => continue,
                Line::Bad(reason) => {
                    lines += 1;
                    failed += 1;
                    if samples.len() < 20 {
                        let t = text.trim_end();
                        let shown: String = if opts.store_raw_errors { t.chars().take(500).collect() } else { redact_ips(&t.chars().take(300).collect::<String>()) };
                        samples.push(json!({ "line": lines, "reason": reason, "text": shown }));
                    }
                    continue;
                }
                Line::Rec(r) => r,
            };
            lines += 1;
            parsed += 1;
            min_ts = min_ts.min(rec.ts);
            max_ts = max_ts.max(rec.ts);
            let day_num = rec.ts.div_euclid(86_400);
            if day_num != cur_day_num {
                cur_day_num = day_num;
                cur_day = parse::day_string(rec.ts);
            }
            let (bot, device) = match ua_cache.get(rec.ua.as_ref()) {
                Some(v) => *v,
                None => {
                    let v = (bots::classify(&rec.ua), bots::device_class(&rec.ua));
                    if ua_cache.len() > 200_000 {
                        ua_cache.clear();
                    }
                    ua_cache.insert(rec.ua.to_string(), v);
                    v
                }
            };
            let Some(def) = bot else {
                *human.entry((day_num, status_class(rec.status), device)).or_insert(0) += rec.weight as u64;
                continue;
            };
            if rec.ts != cur_ts {
                cur_ts = rec.ts;
                cur_iso = parse::iso_string(rec.ts);
            }
            let (target_host, raw_path, query) = parse::split_target(&rec.target);
            let host = rec.host.as_deref().or(target_host).map(|h| h.to_ascii_lowercase());
            let path_decoded = if raw_path.contains('%') { parse::percent_decode(raw_path) } else { raw_path.to_string() };
            let referer = rec.referer.as_deref().and_then(parse::referer_host);
            let ip = rec.ip.trim();
            let verdict = if rec.edge_verified { Verdict::Verified { method: "cloudflare", hostname: None } } else { verifier.quick(&tx, ip, def.family) };
            if verdict == Verdict::Pending {
                pending.insert((ip.to_string(), def.family));
                for _ in 0..rec.weight {
                    park.execute(params![project_id, file_id, cur_iso, cur_day, def.id, def.category.as_str(), ip, rec.method.as_ref(), host, path_decoded, query, rec.status as i64, rec.bytes, rec.response_ms, referer, def.family.as_str()])
                        .map_err(|e| e.to_string())?;
                }
                parked += rec.weight as u64;
            } else {
                let stored = stored_ip(ip, &verdict, opts.hash_ips, &verifier.salt);
                for _ in 0..rec.weight {
                    insert
                        .execute(params![project_id, file_id, cur_iso, cur_day, def.id, def.category.as_str(), verdict.as_str(), stored, rec.method.as_ref(), host, path_decoded, query, rec.status as i64, rec.bytes, rec.response_ms, referer])
                        .map_err(|e| e.to_string())?;
                }
                hits += rec.weight as u64;
            }
            if lines % 50_000 == 0 && last_emit.elapsed().as_millis() >= 150 {
                last_emit = Instant::now();
                progress(&Progress {
                    file: result.name.clone(),
                    file_index: index,
                    file_count: count,
                    phase: "parse",
                    lines,
                    parsed,
                    failed,
                    bot_hits: hits + parked,
                    bytes_read: bytes_read.load(Ordering::Relaxed),
                    bytes_total,
                    lines_per_sec: lines as f64 / started.elapsed().as_secs_f64().max(0.001),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                });
            }
        }
    }

    result.parse_ms = started.elapsed().as_millis() as u64;
    let dns_started = Instant::now();
    // Reverse-DNS phase for addresses outside the published ranges.
    if !pending.is_empty() {
        progress(&Progress {
            file: result.name.clone(),
            file_index: index,
            file_count: count,
            phase: "verify",
            lines,
            parsed,
            failed,
            bot_hits: hits + parked,
            bytes_read: bytes_total,
            bytes_total,
            lines_per_sec: lines as f64 / started.elapsed().as_secs_f64().max(0.001),
            elapsed_ms: started.elapsed().as_millis() as u64,
        });
        let items: Vec<(String, Family)> = pending.drain().collect();
        result.dns_checked = items.len() as u64;
        let verdicts = if verify::resolver_available() {
            verify::dns_verify_batch(items)
        } else {
            items.into_iter().map(|(ip, f)| (ip, f, Verdict::Unverifiable)).collect()
        };
        {
            let mut put = tx.prepare("INSERT OR REPLACE INTO temp.ip_results (ip, family, verification, stored_ip) VALUES (?1, ?2, ?3, ?4)").map_err(|e| e.to_string())?;
            for (ip, family, verdict) in verdicts {
                verify::cache_put(&tx, &ip, &verifier.salt, family, &verdict)?;
                put.execute(params![ip, family.as_str(), verdict.as_str(), stored_ip(&ip, &verdict, opts.hash_ips, &verifier.salt)]).map_err(|e| e.to_string())?;
                verifier.remember(&ip, family, verdict);
            }
        }
        let cols_p = BOT_COLS.split(", ").map(|c| match c {
            "verification" => "r.verification".to_string(),
            "ip" => "r.stored_ip".to_string(),
            other => format!("p.{other}"),
        });
        let select = cols_p.collect::<Vec<_>>().join(", ");
        let moved = tx
            .execute(&format!("INSERT INTO bot_hits ({BOT_COLS}) SELECT {select} FROM temp.pending_hits p JOIN temp.ip_results r ON r.ip = p.ip AND r.family = p.family"), [])
            .map_err(|e| e.to_string())?;
        hits += moved as u64;
        tx.execute_batch("DELETE FROM temp.pending_hits; DELETE FROM temp.ip_results;").map_err(|e| e.to_string())?;
    }

    result.dns_ms = dns_started.elapsed().as_millis() as u64;
    let finalize_started = Instant::now();
    progress(&Progress {
        file: result.name.clone(),
        file_index: index,
        file_count: count,
        phase: "finalize",
        lines,
        parsed,
        failed,
        bot_hits: hits,
        bytes_read: bytes_total,
        bytes_total,
        lines_per_sec: lines as f64 / started.elapsed().as_secs_f64().max(0.001),
        elapsed_ms: started.elapsed().as_millis() as u64,
    });
    let human_total: u64 = human.values().sum();
    {
        let mut put = tx
            .prepare("INSERT INTO human_daily (project_id, file_id, day, status_class, device, requests) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")
            .map_err(|e| e.to_string())?;
        for ((day_num, class, device), n) in &human {
            put.execute(params![project_id, file_id, parse::day_string(day_num * 86_400), format!("{class}xx"), device, *n as i64]).map_err(|e| e.to_string())?;
        }
    }
    let (first_ts, last_ts) = if parsed > 0 { (Some(parse::iso_string(min_ts)), Some(parse::iso_string(max_ts))) } else { (None, None) };
    let duration_ms = started.elapsed().as_millis() as u64;
    tx.execute(
        "UPDATE log_files SET lines_total = ?2, lines_parsed = ?3, lines_failed = ?4, bot_hits = ?5, human_requests = ?6, first_ts = ?7, last_ts = ?8, duration_ms = ?9, sample_errors = ?10 WHERE id = ?1",
        params![file_id, lines as i64, parsed as i64, failed as i64, hits as i64, human_total as i64, first_ts, last_ts, duration_ms as i64, Value::Array(samples).to_string()],
    )
    .map_err(|e| e.to_string())?;
    if parsed > 0 {
        let (a, b) = (parse::day_string(min_ts), parse::day_string(max_ts));
        let (from, to) = match &rebuild_days {
            Some((x, y)) => (a.clone().min(x.clone()), b.clone().max(y.clone())),
            None => (a, b),
        };
        rebuild_url_daily(&tx, project_id, &from, &to)?;
    } else if let Some((x, y)) = &rebuild_days {
        rebuild_url_daily(&tx, project_id, x, y)?;
    }
    crate::hub::db::log_activity(
        &tx,
        Some(project_id),
        "import",
        &format!("Imported {} ({}): {} lines, {} bot hits, {} failed", result.name, format.as_str(), lines, hits, failed),
    )?;
    tx.commit().map_err(|e| e.to_string())?;
    result.finalize_ms = finalize_started.elapsed().as_millis() as u64;

    result.status = "imported".into();
    result.file_id = Some(file_id);
    result.lines = lines;
    result.parsed = parsed;
    result.failed = failed;
    result.bot_hits = hits;
    result.human_requests = human_total;
    result.first_ts = first_ts;
    result.last_ts = last_ts;
    result.duration_ms = started.elapsed().as_millis() as u64;
    if parsed == 0 {
        result.message = Some("No line could be parsed; check the format".into());
    }
    progress(&Progress {
        file: result.name.clone(),
        file_index: index,
        file_count: count,
        phase: "done",
        lines,
        parsed,
        failed,
        bot_hits: hits,
        bytes_read: bytes_total,
        bytes_total,
        lines_per_sec: lines as f64 / started.elapsed().as_secs_f64().max(0.001),
        elapsed_ms: result.duration_ms,
    });
    Ok(result)
}

/// The address kept for a bot hit: verified bots in full (not personal data), everything
/// else masked (default) or as a salted hash.
fn stored_ip(ip: &str, verdict: &Verdict, hash: bool, salt: &str) -> Option<String> {
    if ip.is_empty() {
        return None;
    }
    Some(match verdict {
        Verdict::Verified { .. } => ip.to_string(),
        _ if hash => hash_ip(ip, salt),
        _ => mask_ip(ip),
    })
}

/// Deletes one imported file's rows; returns its day range for aggregate rebuilds.
fn delete_file_rows(tx: &Transaction, project_id: i64, file_id: i64) -> Result<Option<(String, String)>, String> {
    let range: Option<(Option<String>, Option<String>)> = tx
        .query_row("SELECT first_ts, last_ts FROM log_files WHERE id = ?1 AND project_id = ?2", params![file_id, project_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(range) = range else { return Err(format!("Import {file_id} not found in this project")) };
    tx.execute("DELETE FROM bot_hits WHERE file_id = ?1 AND project_id = ?2", params![file_id, project_id]).map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM human_daily WHERE file_id = ?1 AND project_id = ?2", params![file_id, project_id]).map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM log_files WHERE id = ?1", [file_id]).map_err(|e| e.to_string())?;
    Ok(match range {
        (Some(a), Some(b)) => Some((a[..10].to_string(), b[..10].to_string())),
        _ => None,
    })
}

/// Deletes an import with its rows and refreshes the URL aggregates of its days.
pub fn delete_import(conn: &mut Connection, project_id: i64, file_id: i64) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if let Some((from, to)) = delete_file_rows(&tx, project_id, file_id)? {
        rebuild_url_daily(&tx, project_id, &from, &to)?;
    }
    crate::hub::db::log_activity(&tx, Some(project_id), "import", &format!("Import {file_id} deleted"))?;
    tx.commit().map_err(|e| e.to_string())
}

/// Recomputes `url_daily` (non-fake bot hits per day × URL × bot) for a day range.
pub fn rebuild_url_daily(conn: &Connection, project_id: i64, from: &str, to: &str) -> Result<(), String> {
    conn.execute("DELETE FROM url_daily WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3", params![project_id, from, to]).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO url_daily (project_id, day, url, bot_id, hits, s2xx, s3xx, s4xx, s5xx, last_ts)
         SELECT project_id, day, path || CASE WHEN query IS NULL THEN '' ELSE '?' || query END AS u, bot_id, COUNT(*),
                SUM(status BETWEEN 200 AND 299), SUM(status BETWEEN 300 AND 399), SUM(status BETWEEN 400 AND 499), SUM(status >= 500), MAX(ts)
         FROM bot_hits WHERE project_id = ?1 AND day BETWEEN ?2 AND ?3 AND verification != 'fake'
         GROUP BY day, u, bot_id",
        params![project_id, from, to],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::logs::verify::tests::test_ranges;
    use std::io::Write;

    pub const GOOGLE_M: &str = "Mozilla/5.0 (Linux; Android 6.0.1; Nexus 5X Build/MMB29P) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.6668.70 Mobile Safari/537.36 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)";
    pub const CHROME: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

    pub fn line(ip: &str, day: u32, path: &str, status: u16, ua: &str) -> String {
        format!("{ip} - - [{day:02}/Oct/2024:10:00:00 +0000] \"GET {path} HTTP/1.1\" {status} 512 \"-\" \"{ua}\"\n")
    }

    pub fn setup() -> (Connection, i64, tempfile::TempDir) {
        let conn = crate::hub::db::open_memory().unwrap();
        let p = crate::hub::db::create_project(&conn, "Test", Some("seobify.com"), None).unwrap();
        (conn, p.id, tempfile::tempdir().unwrap())
    }

    pub fn verifier() -> Verifier {
        // DNS off keeps unit tests offline: outside-range Google IPs become `fake`.
        Verifier::new(test_ranges(), "test-salt".into(), false)
    }

    #[test]
    fn imports_bots_and_counts_humans_without_ips() {
        let (mut conn, pid, dir) = setup();
        let path = dir.path().join("access.log");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(line("66.249.64.5", 1, "/", 200, GOOGLE_M).as_bytes()).unwrap();
        f.write_all(line("66.249.64.6", 1, "/blog/?utm_source=x", 301, GOOGLE_M).as_bytes()).unwrap();
        f.write_all(line("203.0.113.77", 2, "/wp-login.php", 404, GOOGLE_M).as_bytes()).unwrap();
        f.write_all(line("198.51.100.23", 2, "/", 200, CHROME).as_bytes()).unwrap();
        f.write_all(line("198.51.100.24", 2, "/iletisim/", 500, "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Mobile/15E148").as_bytes()).unwrap();
        f.write_all(b"this line is broken 198.51.100.99\n").unwrap();
        drop(f);
        let mut events = 0;
        let s = import_paths(&mut conn, pid, &[path.display().to_string()], &ImportOptions::default(), &mut verifier(), &mut |_| events += 1).unwrap();
        assert_eq!((s.imported, s.lines, s.parsed, s.failed, s.bot_hits, s.human_requests), (1, 6, 5, 1, 3, 2));
        assert!(events >= 2);
        let fake: (String, String) = conn.query_row("SELECT verification, ip FROM bot_hits WHERE path = '/wp-login.php'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(fake, ("fake".into(), "203.0.113.0".into()), "fake bot IP is masked");
        let verified: String = conn.query_row("SELECT ip FROM bot_hits WHERE path = '/'", [], |r| r.get(0)).unwrap();
        assert_eq!(verified, "66.249.64.5");
        let q: Option<String> = conn.query_row("SELECT query FROM bot_hits WHERE path = '/blog/'", [], |r| r.get(0)).unwrap();
        assert_eq!(q.as_deref(), Some("utm_source=x"));
        let human: i64 = conn.query_row("SELECT SUM(requests) FROM human_daily WHERE project_id = ?1", [pid], |r| r.get(0)).unwrap();
        assert_eq!(human, 2);
        let devices: Vec<String> = conn.prepare("SELECT device FROM human_daily ORDER BY device").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        assert_eq!(devices, vec!["desktop", "mobile"]);
        let ud: i64 = conn.query_row("SELECT SUM(hits) FROM url_daily WHERE project_id = ?1", [pid], |r| r.get(0)).unwrap();
        assert_eq!(ud, 2, "url_daily excludes fake hits");
        let samples: String = conn.query_row("SELECT sample_errors FROM log_files", [], |r| r.get(0)).unwrap();
        assert!(samples.contains("broken") && !samples.contains("198.51.100.99"), "error samples are IP-redacted by default");
    }

    /// Acceptance 5: no human IP anywhere in the database; no human rows in bot_hits.
    #[test]
    fn human_ips_never_stored() {
        let (mut conn, pid, dir) = setup();
        let path = dir.path().join("mix.log");
        let mut f = std::fs::File::create(&path).unwrap();
        let humans: Vec<String> = (1..=40).map(|i| format!("198.51.100.{i}")).collect();
        for (i, ip) in humans.iter().enumerate() {
            f.write_all(line(ip, 1 + (i % 3) as u32, &format!("/sayfa-{i}/"), if i % 7 == 0 { 404 } else { 200 }, CHROME).as_bytes()).unwrap();
        }
        f.write_all(line("66.249.64.9", 1, "/", 200, GOOGLE_M).as_bytes()).unwrap();
        f.write_all(b"garbage 198.51.100.1 [x\n").unwrap();
        drop(f);
        let opts = ImportOptions { hash_ips: true, ..Default::default() };
        import_paths(&mut conn, pid, &[path.display().to_string()], &opts, &mut verifier(), &mut |_| {}).unwrap();
        let tables: Vec<String> = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").unwrap().query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        for table in &tables {
            let cols: Vec<String> = conn.prepare(&format!("PRAGMA table_info({table})")).unwrap().query_map([], |r| r.get(1)).unwrap().map(Result::unwrap).collect();
            if table != "bot_hits" {
                assert!(!cols.iter().any(|c| c == "ip"), "{table} must not have an ip column");
            }
            let mut stmt = conn.prepare(&format!("SELECT * FROM {table}")).unwrap();
            let n = stmt.column_count();
            let mut rows = stmt.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                for i in 0..n {
                    if let Ok(Some(text)) = row.get::<_, Option<String>>(i) {
                        for ip in &humans {
                            assert!(!text.contains(&format!("{ip} ")) && text != *ip && !text.contains(&format!("\"{ip}\"")), "human IP {ip} found in {table}: {text}");
                        }
                    }
                }
            }
        }
        let human_rows: i64 = conn.query_row("SELECT COUNT(*) FROM bot_hits WHERE category = 'human' OR bot_id = 'human'", [], |r| r.get(0)).unwrap();
        assert_eq!(human_rows, 0);
    }

    #[test]
    fn duplicate_detection_force_and_delete() {
        let (mut conn, pid, dir) = setup();
        let path = dir.path().join("a.log");
        std::fs::write(&path, line("66.249.64.5", 1, "/", 200, GOOGLE_M) + &line("66.249.64.5", 2, "/x/", 200, GOOGLE_M)).unwrap();
        let p = vec![path.display().to_string()];
        let first = import_paths(&mut conn, pid, &p, &ImportOptions::default(), &mut verifier(), &mut |_| {}).unwrap();
        assert_eq!(first.imported, 1);
        // A renamed copy is still recognised.
        let copy = dir.path().join("renamed.log");
        std::fs::copy(&path, &copy).unwrap();
        let again = import_paths(&mut conn, pid, &[copy.display().to_string()], &ImportOptions::default(), &mut verifier(), &mut |_| {}).unwrap();
        assert_eq!((again.imported, again.duplicates), (0, 1));
        let forced = import_paths(&mut conn, pid, &p, &ImportOptions { force: true, ..Default::default() }, &mut verifier(), &mut |_| {}).unwrap();
        assert_eq!(forced.imported, 1);
        let (files, hits): (i64, i64) = conn.query_row("SELECT (SELECT COUNT(*) FROM log_files), (SELECT COUNT(*) FROM bot_hits)", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((files, hits), (1, 2), "force replaces instead of duplicating");
        let id: i64 = conn.query_row("SELECT id FROM log_files", [], |r| r.get(0)).unwrap();
        delete_import(&mut conn, pid, id).unwrap();
        let left: i64 = conn.query_row("SELECT (SELECT COUNT(*) FROM bot_hits) + (SELECT COUNT(*) FROM url_daily) + (SELECT COUNT(*) FROM log_files)", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn gzip_and_folder_import_and_project_delete() {
        let (mut conn, pid, dir) = setup();
        let sub = dir.path().join("logs").join("2024");
        std::fs::create_dir_all(&sub).unwrap();
        let gz = sub.join("access.log.1.gz");
        let mut enc = flate2::write::GzEncoder::new(std::fs::File::create(&gz).unwrap(), flate2::Compression::fast());
        for d in 1..=3 {
            enc.write_all(line("66.249.64.5", d, "/gz/", 200, GOOGLE_M).as_bytes()).unwrap();
        }
        enc.finish().unwrap();
        std::fs::write(sub.join("access.log"), line("66.249.64.6", 4, "/plain/", 200, GOOGLE_M)).unwrap();
        std::fs::write(sub.join("notes.png"), b"\x89PNG").unwrap();
        let s = import_paths(&mut conn, pid, &[dir.path().join("logs").display().to_string()], &ImportOptions::default(), &mut verifier(), &mut |_| {}).unwrap();
        assert_eq!((s.imported, s.bot_hits), (2, 4));
        assert!(s.files.iter().all(|f| f.format.as_deref() == Some("combined")));
        crate::hub::db::delete_project(&conn, pid).unwrap();
        let left: i64 = conn.query_row("SELECT (SELECT COUNT(*) FROM bot_hits) + (SELECT COUNT(*) FROM human_daily) + (SELECT COUNT(*) FROM url_daily) + (SELECT COUNT(*) FROM log_files)", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0, "project deletion removes every row");
    }

    #[test]
    fn unknown_files_are_skipped() {
        let (mut conn, pid, dir) = setup();
        let path = dir.path().join("readme.txt");
        std::fs::write(&path, "hello\nworld\n").unwrap();
        let s = import_paths(&mut conn, pid, &[path.display().to_string()], &ImportOptions::default(), &mut verifier(), &mut |_| {}).unwrap();
        assert_eq!(s.skipped, 1);
        assert!(expand_paths(&["Z:/definitely/missing".into()]).is_err());
    }
}
