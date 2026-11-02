//! PDF carriers. Cleaning loads the document with lopdf, edits the object graph, prunes
//! everything no longer referenced and writes a single-revision file, so earlier versions
//! kept by incremental updates (and original object streams) do not survive.

use super::{join_names, Ctx, Scope};
use crate::services::metadata_cleaner::{clean_image_bytes, inspect_image_bytes, Severity};
use lopdf::xref::XrefType;
use lopdf::{Dictionary, Document, Object, ObjectId, StringFormat};

const XMP_MARKERS: &[&str] = &["dc:creator", "xmp:CreatorTool", "pdf:Producer", "xmpMM:DocumentID", "xmpMM:InstanceID", "xmpMM:History", "photoshop:", "exif:", "pdfaid:part"];

/// Annotation subtypes whose `/T` is the author (for widgets `/T` is the form-field name).
const MARKUP_SUBTYPES: &[&[u8]] = &[
    b"Text", b"FreeText", b"Line", b"Square", b"Circle", b"Polygon", b"PolyLine", b"Highlight", b"Underline", b"Squiggly", b"StrikeOut", b"Stamp", b"Caret", b"Ink",
    b"FileAttachment", b"Sound", b"Redact", b"Popup",
];

pub fn text_string(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).to_string();
    }
    bytes.iter().map(|b| *b as char).collect()
}

fn encode_text(s: &str) -> Object {
    if s.is_ascii() {
        Object::String(s.as_bytes().to_vec(), StringFormat::Literal)
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for u in s.encode_utf16() {
            bytes.extend_from_slice(&u.to_be_bytes());
        }
        Object::String(bytes, StringFormat::Hexadecimal)
    }
}

fn object_text(o: &Object) -> String {
    match o {
        Object::String(b, _) => text_string(b),
        Object::Name(n) => String::from_utf8_lossy(n).to_string(),
        Object::Integer(i) => i.to_string(),
        Object::Real(r) => r.to_string(),
        Object::Boolean(b) => b.to_string(),
        _ => String::new(),
    }
}

fn name_is(d: &Dictionary, key: &[u8], value: &[u8]) -> bool {
    d.get(key).and_then(|o| o.as_name()).map(|n| n == value).unwrap_or(false)
}

/// Visits every dictionary in the object graph (direct objects included), parents first.
fn visit_dicts(doc: &mut Document, f: &mut dyn FnMut(ObjectId, &mut Dictionary)) {
    fn visit(owner: ObjectId, obj: &mut Object, f: &mut dyn FnMut(ObjectId, &mut Dictionary)) {
        match obj {
            Object::Dictionary(d) => {
                f(owner, d);
                for (_, v) in d.iter_mut() {
                    visit(owner, v, f);
                }
            }
            Object::Stream(s) => {
                f(owner, &mut s.dict);
                for (_, v) in s.dict.iter_mut() {
                    visit(owner, v, f);
                }
            }
            Object::Array(a) => {
                for v in a.iter_mut() {
                    visit(owner, v, f);
                }
            }
            _ => {}
        }
    }
    for (id, obj) in doc.objects.iter_mut() {
        visit(*id, obj, f);
    }
}

/// Saved revisions beyond the first (incremental updates), from the `startxref` markers.
fn earlier_revisions(data: &[u8]) -> usize {
    let markers = data.windows(9).filter(|w| *w == b"startxref").count();
    let linearized = data[..data.len().min(1024)].windows(11).any(|w| w == b"/Linearized");
    markers.saturating_sub(1 + usize::from(linearized && markers > 1))
}

