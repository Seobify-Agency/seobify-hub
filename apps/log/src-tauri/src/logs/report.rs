//! Branded HTML report for a date range (summary, SVG chart, findings, tables).

use super::bots::Category;
use super::query::{self, Range, UrlFilter};
use super::rules::{self, Finding, Metrics};
use super::waste::{self, Scope};
use crate::hub::db::Project;
use crate::hub::report::{Block, Report};
use crate::hub::text::html_escape;
use rusqlite::Connection;
use serde_json::Value;

pub struct SpiderPart {
    pub meta: Value,
    pub gaps: Value,
    pub pages: Vec<super::gaps::SpiderPage>,
}

fn n(v: &Value) -> i64 {
    v.as_i64().unwrap_or(0)
}

fn s(v: &Value) -> String {
    match v {
        Value::String(x) => x.clone(),
        Value::Null => "—".into(),
        other => other.to_string(),
    }
}

fn num(x: i64, lang: &str) -> String {
    let raw = x.abs().to_string();
    let sep = if lang == "en" { ',' } else { '.' };
    let mut out = String::new();
    for (i, c) in raw.chars().enumerate() {
        if i > 0 && (raw.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(c);
    }
    if x < 0 { format!("-{out}") } else { out }
}

fn pct(x: f64, lang: &str) -> String {
    if lang == "en" { format!("{:.1}%", x * 100.0) } else { format!("%{}", format!("{:.1}", x * 100.0).replace('.', ",")) }
}

fn tr<'a>(lang: &str, tr: &'a str, en: &'a str) -> &'a str {
    if lang == "en" { en } else { tr }
}

fn waste_label(key: &str, lang: &str) -> &'static str {
    let en = lang == "en";
    match key {
        "parameters" => if en { "Parameterised URLs" } else { "Parametreli URL'ler" },
        "redirects" => if en { "Redirects (3xx)" } else { "Yönlendirmeler (3xx)" },
        "client_errors" => if en { "Client errors (4xx)" } else { "İstemci hataları (4xx)" },
        "server_errors" => if en { "Server errors (5xx)" } else { "Sunucu hataları (5xx)" },
        "static" => if en { "Static resources" } else { "Statik kaynaklar" },
        "non_indexable" => if en { "Non-indexable / canonicalised (Spider)" } else { "Non-indexable / farklı canonical (Spider)" },
        _ => "—",
    }
}

/// Inline SVG line chart (one line per series), print-friendly.
pub fn svg_chart(days: &[String], series: &[(String, &str, Vec<i64>)]) -> String {
    let (w, h, pl, pr, pt, pb) = (960.0, 240.0, 56.0, 12.0, 12.0, 28.0);
    let max = series.iter().flat_map(|s| s.2.iter().copied()).max().unwrap_or(0).max(1) as f64;
    let n = days.len().max(1);
    let x = |i: usize| pl + if n <= 1 { (w - pl - pr) / 2.0 } else { i as f64 / (n - 1) as f64 * (w - pl - pr) };
    let y = |v: i64| pt + (1.0 - v as f64 / max) * (h - pt - pb);
    let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"100%\" role=\"img\" style=\"font-family:Segoe UI,Arial;font-size:11px\">");
    for k in 0..=4 {
        let v = (max * k as f64 / 4.0).round() as i64;
        let yy = y(v);
        svg.push_str(&format!("<line x1=\"{pl}\" x2=\"{}\" y1=\"{yy:.1}\" y2=\"{yy:.1}\" stroke=\"#e5e5e5\"/><text x=\"{}\" y=\"{:.1}\" text-anchor=\"end\" fill=\"#777\">{v}</text>", w - pr, pl - 6.0, yy + 4.0));
    }
    let every = (n / 8).max(1);
    for (i, d) in days.iter().enumerate() {
        if i % every == 0 || i == n - 1 {
            svg.push_str(&format!("<text x=\"{:.1}\" y=\"{}\" text-anchor=\"middle\" fill=\"#777\">{}</text>", x(i), h - 8.0, html_escape(d.get(5..).unwrap_or(d))));
        }
    }
    for (_, color, values) in series {
        if values.iter().all(|v| *v == 0) {
            continue;
        }
        let pts: Vec<String> = values.iter().enumerate().map(|(i, v)| format!("{:.1},{:.1}", x(i), y(*v))).collect();
        svg.push_str(&format!("<polyline fill=\"none\" stroke=\"{color}\" stroke-width=\"2\" points=\"{}\"/>", pts.join(" ")));
    }
    svg.push_str("</svg><div style=\"display:flex;gap:14px;flex-wrap:wrap;font-size:12px;color:#555;margin:4px 0 12px\">");
    for (label, color, values) in series {
        if values.iter().any(|v| *v > 0) {
            svg.push_str(&format!("<span><span style=\"display:inline-block;width:10px;height:10px;border-radius:2px;background:{color};margin-right:5px\"></span>{}</span>", html_escape(label)));
        }
    }
    svg.push_str("</div>");
    svg
}

