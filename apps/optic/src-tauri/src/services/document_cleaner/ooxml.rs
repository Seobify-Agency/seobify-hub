//! Office Open XML (DOCX/XLSX/PPTX) carriers.
//!
//! Parts are removed together with every relationship that targets them, their own
//! relationship part, their `[Content_Types].xml` override, the `r:id` references in the
//! source part (empty extension lists / alternate-content blocks are pruned with them) and
//! any part that was only reachable through them.

use super::package::{self, rels_name_for, rels_source, resolve_target, Entry};
use super::xml::{Element, Node, XmlDoc};
use super::{contains_local_path, join_names, local_path_spans, strip_local_paths, Ctx, Scope};
use crate::services::metadata_cleaner::{clean_image_bytes, inspect_image_bytes, Severity};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

const NS_CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const NS_PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_R_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
const NS_CP: &str = "http://schemas.openxmlformats.org/package/2006/metadata/core-properties";
const NS_DC: &str = "http://purl.org/dc/elements/1.1/";
const NS_DCTERMS: &str = "http://purl.org/dc/terms/";
const NS_EP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/extended-properties";
const NS_EP_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/extendedProperties";
const NS_CUSTOM: &str = "http://schemas.openxmlformats.org/officeDocument/2006/custom-properties";
const NS_VT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes";
const NS_W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const NS_W_STRICT: &str = "http://purl.oclc.org/ooxml/wordprocessingml/main";
const NS_W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const NS_W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
const NS_X15AC: &str = "http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac";
const NS_XR: &str = "http://schemas.microsoft.com/office/spreadsheetml/2014/revision";
const NS_MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";

const REL_TYPE_EXTENDED: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";
const REL_TYPE_CUSTOM: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties";
const CT_EXTENDED: &str = "application/vnd.openxmlformats-officedocument.extended-properties+xml";
const CT_CUSTOM: &str = "application/vnd.openxmlformats-officedocument.custom-properties+xml";
const FMTID_USER: &str = "{D5CDD505-2E9C-101B-9397-08002B2CF9AE}";

/// Word writes this neutral author when personal information is removed; it is not a trace.
pub const NEUTRAL_AUTHOR: &str = "Author";

/// Relationship kinds (last segment of the type URI) of comment parts removed in strict mode.
const COMMENT_KINDS: &[&str] = &["comments", "commentsExtended", "commentsIds", "commentsExtensible", "threadedComment", "commentAuthors"];
/// Author lists that also carry account IDs (`userId`/`providerId` or presence info).
const PERSON_KINDS: &[&str] = &["people", "person", "authors"];

fn is_w(ns: &str) -> bool {
    ns == NS_W || ns == NS_W_STRICT
}

fn is_r(ns: &str) -> bool {
    ns == NS_R || ns == NS_R_STRICT
}

fn is_ep(ns: &str) -> bool {
    ns == NS_EP || ns == NS_EP_STRICT
}

#[derive(Debug, Clone)]
struct Rel {
    source: String,
    id: String,
    kind: String,
    target: String,
    external: bool,
    resolved: String,
}

/// Parsed view of a ZIP package (shared with the ODF walker): XML parts are parsed lazily and
/// only re-serialised when marked dirty.
pub(super) struct Pkg {
    pub entries: Vec<Entry>,
    lc_index: HashMap<String, usize>,
    docs: HashMap<usize, Option<XmlDoc>>,
    pub dirty: HashSet<usize>,
    pub removed: HashSet<usize>,
    added: Vec<Entry>,
}

impl Pkg {
    pub fn new(entries: Vec<Entry>) -> Self {
        let lc_index = entries.iter().enumerate().map(|(i, e)| (e.name.to_ascii_lowercase(), i)).collect();
        Self { entries, lc_index, docs: HashMap::new(), dirty: HashSet::new(), removed: HashSet::new(), added: Vec::new() }
    }

    /// Live (not removed) entry by case-insensitive part name.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.lc_index.get(&name.to_ascii_lowercase()).copied().filter(|i| !self.removed.contains(i))
    }

    pub fn name(&self, idx: usize) -> &str {
        &self.entries[idx].name
    }

    pub fn live(&self) -> Vec<usize> {
        (0..self.entries.len()).filter(|i| !self.removed.contains(i) && !self.entries[*i].is_dir).collect()
    }

    pub fn live_dirs(&self) -> Vec<usize> {
        (0..self.entries.len()).filter(|i| !self.removed.contains(i) && self.entries[*i].is_dir).collect()
    }

    pub fn is_xml(&self, idx: usize) -> bool {
        let n = self.entries[idx].name.to_ascii_lowercase();
        n.ends_with(".xml") || n.ends_with(".rels")
    }

    pub fn doc(&mut self, idx: usize) -> Option<&mut XmlDoc> {
        if !self.docs.contains_key(&idx) {
            let parsed = XmlDoc::parse(&self.entries[idx].data).ok();
            self.docs.insert(idx, parsed);
        }
        self.docs.get_mut(&idx).and_then(|d| d.as_mut())
    }

    fn rels(&mut self) -> Vec<Rel> {
        let mut out = Vec::new();
        for idx in self.live() {
            let name = self.name(idx).to_string();
            if !name.to_ascii_lowercase().ends_with(".rels") {
                continue;
            }
            let Some(source) = rels_source(&name) else { continue };
            let Some(doc) = self.doc(idx) else { continue };
            for el in doc.root().elements().filter(|e| e.local == "Relationship") {
                let target = el.attr("", "Target").unwrap_or_default();
                let external = el.attr("", "TargetMode").map(|m| m.eq_ignore_ascii_case("External")).unwrap_or(false);
                let rtype = el.attr("", "Type").unwrap_or_default();
                out.push(Rel {
                    source: source.clone(),
                    id: el.attr("", "Id").unwrap_or_default(),
                    kind: rtype.rsplit('/').next().unwrap_or("").to_string(),
                    resolved: if external { String::new() } else { resolve_target(&source, &target) },
                    target,
                    external,
                });
            }
        }
        out
    }

    /// Lower-cased names of every part reachable from the package relationships.
    fn reachable(&mut self) -> HashSet<String> {
        let rels = self.rels();
        let mut by_source: HashMap<String, Vec<String>> = HashMap::new();
        for r in rels.iter().filter(|r| !r.external) {
            by_source.entry(r.source.to_ascii_lowercase()).or_default().push(r.resolved.to_ascii_lowercase());
        }
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([String::new()]);
        while let Some(part) = queue.pop_front() {
            for t in by_source.get(&part).cloned().unwrap_or_default() {
                if seen.insert(t.clone()) {
                    queue.push_back(t);
                }
            }
        }
        seen
    }

    fn targets(&self, rels: &[Rel], kinds: &[&str]) -> Vec<usize> {
        let mut out: Vec<usize> = rels.iter().filter(|r| !r.external && kinds.contains(&r.kind.as_str())).filter_map(|r| self.find(&r.resolved)).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn into_entries(mut self) -> Vec<Entry> {
        let mut out = Vec::new();
        for (i, e) in std::mem::take(&mut self.entries).into_iter().enumerate() {
            if self.removed.contains(&i) {
                continue;
            }
            let data = match self.docs.get(&i) {
                Some(Some(doc)) if self.dirty.contains(&i) => doc.to_bytes(),
                _ => e.data,
            };
            out.push(Entry { data, ..e });
        }
        out.extend(self.added);
        out
    }
}

