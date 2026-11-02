//! ZIP container access shared by OOXML and ODF, plus OPC part-name helpers.

use super::{Ctx, Scope};
use crate::services::metadata_cleaner::Severity;
use std::io::{Cursor, Read, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

/// Decompression-bomb guards.
const MAX_ENTRIES: usize = 50_000;
const MAX_TOTAL_BYTES: u64 = 2 << 30;

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
    pub is_dir: bool,
}

#[derive(Debug, Default)]
pub struct ZipMeta {
    pub comment: Vec<u8>,
    pub dated_entries: usize,
    pub latest: Option<String>,
    pub extra_fields: usize,
}

pub fn entry_names(data: &[u8]) -> Result<Vec<String>, String> {
    let archive = ZipArchive::new(Cursor::new(data)).map_err(|e| format!("Invalid ZIP container: {}", e))?;
    Ok(archive.file_names().filter_map(|s| s.ok().map(|n| n.to_string())).collect())
}

pub fn read(data: &[u8]) -> Result<(Vec<Entry>, ZipMeta), String> {
    let mut archive = ZipArchive::new(Cursor::new(data)).map_err(|e| format!("Invalid ZIP container: {}", e))?;
    if archive.len() > MAX_ENTRIES {
        return Err(format!("ZIP container has too many entries ({})", archive.len()));
    }
    let mut meta = ZipMeta { comment: archive.comment().to_vec(), ..Default::default() };
    let mut entries = Vec::with_capacity(archive.len());
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| format!("ZIP entry {} unreadable: {}", i, e))?;
        if file.encrypted() {
            return Err("Password-protected ZIP entries are not supported".to_string());
        }
        let name = file.name().map_err(|e| format!("ZIP entry name: {}", e))?.to_string();
        if let Some(t) = file.last_modified() {
            if t != DateTime::DEFAULT {
                meta.dated_entries += 1;
                let stamp = t.to_string();
                if meta.latest.as_ref().map(|l| stamp > *l).unwrap_or(true) {
                    meta.latest = Some(stamp);
                }
            }
        }
        if file.extra_data().map(|d| !d.is_empty()).unwrap_or(false) {
            meta.extra_fields += 1;
        }
        let is_dir = file.is_dir();
        let mut buf = Vec::with_capacity(file.size().min(64 << 20) as usize);
        let limit = MAX_TOTAL_BYTES.saturating_sub(total) + 1;
        (&mut file).take(limit).read_to_end(&mut buf).map_err(|e| format!("ZIP entry {} unreadable: {}", name, e))?;
        total += buf.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err("ZIP container expands beyond 2 GB; refusing to process".to_string());
        }
        entries.push(Entry { name, data: buf, is_dir });
    }
    Ok((entries, meta))
}

/// Writes entries with normalised 1980-01-01 timestamps, no extra fields and no comment.
/// `stored_first` (ODF `mimetype`) is written first and uncompressed.
pub fn write(entries: &[Entry], first: &[&str], stored_first: Option<&str>) -> Result<Vec<u8>, String> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let deflated = SimpleFileOptions::DEFAULT.compression_method(CompressionMethod::Deflated).last_modified_time(DateTime::DEFAULT);
    let stored = SimpleFileOptions::DEFAULT.compression_method(CompressionMethod::Stored).last_modified_time(DateTime::DEFAULT);
    let mut order: Vec<&Entry> = Vec::with_capacity(entries.len());
    if let Some(name) = stored_first {
        order.extend(entries.iter().filter(|e| e.name == name));
    }
    for f in first {
        order.extend(entries.iter().filter(|e| e.name.eq_ignore_ascii_case(f) && Some(e.name.as_str()) != stored_first));
    }
    for e in entries {
        if !order.iter().any(|o| std::ptr::eq(*o, e)) {
            order.push(e);
        }
    }
    for e in order {
        if e.is_dir {
            zip.add_directory(e.name.trim_end_matches('/'), deflated).map_err(|err| format!("ZIP write error: {}", err))?;
            continue;
        }
        let opts = if Some(e.name.as_str()) == stored_first { stored } else { deflated.large_file(e.data.len() as u64 >= u32::MAX as u64) };
        zip.start_file(e.name.as_str(), opts).map_err(|err| format!("ZIP write error: {}", err))?;
        zip.write_all(&e.data).map_err(|err| format!("ZIP write error: {}", err))?;
    }
    Ok(zip.finish().map_err(|err| format!("ZIP write error: {}", err))?.into_inner())
}

