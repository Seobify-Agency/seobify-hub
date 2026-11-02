use super::test_support::*;
use super::*;
use crate::services::metadata_cleaner::inspect_image_bytes;

fn opts(mode: DocMode, remove_notes: bool) -> DocCleanOptions {
    DocCleanOptions { mode, remove_notes, brand_stamp: None }
}

fn names(report: &DocInspectReport) -> Vec<String> {
    report.findings.iter().map(|f| f.name.clone()).collect()
}

fn find<'a>(report: &'a DocInspectReport, name: &str) -> &'a DocFinding {
    report.findings.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no finding '{}' in {:?}", name, names(report)))
}

fn has(report: &DocInspectReport, name: &str) -> bool {
    report.findings.iter().any(|f| f.name == name)
}

fn clean(data: &[u8], o: &DocCleanOptions) -> Vec<u8> {
    clean_document_bytes(data, o).unwrap().data
}

#[test]
fn local_paths_are_found_and_reduced_to_file_names() {
    assert_eq!(strip_local_paths("file:///C:\\Users\\deniz\\Desktop\\brief.pdf"), "brief.pdf");
    assert_eq!(strip_local_paths("C:\\Users\\deniz\\Desktop\\"), "");
    assert_eq!(strip_local_paths("Provider=X;Data Source=D:\\Data\\crm.accdb;Mode=Read"), "Provider=X;Data Source=crm.accdb;Mode=Read");
    assert_eq!(strip_local_paths("\\\\CORP-FS01\\share\\plan.xlsx"), "plan.xlsx");
    assert_eq!(strip_local_paths("/Users/deniz/Documents/a.key"), "a.key");
    for safe in ["https://seobify.com/Users/x", "urn:schemas-microsoft-com:vml", "../media/image1.png", "http://schemas.openxmlformats.org/a", "Normal.dotm"] {
        assert!(!contains_local_path(safe), "{}", safe);
    }
}

#[test]
fn unsupported_inputs_are_rejected_with_a_clear_message() {
    assert!(inspect_document_bytes(&[0xD0, 0xCF, 0x11, 0xE0, 0, 0], None).unwrap_err().contains("Legacy binary Office"));
    assert!(inspect_document_bytes(b"hello", None).unwrap_err().contains("Unsupported"));
    assert!(inspect_document_bytes(&zip_bytes(&[("a.txt", b"x".to_vec())], ""), None).unwrap_err().contains("neither"));
}

#[test]
fn docx_inspection_reports_every_carrier() {
    let r = inspect_document_bytes(&docx(), None).unwrap();
    assert_eq!((r.format.as_str(), r.family.as_str()), ("docx", "word"));
    for (name, scope) in [
        ("Author", Scope::Safe),
        ("Last modified by", Scope::Safe),
        ("Created date", Scope::Safe),
        ("Company", Scope::Safe),
        ("Manager", Scope::Safe),
        ("Total editing time (minutes)", Scope::Safe),
        ("Hyperlink base", Scope::Safe),
        ("Custom property: Client", Scope::Safe),
        ("Thumbnail preview", Scope::Safe),
        ("Custom XML data", Scope::Safe),
        ("Author account IDs", Scope::Safe),
        ("Attached template", Scope::Safe),
        ("Document GUID", Scope::Safe),
        ("Document variables", Scope::Safe),
        ("Revision session IDs (rsid)", Scope::Safe),
        ("Local file paths", Scope::Safe),
        ("Embedded image metadata", Scope::Safe),
        ("ZIP entry timestamps", Scope::Safe),
        ("Comments", Scope::Strict),
        ("Comment threading data", Scope::Strict),
        ("Comment/revision author list", Scope::Strict),
        ("Tracked changes", Scope::Strict),
        ("Embedded object: oleObject1.bin", Scope::Report),
    ] {
        assert_eq!(find(&r, name).scope, scope, "{}", name);
    }
    assert_eq!(find(&r, "Author").value_preview, PERSON);
    assert_eq!(find(&r, "Author account IDs").severity, Severity::High);
    assert!(find(&r, "Author account IDs").value_preview.contains(ACCOUNT));
    assert!(find(&r, "Attached template").value_preview.contains("Kurumsal.dotm"));
    assert_eq!(find(&r, "Attached template").severity, Severity::High);
    assert!(find(&r, "Local file paths").value_preview.contains("brief.pdf"));
    assert!(find(&r, "Tracked changes").value_preview.contains("Ayşe Demir"));
    assert!(!find(&r, "Embedded object: oleObject1.bin").removable);
    assert_eq!(r.trace_count, r.safe_count + r.strict_count);
    assert!(r.strict_count >= 4);
    assert!(!r.clean);
}