pub fn run(data: &[u8], ctx: &mut Ctx) -> Result<(String, String, Option<Vec<u8>>), String> {
    let mut doc = Document::load_mem(data).map_err(|e| format!("Could not parse the PDF: {}", e))?;
    if doc.was_encrypted() || doc.trailer.has(b"Encrypt") {
        return Err("Encrypted PDFs are not supported; remove the password or encryption first".to_string());
    }
    let catalog_id = doc.trailer.get(b"Root").and_then(|o| o.as_reference()).map_err(|_| "PDF has no document catalog (/Root)".to_string())?;
    let modern = matches!(doc.reference_table.cross_reference_type, XrefType::CrossReferenceStream);

    info_dict(&mut doc, ctx);
    xmp(&mut doc, catalog_id, ctx);
    trailer_id(&mut doc, ctx);
    private_data(&mut doc, ctx);
    annotations(&mut doc, ctx);
    images(&mut doc, ctx);
    active_content(&mut doc, ctx);
    let revisions = earlier_revisions(data);
    if revisions > 0 {
        ctx.add(
            Scope::Safe,
            "revisions",
            Severity::Medium,
            "file structure",
            "Earlier saved revisions",
            format!("{} incremental update(s) appended; content and metadata of earlier versions can be recovered. Cleaning writes a single-revision file", revisions),
            0,
            format!("{} revision(s)", revisions + 1),
        );
    }

    if !ctx.cleaning() {
        return Ok(("pdf".to_string(), "pdf".to_string(), None));
    }
    if let Some(stamp) = ctx.stamp() {
        let mut info = Dictionary::new();
        if !stamp.artist.trim().is_empty() {
            info.set("Producer", encode_text(stamp.artist.trim()));
        }
        if !stamp.copyright.trim().is_empty() {
            info.set("Copyright", encode_text(stamp.copyright.trim()));
        }
        let id = doc.add_object(Object::Dictionary(info));
        doc.trailer.set("Info", Object::Reference(id));
    }
    doc.prune_objects();
    let mut out = Vec::new();
    let saved = if modern { doc.save_modern(&mut out) } else { doc.save_to(&mut out) };
    saved.map_err(|e| format!("PDF write error: {}", e))?;
    Ok(("pdf".to_string(), "pdf".to_string(), Some(out)))
}

fn info_dict(doc: &mut Document, ctx: &mut Ctx) {
    let entries: Vec<(String, String)> = match doc.trailer.get(b"Info") {
        Ok(Object::Reference(id)) => doc.get_dictionary(*id).map(|d| d.iter().map(|(k, v)| (String::from_utf8_lossy(k).to_string(), object_text(v))).collect()).unwrap_or_default(),
        Ok(Object::Dictionary(d)) => d.iter().map(|(k, v)| (String::from_utf8_lossy(k).to_string(), object_text(v))).collect(),
        _ => return,
    };
    let mut remove = false;
    for (key, value) in entries {
        if value.trim().is_empty() || key == "Trapped" {
            continue;
        }
        let location = format!("Info › /{}", key);
        if (key == "Producer" || key == "Copyright") && ctx.is_stamp_value(&value) {
            ctx.stamp_finding(location, &format!("{} (brand stamp)", key), &value);
            continue;
        }
        let (severity, label) = match key.as_str() {
            "Author" => (Severity::High, "Author".to_string()),
            "Creator" => (Severity::Low, "Creator application".to_string()),
            "Producer" => (Severity::Low, "Producer (PDF library)".to_string()),
            "Title" => (Severity::Low, "Title".to_string()),
            "Subject" => (Severity::Low, "Subject".to_string()),
            "Keywords" => (Severity::Low, "Keywords".to_string()),
            "CreationDate" => (Severity::Medium, "Created date".to_string()),
            "ModDate" => (Severity::Medium, "Modified date".to_string()),
            other => (Severity::Medium, format!("Custom property: {}", other)),
        };
        remove |= ctx.add(Scope::Safe, "document-properties", severity, location, label, "Document information dictionary", value.len(), &value);
    }
    if remove || ctx.stamp().is_some() {
        doc.trailer.remove(b"Info");
    }
}

fn stream_text(doc: &Document, id: ObjectId) -> (String, usize) {
    match doc.get_object(id).and_then(|o| o.as_stream()) {
        Ok(s) => {
            let content = s.decompressed_content().unwrap_or_else(|_| s.content.clone());
            (String::from_utf8_lossy(&content).to_string(), content.len())
        }
        Err(_) => (String::new(), 0),
    }
}