fn severity_max(a: Severity, b: Severity) -> Severity {
    if b > a {
        b
    } else {
        a
    }
}

/// Printer name stored at the start of a Windows DEVMODE blob (UTF-16LE, 32 chars).
pub(crate) fn devmode_device_name(data: &[u8]) -> String {
    let units: Vec<u16> = data.chunks_exact(2).take(32).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|u| *u != 0).collect();
    String::from_utf16_lossy(&units)
}

pub fn run(data: &[u8], ctx: &mut Ctx) -> Result<(String, String, Option<Vec<u8>>), String> {
    let (entries, meta) = package::read(data)?;
    let mut pkg = Pkg::new(entries);
    let ct_idx = pkg.find("[Content_Types].xml").ok_or("Missing [Content_Types].xml")?;
    if pkg.doc(ct_idx).is_none() {
        return Err("[Content_Types].xml is not valid XML".to_string());
    }
    let rels = pkg.rels();
    let main = rels.iter().find(|r| r.source.is_empty() && r.kind == "officeDocument").map(|r| r.resolved.to_ascii_lowercase()).unwrap_or_default();
    let (format, family) = if main.starts_with("word/") {
        ("docx", "word")
    } else if main.starts_with("xl/") {
        ("xlsx", "spreadsheet")
    } else if main.starts_with("ppt/") {
        ("pptx", "presentation")
    } else {
        return Err("Unknown Office Open XML package (no word/, xl/ or ppt/ main part)".to_string());
    };
    let reachable_before = pkg.reachable();

    let mut s = State { to_remove: BTreeSet::new(), covered: HashSet::new() };
    package::report_zip_meta(&meta, ctx);
    core_props(&mut pkg, &rels, ctx);
    app_props(&mut pkg, &rels, ctx);
    custom_props(&mut pkg, &rels, ctx, &mut s);
    whole_parts(&mut pkg, &rels, ctx, &mut s);
    persons(&mut pkg, &rels, ctx, &mut s);
    comments(&mut pkg, &rels, ctx, &mut s);
    tracked_changes(&mut pkg, ctx, &mut s);
    word_settings(&mut pkg, &rels, ctx);
    rsids(&mut pkg, ctx, &s);
    workbook(&mut pkg, &main, ctx);
    notes(&mut pkg, &rels, ctx, &mut s);
    path_leaks(&mut pkg, ctx, &s.covered);
    embedded_images(&mut pkg, ctx, &s);
    active_content(&pkg, ctx);

    if !ctx.cleaning() {
        return Ok((format.to_string(), family.to_string(), None));
    }
    remove_parts(&mut pkg, ct_idx, s.to_remove, &reachable_before);
    if let Some(stamp) = ctx.stamp() {
        write_stamp(&mut pkg, ct_idx, &stamp.artist, &stamp.copyright);
    }
    let entries = pkg.into_entries();
    let bytes = package::write(&entries, &["[Content_Types].xml", "_rels/.rels"], None)?;
    Ok((format.to_string(), family.to_string(), Some(bytes)))
}

struct State {
    /// Parts the current run deletes.
    to_remove: BTreeSet<usize>,
    /// Parts already reported as a whole (skipped by the generic scans).
    covered: HashSet<usize>,
}

fn part_of_kind(pkg: &Pkg, rels: &[Rel], kind: &str, fallback: &str) -> Option<usize> {
    rels.iter().filter(|r| r.source.is_empty() && r.kind == kind).find_map(|r| pkg.find(&r.resolved)).or_else(|| pkg.find(fallback))
}

// ---------------------------------------------------------------------------
// Document properties
// ---------------------------------------------------------------------------

fn core_props(pkg: &mut Pkg, rels: &[Rel], ctx: &mut Ctx) {
    let Some(idx) = part_of_kind(pkg, rels, "core-properties", "docProps/core.xml") else { return };
    let part = pkg.name(idx).to_string();
    let Some(doc) = pkg.doc(idx) else { return };
    let mut remove = false;
    for el in doc.root().elements() {
        let value = el.text();
        if value.trim().is_empty() {
            continue;
        }
        let (severity, label) = match (el.ns.as_str(), el.local.as_str()) {
            (NS_DC, "creator") => (Severity::High, "Author"),
            (NS_CP, "lastModifiedBy") => (Severity::High, "Last modified by"),
            (NS_DCTERMS, "created") => (Severity::Medium, "Created date"),
            (NS_DCTERMS, "modified") => (Severity::Medium, "Modified date"),
            (NS_CP, "lastPrinted") => (Severity::Medium, "Last printed date"),
            (NS_CP, "revision") => (Severity::Low, "Revision number"),
            (NS_DC, "title") => (Severity::Low, "Title"),
            (NS_DC, "subject") => (Severity::Low, "Subject"),
            (NS_DC, "description") => (Severity::Low, "Comments (description)"),
            (NS_CP, "keywords") => (Severity::Low, "Keywords"),
            (NS_CP, "category") => (Severity::Low, "Category"),
            (NS_CP, "contentStatus") => (Severity::Low, "Content status"),
            _ => (Severity::Low, el.local.as_str()),
        };
        remove |= ctx.add(Scope::Safe, "document-properties", severity, format!("{} › {}", part, el.name), label, "Core document property", value.len(), &value);
    }
    if remove {
        doc.root_mut().children.clear();
        pkg.dirty.insert(idx);
    }
}