#[test]
fn docx_safe_clean_removes_metadata_and_keeps_review_content() {
    let src = docx();
    let out = clean(&src, &opts(DocMode::Safe, false));
    assert_ooxml_consistent(&out);
    let r = inspect_document_bytes(&out, None).unwrap();
    assert_eq!(r.safe_count, 0, "{:?}", names(&r));
    assert!(has(&r, "Comments") && has(&r, "Tracked changes"), "strict-only traces stay in safe mode");
    assert_eq!(r.trace_count, r.strict_count);

    let core = part(&out, "docProps/core.xml").unwrap();
    assert!(!core.contains(PERSON) && !core.contains("dcterms:created"));
    let app = part(&out, "docProps/app.xml").unwrap();
    assert!(!app.contains("Company") && !app.contains("Manager") && !app.contains("TotalTime") && app.contains("DocSecurity"));
    assert!(part(&out, "docProps/custom.xml").is_none() && part(&out, "docProps/thumbnail.jpeg").is_none());
    assert!(part(&out, "customXml/item1.xml").is_none() && part(&out, "customXml/itemProps1.xml").is_none());
    let settings = part(&out, "word/settings.xml").unwrap();
    assert!(!settings.contains("rsid") && !settings.contains("attachedTemplate") && !settings.contains("docId") && !settings.contains("docVars"));
    assert!(settings.contains("w:defaultTabStop"));
    assert!(!part(&out, "word/_rels/settings.xml.rels").unwrap().contains("Kurumsal"));
    let document = part(&out, "word/document.xml").unwrap();
    assert!(!document.contains("w:rsid"));
    assert!(document.contains("Fiyat teklifi") && document.contains("w:commentReference"), "safe keeps comments");
    let rels = part(&out, "word/_rels/document.xml.rels").unwrap();
    assert!(rels.contains("Target=\"brief.pdf\"") && !rels.contains("Users"));
    assert!(!part(&out, "word/people.xml").unwrap().contains(ACCOUNT));
    let media = zip_entries(&out).into_iter().find(|e| e.0 == "word/media/image1.jpeg").unwrap().1;
    assert!(inspect_image_bytes(&media).clean);
    assert!(part(&out, "word/embeddings/oleObject1.bin").is_some(), "embedded objects are reported, not removed");
}

#[test]
fn docx_strict_clean_leaves_zero_traces() {
    let out = clean(&docx(), &opts(DocMode::Strict, false));
    assert_ooxml_consistent(&out);
    let r = inspect_document_bytes(&out, None).unwrap();
    assert!(r.clean, "{:?}", names(&r));
    assert_eq!(r.trace_count, 0);
    for gone in ["word/comments.xml", "word/commentsExtended.xml", "word/people.xml"] {
        assert!(part(&out, gone).is_none(), "{}", gone);
    }
    let ct = part(&out, "[Content_Types].xml").unwrap();
    assert!(!ct.contains("comments") && !ct.contains("people") && !ct.contains("custom.xml") && !ct.contains("customXml"));
    let document = part(&out, "word/document.xml").unwrap();
    assert!(!document.contains("commentRange") && !document.contains("commentReference"));
    assert!(document.contains("w:author=\"Author\"") && !document.contains("w:date") && !document.contains(PERSON));
    assert!(document.contains("eklenen metin") && document.contains("silinen"), "tracked changes stay, only anonymised");
    // Idempotent: a second strict pass finds nothing.
    let again = clean(&out, &opts(DocMode::Strict, false));
    assert!(inspect_document_bytes(&again, None).unwrap().clean);
}