fn xmp(doc: &mut Document, catalog_id: ObjectId, ctx: &mut Ctx) {
    let mut catalog_ref: Option<ObjectId> = None;
    let mut object_refs: Vec<ObjectId> = Vec::new();
    visit_dicts(doc, &mut |owner, d| {
        if d.has(b"Metadata") && !name_is(d, b"Type", b"Metadata") {
            if let Ok(id) = d.get(b"Metadata").and_then(|o| o.as_reference()) {
                if owner == catalog_id && name_is(d, b"Type", b"Catalog") {
                    catalog_ref = Some(id);
                } else {
                    object_refs.push(id);
                }
            } else if owner != catalog_id {
                object_refs.push((0, 0));
            }
        }
    });
    let mut remove = false;
    if let Some(id) = catalog_ref {
        let (text, size) = stream_text(doc, id);
        let markers: Vec<String> = XMP_MARKERS.iter().filter(|m| text.contains(**m)).map(|m| m.to_string()).collect();
        let severity = if text.contains("dc:creator") || text.contains("xmpMM:History") { Severity::High } else { Severity::Medium };
        remove |= ctx.add(Scope::Safe, "xmp", severity, "Catalog › /Metadata", "XMP metadata packet", "Document-level XMP (authors, tool, document/instance IDs, edit history)", size, join_names(&markers));
        if text.contains("pdfaid:part") {
            ctx.add(Scope::Report, "structure", Severity::Info, "Catalog › /Metadata", "PDF/A identification", "The PDF/A claim lives in the XMP packet; a cleaned copy is no longer declared PDF/A", 0, "pdfaid");
        }
    }
    if !object_refs.is_empty() {
        let mut size = 0;
        let mut markers = Vec::new();
        for id in object_refs.iter().filter(|id| id.0 != 0) {
            let (text, n) = stream_text(doc, *id);
            size += n;
            markers.extend(XMP_MARKERS.iter().filter(|m| text.contains(**m)).map(|m| m.to_string()));
        }
        remove |= ctx.add(Scope::Safe, "xmp", Severity::Medium, "pages / images / fonts › /Metadata", format!("Object-level XMP ({} object(s))", object_refs.len()), "XMP attached to pages, images or fonts (often camera or editing-tool data)", size, join_names(&markers));
    }
    if remove {
        visit_dicts(doc, &mut |_, d| {
            if d.has(b"Metadata") && !name_is(d, b"Type", b"Metadata") {
                d.remove(b"Metadata");
            }
        });
    }
}

fn trailer_id(doc: &mut Document, ctx: &mut Ctx) {
    let Ok(Object::Array(ids)) = doc.trailer.get(b"ID") else { return };
    let preview = ids.first().and_then(|o| o.as_str().ok()).map(|b| b.iter().take(16).map(|x| format!("{:02x}", x)).collect::<String>()).unwrap_or_default();
    if ctx.add(Scope::Safe, "identifier", Severity::Low, "trailer › /ID", "Document ID", "File identifier that links copies and revisions of the same document", 0, preview) {
        doc.trailer.remove(b"ID");
    }
}

fn private_data(doc: &mut Document, ctx: &mut Ctx) {
    let (mut piece, mut modified) = (0usize, 0usize);
    visit_dicts(doc, &mut |_, d| {
        piece += usize::from(d.has(b"PieceInfo"));
        modified += usize::from(d.has(b"LastModified"));
    });
    let remove_piece = piece > 0 && ctx.add(Scope::Safe, "application-data", Severity::Medium, "/PieceInfo", "Application private data", "Editor-specific data (e.g. Illustrator/Acrobat) that can include paths and earlier content", 0, format!("{} dictionary(ies)", piece));
    let remove_mod = modified > 0 && ctx.add(Scope::Safe, "document-properties", Severity::Low, "/LastModified", "Last-modified timestamps", "Modification dates on pages or the catalog", 0, format!("{} dictionary(ies)", modified));
    if remove_piece || remove_mod {
        visit_dicts(doc, &mut |_, d| {
            if remove_piece {
                d.remove(b"PieceInfo");
            }
            if remove_mod {
                d.remove(b"LastModified");
            }
        });
    }
}

