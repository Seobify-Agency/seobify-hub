//! Branded, self-contained HTML reports (print-friendly, light theme) and CSV export.
//! Every report carries the Seobify brand stamp: product, version, generation time and a
//! `<meta name="generator">` tag, so deliverables can be traced back to the tool.

use super::text::html_escape;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Block {
    Heading { text: String },
    Paragraph { text: String },
    /// Key figures shown as tiles: `(label, value)`.
    Stats { items: Vec<(String, String)> },
    Table { headers: Vec<String>, rows: Vec<Vec<String>> },
    /// Bulleted findings with a severity: `critical | high | medium | low | info`.
    Findings { items: Vec<(String, String)> },
    /// Trusted HTML produced by the tool itself (e.g. an inline SVG chart). Never user input.
    RawHtml { html: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub title: String,
    pub subtitle: Option<String>,
    /// Report language for labels: `tr` or `en`.
    pub lang: String,
    pub blocks: Vec<Block>,
    /// The one-line expert-support link under the report (Settings → About can turn it off).
    pub support_line: bool,
}

impl Report {
    pub fn new(title: impl Into<String>, lang: &str) -> Self {
        Report { title: title.into(), subtitle: None, lang: if lang == "en" { "en".into() } else { "tr".into() }, blocks: Vec::new(), support_line: true }
    }
    pub fn subtitle(mut self, s: impl Into<String>) -> Self {
        self.subtitle = Some(s.into());
        self
    }
    pub fn push(&mut self, block: Block) -> &mut Self {
        self.blocks.push(block);
        self
    }
}

const CSS: &str = r#"
@page { margin: 16mm 14mm; }
* { box-sizing: border-box; }
body { margin: 0; font-family: "Quicksand", "Segoe UI", Arial, sans-serif; color: #1c1c1c; background: #fff; line-height: 1.5; }
.wrap { max-width: 1040px; margin: 0 auto; padding: 32px 28px 48px; }
header.r { display: flex; justify-content: space-between; align-items: flex-end; gap: 16px; border-bottom: 3px solid #c8a74d; padding-bottom: 14px; margin-bottom: 24px; }
.brand { font-weight: 700; letter-spacing: .22em; font-size: 12px; color: #8a6d1f; text-transform: uppercase; }
h1 { font-size: 26px; margin: 4px 0 0; }
.sub { color: #666; font-size: 14px; margin-top: 4px; }
.meta { text-align: right; color: #777; font-size: 12px; }
h2 { font-size: 18px; margin: 28px 0 10px; padding-left: 10px; border-left: 4px solid #c8a74d; }
p { margin: 8px 0; }
.stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 10px; margin: 12px 0; }
.stat { border: 1px solid #ead9a8; background: #fdf8ea; border-radius: 10px; padding: 10px 12px; }
.stat .v { font-size: 22px; font-weight: 700; }
.stat .l { font-size: 12px; color: #6b5a2a; text-transform: uppercase; letter-spacing: .05em; }
table { width: 100%; border-collapse: collapse; font-size: 12.5px; margin: 10px 0; }
th { text-align: left; background: #1c1c1c; color: #f3e3b3; font-weight: 600; padding: 7px 8px; }
td { border-bottom: 1px solid #eee; padding: 6px 8px; vertical-align: top; word-break: break-word; }
tr:nth-child(even) td { background: #fafafa; }
ul.findings { list-style: none; padding: 0; margin: 10px 0; }
ul.findings li { padding: 8px 10px; margin: 6px 0; border-radius: 8px; border-left: 5px solid #999; background: #f7f7f7; }
li.critical { border-color: #b91c1c !important; background: #fef2f2 !important; }
li.high { border-color: #ea580c !important; background: #fff7ed !important; }
li.medium { border-color: #ca8a04 !important; background: #fefce8 !important; }
li.low { border-color: #2563eb !important; background: #eff6ff !important; }
li.info { border-color: #6b7280 !important; }
.sev { font-size: 11px; font-weight: 700; text-transform: uppercase; margin-right: 8px; }
p.support { margin-top: 32px; color: #777; font-size: 12px; }
p.support a { color: inherit; }
footer.r { margin-top: 40px; border-top: 1px solid #ddd; padding-top: 10px; color: #888; font-size: 11px; display: flex; justify-content: space-between; }
@media print { .wrap { padding: 0; } h2 { break-after: avoid; } tr { break-inside: avoid; } }
"#;

fn severity_label(sev: &str, lang: &str) -> &'static str {
    match (sev, lang == "en") {
        ("critical", true) => "Critical",
        ("critical", false) => "Kritik",
        ("high", true) => "High",
        ("high", false) => "Yüksek",
        ("medium", true) => "Medium",
        ("medium", false) => "Orta",
        ("low", true) => "Low",
        ("low", false) => "Düşük",
        (_, true) => "Info",
        (_, false) => "Bilgi",
    }
}

/// One quiet line under every report pointing to Seobify. The link carries a source tag only.
fn support_line(lang: &str) -> String {
    let source = crate::hub::text::slugify(crate::PRODUCT_NAME);
    let (text, url, label) = if lang == "en" {
        ("Need help applying these findings?", "https://seobify.com/en/contact/", "Talk to Seobify")
    } else {
        ("Bulguları sitenizde uygulamak için destek isterseniz:", "https://seobify.com/iletisim/", "Seobify ile görüşün")
    };
    format!(
        "<p class=\"support\">{} <a href=\"{url}?utm_source={source}&amp;utm_medium=report&amp;utm_campaign=seobify-hub\">{}</a></p>\n",
        html_escape(text),
        html_escape(label)
    )
}

pub fn render(report: &Report) -> String {
    let now = chrono::Local::now();
    let stamp = format!("{} v{}", crate::PRODUCT_NAME, crate::VERSION);
    let generated = if report.lang == "en" { "Generated" } else { "Oluşturulma" };
    let mut body = String::new();
    for block in &report.blocks {
        match block {
            Block::Heading { text } => body.push_str(&format!("<h2>{}</h2>\n", html_escape(text))),
            Block::Paragraph { text } => body.push_str(&format!("<p>{}</p>\n", html_escape(text))),
            Block::Stats { items } => {
                body.push_str("<div class=\"stats\">");
                for (label, value) in items {
                    body.push_str(&format!("<div class=\"stat\"><div class=\"v\">{}</div><div class=\"l\">{}</div></div>", html_escape(value), html_escape(label)));
                }
                body.push_str("</div>\n");
            }
            Block::Table { headers, rows } => {
                body.push_str("<table><thead><tr>");
                for h in headers {
                    body.push_str(&format!("<th>{}</th>", html_escape(h)));
                }
                body.push_str("</tr></thead><tbody>");
                for row in rows {
                    body.push_str("<tr>");
                    for cell in row {
                        body.push_str(&format!("<td>{}</td>", html_escape(cell)));
                    }
                    body.push_str("</tr>");
                }
                body.push_str("</tbody></table>\n");
            }
            Block::Findings { items } => {
                body.push_str("<ul class=\"findings\">");
                for (sev, text) in items {
                    let sev_class = match sev.as_str() {
                        "critical" | "high" | "medium" | "low" => sev.as_str(),
                        _ => "info",
                    };
                    body.push_str(&format!(
                        "<li class=\"{sev_class}\"><span class=\"sev\">{}</span>{}</li>",
                        severity_label(sev_class, &report.lang),
                        html_escape(text)
                    ));
                }
                body.push_str("</ul>\n");
            }
            Block::RawHtml { html } => {
                body.push_str(html);
                body.push('\n');
            }
        }
    }
    format!(
        "<!doctype html>\n<html lang=\"{lang}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta name=\"generator\" content=\"{stamp_e} (Seobify Hub)\"><title>{title}</title><style>{CSS}</style></head><body><div class=\"wrap\">\
<header class=\"r\"><div><div class=\"brand\">Seobify · {product}</div><h1>{title}</h1>{sub}</div>\
<div class=\"meta\">{generated}: {date}<br>{stamp_e}</div></header>\n{body}{support}\
<footer class=\"r\"><span>{stamp_e} · seobify.com</span><span>{date}</span></footer></div></body></html>\n",
        lang = report.lang,
        support = if report.support_line { support_line(&report.lang) } else { String::new() },
        stamp_e = html_escape(&stamp),
        title = html_escape(&report.title),
        product = html_escape(crate::PRODUCT_NAME),
        sub = report.subtitle.as_deref().map(|s| format!("<div class=\"sub\">{}</div>", html_escape(s))).unwrap_or_default(),
        date = now.format("%Y-%m-%d %H:%M"),
    )
}

/// The user's choice from Settings → About (`reportSupportLine`, on unless turned off).
fn support_line_enabled() -> bool {
    super::db::open().ok().and_then(|c| super::db::get_settings(&c).ok()).and_then(|s| s.get("reportSupportLine").and_then(|v| v.as_bool())).unwrap_or(true)
}

/// Writes the report to `reports_dir()/<slug>-<timestamp>.html` (or `dest` when given).
pub fn save(report: &Report, dest: Option<&Path>) -> Result<PathBuf, String> {
    let path = match dest {
        Some(p) => p.to_path_buf(),
        None => {
            let slug = super::text::slugify(&report.title);
            let slug = if slug.is_empty() { "report".to_string() } else { slug };
            super::paths::reports_dir().join(format!("{slug}-{}.html", chrono::Local::now().format("%Y%m%d-%H%M%S")))
        }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut report = report.clone();
    report.support_line = report.support_line && support_line_enabled();
    std::fs::write(&path, render(&report)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Writes a UTF-8 CSV with BOM (opens correctly in Excel with Turkish characters).
pub fn write_csv(path: &Path, headers: &[&str], rows: &[Vec<String>]) -> Result<(), String> {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(headers).map_err(|e| e.to_string())?;
    for row in rows {
        wtr.write_record(row).map_err(|e| e.to_string())?;
    }
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend(wtr.into_inner().map_err(|e| e.to_string())?);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_escaped_branded_html() {
        let mut r = Report::new("Test <Rapor>", "tr").subtitle("örnek.com");
        r.push(Block::Stats { items: vec![("URL".into(), "12".into())] })
            .push(Block::Table { headers: vec!["A".into()], rows: vec![vec!["<b>x</b>".into()]] })
            .push(Block::Findings { items: vec![("critical".into(), "noindex eklendi".into())] });
        let html = render(&r);
        assert!(html.contains("Test &lt;Rapor&gt;"));
        assert!(html.contains("&lt;b&gt;x&lt;/b&gt;"));
        assert!(html.contains("<meta name=\"generator\""));
        assert!(html.contains("Kritik"));
        assert!(html.contains(crate::PRODUCT_NAME));
    }

    #[test]
    fn support_line_once_with_source_tag_only() {
        let source = crate::hub::text::slugify(crate::PRODUCT_NAME);
        let tr = render(&Report::new("Rapor", "tr"));
        assert_eq!(tr.matches("class=\"support\"").count(), 1);
        assert!(tr.contains(&format!("https://seobify.com/iletisim/?utm_source={source}&amp;utm_medium=report&amp;utm_campaign=seobify-hub")));
        let en = render(&Report::new("Report", "en"));
        assert!(en.contains("https://seobify.com/en/contact/?utm_source=") && en.contains("Talk to Seobify"));
    }

    #[test]
    fn support_line_can_be_turned_off() {
        let mut r = Report::new("Rapor", "tr");
        r.support_line = false;
        assert!(!render(&r).contains("class=\"support\""));
    }

    #[test]
    fn csv_has_bom() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.csv");
        write_csv(&p, &["url", "başlık"], &[vec!["https://a.com".into(), "Çay".into()]]).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF]);
        assert!(String::from_utf8_lossy(&bytes).contains("Çay"));
    }
}