#[test]
fn xlsx_strict_clean_keeps_workbook_consistent() {
    let src = xlsx();
    let r = inspect_document_bytes(&src, None).unwrap();
    assert_eq!(r.family, "spreadsheet");
    for name in ["Workbook save folder", "Reservation user name", "Cloud document ID", "Printer settings", "Local file paths", "Author account IDs", "ZIP archive comment", "Comments"] {
        assert!(has(&r, name), "{} in {:?}", name, names(&r));
    }
    assert!(find(&r, "Printer settings").value_preview.contains("CORP-PRN01"));
    assert!(find(&r, "Workbook save folder").value_preview.contains("Users"));

    let safe = clean(&src, &opts(DocMode::Safe, false));
    assert_ooxml_consistent(&safe);
    let rs = inspect_document_bytes(&safe, None).unwrap();
    assert_eq!(rs.safe_count, 0, "{:?}", names(&rs));
    assert!(!part(&safe, "xl/persons/person.xml").unwrap().contains(ACCOUNT));

    let out = clean(&src, &opts(DocMode::Strict, false));
    assert_ooxml_consistent(&out);
    let after = inspect_document_bytes(&out, None).unwrap();
    assert!(after.clean, "{:?}", names(&after));
    for gone in ["xl/comments1.xml", "xl/threadedComments/threadedComment1.xml", "xl/persons/person.xml", "xl/drawings/vmlDrawing1.vml", "xl/printerSettings/printerSettings1.bin"] {
        assert!(part(&out, gone).is_none(), "{}", gone);
    }
    let sheet = part(&out, "xl/worksheets/sheet1.xml").unwrap();
    assert!(sheet.contains("<pageSetup paperSize=\"9\" orientation=\"landscape\"/>"), "page setup survives without the printer link: {}", sheet);
    assert!(!sheet.contains("legacyDrawing"));
    let wb = part(&out, "xl/workbook.xml").unwrap();
    assert!(!wb.contains("absPath") && !wb.contains("AlternateContent>") && !wb.contains("userName") && !wb.contains("revisionPtr"));
    assert!(wb.contains("<sheet name=\"Bütçe\""));
    assert!(part(&out, "xl/externalLinks/_rels/externalLink1.xml.rels").unwrap().contains("Target=\"Budget%202026.xlsx\""));
}

#[test]
fn pptx_notes_are_removed_only_on_request() {
    let src = pptx();
    let r = inspect_document_bytes(&src, None).unwrap();
    assert_eq!(r.family, "presentation");
    let notes = find(&r, "Speaker notes on 1 slide(s)");
    assert_eq!(notes.scope, Scope::Notes);
    assert!(notes.value_preview.contains("indirim"), "{}", notes.value_preview);
    assert!(!notes.value_preview.contains('1'), "slide-number fields are not notes");
    for name in ["Editing client IDs", "Author account IDs", "Comments", "Comment author list", "Embedded object: oleObject1.bin"] {
        assert!(has(&r, name), "{} in {:?}", name, names(&r));
    }
    assert_eq!(r.notes_count, 1);

    let strict = clean(&src, &opts(DocMode::Strict, false));
    assert_ooxml_consistent(&strict);
    let rs = inspect_document_bytes(&strict, None).unwrap();
    assert_eq!((rs.trace_count, rs.notes_count), (1, 1), "{:?}", names(&rs));
    let slide = part(&strict, "ppt/slides/slide1.xml").unwrap();
    assert!(!slide.contains("commentRel") && !slide.contains("extLst"), "{}", slide);
    for gone in ["ppt/comments/comment1.xml", "ppt/comments/modernComment_1.xml", "ppt/commentAuthors.xml", "ppt/authors.xml", "ppt/revisionInfo.xml"] {
        assert!(part(&strict, gone).is_none(), "{}", gone);
    }

    let all = clean(&src, &opts(DocMode::Strict, true));
    assert_ooxml_consistent(&all);
    assert!(inspect_document_bytes(&all, None).unwrap().clean);
    assert!(part(&all, "ppt/notesSlides/notesSlide1.xml").is_none());
    assert!(part(&all, "ppt/embeddings/oleObject1.bin").is_some());

    let warned = clean_document_bytes(&src, &opts(DocMode::Safe, true)).unwrap();
    assert!(warned.warnings.iter().any(|w| w.contains("strict")));
    assert!(part(&warned.data, "ppt/notesSlides/notesSlide1.xml").is_some());
}

