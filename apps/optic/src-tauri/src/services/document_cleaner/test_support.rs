//! Programmatically built DOCX/XLSX/PPTX/ODT/PDF samples carrying every supported metadata
//! carrier, plus structural consistency checks for cleaned packages.

use super::package;
use super::xml::XmlDoc;
use crate::services::metadata_cleaner::test_support::{dirty_jpeg, dirty_png};
use lopdf::{dictionary, Document, Object, Stream, StringFormat};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Cursor, Write};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

pub const PERSON: &str = "Deniz Yılmaz";
pub const ACCOUNT: &str = "deniz@example.com";

const XML_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n";
const RELS_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A ZIP written the way Office/LibreOffice do: real timestamps, optional archive comment.
pub fn zip_bytes(entries: &[(&str, Vec<u8>)], comment: &str) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let when = DateTime::from_date_and_time(2026, 10, 29, 7, 12, 30).unwrap();
    for (name, data) in entries {
        let method = if *name == "mimetype" { CompressionMethod::Stored } else { CompressionMethod::Deflated };
        zip.start_file(*name, SimpleFileOptions::DEFAULT.compression_method(method).last_modified_time(when)).unwrap();
        zip.write_all(data).unwrap();
    }
    if !comment.is_empty() {
        zip.set_comment(comment).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn x(s: &str) -> Vec<u8> {
    format!("{}{}", XML_DECL, s).into_bytes()
}

fn rels(list: &[(&str, &str, &str, bool)]) -> Vec<u8> {
    let mut s = format!("<Relationships xmlns=\"{}\">", RELS_NS);
    for (id, ty, target, external) in list {
        s.push_str(&format!(
            "<Relationship Id=\"{}\" Type=\"{}\" Target=\"{}\"{}/>",
            id,
            ty,
            target,
            if *external { " TargetMode=\"External\"" } else { "" }
        ));
    }
    s.push_str("</Relationships>");
    x(&s)
}

fn content_types(overrides: &[(&str, &str)]) -> Vec<u8> {
    let mut s = String::from("<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"bin\" ContentType=\"application/vnd.openxmlformats-officedocument.oleObject\"/><Default Extension=\"vml\" ContentType=\"application/vnd.openxmlformats-officedocument.vmlDrawing\"/>");
    for (part, ct) in overrides {
        s.push_str(&format!("<Override PartName=\"/{}\" ContentType=\"{}\"/>", part, ct));
    }
    s.push_str("</Types>");
    x(&s)
}

fn core_xml() -> Vec<u8> {
    x(&format!("<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><dc:title>Teklif 2026</dc:title><dc:creator>{p}</dc:creator><cp:keywords>gizli, müşteri</cp:keywords><cp:lastModifiedBy>Ayşe Demir</cp:lastModifiedBy><cp:revision>7</cp:revision><dcterms:created xsi:type=\"dcterms:W3CDTF\">2026-10-01T09:00:00Z</dcterms:created><dcterms:modified xsi:type=\"dcterms:W3CDTF\">2026-10-28T18:30:00Z</dcterms:modified></cp:coreProperties>", p = PERSON))
}

fn app_xml(extra: &str) -> Vec<u8> {
    x(&format!("<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\" xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes\"><Template>Normal.dotm</Template><TotalTime>42</TotalTime><Application>Microsoft Office Word</Application><DocSecurity>0</DocSecurity>{}<Company>Seobify Ajans</Company><Manager>Ayşe Demir</Manager><HyperlinkBase>C:\\Users\\deniz\\Proje\\</HyperlinkBase><AppVersion>16.0000</AppVersion></Properties>", extra))
}

fn custom_xml() -> Vec<u8> {
    x("<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/custom-properties\" xmlns:vt=\"http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes\"><property fmtid=\"{D5CDD505-2E9C-101B-9397-08002B2CF9AE}\" pid=\"2\" name=\"Client\"><vt:lpwstr>ACME Holding</vt:lpwstr></property></Properties>")
}

/// DEVMODE blob whose device name is a network printer share.
pub fn devmode() -> Vec<u8> {
    let mut v: Vec<u8> = "\\\\CORP-PRN01\\HP LaserJet".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    v.resize(220, 0);
    v
}

const CT_CORE: &str = "application/vnd.openxmlformats-package.core-properties+xml";
const CT_APP: &str = "application/vnd.openxmlformats-officedocument.extended-properties+xml";
const CT_CUSTOM: &str = "application/vnd.openxmlformats-officedocument.custom-properties+xml";
const T_DOC: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const T_CORE: &str = "http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties";
const T_APP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties";
const T_CUSTOM: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/custom-properties";
const T_THUMB: &str = "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail";
const T_COMMENTS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
const T_OLE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/oleObject";
const T_IMAGE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const T_HYPERLINK: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink";
const T_PRINTER: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/printerSettings";

pub fn docx() -> Vec<u8> {
    let w = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\" xmlns:w15=\"http://schemas.microsoft.com/office/word/2012/wordml\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"";
    let document = x(&format!(
        "<w:document {w}><w:body>\
<w:p w:rsidR=\"00A1B2C3\" w:rsidRDefault=\"00A1B2C3\"><w:commentRangeStart w:id=\"0\"/><w:r w:rsidRPr=\"00D4E5F6\"><w:t>Fiyat teklifi</w:t></w:r><w:commentRangeEnd w:id=\"0\"/><w:r><w:rPr><w:rStyle w:val=\"CommentReference\"/></w:rPr><w:commentReference w:id=\"0\"/></w:r></w:p>\
<w:p w:rsidR=\"00A1B2C3\"><w:ins w:id=\"1\" w:author=\"{p}\" w:date=\"2026-10-28T10:00:00Z\"><w:r><w:t xml:space=\"preserve\">eklenen metin </w:t></w:r></w:ins><w:del w:id=\"2\" w:author=\"Ayşe Demir\" w:date=\"2026-10-28T11:00:00Z\"><w:r><w:delText>silinen</w:delText></w:r></w:del>\
<w:hyperlink r:id=\"rId9\"><w:r><w:t>brief</w:t></w:r></w:hyperlink></w:p>\
<w:p><w:r><w:drawing><wp:inline><wp:extent cx=\"609600\" cy=\"457200\"/><wp:docPr id=\"1\" name=\"Picture 1\"/><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"0\" name=\"photo.jpeg\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"rId8\"/></pic:blipFill><pic:spPr/></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>\
<w:sectPr w:rsidR=\"00A1B2C3\"><w:pgSz w:w=\"11906\" w:h=\"16838\"/></w:sectPr></w:body></w:document>",
        w = w,
        p = PERSON
    ));
    let settings = x(&format!(
        "<w:settings {w}><w:zoom w:percent=\"100\"/><w:attachedTemplate r:id=\"rId1\"/><w:defaultTabStop w:val=\"708\"/><w:docVars><w:docVar w:name=\"Client\" w:val=\"ACME\"/></w:docVars><w:rsids><w:rsidRoot w:val=\"00A1B2C3\"/><w:rsid w:val=\"00A1B2C3\"/><w:rsid w:val=\"00D4E5F6\"/></w:rsids><w14:docId w14:val=\"1A2B3C4D\"/><w15:docId w15:val=\"{{0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0}}\"/></w:settings>",
        w = w
    ));
    let comments = x(&format!("<w:comments {w}><w:comment w:id=\"0\" w:author=\"{p}\" w:date=\"2026-10-28T09:00:00Z\" w:initials=\"KY\"><w:p w14:paraId=\"11111111\"><w:r><w:annotationRef/></w:r><w:r><w:t>Bu rakamı düşür</w:t></w:r></w:p></w:comment></w:comments>", w = w, p = PERSON));
    let comments_ext = x(&format!("<w15:commentsEx {w}><w15:commentEx w15:paraId=\"11111111\" w15:done=\"0\"/></w15:commentsEx>", w = w));
    let people = x(&format!("<w15:people {w}><w15:person w15:author=\"{p}\"><w15:presenceInfo w15:providerId=\"AD\" w15:userId=\"S::{a}::0f1e2d3c\"/></w15:person></w15:people>", w = w, p = PERSON, a = ACCOUNT));
    let ct = content_types(&[
        ("word/document.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"),
        ("word/settings.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"),
        ("word/comments.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"),
        ("word/commentsExtended.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml"),
        ("word/people.xml", "application/vnd.openxmlformats-officedocument.wordprocessingml.people+xml"),
        ("customXml/itemProps1.xml", "application/vnd.openxmlformats-officedocument.customXmlProperties+xml"),
        ("docProps/core.xml", CT_CORE),
        ("docProps/app.xml", CT_APP),
        ("docProps/custom.xml", CT_CUSTOM),
    ]);
    zip_bytes(
        &[
            ("[Content_Types].xml", ct),
            ("_rels/.rels", rels(&[("rId1", T_DOC, "word/document.xml", false), ("rId2", T_CORE, "docProps/core.xml", false), ("rId3", T_APP, "docProps/app.xml", false), ("rId4", T_CUSTOM, "docProps/custom.xml", false), ("rId5", T_THUMB, "docProps/thumbnail.jpeg", false)])),
            ("word/document.xml", document),
            (
                "word/_rels/document.xml.rels",
                rels(&[
                    ("rId1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings", "settings.xml", false),
                    ("rId2", T_COMMENTS, "comments.xml", false),
                    ("rId3", "http://schemas.microsoft.com/office/2011/relationships/commentsExtended", "commentsExtended.xml", false),
                    ("rId4", "http://schemas.microsoft.com/office/2011/relationships/people", "people.xml", false),
                    ("rId5", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXml", "../customXml/item1.xml", false),
                    ("rId6", T_OLE, "embeddings/oleObject1.bin", false),
                    ("rId8", T_IMAGE, "media/image1.jpeg", false),
                    ("rId9", T_HYPERLINK, "file:///C:\\Users\\deniz\\Desktop\\brief.pdf", true),
                ]),
            ),
            ("word/settings.xml", settings),
            ("word/_rels/settings.xml.rels", rels(&[("rId1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/attachedTemplate", "file:///C:\\Users\\deniz\\AppData\\Roaming\\Microsoft\\Templates\\Kurumsal.dotm", true)])),
            ("word/comments.xml", comments),
            ("word/commentsExtended.xml", comments_ext),
            ("word/people.xml", people),
            ("word/media/image1.jpeg", dirty_jpeg()),
            ("word/embeddings/oleObject1.bin", b"\xD0\xCF\x11\xE0 embedded workbook".to_vec()),
            ("customXml/item1.xml", x("<b:Sources xmlns:b=\"http://schemas.openxmlformats.org/officeDocument/2006/bibliography\" SelectedStyle=\"\\APA.XSL\"/>")),
            ("customXml/itemProps1.xml", x("<ds:datastoreItem ds:itemID=\"{11111111-2222-3333-4444-555555555555}\" xmlns:ds=\"http://schemas.openxmlformats.org/officeDocument/2006/customXml\"/>")),
            ("customXml/_rels/item1.xml.rels", rels(&[("rId1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/customXmlProps", "itemProps1.xml", false)])),
            ("docProps/core.xml", core_xml()),
            ("docProps/app.xml", app_xml("")),
            ("docProps/custom.xml", custom_xml()),
            ("docProps/thumbnail.jpeg", dirty_jpeg()),
        ],
        "",
    )
}

pub fn xlsx() -> Vec<u8> {
    let workbook = x("<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"x15 xr\" xmlns:x15=\"http://schemas.microsoft.com/office/spreadsheetml/2010/11/main\" xmlns:xr=\"http://schemas.microsoft.com/office/spreadsheetml/2014/revision\"><fileVersion appName=\"xl\" lastEdited=\"7\"/><fileSharing userName=\"Deniz Yılmaz\"/><workbookPr defaultThemeVersion=\"166925\"/><mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice Requires=\"x15\"><x15ac:absPath url=\"C:\\Users\\deniz\\Desktop\\\" xmlns:x15ac=\"http://schemas.microsoft.com/office/spreadsheetml/2010/11/ac\"/></mc:Choice></mc:AlternateContent><xr:revisionPtr revIDLastSave=\"0\" documentId=\"8_{AB12CD34-0000-1111-2222-333344445555}\"/><bookViews><workbookView xWindow=\"0\" yWindow=\"0\" windowWidth=\"20000\" windowHeight=\"10000\"/></bookViews><sheets><sheet name=\"Bütçe\" sheetId=\"1\" r:id=\"rId1\"/></sheets><externalReferences><externalReference r:id=\"rId4\"/></externalReferences></workbook>");
    let sheet = x("<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheetData><row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Toplam</t></is></c></row></sheetData><pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/><pageSetup paperSize=\"9\" orientation=\"landscape\" r:id=\"rId3\"/><legacyDrawing r:id=\"rId2\"/></worksheet>");
    let comments = x(&format!("<comments xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><authors><author>tc={{7F2C0000-0000-0000-0000-000000000001}}</author><author>{p}</author></authors><commentList><comment ref=\"A1\" authorId=\"1\"><text><t>Kontrol et</t></text></comment></commentList></comments>", p = PERSON));
    let threaded = x("<ThreadedComments xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments\"><threadedComment ref=\"A1\" dT=\"2026-10-28T09:00:00.00\" personId=\"{0A0A0A0A-0000-0000-0000-000000000001}\" id=\"{7F2C0000-0000-0000-0000-000000000001}\"><text>Kontrol et</text></threadedComment></ThreadedComments>");
    let persons = x(&format!("<personList xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments\"><person displayName=\"{p}\" id=\"{{0A0A0A0A-0000-0000-0000-000000000001}}\" userId=\"{a}\" providerId=\"AD\"/></personList>", p = PERSON, a = ACCOUNT));
    let vml = b"<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\"><v:shape id=\"_x0000_s1025\" type=\"#_x0000_t202\"><x:ClientData ObjectType=\"Note\"><x:Row>0</x:Row><x:Column>0</x:Column></x:ClientData></v:shape></xml>".to_vec();
    let external = x("<externalLink xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><externalBook r:id=\"rId1\"><sheetNames><sheetName val=\"Sheet1\"/></sheetNames></externalBook></externalLink>");
    let ct = content_types(&[
        ("xl/workbook.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"),
        ("xl/worksheets/sheet1.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"),
        ("xl/comments1.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"),
        ("xl/threadedComments/threadedComment1.xml", "application/vnd.ms-excel.threadedcomments+xml"),
        ("xl/persons/person.xml", "application/vnd.ms-excel.person+xml"),
        ("xl/printerSettings/printerSettings1.bin", "application/vnd.openxmlformats-officedocument.spreadsheetml.printerSettings"),
        ("xl/externalLinks/externalLink1.xml", "application/vnd.openxmlformats-officedocument.spreadsheetml.externalLink+xml"),
        ("docProps/core.xml", CT_CORE),
        ("docProps/app.xml", CT_APP),
    ]);
    zip_bytes(
        &[
            ("[Content_Types].xml", ct),
            ("_rels/.rels", rels(&[("rId1", T_DOC, "xl/workbook.xml", false), ("rId2", T_CORE, "docProps/core.xml", false), ("rId3", T_APP, "docProps/app.xml", false)])),
            ("xl/workbook.xml", workbook),
            (
                "xl/_rels/workbook.xml.rels",
                rels(&[
                    ("rId1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet", "worksheets/sheet1.xml", false),
                    ("rId2", "http://schemas.microsoft.com/office/2017/10/relationships/person", "persons/person.xml", false),
                    ("rId4", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLink", "externalLinks/externalLink1.xml", false),
                ]),
            ),
            ("xl/worksheets/sheet1.xml", sheet),
            (
                "xl/worksheets/_rels/sheet1.xml.rels",
                rels(&[
                    ("rId1", T_COMMENTS, "../comments1.xml", false),
                    ("rId2", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing", "../drawings/vmlDrawing1.vml", false),
                    ("rId3", T_PRINTER, "../printerSettings/printerSettings1.bin", false),
                    ("rId4", "http://schemas.microsoft.com/office/2017/10/relationships/threadedComment", "../threadedComments/threadedComment1.xml", false),
                ]),
            ),
            ("xl/comments1.xml", comments),
            ("xl/threadedComments/threadedComment1.xml", threaded),
            ("xl/persons/person.xml", persons),
            ("xl/drawings/vmlDrawing1.vml", vml),
            ("xl/printerSettings/printerSettings1.bin", devmode()),
            ("xl/externalLinks/externalLink1.xml", external),
            ("xl/externalLinks/_rels/externalLink1.xml.rels", rels(&[("rId1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/externalLinkPath", "file:///C:\\Users\\deniz\\Documents\\Budget%202026.xlsx", true)])),
            ("docProps/core.xml", core_xml()),
            ("docProps/app.xml", app_xml("")),
        ],
        "Created by Deniz",
    )
}

pub fn pptx() -> Vec<u8> {
    let p = "xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"";
    let presentation = x(&format!("<p:presentation {p}><p:sldIdLst><p:sldId id=\"256\" r:id=\"rId2\"/></p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>", p = p));
    let slide = x(&format!("<p:sld {p}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Title 1\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"title\"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>Strateji</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr><p:extLst><p:ext uri=\"{{6950BFC3-D8DA-4A85-94F7-54DA5524770B}}\"><p188:commentRel xmlns:p188=\"http://schemas.microsoft.com/office/powerpoint/2018/8/main\" r:id=\"rId3\"/></p:ext></p:extLst></p:sld>", p = p));
    let notes = x(&format!("<p:notes {p}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id=\"3\" name=\"Notes Placeholder 2\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"body\" idx=\"1\"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:r><a:t>Müşteriye indirim oranını söyleme</a:t></a:r></a:p></p:txBody></p:sp><p:sp><p:nvSpPr><p:cNvPr id=\"4\" name=\"Slide Number\"/><p:cNvSpPr/><p:nvPr><p:ph type=\"sldNum\" idx=\"5\"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:p><a:fld id=\"{{1}}\" type=\"slidenum\"><a:t>1</a:t></a:fld></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:notes>", p = p));
    let legacy = x(&format!("<p:cmLst {p}><p:cm authorId=\"0\" dt=\"2026-10-28T09:00:00.000\" idx=\"1\"><p:pos x=\"10\" y=\"10\"/><p:text>Bu slaytı kısalt</p:text></p:cm></p:cmLst>", p = p));
    let authors_legacy = x(&format!("<p:cmAuthorLst {p}><p:cmAuthor id=\"0\" name=\"{n}\" initials=\"KY\" lastIdx=\"1\" clrIdx=\"0\"/></p:cmAuthorLst>", p = p, n = PERSON));
    let modern = x("<p188:cmLst xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:p188=\"http://schemas.microsoft.com/office/powerpoint/2018/8/main\"><p188:cm id=\"{AAAA0000-0000-0000-0000-000000000001}\" authorId=\"{BBBB0000-0000-0000-0000-000000000001}\" created=\"2026-10-28T09:00:00.000\"><p188:txBody><a:bodyPr/><a:p><a:r><a:t>Görseli değiştir</a:t></a:r></a:p></p188:txBody></p188:cm></p188:cmLst>");
    let authors = x(&format!("<p188:authorLst xmlns:p188=\"http://schemas.microsoft.com/office/powerpoint/2018/8/main\"><p188:author id=\"{{BBBB0000-0000-0000-0000-000000000001}}\" name=\"{n}\" initials=\"KY\" userId=\"{a}\" providerId=\"AD\"/></p188:authorLst>", n = PERSON, a = ACCOUNT));
    let revinfo = x("<p1510:revInfo xmlns:p1510=\"http://schemas.microsoft.com/office/powerpoint/2015/10/main\"><p1510:revLst><p1510:client id=\"{CCCC0000-0000-0000-0000-000000000001}\" v=\"2\"/></p1510:revLst></p1510:revInfo>");
    let ct = content_types(&[
        ("ppt/presentation.xml", "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"),
        ("ppt/slides/slide1.xml", "application/vnd.openxmlformats-officedocument.presentationml.slide+xml"),
        ("ppt/notesSlides/notesSlide1.xml", "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml"),
        ("ppt/comments/comment1.xml", "application/vnd.openxmlformats-officedocument.presentationml.comments+xml"),
        ("ppt/commentAuthors.xml", "application/vnd.openxmlformats-officedocument.presentationml.commentAuthors+xml"),
        ("ppt/comments/modernComment_1.xml", "application/vnd.ms-powerpoint.comments+xml"),
        ("ppt/authors.xml", "application/vnd.ms-powerpoint.authors+xml"),
        ("ppt/revisionInfo.xml", "application/vnd.ms-powerpoint.revisioninfo+xml"),
        ("docProps/core.xml", CT_CORE),
        ("docProps/app.xml", CT_APP),
    ]);
    zip_bytes(
        &[
            ("[Content_Types].xml", ct),
            ("_rels/.rels", rels(&[("rId1", T_DOC, "ppt/presentation.xml", false), ("rId2", T_CORE, "docProps/core.xml", false), ("rId3", T_APP, "docProps/app.xml", false)])),
            ("ppt/presentation.xml", presentation),
            (
                "ppt/_rels/presentation.xml.rels",
                rels(&[
                    ("rId2", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide", "slides/slide1.xml", false),
                    ("rId5", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/commentAuthors", "commentAuthors.xml", false),
                    ("rId6", "http://schemas.microsoft.com/office/2018/10/relationships/authors", "authors.xml", false),
                    ("rId7", "http://schemas.microsoft.com/office/2015/10/relationships/revisionInfo", "revisionInfo.xml", false),
                ]),
            ),
            ("ppt/slides/slide1.xml", slide),
            (
                "ppt/slides/_rels/slide1.xml.rels",
                rels(&[
                    ("rId1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide", "../notesSlides/notesSlide1.xml", false),
                    ("rId2", T_COMMENTS, "../comments/comment1.xml", false),
                    ("rId3", "http://schemas.microsoft.com/office/2018/10/relationships/comments", "../comments/modernComment_1.xml", false),
                    ("rId4", T_OLE, "../embeddings/oleObject1.bin", false),
                ]),
            ),
            ("ppt/notesSlides/notesSlide1.xml", notes),
            ("ppt/notesSlides/_rels/notesSlide1.xml.rels", rels(&[("rId1", "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide", "../slides/slide1.xml", false)])),
            ("ppt/comments/comment1.xml", legacy),
            ("ppt/commentAuthors.xml", authors_legacy),
            ("ppt/comments/modernComment_1.xml", modern),
            ("ppt/authors.xml", authors),
            ("ppt/revisionInfo.xml", revinfo),
            ("ppt/embeddings/oleObject1.bin", b"\xD0\xCF\x11\xE0 embedded chart".to_vec()),
            ("docProps/core.xml", core_xml()),
            ("docProps/app.xml", app_xml("")),
        ],
        "",
    )
}

pub fn odt() -> Vec<u8> {
    let ns = "xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" xmlns:config=\"urn:oasis:names:tc:opendocument:xmlns:config:1.0\" xmlns:officeooo=\"http://openoffice.org/2009/office\" xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\"";
    let meta = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-meta {ns} office:version=\"1.3\"><office:meta><meta:initial-creator>{p}</meta:initial-creator><meta:creation-date>2026-10-01T09:00:00.000000000</meta:creation-date><dc:date>2026-10-28T18:30:00.000000000</dc:date><dc:creator>Ayşe Demir</dc:creator><meta:editing-duration>PT2H13M</meta:editing-duration><meta:editing-cycles>14</meta:editing-cycles><meta:generator>LibreOffice/24.8.2.1$Windows_X86_64 LibreOffice_project/0a1b2c</meta:generator><meta:printed-by>{p}</meta:printed-by><meta:print-date>2026-10-27T08:00:00</meta:print-date><meta:template xlink:type=\"simple\" xlink:actuate=\"onRequest\" xlink:title=\"Kurumsal\" xlink:href=\"file:///C:/Users/deniz/Templates/Kurumsal.ott\"/><dc:title>Teklif</dc:title><meta:document-statistic meta:page-count=\"1\" meta:word-count=\"12\"/><meta:user-defined meta:name=\"Client\">ACME Holding</meta:user-defined></office:meta></office:document-meta>", ns = ns, p = PERSON);
    let content = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content {ns} office:version=\"1.3\"><office:automatic-styles><style:style style:name=\"P1\" style:family=\"paragraph\"><style:text-properties officeooo:rsid=\"0012ab34\" officeooo:paragraph-rsid=\"0012ab34\"/></style:style></office:automatic-styles><office:body><office:text><text:tracked-changes><text:changed-region text:id=\"ct1\"><text:insertion><office:change-info><dc:creator>{p}</dc:creator><dc:date>2026-10-28T10:00:00</dc:date></office:change-info></text:insertion></text:changed-region></text:tracked-changes><text:p text:style-name=\"P1\"><office:annotation office:name=\"__Annotation__1\"><dc:creator>{p}</dc:creator><dc:date>2026-10-28T09:00:00</dc:date><meta:creator-initials>KY</meta:creator-initials><text:p>Bunu kontrol et</text:p></office:annotation>Fiyat <text:change-start text:change-id=\"ct1\"/>yeni<text:change-end text:change-id=\"ct1\"/> teklif<office:annotation-end office:name=\"__Annotation__1\"/> <text:a xlink:type=\"simple\" xlink:href=\"file:///C:/Users/deniz/Desktop/brief.pdf\">brief</text:a></text:p><text:p><draw:frame draw:name=\"Image1\"><draw:image xlink:href=\"Pictures/photo.jpg\" xlink:type=\"simple\"/></draw:frame></text:p></office:text></office:body></office:document-content>", ns = ns, p = PERSON);
    let styles = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles {ns} office:version=\"1.3\"><office:styles/></office:document-styles>", ns = ns);
    let settings = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-settings {ns} office:version=\"1.3\"><office:settings><config:config-item-set config:name=\"ooo:configuration-settings\"><config:config-item config:name=\"PrinterName\" config:type=\"string\">\\\\CORP-PRN01\\HP LaserJet</config:config-item><config:config-item config:name=\"PrinterSetup\" config:type=\"base64Binary\">AAAAAQAAAAIAAAAD</config:config-item><config:config-item config:name=\"Rsid\" config:type=\"int\">1218356</config:config-item><config:config-item config:name=\"RsidRoot\" config:type=\"int\">1218356</config:config-item><config:config-item config:name=\"EmbedFonts\" config:type=\"boolean\">false</config:config-item></config:config-item-set></office:settings></office:document-settings>", ns = ns);
    let manifest = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\"><manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"Thumbnails/thumbnail.png\" manifest:media-type=\"image/png\"/><manifest:file-entry manifest:full-path=\"Pictures/photo.jpg\" manifest:media-type=\"image/jpeg\"/><manifest:file-entry manifest:full-path=\"settings.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/></manifest:manifest>";
    zip_bytes(
        &[
            ("mimetype", b"application/vnd.oasis.opendocument.text".to_vec()),
            ("Thumbnails/thumbnail.png", dirty_png()),
            ("Pictures/photo.jpg", dirty_jpeg()),
            ("settings.xml", settings.into_bytes()),
            ("content.xml", content.into_bytes()),
            ("meta.xml", meta.into_bytes()),
            ("styles.xml", styles.into_bytes()),
            ("META-INF/manifest.xml", manifest.as_bytes().to_vec()),
        ],
        "",
    )
}

fn s(text: &str) -> Object {
    Object::String(text.as_bytes().to_vec(), StringFormat::Literal)
}

fn xmp_stream(body: &str) -> Stream {
    Stream::new(dictionary! { "Type" => "Metadata", "Subtype" => "XML" }, format!("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" {}</rdf:Description></rdf:RDF></x:xmpmeta>", body).into_bytes()).with_compression(false)
}

/// PDF with an Info dictionary, document- and page-level XMP, /ID, PieceInfo, LastModified,
/// a sticky note with an author, a form field, an EXIF-laden JPEG, JavaScript and an attachment.
pub fn pdf_document() -> Document {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let xmp_doc = doc.add_object(xmp_stream(&format!("xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" xmlns:xmpMM=\"http://ns.adobe.com/xap/1.0/mm/\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\" xmp:CreatorTool=\"Microsoft Word\" xmpMM:DocumentID=\"uuid:1111\" pdfaid:part=\"2\"><dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>", PERSON)));
    let xmp_page = doc.add_object(xmp_stream("xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" xmp:CreatorTool=\"Adobe Illustrator\">"));
    let image = doc.add_object(Stream::new(dictionary! { "Type" => "XObject", "Subtype" => "Image", "Width" => 64, "Height" => 48, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode" }, dirty_jpeg()).with_compression(false));
    let content = doc.add_object(Stream::new(dictionary! {}, b"q 64 0 0 48 10 10 cm /Im1 Do Q".to_vec()).with_compression(false));
    let note = doc.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Text", "Rect" => vec![10.into(), 10.into(), 30.into(), 30.into()], "Contents" => s("Rakamı kontrol et"), "T" => s("Deniz Y"), "M" => s("D:20261028090000+03'00'"), "CreationDate" => s("D:20261028090000+03'00'") });
    let field = doc.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Widget", "FT" => "Tx", "T" => s("musteri_adi"), "Rect" => vec![40.into(), 40.into(), 140.into(), 60.into()] });
    let page = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 200.into(), 100.into()],
        "Contents" => content,
        "Resources" => dictionary! { "XObject" => dictionary! { "Im1" => image } },
        "Annots" => vec![note.into(), field.into()],
        "Metadata" => xmp_page,
        "PieceInfo" => dictionary! { "Illustrator" => dictionary! { "Private" => s("C:\\Users\\deniz\\art.ai") } },
        "LastModified" => s("D:20261028090000"),
    });
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }));
    let js = doc.add_object(dictionary! { "S" => "JavaScript", "JS" => s("app.alert('hi');") });
    let attachment = doc.add_object(Stream::new(dictionary! { "Type" => "EmbeddedFile" }, b"budget numbers".to_vec()).with_compression(false));
    let filespec = doc.add_object(dictionary! { "Type" => "Filespec", "F" => s("ek.txt"), "UF" => s("ek.txt"), "EF" => dictionary! { "F" => attachment } });
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
        "Metadata" => xmp_doc,
        "OpenAction" => js,
        "Names" => dictionary! { "EmbeddedFiles" => dictionary! { "Names" => vec![s("ek.txt"), filespec.into()] } },
    });
    let info = doc.add_object(dictionary! {
        "Title" => s("Teklif 2026"),
        "Author" => s("Deniz Y"),
        "Creator" => s("Microsoft Word for Microsoft 365"),
        "Producer" => s("Microsoft Word for Microsoft 365"),
        "CreationDate" => s("D:20261001090000+03'00'"),
        "ModDate" => s("D:20261028183000+03'00'"),
        "Company" => s("Seobify Ajans"),
    });
    doc.trailer.set("Root", catalog);
    doc.trailer.set("Info", info);
    doc.trailer.set("ID", vec![Object::String(vec![0x11; 16], StringFormat::Hexadecimal), Object::String(vec![0x22; 16], StringFormat::Hexadecimal)]);
    doc
}

