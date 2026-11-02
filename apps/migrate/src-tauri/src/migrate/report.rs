//! Branded HTML report (plan summary, mapping statistics, validation, last verification,
//! recommendations) and the mappings CSV.

use super::model::{MappingRow, Plan};
use super::store::{MappingStats, SideStats, VerifyResultRow, VerifyRun};
use super::validate::Validation;
use crate::hub::report::{Block, Report};

fn l(lang: &str, key: &str) -> String {
    let en = lang == "en";
    let s = match key {
        "title" => if en { "Migration report" } else { "Taşıma raporu" },
        "summary" => if en { "Plan summary" } else { "Plan özeti" },
        "old" => if en { "Old URLs" } else { "Eski URL" },
        "new" => if en { "New URLs" } else { "Yeni URL" },
        "match_rate" => if en { "Match rate (≥ 60)" } else { "Eşleşme oranı (≥ 60)" },
        "approved" => if en { "Approved" } else { "Onaylı" },
        "patterns" => if en { "Pattern rules" } else { "Desen kuralı" },
        "mapping" => if en { "Mapping statistics" } else { "Eşleme istatistikleri" },
        "status" => if en { "Status" } else { "Durum" },
        "count" => if en { "Count" } else { "Adet" },
        "share" => if en { "Share" } else { "Pay" },
        "stage" => if en { "Matched by" } else { "Eşleşme aşaması" },
        "validation" => if en { "Validation" } else { "Doğrulama paneli" },
        "no_issues" => if en { "No chains, loops or missing targets." } else { "Zincir, döngü ya da eksik hedef yok." },
        "verify" => if en { "Latest verification" } else { "Son doğrulama" },
        "no_verify" => if en { "No verification run yet." } else { "Henüz doğrulama çalıştırılmadı." },
        "kind" => if en { "Type" } else { "Tür" },
        "checked" => if en { "Checked" } else { "Kontrol edilen" },
        "ok" => if en { "Passed" } else { "Başarılı" },
        "failed" => if en { "Failed" } else { "Hatalı" },
        "success" => if en { "Success rate" } else { "Başarı oranı" },
        "failures" => if en { "Failed URLs" } else { "Hatalı URL'ler" },
        "source" => if en { "Old URL" } else { "Eski URL" },
        "expected" => if en { "Expected" } else { "Beklenen" },
        "outcome" => if en { "Outcome" } else { "Sonuç" },
        "final" => if en { "Final status" } else { "Son durum" },
        "final_url" => if en { "Final URL" } else { "Son URL" },
        "recs" => if en { "Recommendations" } else { "Öneriler" },
        "post" => if en { "Post-launch" } else { "Yayın sonrası" },
        "staging" => if en { "Staging" } else { "Staging (yayın öncesi)" },
        _ => key,
    };
    s.to_string()
}

fn status_label(lang: &str, s: &str) -> String {
    let en = lang == "en";
    match s {
        "auto" => if en { "Auto (≥ 90)" } else { "Otomatik (≥ 90)" },
        "review" => if en { "Review (60–89)" } else { "İncele (60–89)" },
        "weak" => if en { "Weak (< 60)" } else { "Zayıf (< 60)" },
        "manual" => if en { "Manual" } else { "Elle" },
        "gone" => "410",
        "keep" => if en { "Keep (same URL)" } else { "Aynı kalır" },
        other => other,
    }
    .to_string()
}

fn issue_text(lang: &str, kind: &str, n: usize) -> (String, String) {
    let en = lang == "en";
    let (sev, text) = match kind {
        "loop" => ("critical", if en { format!("{n} mappings form redirect loops") } else { format!("{n} eşleme yönlendirme döngüsü oluşturuyor") }),
        "chain" => ("high", if en { format!("{n} redirect chains (A→B→C); collapse them to one hop") } else { format!("{n} yönlendirme zinciri (A→B→C); tek adıma indirin") }),
        "missing_target" => ("high", if en { format!("{n} targets are not in the new URL set") } else { format!("{n} hedef yeni URL kümesinde yok") }),
        "target_not_200" => ("medium", if en { format!("{n} targets have a known status other than 200") } else { format!("{n} hedefin bilinen durumu 200 değil") }),
        "self_redirect" => ("medium", if en { format!("{n} rules redirect a URL to itself") } else { format!("{n} kural URL'yi kendisine yönlendiriyor") }),
        "redundant" => ("low", if en { format!("{n} rules differ only by letter case or trailing slash") } else { format!("{n} kural yalnız büyük/küçük harf ya da sondaki / farkı içeriyor") }),
        _ => ("info", if en { format!("{n} targets receive several old URLs") } else { format!("{n} hedefe birden çok eski URL yönleniyor") }),
    };
    (sev.into(), text)
}

