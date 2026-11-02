//! Synthetic combined-log generator (test helper, benchmark and demo data).
//!
//! Produces a realistic mix of verified crawlers (addresses inside the published Google,
//! Bing, OpenAI and Perplexity ranges), unverifiable AI/SEO bots, fake Googlebots (from the
//! RFC 5737 documentation ranges) and human visitors (from the RFC 2544 benchmarking range,
//! so no real person's address is ever generated). Output is deterministic for a seed.

use super::parse;
use std::collections::BTreeMap;
use std::io::{self, Write};

pub struct SynthConfig {
    pub lines: usize,
    /// First day (`YYYY-MM-DD`, local +03:00).
    pub start_day: String,
    pub days: u32,
    /// Pages of the site (e.g. from a Spider crawl).
    pub site_paths: Vec<String>,
    /// Pages Googlebot visits disproportionately often (e.g. noindex pages).
    pub hot_paths: Vec<String>,
    /// URLs that do not exist on the site any more (orphans / legacy URLs).
    pub orphan_paths: Vec<String>,
    /// Share of site pages Googlebot never visits.
    pub cold_share: f64,
    /// Day index (0-based) with a 5xx burst.
    pub spike_day: Option<u32>,
    pub seed: u64,
    /// Include YandexBot and fake bots (their verification needs reverse DNS).
    pub dns_bots: bool,
    /// Append the nginx `$request_time` (seconds) after the user agent.
    pub timing: bool,
}

impl Default for SynthConfig {
    fn default() -> Self {
        SynthConfig {
            lines: 10_000,
            start_day: "2026-10-01".into(),
            days: 28,
            site_paths: demo_site(400),
            hot_paths: Vec::new(),
            orphan_paths: default_orphans(),
            cold_share: 0.35,
            spike_day: Some(20),
            seed: 42,
            dns_bots: true,
            timing: false,
        }
    }
}

#[derive(Debug, Default)]
pub struct SynthStats {
    pub lines: u64,
    pub by_actor: BTreeMap<&'static str, u64>,
    pub cold_paths: usize,
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
    /// Skewed index (popular items first).
    fn skewed(&mut self, n: usize) -> usize {
        let r = self.f();
        ((r * r * r) * n as f64) as usize % n.max(1)
    }
}

pub fn demo_site(n: usize) -> Vec<String> {
    let mut v = vec!["/".to_string(), "/hizmetler/".into(), "/blog/".into(), "/iletisim/".into(), "/hakkimizda/".into()];
    let mut i = 0;
    while v.len() < n {
        i += 1;
        v.push(if i % 3 == 0 { format!("/urun/kategori-{}/urun-{i}/", i % 17) } else { format!("/blog/yazi-{i}/") });
    }
    v
}

pub fn default_orphans() -> Vec<String> {
    [
        "/eski-blog/seo-nedir/",
        "/etiket/seo/",
        "/etiket/dijital-pazarlama/",
        "/kategori/genel/",
        "/author/admin/",
        "/2019/05/eski-kampanya/",
        "/hizmetlerimiz/web-tasarim-paketleri/",
        "/kampanya/yaz-indirimi/",
        "/eski-urunler/",
        "/feed/",
        "/en/services/",
        "/blog/page/7/",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

const CHROME_W: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";
const SAFARI_I: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
const CHROME_A: &str = "Mozilla/5.0 (Linux; Android 14; SM-S918B) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36";
const FIREFOX: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:131.0) Gecko/20100101 Firefox/131.0";
const MAC: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
const IPAD: &str = "Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1";
const G_MOBILE: &str = "Mozilla/5.0 (Linux; Android 6.0.1; Nexus 5X Build/MMB29P) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.6723.69 Mobile Safari/537.36 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)";
const G_DESKTOP: &str = "Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; Googlebot/2.1; +http://www.google.com/bot.html) Chrome/130.0.6723.69 Safari/537.36";

#[derive(Clone, Copy)]
enum Pages {
    Human,
    Google,
    Images,
    Crawler,
    Probe,
}

struct Actor {
    name: &'static str,
    weight: u32,
    ua: &'static [&'static str],
    ips: fn(&mut Rng) -> String,
    pages: Pages,
    dns: bool,
}