/// Classic-xref PDF with one incremental update that rewrites the Info dictionary.
pub fn pdf_with_update() -> Vec<u8> {
    let mut doc = pdf_document();
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    let text = String::from_utf8_lossy(&out).to_string();
    let prev: usize = text.rsplit("startxref").next().unwrap().split_whitespace().next().unwrap().parse().unwrap();
    let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap().0;
    let id = doc.max_id + 1;
    out.push(b'\n');
    let offset = out.len();
    out.extend_from_slice(format!("{} 0 obj\n<< /Title (Taslak v1) /Author (Deniz Y) /ModDate (D:20261020090000) >>\nendobj\n", id).as_bytes());
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 1\n0000000000 65535 f \n{} 1\n{:010} 00000 n \ntrailer\n<< /Size {} /Root {} 0 R /Info {} 0 R /Prev {} /ID [<1111111111111111><2222222222222222>] >>\nstartxref\n{}\n%%EOF\n", id, offset, id + 1, root, id, prev, xref).as_bytes());
    out
}

/// Same document written with object streams and a cross-reference stream (PDF 1.5+).
pub fn pdf_modern() -> Vec<u8> {
    let mut doc = pdf_document();
    let mut out = Vec::new();
    doc.save_modern(&mut out).unwrap();
    out
}