fn outcome_label(lang: &str, o: &str) -> String {
    let en = lang == "en";
    match o {
        "ok" => if en { "OK" } else { "Doğru" },
        "wrong_target" => if en { "Wrong target" } else { "Yanlış hedef" },
        "chain" => if en { "Chain" } else { "Zincir" },
        "temporary" => if en { "Temporary (302/307)" } else { "Geçici (302/307)" },
        "not_redirected" => if en { "Not redirected" } else { "Yönlendirilmiyor" },
        "target_error" => if en { "Target error" } else { "Hedef hatalı" },
        "loop" => if en { "Loop" } else { "Döngü" },
        _ => if en { "Error" } else { "Hata" },
    }
    .to_string()
}

fn pct(n: i64, d: i64) -> String {
    if d <= 0 {
        "—".into()
    } else {
        format!("{:.1}%", n as f64 * 100.0 / d as f64)
    }
}

pub struct ReportInput<'a> {
    pub plan: &'a Plan,
    pub stats: &'a MappingStats,
    pub old: &'a SideStats,
    pub new: &'a SideStats,
    pub patterns: usize,
    pub validation: &'a Validation,
    pub run: Option<&'a VerifyRun>,
    pub results: &'a [VerifyResultRow],
}

/// Report title without the plan name (also the start of the default file name).
pub fn title(lang: &str) -> String {
    l(lang, "title")
}

pub fn build(input: &ReportInput, lang: &str) -> Report {
    let ReportInput { plan, stats, old, new, patterns, validation, run, results } = input;
    let mut r = Report::new(format!("{} — {}", l(lang, "title"), plan.name), lang).subtitle(format!("{} → {}", plan.old_base, plan.new_base));
    let matched: i64 = stats.by_status.iter().filter(|(s, _)| s.as_str() != "weak").map(|(_, n)| n).sum();
    r.push(Block::Heading { text: l(lang, "summary") });
    let mut para = format!("{} → {}", plan.old_base, plan.new_base);
    if let Some(s) = &plan.staging_base {
        para.push_str(&format!(" · staging: {s}"));
    }
    if let Some(n) = &plan.notes {
        para.push_str(&format!(" · {n}"));
    }
    r.push(Block::Paragraph { text: para });
    r.push(Block::Stats {
        items: vec![
            (l(lang, "old"), old.total.to_string()),
            (l(lang, "new"), new.total.to_string()),
            (l(lang, "match_rate"), pct(matched, old.total)),
            (l(lang, "approved"), pct(stats.approved, stats.total)),
            (l(lang, "patterns"), patterns.to_string()),
        ],
    });
    r.push(Block::Heading { text: l(lang, "mapping") });
    let mut rows: Vec<Vec<String>> = super::model::STATUSES
        .iter()
        .map(|s| {
            let n = stats.by_status.get(*s).copied().unwrap_or(0);
            vec![status_label(lang, s), n.to_string(), pct(n, stats.total)]
        })
        .collect();
    if stats.unmapped > 0 {
        rows.push(vec![if lang == "en" { "Not mapped yet".into() } else { "Henüz eşlenmedi".into() }, stats.unmapped.to_string(), "—".into()]);
    }
    r.push(Block::Table { headers: vec![l(lang, "status"), l(lang, "count"), l(lang, "share")], rows });
    let mut reasons: Vec<(&String, &i64)> = stats.by_reason.iter().collect();
    reasons.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    r.push(Block::Table { headers: vec![l(lang, "stage"), l(lang, "count")], rows: reasons.into_iter().map(|(k, v)| vec![k.clone(), v.to_string()]).collect() });

    r.push(Block::Heading { text: l(lang, "validation") });
    let findings: Vec<(String, String)> = validation.counts.iter().filter(|(_, n)| **n > 0).map(|(k, n)| issue_text(lang, k, *n)).collect();
    if findings.is_empty() {
        r.push(Block::Paragraph { text: l(lang, "no_issues") });
    } else {
        r.push(Block::Findings { items: findings });
    }

    r.push(Block::Heading { text: l(lang, "verify") });
    match run {
        None => {
            r.push(Block::Paragraph { text: l(lang, "no_verify") });
        }
        Some(run) => {
            r.push(Block::Stats {
                items: vec![
                    (l(lang, "kind"), l(lang, &run.kind)),
                    (l(lang, "checked"), run.total.to_string()),
                    (l(lang, "ok"), run.ok.to_string()),
                    (l(lang, "failed"), run.failed.to_string()),
                    (l(lang, "success"), pct(run.ok, run.total)),
                ],
            });
            let failed: Vec<Vec<String>> = results
                .iter()
                .filter(|x| x.outcome != "ok")
                .take(500)
                .map(|x| {
                    vec![
                        x.source_url.clone(),
                        x.expected_url.clone().unwrap_or_default(),
                        outcome_label(lang, &x.outcome),
                        x.final_status.map(|s| s.to_string()).unwrap_or_else(|| x.error.clone().unwrap_or_default()),
                        x.final_url.clone().unwrap_or_default(),
                    ]
                })
                .collect();
            if !failed.is_empty() {
                r.push(Block::Heading { text: l(lang, "failures") });
                r.push(Block::Table { headers: vec![l(lang, "source"), l(lang, "expected"), l(lang, "outcome"), l(lang, "final"), l(lang, "final_url")], rows: failed });
            }
        }
    }

    r.push(Block::Heading { text: l(lang, "recs") });
    r.push(Block::Findings { items: recommendations(input, lang) });
    r
}