fn ip_human(r: &mut Rng) -> String {
    format!("198.{}.{}.{}", 18 + r.below(2), r.below(256), 1 + r.below(254))
}
fn ip_google(r: &mut Rng) -> String {
    format!("66.249.64.{}", 1 + r.below(254))
}
fn ip_bing(r: &mut Rng) -> String {
    let nets = ["157.55.39", "207.46.13", "40.77.167"];
    format!("{}.{}", r.pick(&nets), 1 + r.below(254))
}
fn ip_gpt(r: &mut Rng) -> String {
    format!("132.196.86.{}", 1 + r.below(254))
}
fn ip_oai_search(r: &mut Rng) -> String {
    format!("104.210.140.{}", 129 + r.below(14))
}
fn ip_chatgpt_user(r: &mut Rng) -> String {
    format!("104.208.184.{}", 193 + r.below(14))
}
fn ip_perplexity(r: &mut Rng) -> String {
    r.pick(&["107.20.236.150", "18.210.92.235", "18.97.1.228"]).to_string()
}
fn ip_anthropic(r: &mut Rng) -> String {
    format!("160.79.104.{}", 10 + r.below(40))
}
fn ip_cloud(r: &mut Rng) -> String {
    let nets = ["54.36.148", "85.208.96", "47.128.33", "3.224.220", "52.70.240", "17.58.98", "57.141.0"];
    format!("{}.{}", r.pick(&nets), 1 + r.below(254))
}
fn ip_fake(r: &mut Rng) -> String {
    // 24 addresses from the RFC 5737 documentation ranges.
    let pool = ["203.0.113", "198.51.100", "192.0.2"];
    format!("{}.{}", r.pick(&pool), 10 + r.below(8))
}
fn ip_yandex(r: &mut Rng) -> String {
    r.pick(&["5.255.231.100", "213.180.203.10", "95.108.213.10", "5.255.253.20"]).to_string()
}