/// Raw bytes plus every decodable stream body, for "the name is gone everywhere" checks.
pub fn pdf_all_text(data: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(data).to_string();
    if let Ok(doc) = Document::load_mem(data) {
        for obj in doc.objects.values() {
            if let Object::Stream(s) = obj {
                if let Ok(c) = s.decompressed_content() {
                    text.push_str(&String::from_utf8_lossy(&c));
                }
            }
        }
    }
    text
}

// ---------------------------------------------------------------------------
// Consistency checks
// ---------------------------------------------------------------------------

pub fn zip_entries(data: &[u8]) -> Vec<(String, Vec<u8>, CompressionMethod, DateTime)> {
    let mut a = ZipArchive::new(Cursor::new(data)).unwrap();
    (0..a.len())
        .map(|i| {
            let mut f = a.by_index(i).unwrap();
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut buf).unwrap();
            (f.name().unwrap().to_string(), buf, f.compression(), f.last_modified().unwrap_or_default())
        })
        .collect()
}

pub fn part(data: &[u8], name: &str) -> Option<String> {
    zip_entries(data).into_iter().find(|(n, ..)| n == name).map(|(_, d, ..)| String::from_utf8_lossy(&d).to_string())
}

/// Every XML part parses; every internal relationship target and content-type override
/// exists; every part has a content type; every `r:*` attribute resolves to a relationship
/// of its own part; every part is reachable from the package relationships; timestamps are normalised.
pub fn assert_ooxml_consistent(data: &[u8]) {
    let entries = zip_entries(data);
    assert_eq!(entries[0].0, "[Content_Types].xml", "content types first");
    let names: HashSet<String> = entries.iter().map(|e| e.0.to_ascii_lowercase()).collect();
    let mut docs: HashMap<String, XmlDoc> = HashMap::new();
    for (name, bytes, _, when) in &entries {
        assert_eq!(*when, DateTime::DEFAULT, "{} timestamp normalised", name);
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".xml") || lower.ends_with(".rels") {
            docs.insert(name.clone(), XmlDoc::parse(bytes).unwrap_or_else(|e| panic!("{} does not parse: {}", name, e)));
        }
    }
    let ct = &docs["[Content_Types].xml"];
    let mut defaults = HashSet::new();
    let mut overrides = HashSet::new();
    for e in ct.root().elements() {
        match e.local.as_str() {
            "Default" => {
                defaults.insert(e.attr("", "Extension").unwrap().to_ascii_lowercase());
            }
            "Override" => {
                let p = e.attr("", "PartName").unwrap().trim_start_matches('/').to_ascii_lowercase();
                assert!(names.contains(&p), "override for missing part {}", p);
                overrides.insert(p);
            }
            _ => {}
        }
    }
    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    for (name, doc) in &docs {
        let Some(source) = package::rels_source(name) else { continue };
        if !source.is_empty() {
            assert!(names.contains(&source.to_ascii_lowercase()), "rels {} without source part", name);
        }
        let mut ids = HashSet::new();
        for r in doc.root().elements() {
            let id = r.attr("", "Id").unwrap();
            assert!(ids.insert(id.clone()), "duplicate rel id {} in {}", id, name);
            if r.attr("", "TargetMode").as_deref() == Some("External") {
                continue;
            }
            let target = package::resolve_target(&source, &r.attr("", "Target").unwrap()).to_ascii_lowercase();
            assert!(names.contains(&target), "{} → missing {}", name, target);
            graph.entry(source.to_ascii_lowercase()).or_default().push(target);
        }
        if let Some(src) = docs.get(&source) {
            let root = src.root();
            for el in std::iter::once(root).chain(root.descendants()) {
                for a in el.attrs.iter().filter(|a| a.ns == R) {
                    assert!(ids.contains(&super::xml::unescape(&a.value)), "{}: dangling r:{}={} on <{}>", source, a.local, a.value, el.name);
                }
            }
        }
    }
    let mut reachable = HashSet::new();
    let mut queue = VecDeque::from([String::new()]);
    while let Some(p) = queue.pop_front() {
        for t in graph.get(&p).cloned().unwrap_or_default() {
            if reachable.insert(t.clone()) {
                queue.push_back(t);
            }
        }
    }
    for (name, ..) in &entries {
        let lower = name.to_ascii_lowercase();
        if lower == "[content_types].xml" || lower.ends_with('/') {
            continue;
        }
        let ext = lower.rsplit('.').next().unwrap_or("");
        assert!(overrides.contains(&lower) || defaults.contains(ext), "no content type for {}", name);
        assert!(lower.ends_with(".rels") || reachable.contains(&lower), "{} is unreachable", name);
    }
}

/// `mimetype` first and stored; manifest and archive list the same files.
pub fn assert_odf_consistent(data: &[u8]) {
    let entries = zip_entries(data);
    assert_eq!(entries[0].0, "mimetype");
    assert_eq!(entries[0].2, CompressionMethod::Stored);
    for (name, bytes, _, when) in &entries {
        assert_eq!(*when, DateTime::DEFAULT, "{} timestamp normalised", name);
        if name.ends_with(".xml") {
            XmlDoc::parse(bytes).unwrap_or_else(|e| panic!("{} does not parse: {}", name, e));
        }
    }
    let manifest = XmlDoc::parse(&entries.iter().find(|e| e.0 == "META-INF/manifest.xml").unwrap().1).unwrap();
    let listed: HashSet<String> = manifest.root().elements().filter_map(|e| e.attr("urn:oasis:names:tc:opendocument:xmlns:manifest:1.0", "full-path")).filter(|p| p != "/").collect();
    let files: HashSet<String> = entries.iter().map(|e| e.0.clone()).filter(|n| n != "mimetype" && !n.starts_with("META-INF/") && !n.ends_with('/')).collect();
    assert_eq!(listed, files, "manifest lists exactly the stored files");
}