/// ZIP-level traces (entry timestamps, extra fields, archive comment). Any clean rewrites the
/// container, so these are always removed.
pub fn report_zip_meta(meta: &ZipMeta, ctx: &mut Ctx) {
    if meta.dated_entries > 0 {
        ctx.add(
            Scope::Safe,
            "container",
            Severity::Low,
            "ZIP central directory",
            "ZIP entry timestamps",
            format!("{} entries carry the save time of the original file; normalised to 1980-01-01", meta.dated_entries),
            0,
            meta.latest.clone().unwrap_or_default(),
        );
    }
    if meta.extra_fields > 0 {
        ctx.add(
            Scope::Safe,
            "container",
            Severity::Low,
            "ZIP local headers",
            "ZIP extra fields",
            format!("{} entries carry extra fields (NTFS/Unix timestamps or owner IDs)", meta.extra_fields),
            0,
            "",
        );
    }
    if !meta.comment.is_empty() {
        ctx.add(Scope::Safe, "container", Severity::Low, "ZIP end of central directory", "ZIP archive comment", "Free-text archive comment", meta.comment.len(), String::from_utf8_lossy(&meta.comment));
    }
}

/// Directory of a part name (`word/document.xml` → `word/`).
pub fn part_dir(part: &str) -> &str {
    match part.rfind('/') {
        Some(i) => &part[..=i],
        None => "",
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| (b as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// Resolves a relationship target against its source part (`""` for the package) into a
/// normalised part name without a leading slash.
pub fn resolve_target(source_part: &str, target: &str) -> String {
    let target = percent_decode(target.split('#').next().unwrap_or(""));
    let joined = if let Some(abs) = target.strip_prefix('/') { abs.to_string() } else { format!("{}{}", part_dir(source_part), target) };
    let mut segs: Vec<&str> = Vec::new();
    for seg in joined.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            s => segs.push(s),
        }
    }
    segs.join("/")
}

/// `word/_rels/document.xml.rels` → `word/document.xml`; `_rels/.rels` → `""` (package).
pub fn rels_source(rels_name: &str) -> Option<String> {
    let (dir, file) = match rels_name.rfind('/') {
        Some(i) => (&rels_name[..=i], &rels_name[i + 1..]),
        None => ("", rels_name),
    };
    let parent = dir.strip_suffix("_rels/")?;
    let source = file.strip_suffix(".rels")?;
    Some(format!("{}{}", parent, source))
}

/// `word/document.xml` → `word/_rels/document.xml.rels`.
pub fn rels_name_for(part: &str) -> String {
    let dir = part_dir(part);
    let file = &part[dir.len()..];
    format!("{}_rels/{}.rels", dir, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opc_paths() {
        assert_eq!(resolve_target("word/document.xml", "comments.xml"), "word/comments.xml");
        assert_eq!(resolve_target("word/document.xml", "../customXml/item1.xml"), "customXml/item1.xml");
        assert_eq!(resolve_target("", "docProps/core.xml"), "docProps/core.xml");
        assert_eq!(resolve_target("xl/worksheets/sheet1.xml", "/xl/comments1.xml"), "xl/comments1.xml");
        assert_eq!(resolve_target("ppt/slides/slide1.xml", "../media/image%201.png"), "ppt/media/image 1.png");
        assert_eq!(rels_source("_rels/.rels").as_deref(), Some(""));
        assert_eq!(rels_source("word/_rels/document.xml.rels").as_deref(), Some("word/document.xml"));
        assert_eq!(rels_name_for("xl/worksheets/sheet1.xml"), "xl/worksheets/_rels/sheet1.xml.rels");
    }
}