fn recommendations(input: &ReportInput, lang: &str) -> Vec<(String, String)> {
    let en = lang == "en";
    let s = input.stats;
    let get = |k: &str| s.by_status.get(k).copied().unwrap_or(0);
    let mut out = Vec::new();
    if s.unmapped > 0 {
        out.push(("high".into(), if en { format!("Run auto matching: {} old URLs have no mapping.", s.unmapped) } else { format!("Otomatik eşlemeyi çalıştırın: {} eski URL'nin eşlemesi yok.", s.unmapped) }));
    }
    if get("weak") > 0 {
        out.push(("high".into(), if en { format!("Review {} weak mappings (parent/homepage fallbacks); pick a closer page or mark them 410.", get("weak")) } else { format!("{} zayıf eşlemeyi (üst kategori/ana sayfa) gözden geçirin; daha yakın bir sayfa seçin ya da 410 işaretleyin.", get("weak")) }));
    }
    if get("review") > 0 {
        out.push(("medium".into(), if en { format!("Confirm {} mappings in the review band (60–89).", get("review")) } else { format!("İnceleme bandındaki (60–89) {} eşlemeyi onaylayın.", get("review")) }));
    }
    let v = &input.validation.counts;
    if v.get("chain").copied().unwrap_or(0) > 0 {
        out.push(("high".into(), if en { "Collapse redirect chains to a single hop before export.".into() } else { "Dışa aktarmadan önce zincirleri tek adıma indirin.".into() }));
    }
    if v.get("loop").copied().unwrap_or(0) > 0 {
        out.push(("critical".into(), if en { "Fix redirect loops; they make pages unreachable.".into() } else { "Döngüleri düzeltin; sayfalara erişilemez hale getirir.".into() }));
    }
    if input.old.with_value == 0 {
        out.push(("low".into(), if en { "Import traffic/backlink values so the most valuable URLs are reviewed first.".into() } else { "En değerli URL'lerin önce incelenmesi için trafik/backlink değerlerini içe aktarın.".into() }));
    }
    match input.run {
        None => out.push(("medium".into(), if en { "Run the staging check before launch and the post-launch verification after it.".into() } else { "Yayından önce staging kontrolünü, yayından sonra doğrulamayı çalıştırın.".into() })),
        Some(r) if r.failed > 0 => out.push(("high".into(), if en { format!("{} URLs failed the latest verification; fix the rules and run it again.", r.failed) } else { format!("Son doğrulamada {} URL hatalı; kuralları düzeltip yeniden çalıştırın.", r.failed) })),
        _ => {}
    }
    if out.is_empty() {
        out.push(("info".into(), if en { "The plan is consistent; keep monitoring the redirects for a few weeks after launch.".into() } else { "Plan tutarlı; yayından sonraki birkaç hafta yönlendirmeleri izlemeye devam edin.".into() }));
    }
    out
}

pub const CSV_HEADERS: &[&str] = &["old_url", "old_title", "new_url", "new_title", "status", "confidence", "reason", "approved", "value"];

pub fn csv_rows(rows: &[MappingRow]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|m| {
            vec![
                m.old_url.clone(),
                m.old_title.clone().unwrap_or_default(),
                m.new_url.clone().unwrap_or_default(),
                m.new_title.clone().unwrap_or_default(),
                m.status.clone(),
                m.confidence.to_string(),
                m.reason.clone(),
                if m.approved { "1".into() } else { "0".into() },
                m.value.map(|v| v.to_string()).unwrap_or_default(),
            ]
        })
        .collect()
}