fn actors() -> Vec<Actor> {
    vec![
        Actor { name: "human", weight: 470, ua: &[CHROME_W, SAFARI_I, CHROME_A, FIREFOX, MAC, IPAD], ips: ip_human, pages: Pages::Human, dns: false },
        Actor { name: "googlebot-smartphone", weight: 150, ua: &[G_MOBILE], ips: ip_google, pages: Pages::Google, dns: false },
        Actor { name: "googlebot-desktop", weight: 30, ua: &[G_DESKTOP], ips: ip_google, pages: Pages::Google, dns: false },
        Actor { name: "googlebot-image", weight: 10, ua: &["Googlebot-Image/1.0"], ips: ip_google, pages: Pages::Images, dns: false },
        Actor { name: "bingbot", weight: 50, ua: &["Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; bingbot/2.0; +http://www.bing.com/bingbot.htm) Chrome/116.0.1938.76 Safari/537.36"], ips: ip_bing, pages: Pages::Crawler, dns: false },
        Actor { name: "gptbot", weight: 30, ua: &["Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; GPTBot/1.2; +https://openai.com/gptbot"], ips: ip_gpt, pages: Pages::Crawler, dns: false },
        Actor { name: "oai-searchbot", weight: 8, ua: &["Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; OAI-SearchBot/1.0; +https://openai.com/searchbot"], ips: ip_oai_search, pages: Pages::Crawler, dns: false },
        Actor { name: "chatgpt-user", weight: 12, ua: &["Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko); compatible; ChatGPT-User/1.0; +https://openai.com/bot"], ips: ip_chatgpt_user, pages: Pages::Crawler, dns: false },
        Actor { name: "claudebot", weight: 25, ua: &["Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; ClaudeBot/1.0; +claudebot@anthropic.com)"], ips: ip_anthropic, pages: Pages::Crawler, dns: false },
        Actor { name: "claude-user", weight: 5, ua: &["Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; Claude-User/1.0; +Claude-User@anthropic.com)"], ips: ip_anthropic, pages: Pages::Crawler, dns: false },
        Actor { name: "perplexitybot", weight: 8, ua: &["Mozilla/5.0 AppleWebKit/537.36 (KHTML, like Gecko; compatible; PerplexityBot/1.0; +https://perplexity.ai/perplexitybot)"], ips: ip_perplexity, pages: Pages::Crawler, dns: false },
        Actor { name: "meta-externalagent", weight: 12, ua: &["meta-externalagent/1.1 (+https://developers.facebook.com/docs/sharing/webmasters/crawler)"], ips: ip_cloud, pages: Pages::Crawler, dns: false },
        Actor { name: "bytespider", weight: 10, ua: &["Mozilla/5.0 (Linux; Android 5.0) AppleWebKit/537.36 (KHTML, like Gecko) Mobile Safari/537.36 (compatible; Bytespider; spider-feedback@bytedance.com)"], ips: ip_cloud, pages: Pages::Crawler, dns: false },
        Actor { name: "ccbot", weight: 5, ua: &["CCBot/2.0 (https://commoncrawl.org/faq/)"], ips: ip_cloud, pages: Pages::Crawler, dns: false },
        Actor { name: "ahrefsbot", weight: 30, ua: &["Mozilla/5.0 (compatible; AhrefsBot/7.0; +http://ahrefs.com/robot/)"], ips: ip_cloud, pages: Pages::Crawler, dns: false },
        Actor { name: "semrushbot", weight: 20, ua: &["Mozilla/5.0 (compatible; SemrushBot/7~bl; +http://www.semrush.com/bot.html)"], ips: ip_cloud, pages: Pages::Crawler, dns: false },
        Actor { name: "fake-googlebot", weight: 25, ua: &[G_MOBILE, "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)"], ips: ip_fake, pages: Pages::Probe, dns: true },
        Actor { name: "yandexbot", weight: 5, ua: &["Mozilla/5.0 (compatible; YandexBot/3.0; +http://yandex.com/bots)"], ips: ip_yandex, pages: Pages::Crawler, dns: true },
        Actor { name: "facebookexternalhit", weight: 8, ua: &["facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)"], ips: ip_cloud, pages: Pages::Crawler, dns: false },
        Actor { name: "twitterbot", weight: 4, ua: &["Twitterbot/1.0"], ips: ip_cloud, pages: Pages::Crawler, dns: false },
        Actor { name: "http-client", weight: 12, ua: &["curl/8.7.1", "python-requests/2.32.3", "Go-http-client/1.1"], ips: ip_cloud, pages: Pages::Probe, dns: false },
        Actor { name: "other-bot", weight: 6, ua: &["Mozilla/5.0 (compatible; MegaIndex.ru/2.0; +http://megaindex.com/crawler)"], ips: ip_cloud, pages: Pages::Crawler, dns: false },
    ]
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const STATIC: &[&str] = &["/wp-content/themes/seobify/style.css?ver=6.6", "/wp-content/uploads/2026/05/hero.webp", "/wp-includes/js/jquery/jquery.min.js?ver=3.7.1", "/assets/app.js", "/wp-content/uploads/2026/03/logo.png", "/favicon.ico"];
const PARAMS: &[&str] = &["utm_source=newsletter", "utm_source=linkedin&utm_medium=social", "page=2", "page=3", "sort=price", "replytocom=118", "s=seo", "fbclid=IwAR0x"];
const PROBES: &[&str] = &["/wp-login.php", "/xmlrpc.php", "/.env", "/wp-admin/", "/admin/", "/wp-json/wp/v2/users", "/phpmyadmin/"];

fn hash(s: &str) -> u64 {
    s.bytes().fold(1469598103934665603u64, |h, b| (h ^ b as u64).wrapping_mul(1099511628211))
}

pub fn generate<W: Write>(cfg: &SynthConfig, out: &mut W) -> io::Result<SynthStats> {
    let mut rng = Rng(cfg.seed.max(1) ^ 0x9E37_79B9_7F4A_7C15);
    let actors: Vec<Actor> = actors().into_iter().filter(|a| cfg.dns_bots || !a.dns).collect();
    let total_weight: u32 = actors.iter().map(|a| a.weight).sum();
    let cold_cut = (cfg.cold_share.clamp(0.0, 1.0) * 1000.0) as u64;
    let warm: Vec<&String> = cfg.site_paths.iter().filter(|p| p.as_str() == "/" || cfg.hot_paths.contains(p) || hash(p) % 1000 >= cold_cut).collect();
    let site: Vec<&String> = cfg.site_paths.iter().collect();
    let mut stats = SynthStats { cold_paths: cfg.site_paths.len() - warm.len(), ..Default::default() };
    let start_day = parse::parse_day(&cfg.start_day).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "start_day must be YYYY-MM-DD"))?;
    let start = start_day * 86_400 - 3 * 3600; // 00:00 at +03:00
    let span = cfg.days.max(1) as i64 * 86_400;
    let mut line = String::with_capacity(512);
    for i in 0..cfg.lines {
        let ts = start + (i as i64 * span) / cfg.lines.max(1) as i64;
        let day_index = ((ts - start) / 86_400) as u32;
        let mut w = rng.below(total_weight as usize) as u32;
        let actor = actors.iter().find(|a| {
            if w < a.weight {
                true
            } else {
                w -= a.weight;
                false
            }
        }).unwrap_or(&actors[0]);
        let ua = *rng.pick(actor.ua);
        let ip = (actor.ips)(&mut rng);
        let (path, mut status): (String, u16) = match actor.pages {
            Pages::Human => {
                let r = rng.f();
                if r < 0.15 {
                    (rng.pick(STATIC).to_string(), if rng.f() < 0.3 { 304 } else { 200 })
                } else if r < 0.17 {
                    (format!("/{}-bulunamadi/", rng.below(50)), 404)
                } else {
                    (site[rng.skewed(site.len())].clone(), 200)
                }
            }
            Pages::Images => (rng.pick(&STATIC[1..]).to_string(), 200),
            Pages::Google => {
                let r = rng.f();
                if r < 0.05 && !cfg.orphan_paths.is_empty() {
                    let p = rng.pick(&cfg.orphan_paths).clone();
                    let s = match hash(&p) % 10 { 0..=5 => 404, 6 => 410, 7 => 301, _ => 200 };
                    (p, s)
                } else if r < 0.17 && !warm.is_empty() {
                    let base = warm[rng.skewed(warm.len())];
                    (format!("{base}?{}", rng.pick(PARAMS)), 200)
                } else if r < 0.25 {
                    (rng.pick(STATIC).to_string(), 200)
                } else if r < 0.27 {
                    (if rng.f() < 0.5 { "/robots.txt".into() } else { "/sitemap_index.xml".into() }, 200)
                } else if r < 0.30 && !warm.is_empty() {
                    let base = warm[rng.skewed(warm.len())];
                    let trimmed = base.trim_end_matches('/');
                    (if trimmed.is_empty() { "/index.php".to_string() } else { trimmed.to_string() }, 301)
                } else if r < 0.42 && !cfg.hot_paths.is_empty() {
                    (rng.pick(&cfg.hot_paths).clone(), 200)
                } else if warm.is_empty() {
                    ("/".into(), 200)
                } else {
                    (warm[rng.skewed(warm.len())].clone(), 200)
                }
            }
            Pages::Crawler => {
                if rng.f() < 0.04 && !cfg.orphan_paths.is_empty() {
                    (rng.pick(&cfg.orphan_paths).clone(), 404)
                } else {
                    (site[rng.below(site.len())].clone(), 200)
                }
            }
            Pages::Probe => {
                if rng.f() < 0.6 {
                    (rng.pick(PROBES).to_string(), if rng.f() < 0.8 { 404 } else { 403 })
                } else {
                    (site[rng.below(site.len())].clone(), 200)
                }
            }
        };
        if status == 200 {
            let p5 = if Some(day_index) == cfg.spike_day { 0.10 } else { 0.004 };
            if rng.f() < p5 {
                status = if rng.f() < 0.7 { 503 } else { 500 };
            }
        }
        let bytes = if status == 304 || status == 301 { 0 } else { 400 + rng.below(60_000) };
        let referer = if matches!(actor.pages, Pages::Human) && rng.f() < 0.35 { "https://www.google.com/" } else { "-" };
        let local = ts + 3 * 3600;
        let (y, mo, d) = parse::civil_from_days(local.div_euclid(86_400));
        let secs = local.rem_euclid(86_400);
        line.clear();
        use std::fmt::Write as _;
        let _ = write!(
            line,
            "{ip} - - [{d:02}/{}/{y}:{:02}:{:02}:{:02} +0300] \"GET {path} HTTP/1.1\" {status} {bytes} \"{referer}\" \"{ua}\"",
            MONTHS[(mo - 1) as usize],
            secs / 3600,
            (secs / 60) % 60,
            secs % 60
        );
        if cfg.timing {
            let _ = write!(line, " {:.3}", 0.02 + rng.f() * if status >= 500 { 2.5 } else { 0.4 });
        }
        line.push('\n');
        out.write_all(line.as_bytes())?;
        stats.lines += 1;
        *stats.by_actor.entry(actor.name).or_insert(0) += 1;
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::parse::{Format, Line, Parser};

    #[test]
    fn generates_parseable_deterministic_lines() {
        let cfg = SynthConfig { lines: 3000, ..Default::default() };
        let mut a = Vec::new();
        let stats = generate(&cfg, &mut a).unwrap();
        let mut b = Vec::new();
        generate(&cfg, &mut b).unwrap();
        assert_eq!(a, b, "same seed, same output");
        assert_eq!(stats.lines, 3000);
        assert!(stats.cold_paths > 50);
        let text = String::from_utf8(a).unwrap();
        let mut parser = Parser::new(Format::Combined);
        let ok = text.lines().filter(|l| matches!(parser.parse(l), Line::Rec(_))).count();
        assert_eq!(ok, 3000);
        assert!(text.contains("Googlebot/2.1") && text.contains("GPTBot") && text.contains("203.0.113."));
        assert!(stats.by_actor["human"] > 1000);
    }
}