const APP_FIELDS: &[&str] = &["Company", "Manager", "Application", "AppVersion", "TotalTime", "Template", "HyperlinkBase"];

fn app_props(pkg: &mut Pkg, rels: &[Rel], ctx: &mut Ctx) {
    let Some(idx) = part_of_kind(pkg, rels, "extended-properties", "docProps/app.xml") else { return };
    let part = pkg.name(idx).to_string();
    let Some(doc) = pkg.doc(idx) else { return };
    let mut remove = false;
    for el in doc.root().elements().filter(|e| is_ep(&e.ns)) {
        let value = el.text();
        let v = value.trim();
        if v.is_empty() {
            continue;
        }
        let location = format!("{} › {}", part, el.local);
        let path = contains_local_path(v);
        let (severity, label) = match el.local.as_str() {
            "Company" if ctx.is_stamp_value(v) => {
                ctx.stamp_finding(location, "Company (brand stamp)", v);
                continue;
            }
            "Company" => (Severity::Medium, "Company"),
            "Manager" => (Severity::High, "Manager"),
            "Application" => (Severity::Low, "Application"),
            "AppVersion" => (Severity::Low, "Application version"),
            "TotalTime" if v == "0" => continue,
            "TotalTime" => (Severity::Low, "Total editing time (minutes)"),
            "Template" => (if path { Severity::High } else { Severity::Low }, "Template"),
            "HyperlinkBase" => (if path { Severity::High } else { Severity::Medium }, "Hyperlink base"),
            _ => continue,
        };
        remove |= ctx.add(Scope::Safe, "document-properties", severity, location, label, "Extended (application) property", value.len(), v);
    }
    if remove || ctx.stamp().is_some() {
        doc.root_mut().children.retain(|n| !matches!(n, Node::Element(e) if is_ep(&e.ns) && APP_FIELDS.contains(&e.local.as_str())));
        pkg.dirty.insert(idx);
    }
}

fn custom_props(pkg: &mut Pkg, rels: &[Rel], ctx: &mut Ctx, s: &mut State) {
    let Some(idx) = part_of_kind(pkg, rels, "custom-properties", "docProps/custom.xml") else { return };
    let part = pkg.name(idx).to_string();
    let Some(doc) = pkg.doc(idx) else { return };
    let props: Vec<(String, String)> = doc.root().elements().filter(|e| e.local == "property").map(|e| (e.attr("", "name").unwrap_or_default(), e.text())).collect();
    s.covered.insert(idx);
    let mut remove = false;
    for (name, value) in &props {
        let location = format!("{} › {}", part, name);
        if name == "Copyright" && ctx.is_stamp_value(value) {
            ctx.stamp_finding(location, "Copyright (brand stamp)", value);
            continue;
        }
        let severity = if contains_local_path(value) { Severity::High } else { Severity::Medium };
        remove |= ctx.add(Scope::Safe, "custom-properties", severity, location, format!("Custom property: {}", name), "User or system-defined custom document property", value.len(), value);
    }
    if remove || (ctx.cleaning() && ctx.stamp().is_some()) {
        s.to_remove.insert(idx);
    }
}

// ---------------------------------------------------------------------------
// Whole metadata parts (thumbnail, signatures, custom XML, printer settings, revisionInfo)
// ---------------------------------------------------------------------------

fn whole_parts(pkg: &mut Pkg, rels: &[Rel], ctx: &mut Ctx, s: &mut State) {
    let live = pkg.live();
    // Thumbnail
    let mut thumbs = pkg.targets(rels, &["thumbnail"]);
    thumbs.extend(live.iter().copied().filter(|i| pkg.name(*i).to_ascii_lowercase().starts_with("docprops/thumbnail.")));
    thumbs.sort_unstable();
    thumbs.dedup();
    for idx in thumbs {
        s.covered.insert(idx);
        let size = pkg.entries[idx].data.len();
        if ctx.add(Scope::Safe, "thumbnail", Severity::Medium, pkg.name(idx), "Thumbnail preview", "Image of the first page/slide as last saved (can show content that was later edited)", size, "") {
            s.to_remove.insert(idx);
        }
    }

    // Digital signatures: any change invalidates them, and they carry the signer identity.
    let sigs: Vec<usize> = live.iter().copied().filter(|i| pkg.name(*i).to_ascii_lowercase().starts_with("_xmlsignatures/")).collect();
    if !sigs.is_empty() {
        let size = sigs.iter().map(|i| pkg.entries[*i].data.len()).sum();
        let remove = ctx.add(Scope::Safe, "signature", Severity::High, "_xmlsignatures/", "Digital signature", "Signer certificate and identity; cleaning changes the file, which invalidates the signature anyway", size, format!("{} part(s)", sigs.len()));
        for idx in sigs {
            s.covered.insert(idx);
            if remove {
                s.to_remove.insert(idx);
            }
        }
    }

    // Custom XML data stores (SharePoint properties, cover-page fields, bibliography, bindings).
    let custom: Vec<usize> = live.iter().copied().filter(|i| pkg.name(*i).to_ascii_lowercase().starts_with("customxml/") && !pkg.name(*i).to_ascii_lowercase().ends_with(".rels")).collect();
    if !custom.is_empty() {
        let mut roots = Vec::new();
        for idx in &custom {
            if pkg.name(*idx).to_ascii_lowercase().contains("itemprops") {
                continue;
            }
            if let Some(doc) = pkg.doc(*idx) {
                roots.push(doc.root().local.clone());
            }
        }
        let size = custom.iter().map(|i| pkg.entries[*i].data.len()).sum();
        let remove = ctx.add(Scope::Safe, "custom-xml", Severity::Medium, "customXml/", "Custom XML data", "Data stores bound to the document (SharePoint/library properties, cover-page fields, bibliography sources)", size, join_names(&roots));
        for idx in custom {
            s.covered.insert(idx);
            if remove {
                s.to_remove.insert(idx);
            }
        }
    }

    // Printer settings (DEVMODE blobs with the printer/share name).
    let mut printers = pkg.targets(rels, &["printerSettings"]);
    printers.extend(live.iter().copied().filter(|i| pkg.name(*i).to_ascii_lowercase().contains("/printersettings/")));
    printers.sort_unstable();
    printers.dedup();
    for idx in printers {
        s.covered.insert(idx);
        let device = devmode_device_name(&pkg.entries[idx].data);
        let size = pkg.entries[idx].data.len();
        if ctx.add(Scope::Safe, "printer", Severity::Medium, pkg.name(idx), "Printer settings", "Windows DEVMODE with the printer (or network share) name and driver settings", size, device) {
            s.to_remove.insert(idx);
        }
    }

    // PowerPoint revision info: per-installation client GUIDs.
    for idx in pkg.targets(rels, &["revisionInfo"]) {
        s.covered.insert(idx);
        let size = pkg.entries[idx].data.len();
        if ctx.add(Scope::Safe, "revision", Severity::Low, pkg.name(idx), "Editing client IDs", "PowerPoint revision info with per-installation client GUIDs", size, "") {
            s.to_remove.insert(idx);
        }
    }
}