#[test]
fn odt_clean_keeps_mimetype_first_and_manifest_in_step() {
    let src = odt();
    let r = inspect_document_bytes(&src, None).unwrap();
    assert_eq!((r.format.as_str(), r.family.as_str()), ("odt", "word"));
    for name in [
        "Initial author",
        "Author",
        "Printed by",
        "Created date",
        "Editing cycles (revision)",
        "Total editing time",
        "Generator (application)",
        "Template",
        "Custom property: Client",
        "Thumbnail preview",
        "Printer name",
        "Printer driver settings",
        "Editing session ID (rsid)",
        "Revision session IDs (rsid)",
        "Local file paths",
        "Embedded image metadata",
        "Comments",
        "Tracked changes",
    ] {
        assert!(has(&r, name), "{} in {:?}", name, names(&r));
    }
    assert_eq!(find(&r, "Template").severity, Severity::High);

    let safe = clean(&src, &opts(DocMode::Safe, false));
    assert_odf_consistent(&safe);
    let rs = inspect_document_bytes(&safe, None).unwrap();
    assert_eq!(rs.safe_count, 0, "{:?}", names(&rs));
    assert!(has(&rs, "Comments") && has(&rs, "Tracked changes"));

    let out = clean(&src, &opts(DocMode::Strict, false));
    assert_odf_consistent(&out);
    assert!(inspect_document_bytes(&out, None).unwrap().clean);
    let meta = part(&out, "meta.xml").unwrap();
    assert!(!meta.contains(PERSON) && !meta.contains("generator") && meta.contains("document-statistic"));
    let settings = part(&out, "settings.xml").unwrap();
    assert!(!settings.contains("PrinterName") && !settings.contains("Rsid") && settings.contains("EmbedFonts"));
    let content = part(&out, "content.xml").unwrap();
    assert!(!content.contains("office:annotation") && !content.contains(PERSON) && !content.contains("rsid"));
    assert!(content.contains("<dc:creator>Author</dc:creator>") && content.contains("text:change-start"));
    assert!(content.contains("xlink:href=\"brief.pdf\""));
    assert!(part(&out, "Thumbnails/thumbnail.png").is_none());
    assert!(!part(&out, "META-INF/manifest.xml").unwrap().contains("Thumbnails"));
}

#[test]
fn pdf_with_incremental_update_is_rewritten_clean() {
    let src = pdf_with_update();
    let r = inspect_document_bytes(&src, None).unwrap();
    assert_eq!(r.format, "pdf");
    for (name, scope) in [
        ("Author", Scope::Safe),
        ("Modified date", Scope::Safe),
        ("XMP metadata packet", Scope::Safe),
        ("Object-level XMP (1 object(s))", Scope::Safe),
        ("Document ID", Scope::Safe),
        ("Application private data", Scope::Safe),
        ("Last-modified timestamps", Scope::Safe),
        ("Embedded image metadata", Scope::Safe),
        ("Earlier saved revisions", Scope::Safe),
        ("Annotation authors", Scope::Strict),
        ("JavaScript", Scope::Report),
        ("Embedded file: ek.txt", Scope::Report),
        ("PDF/A identification", Scope::Report),
    ] {
        assert_eq!(find(&r, name).scope, scope, "{}", name);
    }
    assert_eq!(find(&r, "Title").value_preview, "Taslak v1", "latest revision is inspected");
    assert!(!has(&r, "Creator application"), "the Creator only survives in the earlier revision");
    assert_eq!(find(&r, "XMP metadata packet").severity, Severity::High);

    let safe = clean(&src, &opts(DocMode::Safe, false));
    let rs = inspect_document_bytes(&safe, None).unwrap();
    assert_eq!(rs.safe_count, 0, "{:?}", names(&rs));
    assert!(has(&rs, "Annotation authors"));

    let out = clean(&src, &opts(DocMode::Strict, false));
    let after = inspect_document_bytes(&out, None).unwrap();
    assert!(after.clean, "{:?}", names(&after));
    assert_eq!(out.windows(9).filter(|w| *w == b"startxref").count(), 1);
    let text = pdf_all_text(&out);
    assert!(!text.contains("Deniz") && !text.contains("Taslak") && !text.contains("Microsoft Word") && !text.contains("Illustrator"), "no earlier revision survives");
    let doc = lopdf::Document::load_mem(&out).unwrap();
    assert!(!doc.trailer.has(b"Info") && !doc.trailer.has(b"ID"));
    assert_eq!(doc.get_pages().len(), 1);
    assert!(text.contains("musteri_adi"), "form-field names (/T on widgets) are kept");
    assert!(text.contains("Rakamı kontrol et") || text.contains("Rakam"), "annotation content stays");
}

#[test]
fn pdf_object_streams_do_not_hide_metadata() {
    let src = pdf_modern();
    assert!(!String::from_utf8_lossy(&src).contains("/Author"), "fixture keeps the Info dictionary inside a compressed object stream");
    let r = inspect_document_bytes(&src, None).unwrap();
    assert_eq!(find(&r, "Author").value_preview, "Deniz Y");
    let out = clean(&src, &opts(DocMode::Strict, false));
    assert!(inspect_document_bytes(&out, None).unwrap().clean);
    assert!(!pdf_all_text(&out).contains("Deniz"));
}