fn is_markup(d: &Dictionary) -> bool {
    d.has(b"Rect") && d.get(b"Subtype").and_then(|o| o.as_name()).map(|n| MARKUP_SUBTYPES.contains(&n)).unwrap_or(false)
}

fn annotations(doc: &mut Document, ctx: &mut Ctx) {
    let mut count = 0;
    let mut authors = Vec::new();
    visit_dicts(doc, &mut |_, d| {
        if is_markup(d) && (d.has(b"T") || d.has(b"M") || d.has(b"CreationDate")) {
            count += 1;
            if let Ok(t) = d.get(b"T") {
                authors.push(object_text(t));
            }
        }
    });
    if count == 0 {
        return;
    }
    if ctx.add(Scope::Strict, "comments", Severity::High, "page › /Annots", "Annotation authors", format!("{} comment/markup annotation(s) with author names and timestamps; the annotations themselves stay", count), 0, join_names(&authors)) {
        visit_dicts(doc, &mut |_, d| {
            if is_markup(d) {
                d.remove(b"T");
                d.remove(b"M");
                d.remove(b"CreationDate");
            }
        });
    }
}

fn images(doc: &mut Document, ctx: &mut Ctx) {
    let mut targets: Vec<(ObjectId, Vec<String>, Severity, usize)> = Vec::new();
    for (id, obj) in doc.objects.iter() {
        let Object::Stream(s) = obj else { continue };
        if !name_is(&s.dict, b"Subtype", b"Image") || s.filters().map(|f| f != [b"DCTDecode".as_slice()]).unwrap_or(true) {
            continue;
        }
        let report = inspect_image_bytes(&s.content);
        let traces: Vec<_> = report.findings.iter().filter(|f| f.removable && f.category != "brand-stamp").collect();
        if traces.is_empty() {
            continue;
        }
        let severity = traces.iter().map(|f| f.severity).max().unwrap_or(Severity::Low);
        let bytes = traces.iter().map(|f| f.size_bytes).sum();
        targets.push((*id, traces.iter().map(|f| f.name.clone()).collect(), severity, bytes));
    }
    for (id, names, severity, bytes) in targets {
        let detail = format!("{} metadata trace(s) inside an embedded JPEG (camera EXIF/GPS, XMP, IPTC, C2PA…); removed losslessly", names.len());
        if ctx.add(Scope::Safe, "embedded-image", severity, format!("object {} {} (JPEG image)", id.0, id.1), "Embedded image metadata", detail, bytes, join_names(&names)) {
            if let Ok(Object::Stream(s)) = doc.get_object_mut(id) {
                match clean_image_bytes(&s.content) {
                    Ok(c) => s.set_content(c.data),
                    Err(e) => ctx.warnings.push(format!("object {} {}: image metadata could not be removed ({})", id.0, id.1, e)),
                }
            }
        }
    }
}

fn active_content(doc: &mut Document, ctx: &mut Ctx) {
    let mut scripts = 0;
    let mut files = Vec::new();
    visit_dicts(doc, &mut |_, d| {
        if d.has(b"JS") || name_is(d, b"S", b"JavaScript") {
            scripts += 1;
        }
        if name_is(d, b"Type", b"Filespec") && d.has(b"EF") {
            let name = d.get(b"UF").or_else(|_| d.get(b"F")).map(object_text).unwrap_or_default();
            files.push(name);
        }
    });
    if scripts > 0 {
        ctx.add(Scope::Report, "active-content", Severity::High, "actions", "JavaScript", "JavaScript actions are reported but never modified", 0, format!("{} action(s)", scripts));
    }
    for name in files {
        ctx.add(Scope::Report, "embedded-object", Severity::Medium, "/EmbeddedFiles", format!("Embedded file: {}", name), "Attached files keep their own metadata; extract and clean them separately", 0, &name);
    }
}
