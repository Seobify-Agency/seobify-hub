//! OpenDocument (ODT/ODS/ODP/ODG) carriers. `mimetype` stays the first, uncompressed entry
//! and `META-INF/manifest.xml` is kept in step with removed files.

use super::ooxml::{clean_media, path_leaks, Pkg};
use super::package;
use super::xml::{Element, Node};
use super::{join_names, Ctx, Scope};
use crate::services::metadata_cleaner::Severity;
use std::collections::HashSet;

const NS_OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
const NS_META: &str = "urn:oasis:names:tc:opendocument:xmlns:meta:1.0";
const NS_DC: &str = "http://purl.org/dc/elements/1.1/";
const NS_CONFIG: &str = "urn:oasis:names:tc:opendocument:xmlns:config:1.0";
const NS_MANIFEST: &str = "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0";
const NS_OFFICEOOO: &str = "http://openoffice.org/2009/office";
const NS_XLINK: &str = "http://www.w3.org/1999/xlink";

/// Neutral values written into tracked-change records (both fields are required by ODF).
pub const NEUTRAL_AUTHOR: &str = "Author";
pub const NEUTRAL_DATE: &str = "1970-01-01T00:00:00";

fn basename(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

pub fn run(data: &[u8], ctx: &mut Ctx) -> Result<(String, String, Option<Vec<u8>>), String> {
    let (entries, meta) = package::read(data)?;
    let mimetype = entries.iter().find(|e| e.name == "mimetype").map(|e| String::from_utf8_lossy(&e.data).trim().to_string()).ok_or("Missing mimetype entry")?;
    let sub = mimetype.strip_prefix("application/vnd.oasis.opendocument.").ok_or_else(|| format!("Not an OpenDocument file (mimetype {})", mimetype))?;
    let (format, family) = match sub.trim_end_matches("-template") {
        "text" => ("odt", "word"),
        "spreadsheet" => ("ods", "spreadsheet"),
        "presentation" => ("odp", "presentation"),
        "graphics" => ("odg", "drawing"),
        other => return Err(format!("Unsupported OpenDocument type: {}", other)),
    };
    let mut pkg = Pkg::new(entries);
    let manifest = pkg.find("META-INF/manifest.xml");
    if let Some(m) = manifest {
        if pkg.doc(m).map(|d| d.root().descendants().iter().any(|e| e.is(NS_MANIFEST, "encryption-data"))).unwrap_or(false) {
            return Err("Password-protected OpenDocument files are not supported; remove the password first".to_string());
        }
    }

    let mut covered: HashSet<usize> = HashSet::new();
    covered.extend(manifest);
    package::report_zip_meta(&meta, ctx);
    for idx in pkg.live() {
        match basename(pkg.name(idx)) {
            "meta.xml" => {
                covered.insert(idx);
                meta_part(&mut pkg, idx, ctx);
            }
            "settings.xml" => {
                covered.insert(idx);
                settings_part(&mut pkg, idx, ctx);
            }
            _ => {}
        }
    }
    let mut to_remove: Vec<usize> = Vec::new();
    whole_parts(&mut pkg, ctx, &mut covered, &mut to_remove);
    for idx in pkg.live() {
        let name = pkg.name(idx).to_ascii_lowercase();
        if pkg.is_xml(idx) && !covered.contains(&idx) && !name.starts_with("meta-inf/") {
            content_part(&mut pkg, idx, ctx);
        }
    }
    path_leaks(&mut pkg, ctx, &covered);
    let live = pkg.live();
    clean_media(&mut pkg.entries, &live, &|i| covered.contains(&i), ctx);
    active_content(&pkg, ctx);

    if !ctx.cleaning() {
        return Ok((format.to_string(), family.to_string(), None));
    }
    let removed_names: Vec<String> = to_remove.iter().map(|i| pkg.name(*i).to_string()).collect();
    pkg.removed.extend(to_remove);
    let live_names: Vec<String> = pkg.live().iter().map(|i| pkg.name(*i).to_string()).collect();
    // A folder entry goes when the removals emptied it (e.g. Thumbnails/).
    let emptied = |dir: &str| removed_names.iter().any(|r| r.starts_with(dir)) && !live_names.iter().any(|l| l.starts_with(dir));
    if let Some(m) = manifest {
        if let Some(doc) = pkg.doc(m) {
            doc.root_mut().children.retain(|n| match n {
                Node::Element(e) if e.is(NS_MANIFEST, "file-entry") => {
                    let path = e.attr(NS_MANIFEST, "full-path").unwrap_or_default();
                    !removed_names.contains(&path) && !(path.ends_with('/') && path != "/" && emptied(&path))
                }
                _ => true,
            });
            pkg.dirty.insert(m);
        }
    }
    if let Some(stamp) = ctx.stamp() {
        if let Some(idx) = pkg.find("meta.xml") {
            write_stamp(&mut pkg, idx, &stamp.artist, &stamp.copyright);
        }
    }
    for d in pkg.live_dirs() {
        if emptied(&pkg.entries[d].name) {
            pkg.removed.insert(d);
        }
    }
    let entries = pkg.into_entries();
    let bytes = package::write(&entries, &["META-INF/manifest.xml"], Some("mimetype"))?;
    Ok((format.to_string(), family.to_string(), Some(bytes)))
}

fn meta_part(pkg: &mut Pkg, idx: usize, ctx: &mut Ctx) {
    let part = pkg.name(idx).to_string();
    let Some(doc) = pkg.doc(idx) else { return };
    let Some(meta) = doc.root().elements().find(|e| e.is(NS_OFFICE, "meta")) else { return };
    let mut drop: Vec<(String, String, Option<String>)> = Vec::new();
    for el in meta.elements() {
        let mut value = el.text();
        let user_name = el.attr(NS_META, "name");
        let (severity, label) = match (el.ns.as_str(), el.local.as_str()) {
            (NS_META, "initial-creator") => (Severity::High, "Initial author".to_string()),
            (NS_DC, "creator") => (Severity::High, "Author".to_string()),
            (NS_META, "printed-by") => (Severity::High, "Printed by".to_string()),
            (NS_META, "creation-date") => (Severity::Medium, "Created date".to_string()),
            (NS_DC, "date") => (Severity::Medium, "Modified date".to_string()),
            (NS_META, "print-date") => (Severity::Medium, "Last printed date".to_string()),
            (NS_META, "editing-cycles") => (Severity::Low, "Editing cycles (revision)".to_string()),
            (NS_META, "editing-duration") => (Severity::Low, "Total editing time".to_string()),
            (NS_META, "generator") => (Severity::Low, "Generator (application)".to_string()),
            (NS_META, "template") => {
                value = el.attr(NS_XLINK, "href").unwrap_or_default();
                (if super::contains_local_path(&value) { Severity::High } else { Severity::Low }, "Template".to_string())
            }
            (NS_META, "user-defined") => {
                let name = user_name.clone().unwrap_or_default();
                if (name == "Company" || name == "Copyright") && ctx.is_stamp_value(&value) {
                    ctx.stamp_finding(format!("{} › meta:user-defined {}", part, name), &format!("{} (brand stamp)", name), &value);
                    if ctx.stamp().is_some() {
                        drop.push((el.ns.clone(), el.local.clone(), user_name.clone()));
                    }
                    continue;
                }
                (Severity::Medium, format!("Custom property: {}", name))
            }
            (NS_DC, "title") => (Severity::Low, "Title".to_string()),
            (NS_DC, "subject") => (Severity::Low, "Subject".to_string()),
            (NS_DC, "description") => (Severity::Low, "Comments (description)".to_string()),
            (NS_META, "keyword") => (Severity::Low, "Keywords".to_string()),
            _ => continue,
        };
        if value.trim().is_empty() && el.local != "user-defined" {
            continue;
        }
        if ctx.add(Scope::Safe, "document-properties", severity, format!("{} › {}", part, el.name), label, "Document metadata (meta.xml)", value.len(), &value) {
            drop.push((el.ns.clone(), el.local.clone(), user_name));
        }
    }
    if drop.is_empty() {
        return;
    }
    if let Some(Node::Element(meta)) = doc.root_mut().children.iter_mut().find(|n| matches!(n, Node::Element(e) if e.is(NS_OFFICE, "meta"))) {
        meta.children.retain(|n| match n {
            Node::Element(e) => !drop.iter().any(|(ns, local, name)| e.ns == *ns && e.local == *local && (name.is_none() || e.attr(NS_META, "name") == *name)),
            _ => true,
        });
    }
    pkg.dirty.insert(idx);
}

const SETTINGS_TRACES: &[(&str, Severity, &str)] = &[
    ("PrinterName", Severity::Medium, "Printer name"),
    ("PrinterSetup", Severity::Medium, "Printer driver settings"),
    ("Rsid", Severity::Low, "Editing session ID (rsid)"),
    ("RsidRoot", Severity::Low, "Editing session root ID (rsid)"),
];

fn settings_part(pkg: &mut Pkg, idx: usize, ctx: &mut Ctx) {
    let part = pkg.name(idx).to_string();
    let Some(doc) = pkg.doc(idx) else { return };
    let mut hits = Vec::new();
    for el in doc.root().descendants() {
        if !el.is(NS_CONFIG, "config-item") {
            continue;
        }
        let name = el.attr(NS_CONFIG, "name").unwrap_or_default();
        let value = el.text();
        if let Some((_, severity, label)) = SETTINGS_TRACES.iter().find(|(n, _, _)| *n == name) {
            if value.trim().is_empty() {
                continue;
            }
            let shown = if name == "PrinterSetup" { format!("{} bytes (base64)", value.len()) } else { value.clone() };
            if ctx.add(Scope::Safe, if name.starts_with("Rsid") { "identifier" } else { "printer" }, *severity, format!("{} › {}", part, name), *label, "Application settings stored with the document", value.len(), shown) {
                hits.push(name);
            }
        }
    }
    if !hits.is_empty() {
        doc.root_mut().remove_where(&|e| e.is(NS_CONFIG, "config-item") && hits.contains(&e.attr(NS_CONFIG, "name").unwrap_or_default()));
        pkg.dirty.insert(idx);
    }
}

fn whole_parts(pkg: &mut Pkg, ctx: &mut Ctx, covered: &mut HashSet<usize>, to_remove: &mut Vec<usize>) {
    let live = pkg.live();
    let thumbs: Vec<usize> = live.iter().copied().filter(|i| pkg.name(*i).starts_with("Thumbnails/")).collect();
    for idx in thumbs {
        covered.insert(idx);
        let size = pkg.entries[idx].data.len();
        if ctx.add(Scope::Safe, "thumbnail", Severity::Medium, pkg.name(idx), "Thumbnail preview", "Image of the first page/slide as last saved (can show content that was later edited)", size, "") {
            to_remove.push(idx);
        }
    }
    let sigs: Vec<usize> = live.iter().copied().filter(|i| matches!(pkg.name(*i), "META-INF/documentsignatures.xml" | "META-INF/macrosignatures.xml")).collect();
    if !sigs.is_empty() {
        let size = sigs.iter().map(|i| pkg.entries[*i].data.len()).sum();
        let remove = ctx.add(Scope::Safe, "signature", Severity::High, "META-INF/", "Digital signature", "Signer certificate and identity; cleaning changes the file, which invalidates the signature anyway", size, format!("{} part(s)", sigs.len()));
        for idx in sigs {
            covered.insert(idx);
            if remove {
                to_remove.push(idx);
            }
        }
    }
}

fn is_annotation(e: &Element) -> bool {
    (e.ns == NS_OFFICE || e.ns == NS_OFFICEOOO) && (e.local == "annotation" || e.local == "annotation-end")
}

fn content_part(pkg: &mut Pkg, idx: usize, ctx: &mut Ctx) {
    let part = pkg.name(idx).to_string();
    let Some(doc) = pkg.doc(idx) else { return };
    let root = doc.root();
    let all: Vec<&Element> = std::iter::once(root).chain(root.descendants()).collect();
    let rsids = all.iter().map(|e| e.attrs.iter().filter(|a| a.ns == NS_OFFICEOOO && a.local.contains("rsid")).count()).sum::<usize>();
    let mut notes = 0;
    let mut note_authors = Vec::new();
    let mut changes = 0;
    let mut change_authors = Vec::new();
    for e in &all {
        if is_annotation(e) && e.local == "annotation" {
            notes += 1;
            note_authors.extend(e.elements().filter(|c| c.is(NS_DC, "creator")).map(|c| c.text()));
        }
        if e.is(NS_OFFICE, "change-info") {
            let creator = e.elements().find(|c| c.is(NS_DC, "creator")).map(|c| c.text()).unwrap_or_default();
            let date = e.elements().find(|c| c.is(NS_DC, "date")).map(|c| c.text()).unwrap_or_default();
            if creator != NEUTRAL_AUTHOR || (!date.is_empty() && date != NEUTRAL_DATE) {
                changes += 1;
                change_authors.push(creator);
            }
        }
    }
    let mut changed = false;
    if rsids > 0 && ctx.add(Scope::Safe, "identifier", Severity::Low, part.as_str(), "Revision session IDs (rsid)", format!("{} officeooo:rsid attribute(s) fingerprint editing sessions", rsids), 0, format!("{} attributes", rsids)) {
        doc.root_mut().walk_mut(&mut |e| e.attrs.retain(|a| !(a.ns == NS_OFFICEOOO && a.local.contains("rsid"))));
        changed = true;
    }
    if notes > 0
        && ctx.add(Scope::Strict, "comments", Severity::High, part.as_str(), "Comments", "Annotations with author names, initials and timestamps", 0, format!("{} item(s) — {}", notes, join_names(&note_authors)))
    {
        doc.root_mut().remove_where(&is_annotation);
        changed = true;
    }
    if changes > 0
        && ctx.add(
            Scope::Strict,
            "tracked-changes",
            Severity::High,
            part.as_str(),
            "Tracked changes",
            format!("{} change record(s) with author and timestamp; authors become \"{}\" and dates are reset (the changes themselves stay)", changes, NEUTRAL_AUTHOR),
            0,
            join_names(&change_authors),
        )
    {
        doc.root_mut().walk_mut(&mut |e| {
            if e.is(NS_OFFICE, "change-info") {
                for c in e.children.iter_mut() {
                    if let Node::Element(c) = c {
                        if c.is(NS_DC, "creator") {
                            c.set_text(NEUTRAL_AUTHOR);
                        } else if c.is(NS_DC, "date") {
                            c.set_text(NEUTRAL_DATE);
                        }
                    }
                }
            }
        });
        changed = true;
    }
    if changed {
        pkg.dirty.insert(idx);
    }
}

fn active_content(pkg: &Pkg, ctx: &mut Ctx) {
    let macros: Vec<usize> = pkg.live().into_iter().filter(|i| pkg.name(*i).starts_with("Basic/") || pkg.name(*i).starts_with("Scripts/")).collect();
    if !macros.is_empty() {
        let size = macros.iter().map(|i| pkg.entries[*i].data.len()).sum();
        ctx.add(Scope::Report, "active-content", Severity::High, "Basic/", "Macro library", "Macros are reported but never modified; review them in LibreOffice", size, format!("{} file(s)", macros.len()));
    }
    for idx in pkg.live() {
        let name = pkg.name(idx);
        // Binary OLE objects are stored as top-level "Object N" files (ODF sub-documents are folders).
        if name.starts_with("Object ") && !name.contains('/') {
            ctx.add(Scope::Report, "embedded-object", Severity::Medium, name, format!("Embedded object: {}", name), "Embedded files keep their own metadata; extract and clean them separately", pkg.entries[idx].data.len(), name);
        }
    }
}

fn write_stamp(pkg: &mut Pkg, idx: usize, artist: &str, copyright: &str) {
    let Some(doc) = pkg.doc(idx) else { return };
    let prefix = doc.root().attrs.iter().find(|a| a.ns == super::xml::NS_XMLNS && super::xml::unescape(&a.value) == NS_META).map(|a| a.local.clone()).unwrap_or_else(|| "meta".to_string());
    let Some(Node::Element(meta)) = doc.root_mut().children.iter_mut().find(|n| matches!(n, Node::Element(e) if e.is(NS_OFFICE, "meta"))) else { return };
    for (name, value) in [("Company", artist.trim()), ("Copyright", copyright.trim())] {
        if value.is_empty() {
            continue;
        }
        let mut el = Element::new(&prefix, "user-defined", NS_META);
        el.push_attr(&format!("{}:name", prefix), NS_META, name);
        el.push_attr(&format!("{}:value-type", prefix), NS_META, "string");
        el.set_text(value);
        meta.children.push(Node::Element(el));
    }
    pkg.dirty.insert(idx);
}
