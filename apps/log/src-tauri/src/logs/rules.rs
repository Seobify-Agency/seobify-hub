//! Finding rules for reports (severity critical → info). Thresholds follow SPEC §2.5.

use serde::Serialize;

#[derive(Debug, Clone, Default)]
pub struct Metrics {
    /// Non-fake Googlebot requests in the period.
    pub googlebot: i64,
    pub param: i64,
    pub s3xx: i64,
    pub s4xx: i64,
    pub s5xx: i64,
    pub statics: i64,
    /// Requests claiming to be Googlebot that failed verification.
    pub fake_google: i64,
    pub spikes_5xx: i64,
    pub ai_training: i64,
    pub ai_assistant: i64,
    pub total_bots: i64,
    pub spider_indexable: Option<i64>,
    pub spider_uncrawled: Option<i64>,
    pub spider_days: i64,
    pub orphans: Option<i64>,
    pub problem_pages: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub severity: &'static str,
    pub code: &'static str,
    pub tr: String,
    pub en: String,
}

impl Finding {
    pub fn text(&self, lang: &str) -> &str {
        if lang == "en" { &self.en } else { &self.tr }
    }
}

fn pct_en(x: f64) -> String {
    format!("{:.1}%", x * 100.0)
}

fn pct_tr(x: f64) -> String {
    format!("%{}", format!("{:.1}", x * 100.0).replace('.', ","))
}

fn share(n: i64, d: i64) -> f64 {
    if d > 0 { n as f64 / d as f64 } else { 0.0 }
}

pub fn severity_rank(s: &str) -> u8 {
    match s {
        "critical" => 0,
        "high" => 1,
        "medium" => 2,
        "low" => 3,
        _ => 4,
    }
}