pub fn build(conn: &Connection, project: &Project, range: &Range, lang: &str, spider: Option<&SpiderPart>) -> Result<(Report, Vec<Finding>), String> {
    let lang = if lang == "en" { "en" } else { "tr" };
    let pid = project.id;
    let eff = query::effective(conn, pid, range)?.ok_or("This project has no imported logs yet. Import logs first.")?;
    let ov = query::overview(conn, pid, &eff)?;
    let wst = waste::crawl_waste(conn, pid, &eff, Scope::Googlebot, spider.map(|s| s.pages.as_slice()))?;
    let st = query::status_report(conn, pid, &eff)?;
    let ai = query::ai_activity(conn, pid, &eff)?;
    let fake = query::fake_bots(conn, pid, &eff, 25)?;
    let top = query::urls(conn, pid, &eff, &UrlFilter { limit: Some(25), ..Default::default() })?;

    let cat = |k: &str| wst["categories"].as_array().and_then(|a| a.iter().find(|c| c["key"] == k)).map(|c| n(&c["hits"])).unwrap_or(0);
    let mut m = Metrics {
        googlebot: n(&ov["googlebot"]["requests"]),
        param: cat("parameters"),
        s3xx: cat("redirects"),
        s4xx: cat("client_errors"),
        s5xx: cat("server_errors"),
        statics: cat("static"),
        fake_google: n(&ov["googlebot"]["fakeClaims"]),
        spikes_5xx: st["spikes5xx"].as_array().map(|a| a.len() as i64).unwrap_or(0),
        ai_training: n(&ai["trainingTotal"]),
        ai_assistant: n(&ai["assistantTotal"]),
        total_bots: n(&ov["totalBotRequests"]),
        ..Default::default()
    };
    if let Some(sp) = spider {
        let sum = &sp.gaps["summary"];
        m.spider_indexable = Some(n(&sum["indexable"]));
        m.spider_uncrawled = Some(n(&sum["indexableUncrawled"]));
        m.spider_days = n(&sp.gaps["window"]["days"]);
        m.orphans = Some(n(&sum["orphanCandidates"]));
        m.problem_pages = Some(n(&sum["problemPagesCrawled"]));
    }
    let findings = rules::evaluate(&m);

    let title = format!("{} — {}", tr(lang, "Sunucu Log Analizi", "Server Log Analysis"), project.name);
    let subtitle = match &project.domain {
        Some(d) => format!("{d} · {} → {}", eff.from, eff.to),
        None => format!("{} → {}", eff.from, eff.to),
    };
    let mut r = Report::new(title, lang).subtitle(subtitle);
    let total = n(&ov["totalBotRequests"]);
    let g = m.googlebot;
    r.push(Block::Stats {
        items: vec![
            (tr(lang, "Bot isteği", "Bot requests").into(), num(total, lang)),
            (tr(lang, "Googlebot isteği", "Googlebot requests").into(), num(g, lang)),
            (tr(lang, "Benzersiz URL", "Unique URLs").into(), num(n(&ov["uniqueUrls"]), lang)),
            (tr(lang, "Doğrulanmış", "Verified").into(), pct(ov["verification"]["verifiedShare"].as_f64().unwrap_or(0.0), lang)),
            (tr(lang, "Sahte bot isteği", "Fake bot requests").into(), num(n(&ov["verification"]["fake"]), lang)),
            (tr(lang, "Googlebot 5xx oranı", "Googlebot 5xx rate").into(), pct(if g > 0 { m.s5xx as f64 / g as f64 } else { 0.0 }, lang)),
            (tr(lang, "AI bot isteği", "AI bot requests").into(), num(m.ai_training + m.ai_assistant, lang)),
            (tr(lang, "İnsan isteği (sayaç)", "Human requests (counter)").into(), num(n(&ov["humanRequests"]), lang)),
        ],
    });
    // Daily chart by category.
    let days: Vec<String> = ov["daily"]["days"].as_array().map(|a| a.iter().filter_map(|d| d.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let colors = [("search", "#c8a74d"), ("ai_training", "#ef4444"), ("ai_assistant", "#8b5cf6"), ("seo_tool", "#3b82f6"), ("social", "#10b981"), ("other_bot", "#9ca3af"), ("fake", "#111827")];
    let series: Vec<(String, &str, Vec<i64>)> = colors
        .iter()
        .map(|(key, color)| {
            let label = if *key == "fake" { tr(lang, "Sahte botlar", "Fake bots").to_string() } else { Category::from_name(key).map(|c| c.label(lang).to_string()).unwrap_or_default() };
            let values = ov["daily"]["series"][*key].as_array().map(|a| a.iter().map(n).collect()).unwrap_or_default();
            (label, *color, values)
        })
        .collect();
    r.push(Block::Heading { text: tr(lang, "Bot kategorisine göre günlük istekler", "Daily requests by bot category").into() });
    r.push(Block::RawHtml { html: svg_chart(&days, &series) });

    r.push(Block::Heading { text: tr(lang, "Bulgular", "Findings").into() });
    if findings.is_empty() {
        r.push(Block::Paragraph { text: tr(lang, "Eşikleri aşan bir sorun bulunmadı.", "No issue exceeded the thresholds.").into() });
    } else {
        r.push(Block::Findings { items: findings.iter().map(|f| (f.severity.to_string(), f.text(lang).to_string())).collect() });
    }

    r.push(Block::Heading { text: tr(lang, "En çok taranan URL'ler", "Most crawled URLs").into() });
    r.push(Block::Table {
        headers: vec!["URL".into(), tr(lang, "Bot isteği", "Bot requests").into(), "Googlebot".into(), tr(lang, "Son Googlebot", "Last Googlebot").into(), "4xx+5xx".into()],
        rows: top["urls"].as_array().map(|a| a.iter().map(|u| vec![s(&u["url"]), num(n(&u["hits"]), lang), num(n(&u["googlebotHits"]), lang), s(&u["lastGooglebot"]), num(n(&u["s4xx"]) + n(&u["s5xx"]), lang)]).collect()).unwrap_or_default(),
    });

    r.push(Block::Heading { text: tr(lang, "Hata veren URL'ler", "Error URLs").into() });
    let errs: Vec<Vec<String>> = st["errorUrls"].as_array().map(|a| a.iter().take(30).map(|u| vec![s(&u["url"]), s(&u["status"]), num(n(&u["hits"]), lang), s(&u["bots"]), s(&u["lastSeen"])]).collect()).unwrap_or_default();
    if errs.is_empty() {
        r.push(Block::Paragraph { text: tr(lang, "Botlar seçili dönemde 4xx/5xx almadı.", "Bots got no 4xx/5xx in the period.").into() });
    } else {
        r.push(Block::Table { headers: vec!["URL".into(), tr(lang, "Durum", "Status").into(), tr(lang, "İstek", "Requests").into(), "Bot".into(), tr(lang, "Son görülme", "Last seen").into()], rows: errs });
    }

    r.push(Block::Heading { text: tr(lang, "Tarama bütçesi israfı (Googlebot)", "Crawl budget waste (Googlebot)").into() });
    r.push(Block::Paragraph {
        text: format!("{}: {} / {} ({})", tr(lang, "İsraf sayılan istek", "Requests counted as waste"), num(n(&wst["wasteHits"]), lang), num(n(&wst["totalHits"]), lang), pct(wst["wasteShare"].as_f64().unwrap_or(0.0), lang)),
    });
    r.push(Block::Table {
        headers: vec![tr(lang, "Kategori", "Category").into(), tr(lang, "İstek", "Requests").into(), tr(lang, "Oran", "Share").into(), tr(lang, "Örnek URL'ler", "Example URLs").into()],
        rows: wst["categories"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|c| {
                        let ex: Vec<String> = c["examples"].as_array().map(|e| e.iter().take(3).map(|x| s(&x["url"])).collect()).unwrap_or_default();
                        vec![waste_label(c["key"].as_str().unwrap_or(""), lang).into(), num(n(&c["hits"]), lang), pct(c["share"].as_f64().unwrap_or(0.0), lang), ex.join("  ·  ")]
                    })
                    .collect()
            })
            .unwrap_or_default(),
    });
    if let Some(params) = wst["parameters"].as_array().filter(|p| !p.is_empty()) {
        r.push(Block::Table {
            headers: vec![tr(lang, "Parametre", "Parameter").into(), tr(lang, "İstek", "Requests").into(), tr(lang, "URL sayısı", "URLs").into(), tr(lang, "Örnek", "Example").into()],
            rows: params.iter().take(15).map(|p| vec![s(&p["param"]), num(n(&p["hits"]), lang), num(n(&p["urls"]), lang), p["examples"].get(0).map(s).unwrap_or_default()]).collect(),
        });
    }

    r.push(Block::Heading { text: tr(lang, "AI botları", "AI crawlers").into() });
    let ai_rows: Vec<Vec<String>> = ai["bots"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|b| {
                    let dirs: Vec<String> = b["topDirs"].as_array().map(|d| d.iter().take(3).map(|x| s(&x["dir"])).collect()).unwrap_or_default();
                    let cat = if b["category"] == "ai_training" { tr(lang, "Eğitim", "Training") } else { tr(lang, "Arama/asistan", "Search/assistant") };
                    vec![s(&b["name"]), s(&b["owner"]), cat.into(), num(n(&b["requests"]), lang), num(n(&b["uniquePaths"]), lang), dirs.join(", "), format!("{} / {} / {}", n(&b["verified"]), n(&b["unverifiable"]), n(&b["fake"]))]
                })
                .collect()
        })
        .unwrap_or_default();
    if ai_rows.is_empty() {
        r.push(Block::Paragraph { text: tr(lang, "Seçili dönemde AI botu isteği yok.", "No AI crawler requests in the period.").into() });
    } else {
        r.push(Block::Table {
            headers: vec!["Bot".into(), tr(lang, "Sahip", "Owner").into(), tr(lang, "Tür", "Type").into(), tr(lang, "İstek", "Requests").into(), tr(lang, "Benzersiz yol", "Unique paths").into(), tr(lang, "En çok dizin", "Top directories").into(), tr(lang, "Doğrulanmış / doğrulanamaz / sahte", "Verified / unverifiable / fake").into()],
            rows: ai_rows,
        });
    }

    r.push(Block::Heading { text: tr(lang, "Sahte botlar", "Fake bots").into() });
    let fake_rows: Vec<Vec<String>> = fake["groups"]
        .as_array()
        .map(|a| a.iter().map(|g| vec![s(&g["ip"]), s(&g["claims"]), num(n(&g["requests"]), lang), g["sampleUrls"].as_array().map(|u| u.iter().map(s).collect::<Vec<_>>().join(", ")).unwrap_or_default(), s(&g["firstSeen"]), s(&g["lastSeen"])]).collect())
        .unwrap_or_default();
    if fake_rows.is_empty() {
        r.push(Block::Paragraph { text: tr(lang, "Doğrulamayı geçemeyen bot isteği yok.", "No bot request failed verification.").into() });
    } else {
        r.push(Block::Table {
            headers: vec![tr(lang, "IP (maskeli/özet)", "IP (masked/hashed)").into(), tr(lang, "İddia", "Claims").into(), tr(lang, "İstek", "Requests").into(), tr(lang, "Örnek URL'ler", "Sample URLs").into(), tr(lang, "İlk", "First").into(), tr(lang, "Son", "Last").into()],
            rows: fake_rows,
        });
    }

    if let Some(sp) = spider {
        let sum = &sp.gaps["summary"];
        r.push(Block::Heading { text: format!("{} (#{} · {})", tr(lang, "Spider karşılaştırması", "Spider comparison"), n(&sp.meta["crawlId"]), s(&sp.meta["startUrl"])) });
        r.push(Block::Stats {
            items: vec![
                (tr(lang, "Spider sayfası", "Spider pages").into(), num(n(&sum["spiderPages"]), lang)),
                (tr(lang, "Indexable", "Indexable").into(), num(n(&sum["indexable"]), lang)),
                (tr(lang, "Taranmayan indexable", "Uncrawled indexable").into(), format!("{} ({})", num(n(&sum["indexableUncrawled"]), lang), pct(sum["uncrawledShare"].as_f64().unwrap_or(0.0), lang))),
                (tr(lang, "Yetim adayı", "Orphan candidates").into(), num(n(&sum["orphanCandidates"]), lang)),
                (tr(lang, "Sık taranan sorunlu sayfa", "Crawled problem pages").into(), num(n(&sum["problemPagesCrawled"]), lang)),
            ],
        });
        let table = |key: &str, cols: &[(&str, &str)]| -> Vec<Vec<String>> {
            sp.gaps[key].as_array().map(|a| a.iter().take(25).map(|row| cols.iter().map(|(c, _)| s(&row[*c])).collect()).collect()).unwrap_or_default()
        };
        let orphan_cols = [("url", "URL"), ("googlebotHits", "Googlebot"), ("status", "Durum"), ("lastGooglebot", "Son")];
        r.push(Block::Paragraph { text: tr(lang, "Googlebot'un taradığı ama Spider'da olmayan URL'ler (yetim adayları):", "URLs Googlebot crawled that the Spider crawl lacks (orphan candidates):").into() });
        r.push(Block::Table { headers: vec!["URL".into(), "Googlebot".into(), tr(lang, "Durum", "Status").into(), tr(lang, "Son ziyaret", "Last visit").into()], rows: table("orphans", &orphan_cols) });
        r.push(Block::Paragraph { text: format!("{} {} {}:", tr(lang, "Spider'da indexable olup son", "Indexable Spider pages Googlebot did not crawl in the last"), n(&sp.gaps["window"]["days"]), tr(lang, "günde Googlebot'un taramadığı sayfalar", "days")) });
        r.push(Block::Table { headers: vec!["URL".into(), tr(lang, "Derinlik", "Depth").into(), tr(lang, "Son Googlebot (tüm zamanlar)", "Last Googlebot (all time)").into()], rows: table("uncrawled", &[("url", ""), ("depth", ""), ("lastGooglebot", "")]) });
        r.push(Block::Paragraph { text: tr(lang, "Spider'da noindex/3xx/4xx olup Googlebot'un taradığı sayfalar:", "Pages that are noindex/3xx/4xx in Spider but still crawled by Googlebot:").into() });
        r.push(Block::Table {
            headers: vec!["URL".into(), tr(lang, "Durum", "Status").into(), "Indexability".into(), tr(lang, "Neden", "Reason").into(), "Googlebot".into()],
            rows: table("problems", &[("url", ""), ("status", ""), ("indexability", ""), ("reason", ""), ("googlebotHits", "")]),
        });
    }
    r.push(Block::Paragraph {
        text: tr(
            lang,
            "Yöntem: Bot kimliği yayınlanmış IP aralıkları ve ters DNS + ileri doğrulama ile kontrol edildi. İnsan trafiği yalnız IP'siz günlük sayaçlar olarak işlendi (GDPR); doğrulanamayan bot IP'leri maskeli ya da tuzlu özet olarak saklandı.",
            "Method: crawler identity was checked against published IP ranges and reverse DNS with forward confirmation. Human traffic was processed only as IP-free daily counters (GDPR); unverified bot addresses were stored masked or as salted hashes.",
        )
        .into(),
    });
    Ok((r, findings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_chart() {
        assert_eq!(num(1234567, "tr"), "1.234.567");
        assert_eq!(num(1234567, "en"), "1,234,567");
        assert_eq!(num(12, "tr"), "12");
        assert_eq!(pct(0.125, "tr"), "%12,5");
        let svg = svg_chart(&["2024-10-01".into(), "2024-10-02".into()], &[("A <b>".into(), "#c8a74d", vec![1, 3])]);
        assert!(svg.starts_with("<svg") && svg.contains("polyline") && svg.contains("A &lt;b&gt;"));
    }
}