// ---------------------------------------------------------------------------
// Comments, authors and tracked changes
// ---------------------------------------------------------------------------

fn persons(pkg: &mut Pkg, rels: &[Rel], ctx: &mut Ctx, s: &mut State) {
    for idx in pkg.targets(rels, PERSON_KINDS) {
        s.covered.insert(idx);
        let part = pkg.name(idx).to_string();
        let size = pkg.entries[idx].data.len();
        let Some(doc) = pkg.doc(idx) else { continue };
        let mut names = Vec::new();
        let mut accounts = Vec::new();
        for el in doc.root().descendants() {
            match el.local.as_str() {
                // Word people.xml: <w15:person w15:author="…"><w15:presenceInfo w15:providerId="AD" w15:userId="S::mail@…"/>
                "person" if el.ns == NS_W15 => names.push(el.attr(NS_W15, "author").unwrap_or_default()),
                "presenceInfo" => {
                    let get = |local: &str| el.attrs.iter().find(|a| a.local == local).map(|a| super::xml::unescape(&a.value)).unwrap_or_default();
                    if get("providerId") != "None" {
                        accounts.push(get("userId"));
                    }
                }
                // Excel persons / PowerPoint authors: displayName|name + userId + providerId.
                "person" | "author" => {
                    names.push(el.attr("", "displayName").or_else(|| el.attr("", "name")).unwrap_or_default());
                    let user = el.attr("", "userId").unwrap_or_default();
                    let provider = el.attr("", "providerId").unwrap_or_default();
                    if !user.is_empty() && provider != "None" {
                        accounts.push(user);
                    }
                }
                _ => {}
            }
        }
        if !accounts.is_empty() {
            let detail = "Sign-in account IDs (e-mail / directory IDs) of comment and revision authors";
            if ctx.add(Scope::Safe, "author", Severity::High, part.as_str(), "Author account IDs", detail, size, join_names(&accounts)) {
                doc.root_mut().remove_where(&|e| e.local == "presenceInfo");
                doc.root_mut().walk_mut(&mut |e| {
                    if (e.local == "person" || e.local == "author") && e.ns != NS_W15 && e.attr("", "userId").is_some() {
                        e.set_attr("", "userId", "");
                        e.set_attr("", "providerId", "None");
                    }
                });
                pkg.dirty.insert(idx);
            }
        }
        if !names.iter().all(|n| n.trim().is_empty()) && ctx.add(Scope::Strict, "author", Severity::Medium, part.as_str(), "Comment/revision author list", "Display names of everyone who commented or tracked changes", size, join_names(&names)) {
            s.to_remove.insert(idx);
        }
    }
}

fn comments(pkg: &mut Pkg, rels: &[Rel], ctx: &mut Ctx, s: &mut State) {
    let mut removed_any = false;
    let mut removed_xl: Vec<String> = Vec::new();
    for idx in pkg.targets(rels, COMMENT_KINDS) {
        s.covered.insert(idx);
        let part = pkg.name(idx).to_string();
        let size = pkg.entries[idx].data.len();
        let Some(doc) = pkg.doc(idx) else { continue };
        let mut count = 0;
        let mut names = Vec::new();
        for el in doc.root().descendants() {
            if matches!(el.local.as_str(), "comment" | "cm" | "threadedComment") {
                count += 1;
            }
            if el.local == "author" && el.elements().next().is_none() {
                names.push(el.text());
            }
            if el.local == "cmAuthor" {
                names.push(el.attr("", "name").unwrap_or_default());
            }
            for a in &el.attrs {
                if a.local == "author" {
                    names.push(super::xml::unescape(&a.value));
                }
            }
        }
        let lower = part.to_ascii_lowercase();
        let ext = lower.contains("commentsextended") || lower.contains("commentsids") || lower.contains("commentsextensible");
        let (severity, label, detail) = if ext {
            (Severity::Low, "Comment threading data", "Reply/resolution state and durable IDs of comments")
        } else if count == 0 && !names.is_empty() {
            (Severity::Medium, "Comment author list", "Names and initials of commenters")
        } else {
            (Severity::High, "Comments", "Review comments with author names, initials and timestamps")
        };
        let preview = if names.is_empty() { format!("{} item(s)", count) } else { format!("{} item(s) — {}", count, join_names(&names)) };
        if ctx.add(Scope::Strict, "comments", severity, part.as_str(), label, detail, size, preview) {
            s.to_remove.insert(idx);
            removed_any = true;
            if lower.starts_with("xl/") && !ext {
                removed_xl.push(part.to_ascii_lowercase());
            }
        }
    }
    if !removed_any {
        return;
    }
    // Word: comment anchors in every story part.
    for idx in pkg.live() {
        let lower = pkg.name(idx).to_ascii_lowercase();
        if !lower.starts_with("word/") || !pkg.is_xml(idx) || s.covered.contains(&idx) {
            continue;
        }
        let Some(doc) = pkg.doc(idx) else { continue };
        let n = doc.root_mut().remove_where(&|e| is_w(&e.ns) && matches!(e.local.as_str(), "commentRangeStart" | "commentRangeEnd" | "commentReference"));
        if n > 0 {
            pkg.dirty.insert(idx);
        }
    }
    // Excel: the VML drawing that only holds the note boxes of the removed comments.
    for r in rels.iter().filter(|r| r.kind == "comments" && removed_xl.contains(&r.resolved.to_ascii_lowercase())) {
        for v in rels.iter().filter(|v| v.source == r.source && v.kind == "vmlDrawing") {
            if let Some(vidx) = pkg.find(&v.resolved) {
                let text = String::from_utf8_lossy(&pkg.entries[vidx].data).to_string();
                let notes = text.matches("ObjectType=\"Note\"").count();
                let objects = text.matches("ObjectType=").count();
                if notes > 0 && notes == objects {
                    s.to_remove.insert(vidx);
                }
            }
        }
    }
}