#[test]
fn brand_stamp_is_written_and_recognised() {
    let stamp = crate::services::dsp_pipeline::BrandStamp { artist: "Seobify".to_string(), copyright: "© 2026 Seobify".to_string() };
    let o = DocCleanOptions { mode: DocMode::Strict, remove_notes: true, brand_stamp: Some(stamp.clone()) };
    for (label, src) in [("docx", docx()), ("xlsx", xlsx()), ("pptx", pptx()), ("odt", odt()), ("pdf", pdf_with_update())] {
        let out = clean(&src, &o);
        if label == "odt" {
            assert_odf_consistent(&out);
        } else if label != "pdf" {
            assert_ooxml_consistent(&out);
        }
        let known = inspect_document_bytes(&out, Some(&stamp)).unwrap();
        assert!(known.clean, "{}: {:?}", label, names(&known));
        let stamps: Vec<&DocFinding> = known.findings.iter().filter(|f| f.category == "brand-stamp").collect();
        assert_eq!(stamps.len(), 2, "{}: {:?}", label, names(&known));
        assert!(stamps.iter().all(|f| !f.removable && f.severity == Severity::Info));
        // Without knowing the stamp, an inspector sees the organisation fields as ordinary metadata.
        assert!(!inspect_document_bytes(&out, None).unwrap().clean, "{}", label);
        // Re-stamping replaces instead of duplicating.
        let twice = clean(&out, &o);
        let again = inspect_document_bytes(&twice, Some(&stamp)).unwrap();
        assert!(again.clean);
        assert_eq!(again.findings.iter().filter(|f| f.category == "brand-stamp").count(), 2, "{}", label);
    }
    // Stamp off by default.
    assert!(DocCleanOptions::default().brand_stamp.is_none());
}

#[test]
fn process_document_file_writes_a_clean_copy_and_never_overwrites() {
    let dir = std::env::temp_dir().join(format!("optic-doc-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("Teklif Ekim.docx");
    std::fs::write(&input, docx()).unwrap();
    let output = default_output_path(&input);
    assert_eq!(output.file_name().unwrap().to_string_lossy(), "Teklif Ekim.clean.docx");
    let outcome = process_document_file(&input, &output, &opts(DocMode::Strict, false)).unwrap();
    assert!(outcome.clean);
    assert_eq!(outcome.traces_after, 0);
    assert_eq!(outcome.traces_removed, outcome.traces_before);
    assert!(outcome.traces_before > 15);
    assert_eq!(std::fs::read(&input).unwrap(), docx());
    assert!(process_document_file(&input, &input, &opts(DocMode::Safe, false)).is_err());
    assert!(is_document_path(&input) && !is_document_path(&dir.join("a.png")));
    let mut used = std::collections::HashSet::new();
    assert_eq!(batch_output_path(&input, &dir, &mut used), dir.join("Teklif Ekim.clean.docx"));
    let out_dir = dir.join("out");
    std::fs::create_dir_all(&out_dir).unwrap();
    assert_eq!(batch_output_path(&input, &out_dir, &mut used), out_dir.join("Teklif Ekim.docx"));
    assert_eq!(batch_output_path(&dir.join("sub").join("Teklif Ekim.docx"), &out_dir, &mut used), out_dir.join("Teklif Ekim-2.docx"));
    let _ = std::fs::remove_dir_all(dir);
}

/// With `OPTIC_DOC_DUMP=<dir>` the fixtures and their cleaned copies are written to disk so
/// independent parsers (python-docx/openpyxl/python-pptx/odfpy/pypdf, LibreOffice) can open them.
#[test]
fn fixtures_dump_for_external_validation() {
    let Ok(dir) = std::env::var("OPTIC_DOC_DUMP") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let o = opts(DocMode::Strict, true);
    for (name, src) in [("fixture.docx", docx()), ("fixture.xlsx", xlsx()), ("fixture.pptx", pptx()), ("fixture.odt", odt()), ("fixture.pdf", pdf_with_update())] {
        std::fs::write(dir.join(name), &src).unwrap();
        let (stem, ext) = name.split_once('.').unwrap();
        std::fs::write(dir.join(format!("{}.safe.{}", stem, ext)), clean(&src, &opts(DocMode::Safe, false))).unwrap();
        std::fs::write(dir.join(format!("{}.strict.{}", stem, ext)), clean(&src, &o)).unwrap();
    }
}