pub fn evaluate(m: &Metrics) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut push = |severity: &'static str, code: &'static str, tr: String, en: String| out.push(Finding { severity, code, tr, en });
    let g = m.googlebot;
    if g > 0 {
        let r5 = share(m.s5xx, g);
        if r5 > 0.01 {
            push("critical", "googlebot_5xx", format!("Googlebot isteklerinin {}'i 5xx sunucu hatası aldı ({} istek). Eşik %1: sunucu kararlılığını ve hata veren URL'leri inceleyin.", pct_tr(r5), m.s5xx), format!("{} of Googlebot requests got a 5xx server error ({} requests). Threshold 1%: check server stability and the failing URLs.", pct_en(r5), m.s5xx));
        }
        let rp = share(m.param, g);
        if rp > 0.20 {
            push("high", "param_share", format!("Googlebot isteklerinin {}'i parametreli URL'lere gidiyor. Parametreleri canonical, robots.txt ya da iç bağlantı temizliğiyle sınırlayın.", pct_tr(rp)), format!("{} of Googlebot requests go to parameterised URLs. Limit them with canonicals, robots.txt rules or internal-link clean-up.", pct_en(rp)));
        }
        let r4 = share(m.s4xx, g);
        if r4 > 0.05 {
            push("medium", "googlebot_4xx", format!("Googlebot isteklerinin {}'i 4xx aldı. Kırık iç bağlantıları ve eski URL'leri 301 ile yönlendirin.", pct_tr(r4)), format!("{} of Googlebot requests got a 4xx. Fix broken internal links and 301-redirect retired URLs.", pct_en(r4)));
        }
        let r3 = share(m.s3xx, g);
        if r3 > 0.10 {
            push("medium", "googlebot_3xx", format!("Googlebot isteklerinin {}'i yönlendirmeye gidiyor. İç bağlantıları nihai URL'lere güncelleyin.", pct_tr(r3)), format!("{} of Googlebot requests hit redirects. Point internal links at the final URLs.", pct_en(r3)));
        }
        let rs = share(m.statics, g);
        if rs > 0.30 {
            push("low", "static_share", format!("Googlebot isteklerinin {}'i statik kaynaklara (CSS/JS/görsel) gidiyor. Önbellek başlıklarını ve dosya sürümlemesini kontrol edin.", pct_tr(rs)), format!("{} of Googlebot requests go to static resources (CSS/JS/images). Check cache headers and asset versioning.", pct_en(rs)));
        }
    }
    let claims = g + m.fake_google;
    let rf = share(m.fake_google, claims);
    if rf > 0.05 {
        push("medium", "fake_googlebot", format!("Googlebot olduğunu iddia eden isteklerin {}'i doğrulanamadı (sahte Googlebot, {} istek). Bu IP'leri WAF/CDN'de sınırlamayı düşünün.", pct_tr(rf), m.fake_google), format!("{} of requests claiming to be Googlebot failed verification (fake Googlebot, {} requests). Consider rate-limiting them at the WAF/CDN.", pct_en(rf), m.fake_google));
    }
    if let (Some(idx), Some(un)) = (m.spider_indexable, m.spider_uncrawled) {
        let ru = share(un, idx);
        if idx > 0 && ru > 0.30 {
            push("high", "spider_uncrawled", format!("Spider'daki indexable sayfaların {}'i ({} / {}) son {} günde Googlebot tarafından hiç taranmadı. İç bağlantı ve sitemap kapsamını güçlendirin.", pct_tr(ru), un, idx, m.spider_days), format!("{} of indexable pages in the Spider crawl ({} / {}) were not crawled by Googlebot in the last {} days. Strengthen internal links and sitemap coverage.", pct_en(ru), un, idx, m.spider_days));
        }
    }
    if m.spikes_5xx > 0 {
        push("high", "spikes_5xx", format!("{} günde 5xx sayısı 28 günlük ortalamanın +3σ üstüne çıktı. O günlerin sunucu olaylarını inceleyin.", m.spikes_5xx), format!("On {} day(s) the 5xx count exceeded the 28-day mean by more than 3σ. Review server incidents on those days.", m.spikes_5xx));
    }
    if let Some(o) = m.orphans.filter(|o| *o > 0) {
        push("medium", "orphans", format!("Googlebot'un taradığı {o} URL Spider taramasında yok (yetim adayı ya da eski URL). Değerli olanlara iç bağlantı verin, gerisini yönlendirin."), format!("Googlebot crawled {o} URLs that the Spider crawl does not contain (orphan candidates or legacy URLs). Link the valuable ones internally and redirect the rest."));
    }
    if let Some(p) = m.problem_pages.filter(|p| *p > 0) {
        push("medium", "problem_pages", format!("Spider'da noindex/yönlendirme/hata olan {p} sayfa Googlebot tarafından taranmaya devam ediyor."), format!("{p} pages that are noindex, redirected or erroring in the Spider crawl are still crawled by Googlebot."));
    }
    if m.ai_training > 0 {
        let ra = share(m.ai_training, m.total_bots);
        push("info", "ai_training", format!("AI eğitim botları {} istek yaptı (bot trafiğinin {}'i). robots.txt politikanızın bilinçli olduğundan emin olun.", m.ai_training, pct_tr(ra)), format!("AI training crawlers made {} requests ({} of bot traffic). Make sure your robots.txt policy is deliberate.", m.ai_training, pct_en(ra)));
    }
    if m.ai_assistant > 0 {
        push("info", "ai_assistant", format!("AI arama/asistan botları {} istek yaptı: içeriğiniz AI yanıtlarında kaynak olarak kullanılıyor.", m.ai_assistant), format!("AI search/assistant fetchers made {} requests: your content is being used as a source in AI answers.", m.ai_assistant));
    }
    out.sort_by_key(|f| severity_rank(f.severity));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(m: &Metrics) -> Vec<&'static str> {
        evaluate(m).into_iter().map(|f| f.code).collect()
    }

    #[test]
    fn spec_thresholds() {
        let base = Metrics { googlebot: 1000, ..Default::default() };
        assert!(codes(&base).is_empty());
        assert_eq!(codes(&Metrics { param: 201, ..base.clone() }), vec!["param_share"]);
        assert!(codes(&Metrics { param: 200, ..base.clone() }).is_empty(), "exactly 20% is not above the threshold");
        assert_eq!(codes(&Metrics { s5xx: 11, ..base.clone() }), vec!["googlebot_5xx"]);
        assert!(codes(&Metrics { s5xx: 10, ..base.clone() }).is_empty());
        assert_eq!(codes(&Metrics { fake_google: 60, ..base.clone() }), vec!["fake_googlebot"]);
        assert!(codes(&Metrics { fake_google: 50, ..base.clone() }).is_empty());
        assert_eq!(codes(&Metrics { spider_indexable: Some(100), spider_uncrawled: Some(31), spider_days: 30, ..base.clone() }), vec!["spider_uncrawled"]);
        assert!(codes(&Metrics { spider_indexable: Some(100), spider_uncrawled: Some(30), ..base.clone() }).is_empty());
    }

    #[test]
    fn ordering_and_texts() {
        let m = Metrics { googlebot: 100, s5xx: 5, param: 40, ai_training: 3, total_bots: 200, ..Default::default() };
        let f = evaluate(&m);
        assert_eq!(f.iter().map(|x| x.severity).collect::<Vec<_>>(), vec!["critical", "high", "info"]);
        assert!(f[0].tr.contains("%5,0") && f[0].en.contains("5.0%"));
        assert_eq!(f[1].text("en"), f[1].en);
    }
}