fn tracked_changes(pkg: &mut Pkg, ctx: &mut Ctx, s: &mut State) {
    // Word revision marks: w:ins, w:del, w:moveFrom/To, *PrChange … carry w:author and w:date.
    for idx in pkg.live() {
        let lower = pkg.name(idx).to_ascii_lowercase();
        if !lower.starts_with("word/") || !pkg.is_xml(idx) || s.covered.contains(&idx) || lower.contains("/_rels/") {
            continue;
        }
        let part = pkg.name(idx).to_string();
        let Some(doc) = pkg.doc(idx) else { continue };
        let mut count = 0;
        let mut names = Vec::new();
        for el in doc.root().descendants() {
            let author = el.attrs.iter().find(|a| is_w(&a.ns) && a.local == "author").map(|a| super::xml::unescape(&a.value));
            let dated = el.attrs.iter().any(|a| is_w(&a.ns) && a.local == "date");
            if let Some(a) = &author {
                if a != NEUTRAL_AUTHOR || dated {
                    count += 1;
                    names.push(a.clone());
                }
            }
        }
        if count == 0 {
            continue;
        }
        if ctx.add(Scope::Strict, "tracked-changes", Severity::High, part.as_str(), "Tracked changes", format!("{} revision mark(s) with author and timestamp; authors become \"{}\" and dates are removed (the changes themselves stay)", count, NEUTRAL_AUTHOR), 0, join_names(&names)) {
            doc.root_mut().walk_mut(&mut |e| {
                if e.attrs.iter().any(|a| is_w(&a.ns) && a.local == "author") {
                    let ns = e.attrs.iter().find(|a| is_w(&a.ns) && a.local == "author").map(|a| a.ns.clone()).unwrap_or_default();
                    e.set_attr(&ns, "author", NEUTRAL_AUTHOR);
                    e.remove_attr(&ns, "date");
                }
            });
            pkg.dirty.insert(idx);
        }
    }
    // Excel shared-workbook revision log.
    let revs: Vec<usize> = pkg.live().into_iter().filter(|i| pkg.name(*i).to_ascii_lowercase().starts_with("xl/revisions/") && !pkg.name(*i).to_ascii_lowercase().ends_with(".rels")).collect();
    if !revs.is_empty() {
        let mut names = Vec::new();
        for idx in &revs {
            if let Some(doc) = pkg.doc(*idx) {
                for el in doc.root().descendants() {
                    if let Some(n) = el.attr("", "userName") {
                        names.push(n);
                    }
                }
            }
        }
        let size = revs.iter().map(|i| pkg.entries[*i].data.len()).sum();
        let remove = ctx.add(Scope::Strict, "tracked-changes", Severity::High, "xl/revisions/", "Shared-workbook revision history", "Change log with user names and timestamps", size, join_names(&names));
        for idx in revs {
            s.covered.insert(idx);
            if remove {
                s.to_remove.insert(idx);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Word settings, rsids, workbook paths
// ---------------------------------------------------------------------------

fn word_settings(pkg: &mut Pkg, rels: &[Rel], ctx: &mut Ctx) {
    let Some(r) = rels.iter().find(|r| r.kind == "settings" && r.source.to_ascii_lowercase().starts_with("word/")) else { return };
    let Some(idx) = pkg.find(&r.resolved) else { return };
    let part = pkg.name(idx).to_string();
    let rels_idx = pkg.find(&rels_name_for(&part));
    let settings_rels: Vec<Rel> = rels.iter().filter(|x| x.source.eq_ignore_ascii_case(&part)).cloned().collect();
    let Some(doc) = pkg.doc(idx) else { return };
    let mut changed = false;
    let mut drop_rel_ids: Vec<String> = Vec::new();
    let found: Vec<(String, String, Option<String>)> = doc.root().elements().map(|e| (e.ns.clone(), e.local.clone(), e.attr(NS_R, "id").or_else(|| e.attr(NS_R_STRICT, "id")))).collect();
    for (ns, local, rid) in found {
        let location = format!("{} › {}", part, local);
        let remove = match (ns.as_str(), local.as_str()) {
            (n, "docId") if n == NS_W14 || n == NS_W15 => ctx.add(Scope::Safe, "identifier", Severity::Low, location, "Document GUID", "Persistent document identifier that links copies of the same file", 0, ""),
            (n, "docVars") if is_w(n) => ctx.add(Scope::Safe, "document-properties", Severity::Medium, location, "Document variables", "Hidden name/value pairs stored by templates and macros", 0, ""),
            (n, "attachedTemplate") if is_w(n) => {
                let target = rid.as_ref().and_then(|id| settings_rels.iter().find(|x| &x.id == id)).map(|x| x.target.clone()).unwrap_or_default();
                let severity = if contains_local_path(&target) { Severity::High } else { Severity::Low };
                let hit = ctx.add(Scope::Safe, "path", severity, location, "Attached template", "Template path recorded when the document was created", target.len(), &target);
                if hit {
                    drop_rel_ids.extend(rid.clone());
                }
                hit
            }
            _ => false,
        };
        if remove {
            doc.root_mut().children.retain(|n| !matches!(n, Node::Element(e) if e.ns == ns && e.local == local));
            changed = true;
        }
    }
    if changed {
        pkg.dirty.insert(idx);
    }
    if let (Some(ridx), false) = (rels_idx, drop_rel_ids.is_empty()) {
        if let Some(doc) = pkg.doc(ridx) {
            doc.root_mut().children.retain(|n| !matches!(n, Node::Element(e) if e.local == "Relationship" && drop_rel_ids.contains(&e.attr("", "Id").unwrap_or_default())));
            pkg.dirty.insert(ridx);
        }
    }
}

fn rsids(pkg: &mut Pkg, ctx: &mut Ctx, s: &State) {
    let parts: Vec<usize> = pkg.live().into_iter().filter(|i| pkg.name(*i).to_ascii_lowercase().starts_with("word/") && pkg.is_xml(*i) && !s.covered.contains(i)).collect();
    let mut attrs = 0usize;
    let mut sessions = 0usize;
    for idx in &parts {
        let Some(doc) = pkg.doc(*idx) else { continue };
        let root = doc.root();
        for el in std::iter::once(root).chain(root.descendants()) {
            attrs += el.attrs.iter().filter(|a| is_w(&a.ns) && a.local.starts_with("rsid")).count();
            if is_w(&el.ns) && el.local == "rsids" {
                sessions += el.elements().count();
            }
        }
    }
    if attrs + sessions == 0 {
        return;
    }
    let detail = format!("{} rsid attribute(s) and {} editing-session ID(s); they fingerprint the editing sessions and machines a document went through", attrs, sessions);
    if !ctx.add(Scope::Safe, "identifier", Severity::Low, "word/*.xml", "Revision session IDs (rsid)", detail, 0, format!("{} attributes, {} sessions", attrs, sessions)) {
        return;
    }
    for idx in parts {
        let Some(doc) = pkg.doc(idx) else { continue };
        let mut changed = false;
        doc.root_mut().walk_mut(&mut |e| {
            let before = e.attrs.len();
            e.attrs.retain(|a| !(is_w(&a.ns) && a.local.starts_with("rsid")));
            changed |= before != e.attrs.len();
        });
        changed |= doc.root_mut().remove_where(&|e| is_w(&e.ns) && e.local == "rsids") > 0;
        if changed {
            pkg.dirty.insert(idx);
        }
    }
}

fn workbook(pkg: &mut Pkg, main: &str, ctx: &mut Ctx) {
    if !main.starts_with("xl/") {
        return;
    }
    let Some(idx) = pkg.find(main) else { return };
    let part = pkg.name(idx).to_string();
    let Some(doc) = pkg.doc(idx) else { return };
    let root = doc.root();
    let abs: Vec<String> = root.descendants().iter().filter(|e| e.is(NS_X15AC, "absPath")).map(|e| e.attr("", "url").unwrap_or_default()).collect();
    let users: Vec<String> = root.elements().filter(|e| e.local == "fileSharing").filter_map(|e| e.attr("", "userName")).collect();
    let cloud: Vec<String> = root.elements().filter(|e| e.is(NS_XR, "revisionPtr")).map(|e| e.attr("", "documentId").unwrap_or_default()).collect();
    let mut changed = false;
    if !abs.is_empty() && ctx.add(Scope::Safe, "path", Severity::High, format!("{} › x15ac:absPath", part), "Workbook save folder", "Absolute folder the workbook was last saved in", 0, join_names(&abs)) {
        doc.root_mut().remove_where(&|e| e.is(NS_X15AC, "absPath"));
        prune_wrappers(doc.root_mut());
        changed = true;
    }
    if !users.is_empty() && ctx.add(Scope::Safe, "author", Severity::High, format!("{} › fileSharing", part), "Reservation user name", "Name recorded with write-reservation protection", 0, join_names(&users)) {
        doc.root_mut().walk_mut(&mut |e| {
            if e.local == "fileSharing" {
                e.remove_attr("", "userName");
            }
        });
        changed = true;
    }
    if !cloud.is_empty() && ctx.add(Scope::Safe, "identifier", Severity::Low, format!("{} › xr:revisionPtr", part), "Cloud document ID", "OneDrive/SharePoint document and co-authoring revision IDs", 0, join_names(&cloud)) {
        doc.root_mut().remove_where(&|e| e.is(NS_XR, "revisionPtr"));
        changed = true;
    }
    if changed {
        pkg.dirty.insert(idx);
    }
}

// ---------------------------------------------------------------------------
// Speaker notes, path leaks, embedded images, active content
// ---------------------------------------------------------------------------

fn notes(pkg: &mut Pkg, rels: &[Rel], ctx: &mut Ctx, s: &mut State) {
    let parts = pkg.targets(rels, &["notesSlide"]);
    let mut with_text = 0;
    let mut first = String::new();
    for idx in &parts {
        let Some(doc) = pkg.doc(*idx) else { continue };
        let mut text = String::new();
        for sp in doc.root().descendants().into_iter().filter(|e| e.is(NS_P, "sp")) {
            let body = sp.descendants().iter().any(|e| e.is(NS_P, "ph") && e.attr("", "type").as_deref() == Some("body"));
            if body {
                for t in sp.descendants().into_iter().filter(|e| e.is(NS_A, "t")) {
                    text.push_str(&t.text());
                    text.push(' ');
                }
            }
        }
        if !text.trim().is_empty() {
            with_text += 1;
            if first.is_empty() {
                first = text.trim().to_string();
            }
        }
    }
    if with_text == 0 {
        return;
    }
    let size = parts.iter().map(|i| pkg.entries[*i].data.len()).sum();
    let remove = ctx.add(Scope::Notes, "notes", Severity::Medium, "ppt/notesSlides/", format!("Speaker notes on {} slide(s)", with_text), "Presenter notes travel with the file; removed only in strict mode with \"remove speaker notes\"", size, &first);
    for idx in parts {
        s.covered.insert(idx);
        if remove {
            s.to_remove.insert(idx);
        }
    }
}

pub(super) fn path_leaks(pkg: &mut Pkg, ctx: &mut Ctx, covered: &HashSet<usize>) {
    for idx in pkg.live() {
        if !pkg.is_xml(idx) || covered.contains(&idx) {
            continue;
        }
        let part = pkg.name(idx).to_string();
        let Some(doc) = pkg.doc(idx) else { continue };
        let skip = |e: &Element| e.is(NS_X15AC, "absPath") || (e.local == "Relationship" && e.attr("", "Type").map(|t| t.ends_with("/attachedTemplate")).unwrap_or(false));
        let mut paths = Vec::new();
        let root = doc.root();
        for el in std::iter::once(root).chain(root.descendants()) {
            if skip(el) {
                continue;
            }
            for a in el.attrs.iter().filter(|a| a.ns != super::xml::NS_XMLNS) {
                let v = super::xml::unescape(&a.value);
                for (x, y) in local_path_spans(&v) {
                    paths.push(v[x..y].to_string());
                }
            }
        }
        if paths.is_empty() {
            continue;
        }
        let detail = format!("{} absolute local path(s) in links or references; each is reduced to its file name", paths.len());
        if ctx.add(Scope::Safe, "path", Severity::High, part.as_str(), "Local file paths", detail, 0, join_names(&paths)) {
            doc.root_mut().walk_mut(&mut |e| {
                if skip(e) {
                    return;
                }
                for a in e.attrs.iter_mut().filter(|a| a.ns != super::xml::NS_XMLNS) {
                    let v = super::xml::unescape(&a.value);
                    if contains_local_path(&v) {
                        a.value = super::xml::escape(&strip_local_paths(&v));
                    }
                }
            });
            pkg.dirty.insert(idx);
        }
    }
}

const IMAGE_EXTS: &[&str] = &[".png", ".jpg", ".jpeg", ".jpe", ".webp"];

/// Embedded PNG/JPEG/WebP media: EXIF/GPS, XMP, IPTC, C2PA… removed losslessly with the image cleaner.
pub(crate) fn clean_media(entries: &mut [Entry], live: &[usize], skip: &dyn Fn(usize) -> bool, ctx: &mut Ctx) {
    for &idx in live {
        let lower = entries[idx].name.to_ascii_lowercase();
        if skip(idx) || !IMAGE_EXTS.iter().any(|e| lower.ends_with(e)) {
            continue;
        }
        let report = inspect_image_bytes(&entries[idx].data);
        let traces: Vec<_> = report.findings.iter().filter(|f| f.removable && f.category != "brand-stamp").collect();
        if traces.is_empty() {
            continue;
        }
        let severity = traces.iter().fold(Severity::Info, |acc, f| severity_max(acc, f.severity));
        let names: Vec<String> = traces.iter().map(|f| f.name.clone()).collect();
        let bytes: usize = traces.iter().map(|f| f.size_bytes).sum();
        let detail = format!("{} metadata trace(s) inside an embedded image (camera EXIF/GPS, XMP history, IPTC, C2PA…); removed losslessly", traces.len());
        if ctx.add(Scope::Safe, "embedded-image", severity, entries[idx].name.as_str(), "Embedded image metadata", detail, bytes, join_names(&names)) {
            match clean_image_bytes(&entries[idx].data) {
                Ok(c) => entries[idx].data = c.data,
                Err(e) => ctx.warnings.push(format!("{}: image metadata could not be removed ({})", entries[idx].name, e)),
            }
        }
    }
}

fn embedded_images(pkg: &mut Pkg, ctx: &mut Ctx, s: &State) {
    let live = pkg.live();
    let covered = s.covered.clone();
    clean_media(&mut pkg.entries, &live, &|i| covered.contains(&i), ctx);
}

fn active_content(pkg: &Pkg, ctx: &mut Ctx) {
    for idx in pkg.live() {
        let name = pkg.name(idx);
        let lower = name.to_ascii_lowercase();
        let size = pkg.entries[idx].data.len();
        if lower.ends_with("vbaproject.bin") {
            ctx.add(Scope::Report, "active-content", Severity::High, name, "VBA macro project", "Macros are reported but never modified; review or remove them in Office (save as .docx/.xlsx/.pptx)", size, "");
        } else if lower.contains("/embeddings/") {
            let file = name.rsplit('/').next().unwrap_or(name);
            ctx.add(Scope::Report, "embedded-object", Severity::Medium, name, format!("Embedded object: {}", file), "Embedded files keep their own metadata; extract and clean them separately", size, file);
        } else if lower.contains("/activex/") && lower.ends_with(".xml") {
            ctx.add(Scope::Report, "active-content", Severity::Medium, name, "ActiveX control", "ActiveX controls are reported but never modified", size, "");
        }
    }
}

// ---------------------------------------------------------------------------
// Part removal with relationship / content-type consistency
// ---------------------------------------------------------------------------

/// Drops empty `ext`/`extLst` wrappers and alternate-content blocks whose branches are all empty.
fn prune_wrappers(e: &mut Element) {
    for c in e.children.iter_mut() {
        if let Node::Element(child) = c {
            prune_wrappers(child);
        }
    }
    e.children.retain(|n| match n {
        Node::Element(c) if c.local == "ext" || c.local == "extLst" => c.elements().next().is_some(),
        Node::Element(c) if c.is(NS_MC, "AlternateContent") => c.elements().any(|b| b.elements().next().is_some()),
        _ => true,
    });
}

fn remove_parts(pkg: &mut Pkg, ct_idx: usize, initial: BTreeSet<usize>, reachable_before: &HashSet<String>) {
    let mut pending: Vec<usize> = initial.into_iter().collect();
    while !pending.is_empty() {
        for idx in pending.drain(..) {
            let rels_name = rels_name_for(pkg.name(idx));
            pkg.removed.insert(idx);
            if let Some(r) = pkg.find(&rels_name) {
                pkg.removed.insert(r);
            }
        }
        let removed_names: HashSet<String> = pkg.removed.iter().map(|i| pkg.entries[*i].name.to_ascii_lowercase()).collect();

        // Relationships that target removed parts, and the r:id references they leave behind.
        let mut dangling: HashMap<String, HashSet<String>> = HashMap::new();
        for idx in pkg.live() {
            let name = pkg.name(idx).to_string();
            if !name.to_ascii_lowercase().ends_with(".rels") {
                continue;
            }
            let Some(source) = rels_source(&name) else { continue };
            let Some(doc) = pkg.doc(idx) else { continue };
            let mut ids = HashSet::new();
            doc.root_mut().children.retain(|n| match n {
                Node::Element(e) if e.local == "Relationship" && !e.attr("", "TargetMode").map(|m| m.eq_ignore_ascii_case("External")).unwrap_or(false) => {
                    let target = resolve_target(&source, &e.attr("", "Target").unwrap_or_default()).to_ascii_lowercase();
                    if removed_names.contains(&target) {
                        ids.insert(e.attr("", "Id").unwrap_or_default());
                        false
                    } else {
                        true
                    }
                }
                _ => true,
            });
            if !ids.is_empty() {
                pkg.dirty.insert(idx);
                dangling.entry(source).or_default().extend(ids);
            }
        }

        if let Some(doc) = pkg.doc(ct_idx) {
            let before = doc.root().children.len();
            doc.root_mut().children.retain(|n| match n {
                Node::Element(e) if e.is(NS_CT, "Override") => {
                    let part = e.attr("", "PartName").unwrap_or_default();
                    !removed_names.contains(&part.trim_start_matches('/').to_ascii_lowercase())
                }
                _ => true,
            });
            if doc.root().children.len() != before {
                pkg.dirty.insert(ct_idx);
            }
        }

        for (source, ids) in dangling {
            if source.is_empty() {
                continue;
            }
            let Some(idx) = pkg.find(&source) else { continue };
            if !pkg.is_xml(idx) {
                continue;
            }
            let Some(doc) = pkg.doc(idx) else { continue };
            let refers = |e: &Element| e.attrs.iter().any(|a| is_r(&a.ns) && ids.contains(&super::xml::unescape(&a.value)));
            // <pageSetup r:id> keeps the page setup; only the printer-settings link goes.
            doc.root_mut().walk_mut(&mut |e| {
                if e.local == "pageSetup" {
                    e.attrs.retain(|a| !(is_r(&a.ns) && ids.contains(&super::xml::unescape(&a.value))));
                }
            });
            doc.root_mut().remove_where(&|e| refers(e));
            prune_wrappers(doc.root_mut());
            pkg.dirty.insert(idx);
        }

        // Parts that were only reachable through what was just removed.
        let reachable_now = pkg.reachable();
        pending = pkg
            .live()
            .into_iter()
            .filter(|i| {
                let lower = pkg.entries[*i].name.to_ascii_lowercase();
                !lower.ends_with(".rels") && lower != "[content_types].xml" && reachable_before.contains(&lower) && !reachable_now.contains(&lower)
            })
            .collect();
    }
}

// ---------------------------------------------------------------------------
// Brand stamp
// ---------------------------------------------------------------------------

fn add_package_part(pkg: &mut Pkg, ct_idx: usize, name: &str, content_type: &str, rel_type: &str, data: Vec<u8>) {
    if let Some(doc) = pkg.doc(ct_idx) {
        let root = doc.root_mut();
        let mut o = Element::new(root.prefix(), "Override", NS_CT);
        o.push_attr("PartName", "", &format!("/{}", name));
        o.push_attr("ContentType", "", content_type);
        root.children.push(Node::Element(o));
        pkg.dirty.insert(ct_idx);
    }
    if let Some(ridx) = pkg.find("_rels/.rels") {
        if let Some(doc) = pkg.doc(ridx) {
            let root = doc.root_mut();
            let mut n = 1;
            let ids: HashSet<String> = root.elements().filter_map(|e| e.attr("", "Id")).collect();
            while ids.contains(&format!("rIdSeobify{}", n)) {
                n += 1;
            }
            let mut r = Element::new(root.prefix(), "Relationship", NS_PKG_REL);
            r.push_attr("Id", "", &format!("rIdSeobify{}", n));
            r.push_attr("Type", "", rel_type);
            r.push_attr("Target", "", name);
            root.children.push(Node::Element(r));
            pkg.dirty.insert(ridx);
        }
    }
    pkg.added.push(Entry { name: name.to_string(), data, is_dir: false });
}

fn write_stamp(pkg: &mut Pkg, ct_idx: usize, artist: &str, copyright: &str) {
    let artist = artist.trim();
    let copyright = copyright.trim();
    if !artist.is_empty() {
        let rels = pkg.rels();
        match part_of_kind(pkg, &rels, "extended-properties", "docProps/app.xml") {
            Some(idx) => {
                if let Some(doc) = pkg.doc(idx) {
                    let root = doc.root_mut();
                    let mut company = Element::new(root.prefix(), "Company", &root.ns.clone());
                    company.set_text(artist);
                    root.children.push(Node::Element(company));
                    pkg.dirty.insert(idx);
                }
            }
            None => {
                let xml = format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<Properties xmlns=\"{}\" xmlns:vt=\"{}\"><Company>{}</Company></Properties>",
                    NS_EP,
                    NS_VT,
                    super::xml::escape(artist)
                );
                add_package_part(pkg, ct_idx, "docProps/app.xml", CT_EXTENDED, REL_TYPE_EXTENDED, xml.into_bytes());
            }
        }
    }
    if !copyright.is_empty() {
        let name = if pkg.find("docProps/custom.xml").is_some() { "docProps/custom-seobify.xml" } else { "docProps/custom.xml" };
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<Properties xmlns=\"{}\" xmlns:vt=\"{}\"><property fmtid=\"{}\" pid=\"2\" name=\"Copyright\"><vt:lpwstr>{}</vt:lpwstr></property></Properties>",
            NS_CUSTOM,
            NS_VT,
            FMTID_USER,
            super::xml::escape(copyright)
        );
        add_package_part(pkg, ct_idx, name, CT_CUSTOM, REL_TYPE_CUSTOM, xml.into_bytes());
    }
}
