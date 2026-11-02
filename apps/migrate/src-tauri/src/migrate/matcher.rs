//! Matching engine.
//!
//! Stages, applied in order to every old URL:
//! 1. user regex patterns (target in the new set → 100, otherwise a 70 fallback candidate),
//! 2. same path (normalised) → 98, or `keep` when the URL is identical,
//! 3. same slug (Turkish-folded last segment) with a single candidate → 92,
//! 4. same title or H1 (brand suffix removed) with a single candidate → 90,
//! 5. fuzzy score: slug Jaro-Winkler, path-token Jaccard, title+H1 TF-IDF cosine, simhash,
//!    depth and directory similarity (weighted, max 88); the best 3 candidates are kept,
//! 6. greedy one-to-one preference (highest score keeps a contested target, the others
//!    move to their next free candidate or are flagged `conflict`),
//! 7. fallbacks: parent directory (40), homepage (10) or 410 for unvalued URLs (optional).

use super::model::{status_for, Breakdown, Candidate, MatchOptions, MatchResult, Pattern, UrlRow};
use super::norm;
use crate::hub::text;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

pub const CONF_PATTERN: i64 = 100;
pub const CONF_PATTERN_MISSING: i64 = 70;
pub const CONF_SAME_PATH: i64 = 98;
pub const CONF_SAME_SLUG: i64 = 92;
pub const CONF_SAME_TITLE: i64 = 90;
pub const CONF_FUZZY_MAX: i64 = 88;
pub const CONF_CONFLICT_MAX: i64 = 65;
pub const CONF_PARENT: i64 = 40;
pub const CONF_HOMEPAGE: i64 = 10;
pub const CONF_GONE: i64 = 30;

const W_SLUG: f32 = 0.30;
const W_PATH: f32 = 0.15;
const W_TITLE: f32 = 0.30;
const W_SIMHASH: f32 = 0.15;
const W_DEPTH: f32 = 0.03;
const W_DIR: f32 = 0.07;
const TIE_BONUS: f32 = 0.10;

pub struct MatchInput<'a> {
    pub old: &'a [UrlRow],
    pub new: &'a [UrlRow],
    pub patterns: &'a [Pattern],
    pub options: &'a MatchOptions,
    pub new_base: &'a str,
    /// Old URL id → target of mappings that stay as they are (approved, manual or not
    /// selected for re-matching). They keep their targets reserved for one-to-one.
    pub fixed: &'a HashMap<i64, Option<String>>,
}

/// Compiles the enabled patterns in position order.
pub fn compile_patterns(patterns: &[Pattern]) -> Result<Vec<(i64, regex::Regex, String)>, String> {
    let mut list: Vec<&Pattern> = patterns.iter().filter(|p| p.enabled).collect();
    list.sort_by_key(|p| p.position);
    list.into_iter()
        .map(|p| {
            regex::Regex::new(&p.regex)
                .map(|re| (p.position, re, p.replacement.clone()))
                .map_err(|e| format!("Pattern #{} is not a valid regex ({}): {e}", p.position, p.regex))
        })
        .collect()
}

#[derive(Default)]
struct Interner {
    map: HashMap<String, u32>,
}

impl Interner {
    fn id(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.map.get(s) {
            return id;
        }
        let id = self.map.len() as u32;
        self.map.insert(s.to_string(), id);
        id
    }
    fn ids(&mut self, toks: &[String]) -> Vec<u32> {
        let mut v: Vec<u32> = toks.iter().map(|t| self.id(t)).collect();
        v.sort_unstable();
        v.dedup();
        v
    }
}

struct Feat {
    key: String,
    clean: String,
    slug: String,
    tokens: Vec<u32>,
    dir_tokens: Vec<u32>,
    depth: usize,
    text_tokens: Vec<u32>,
    tfidf: Vec<(u32, f32)>,
    title_key: String,
    h1_key: String,
    simhash: Option<u64>,
}

fn feat(u: &UrlRow, opts: &MatchOptions, brands: &HashSet<String>, interner: &mut Interner) -> Feat {
    let p = norm::parts(&u.url);
    let clean = norm::clean_path(&p.path);
    let dir = norm::dir_of(&clean);
    let title_clean = u.title.as_deref().map(|t| norm::title_key(t, brands)).unwrap_or_default();
    let h1_clean = u.h1.as_deref().map(|t| text::tokens(t).join(" ")).unwrap_or_default();
    let mut words: Vec<String> = title_clean.split(' ').filter(|w| !w.is_empty()).map(str::to_string).collect();
    words.extend(h1_clean.split(' ').filter(|w| !w.is_empty()).map(str::to_string));
    let text_tokens = words.iter().map(|w| interner.id(w)).collect();
    Feat {
        key: norm::match_key(&u.url, opts),
        slug: norm::slug_of(&clean),
        tokens: interner.ids(&text::tokens(&clean)),
        dir_tokens: interner.ids(&text::tokens(&dir)),
        depth: norm::depth(&clean),
        clean,
        text_tokens,
        tfidf: Vec::new(),
        title_key: title_clean,
        h1_key: h1_clean,
        simhash: u.simhash.as_deref().and_then(|h| u64::from_str_radix(h, 16).ok()),
    }
}

fn build_tfidf(feats: &mut [&mut Feat], vocab: usize) {
    let n = feats.len() as f32;
    let mut df = vec![0u32; vocab];
    for f in feats.iter() {
        let mut seen: Vec<u32> = f.text_tokens.clone();
        seen.sort_unstable();
        seen.dedup();
        for t in seen {
            df[t as usize] += 1;
        }
    }
    for f in feats.iter_mut() {
        let mut tf: HashMap<u32, f32> = HashMap::new();
        for &t in &f.text_tokens {
            *tf.entry(t).or_default() += 1.0;
        }
        let mut v: Vec<(u32, f32)> = tf.into_iter().map(|(t, c)| (t, c * (((n + 1.0) / (df[t as usize] as f32 + 1.0)).ln() + 1.0))).collect();
        let norm = v.iter().map(|(_, w)| w * w).sum::<f32>().sqrt();
        if norm > 0.0 {
            for item in &mut v {
                item.1 /= norm;
            }
        }
        v.sort_unstable_by_key(|(t, _)| *t);
        f.tfidf = v;
    }
}

fn cosine(a: &[(u32, f32)], b: &[(u32, f32)]) -> f32 {
    let (mut i, mut j, mut s) = (0, 0, 0f32);
    while i < a.len() && j < b.len() {
        match a[i].0.cmp(&b[j].0) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                s += a[i].1 * b[j].1;
                i += 1;
                j += 1;
            }
        }
    }
    s.clamp(0.0, 1.0)
}

fn jaccard(a: &[u32], b: &[u32]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let (mut i, mut j, mut inter) = (0, 0, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                inter += 1;
                i += 1;
                j += 1;
            }
        }
    }
    let union = a.len() + b.len() - inter;
    if union == 0 {
        0.0
    } else {
        inter as f32 / union as f32
    }
}

fn r2(x: f32) -> f32 {
    (x * 100.0).round() / 100.0
}

/// Weighted fuzzy similarity (0–1) and its per-signal breakdown. Returns 0 when no strong
/// signal (slug, path tokens, title, simhash) is available on both sides.
fn score(o: &Feat, n: &Feat) -> (f32, Breakdown) {
    let mut b = Breakdown::default();
    let (mut sum, mut weight, mut strong) = (0f32, 0f32, false);
    if !o.slug.is_empty() && !n.slug.is_empty() {
        let jw = strsim::jaro_winkler(&o.slug, &n.slug) as f32;
        let s = ((jw - 0.5) / 0.5).clamp(0.0, 1.0);
        b.slug = Some(r2(s));
        sum += W_SLUG * s;
        weight += W_SLUG;
        strong = true;
    }
    if !o.tokens.is_empty() && !n.tokens.is_empty() {
        let s = jaccard(&o.tokens, &n.tokens);
        b.path = Some(r2(s));
        sum += W_PATH * s;
        weight += W_PATH;
        strong = true;
    }
    if !o.tfidf.is_empty() && !n.tfidf.is_empty() {
        let s = cosine(&o.tfidf, &n.tfidf);
        b.title = Some(r2(s));
        sum += W_TITLE * s;
        weight += W_TITLE;
        strong = true;
    }
    if let (Some(x), Some(y)) = (o.simhash, n.simhash) {
        let sim = 1.0 - (x ^ y).count_ones() as f32 / 64.0;
        let s = ((sim - 0.5) / 0.5).clamp(0.0, 1.0);
        b.simhash = Some(r2(s));
        sum += W_SIMHASH * s;
        weight += W_SIMHASH;
        strong = true;
    }
    if !strong {
        return (0.0, b);
    }
    let dmax = o.depth.max(n.depth).max(1) as f32;
    let ds = 1.0 - (o.depth as f32 - n.depth as f32).abs() / dmax;
    b.depth = Some(r2(ds));
    sum += W_DEPTH * ds;
    weight += W_DEPTH;
    let dir = jaccard(&o.dir_tokens, &n.dir_tokens);
    b.dir = Some(r2(dir));
    sum += W_DIR * dir;
    weight += W_DIR;
    (sum / weight, b)
}

#[derive(Clone)]
enum Target {
    New(usize),
    Url(String),
}

struct Pre {
    old: usize,
    target: Option<(Target, i64, String)>,
    /// Fuzzy candidates: (new index, score 0–1, breakdown), best first.
    cands: Vec<(usize, f32, Breakdown)>,
    /// Deterministic targets (patterns, same path, keep) are never displaced.
    exempt: bool,
    keep: bool,
}

fn index_by<'a>(feats: &'a [Feat], f: impl Fn(&'a Feat) -> &'a str) -> HashMap<&'a str, Vec<usize>> {
    let mut m: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, ft) in feats.iter().enumerate() {
        let k = f(ft);
        if !k.is_empty() {
            m.entry(k).or_default().push(i);
        }
    }
    m
}

fn to_conf(score: f32) -> i64 {
    ((score * 100.0).round() as i64).clamp(0, CONF_FUZZY_MAX)
}

/// Runs the engine for every old URL not listed in `input.fixed`.
pub fn run(input: &MatchInput) -> Result<Vec<MatchResult>, String> {
    let opts = input.options;
    let patterns = compile_patterns(input.patterns)?;
    let old_brands = norm::brand_suffixes(input.old.iter().filter_map(|u| u.title.as_deref()));
    let new_brands = norm::brand_suffixes(input.new.iter().filter_map(|u| u.title.as_deref()));
    let mut interner = Interner::default();
    let mut old_f: Vec<Feat> = input.old.iter().map(|u| feat(u, opts, &old_brands, &mut interner)).collect();
    let mut new_f: Vec<Feat> = input.new.iter().map(|u| feat(u, opts, &new_brands, &mut interner)).collect();
    let vocab = interner.map.len();
    {
        let mut all: Vec<&mut Feat> = old_f.iter_mut().chain(new_f.iter_mut()).collect();
        build_tfidf(&mut all, vocab);
    }
    let (old_f, new_f) = (old_f, new_f);

    let by_key = index_by(&new_f, |f| f.key.as_str());
    let by_slug = index_by(&new_f, |f| f.slug.as_str());
    let by_title = index_by(&new_f, |f| f.title_key.as_str());
    let by_h1 = index_by(&new_f, |f| f.h1_key.as_str());
    let old_by_key = index_by(&old_f, |f| f.key.as_str());

    // Inverted index for candidate generation (path + title tokens).
    let mut postings: Vec<Vec<u32>> = vec![Vec::new(); vocab];
    for (j, f) in new_f.iter().enumerate() {
        let mut toks: Vec<u32> = f.tokens.iter().copied().chain(f.tfidf.iter().map(|(t, _)| *t)).collect();
        toks.sort_unstable();
        toks.dedup();
        for t in toks {
            postings[t as usize].push(j as u32);
        }
    }
    let common_limit = (new_f.len() / 2).max(50);
    let mut mark = vec![u32::MAX; new_f.len()];
    let mut cand_buf: Vec<usize> = Vec::new();

    let mut pres: Vec<Pre> = Vec::new();
    for (i, u) in input.old.iter().enumerate() {
        if input.fixed.contains_key(&u.id) {
            continue;
        }
        let o = &old_f[i];
        let mut target: Option<(Target, i64, String)> = None;
        let mut exempt = false;
        let mut keep = false;
        let mut pending: Option<(String, String)> = None;

        // 1. Patterns (first matching enabled pattern wins).
        for (pos, re, rep) in &patterns {
            if let Some(out) = norm::apply_pattern(re, rep, &o.clean) {
                let url = norm::resolve(&out, input.new_base);
                let k = norm::match_key(&url, opts);
                match by_key.get(k.as_str()) {
                    Some(js) => {
                        target = Some((Target::New(js[0]), CONF_PATTERN, format!("pattern:{pos}")));
                        exempt = true;
                    }
                    None => pending = Some((url, format!("pattern_missing:{pos}"))),
                }
                break;
            }
        }

        // 2. Same path (identical URL → keep).
        if target.is_none() {
            if let Some(js) = by_key.get(o.key.as_str()) {
                let j = js.iter().copied().find(|&j| norm::same_url(&u.url, &input.new[j].url)).unwrap_or(js[0]);
                if norm::same_url(&u.url, &input.new[j].url) {
                    keep = true;
                    target = Some((Target::New(j), 100, "same_url".into()));
                } else {
                    target = Some((Target::New(j), CONF_SAME_PATH, "same_path".into()));
                }
                exempt = true;
            } else if o.clean == "/" && pending.is_none() {
                let root = norm::resolve("/", input.new_base);
                if norm::same_url(&u.url, &root) {
                    keep = true;
                    target = Some((Target::Url(root), 100, "same_url".into()));
                } else {
                    target = Some((Target::Url(root), CONF_SAME_PATH, "same_path".into()));
                }
                exempt = true;
            }
        }

        // 3–4. Same slug, same title, same H1 (single candidate), else ties for scoring.
        let mut ties: Vec<usize> = Vec::new();
        if target.is_none() && !o.slug.is_empty() {
            if let Some(js) = by_slug.get(o.slug.as_str()) {
                if js.len() == 1 {
                    target = Some((Target::New(js[0]), CONF_SAME_SLUG, "same_slug".into()));
                } else {
                    ties.extend(js);
                }
            }
        }
        if target.is_none() && !o.title_key.is_empty() {
            if let Some(js) = by_title.get(o.title_key.as_str()) {
                if js.len() == 1 {
                    target = Some((Target::New(js[0]), CONF_SAME_TITLE, "same_title".into()));
                } else {
                    ties.extend(js);
                }
            }
        }
        if target.is_none() && !o.h1_key.is_empty() {
            if let Some(js) = by_h1.get(o.h1_key.as_str()) {
                if js.len() == 1 {
                    target = Some((Target::New(js[0]), CONF_SAME_TITLE, "same_h1".into()));
                } else {
                    ties.extend(js);
                }
            }
        }

        // 5. Fuzzy candidates (also shown in the review panel for deterministic matches).
        let stamp = i as u32;
        cand_buf.clear();
        let toks: Vec<u32> = {
            let mut t: Vec<u32> = o.tokens.iter().copied().chain(o.tfidf.iter().map(|(t, _)| *t)).collect();
            t.sort_unstable();
            t.dedup();
            t
        };
        for pass in 0..2 {
            for &t in &toks {
                let p = &postings[t as usize];
                if pass == 0 && p.len() > common_limit {
                    continue;
                }
                for &j in p {
                    if mark[j as usize] != stamp {
                        mark[j as usize] = stamp;
                        cand_buf.push(j as usize);
                    }
                }
            }
            if !cand_buf.is_empty() {
                break;
            }
        }
        for &j in &ties {
            if mark[j] != stamp {
                mark[j] = stamp;
                cand_buf.push(j);
            }
        }
        let tie_set: HashSet<usize> = ties.iter().copied().collect();
        let mut top: Vec<(usize, f32, Breakdown)> = Vec::with_capacity(4);
        for &j in &cand_buf {
            let (mut s, b) = score(o, &new_f[j]);
            if tie_set.contains(&j) {
                s = (s + TIE_BONUS).min(1.0);
            }
            if s <= 0.0 {
                continue;
            }
            if top.len() < 3 || s > top[top.len() - 1].1 {
                let pos = top.iter().position(|(_, ts, _)| s > *ts).unwrap_or(top.len());
                top.insert(pos, (j, s, b));
                top.truncate(3);
            }
        }
        if target.is_none() {
            if let Some((j, s, _)) = top.first() {
                let conf = to_conf(*s);
                if conf >= opts.fuzzy_min {
                    target = Some((Target::New(*j), conf, "fuzzy".into()));
                }
            }
            if let Some((url, reason)) = pending {
                if target.as_ref().map_or(true, |t| t.1 <= CONF_PATTERN_MISSING) {
                    target = Some((Target::Url(url), CONF_PATTERN_MISSING, reason));
                    exempt = true;
                }
            }
        }
        pres.push(Pre { old: i, target, cands: top, exempt, keep });
    }

    // 6. One-to-one preference.
    let target_key = |t: &Target| -> String {
        match t {
            Target::New(j) => norm::site_key(&input.new[*j].url, opts),
            Target::Url(u) => norm::site_key(u, opts),
        }
    };
    let mut taken: HashSet<String> = input.fixed.values().flatten().map(|u| norm::site_key(u, opts)).collect();
    let mut order: Vec<usize> = (0..pres.len()).collect();
    order.sort_by(|&a, &b| {
        let ca = pres[a].target.as_ref().map_or(-1, |t| t.1);
        let cb = pres[b].target.as_ref().map_or(-1, |t| t.1);
        let va = input.old[pres[a].old].value.unwrap_or(0.0);
        let vb = input.old[pres[b].old].value.unwrap_or(0.0);
        cb.cmp(&ca).then(vb.partial_cmp(&va).unwrap_or(std::cmp::Ordering::Equal)).then(input.old[pres[a].old].url.cmp(&input.old[pres[b].old].url))
    });
    for idx in order {
        let Some((t, conf, _)) = pres[idx].target.clone() else { continue };
        let key = target_key(&t);
        if pres[idx].exempt {
            taken.insert(key);
            continue;
        }
        if taken.insert(key) {
            continue;
        }
        let current = if let Target::New(j) = t { Some(j) } else { None };
        let alt = pres[idx]
            .cands
            .iter()
            .filter(|(j, s, _)| Some(*j) != current && to_conf(*s) >= opts.fuzzy_min)
            .find(|(j, _, _)| !taken.contains(&norm::site_key(&input.new[*j].url, opts)))
            .map(|(j, s, _)| (*j, *s));
        match alt {
            Some((j, s)) => {
                taken.insert(norm::site_key(&input.new[j].url, opts));
                pres[idx].target = Some((Target::New(j), to_conf(s), "fuzzy_shift".into()));
            }
            None => {
                pres[idx].target = Some((t, conf.min(CONF_CONFLICT_MAX), "conflict".into()));
            }
        }
    }

    // 7. Fallbacks for URLs without a target.
    let mut resolved: HashMap<usize, String> = HashMap::new();
    for p in &pres {
        if let Some((t, _, reason)) = &p.target {
            if reason != "conflict" {
                resolved.insert(p.old, target_url(t, input.new));
            }
        }
    }
    for (i, u) in input.old.iter().enumerate() {
        if let Some(Some(t)) = input.fixed.get(&u.id) {
            resolved.insert(i, t.clone());
        }
    }
    let homepage = norm::resolve("/", input.new_base);
    let mut gone: HashSet<usize> = HashSet::new();
    for p in pres.iter_mut() {
        if p.target.is_some() {
            continue;
        }
        let u = &input.old[p.old];
        if opts.gone_without_value && u.value.unwrap_or(0.0) <= 0.0 {
            gone.insert(p.old);
            continue;
        }
        let o = &old_f[p.old];
        let mut found: Option<Target> = None;
        for a in norm::ancestors(&o.clean) {
            let ak = if opts.case_insensitive { a.to_lowercase() } else { a };
            if let Some(ois) = old_by_key.get(ak.as_str()) {
                if let Some(t) = ois.iter().find_map(|oi| resolved.get(oi)) {
                    found = Some(Target::Url(t.clone()));
                    break;
                }
            }
            if let Some(js) = by_key.get(ak.as_str()) {
                found = Some(Target::New(js[0]));
                break;
            }
        }
        p.target = Some(match found {
            Some(t) => (t, CONF_PARENT, "parent".into()),
            None => (Target::Url(homepage.clone()), CONF_HOMEPAGE, "homepage".into()),
        });
    }

    // Results.
    let mut out = Vec::with_capacity(pres.len());
    for p in pres {
        let u = &input.old[p.old];
        let o = &old_f[p.old];
        if gone.contains(&p.old) {
            out.push(MatchResult {
                old_id: u.id,
                new_url: None,
                status: "gone".into(),
                confidence: CONF_GONE,
                reason: "no_value_gone".into(),
                candidates: candidates_of(&p.cands, None, input.new),
                http_status: 410,
            });
            continue;
        }
        let (t, conf, reason) = p.target.expect("every URL has a target after fallbacks");
        let url = target_url(&t, input.new);
        let first = match &t {
            Target::New(j) => Some(Candidate { url: url.clone(), score: conf, reason: reason.clone(), breakdown: score(o, &new_f[*j]).1 }),
            Target::Url(_) => Some(Candidate { url: url.clone(), score: conf, reason: reason.clone(), breakdown: Breakdown::default() }),
        };
        let status = if p.keep { "keep" } else { status_for(conf) };
        out.push(MatchResult {
            old_id: u.id,
            new_url: Some(url),
            status: status.into(),
            confidence: conf,
            reason,
            candidates: candidates_of(&p.cands, first, input.new),
            http_status: 301,
        });
    }
    Ok(out)
}

fn target_url(t: &Target, new: &[UrlRow]) -> String {
    match t {
        Target::New(j) => new[*j].url.clone(),
        Target::Url(u) => u.clone(),
    }
}

fn candidates_of(cands: &[(usize, f32, Breakdown)], first: Option<Candidate>, new: &[UrlRow]) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = first.into_iter().collect();
    for (j, s, b) in cands {
        if out.len() >= 3 {
            break;
        }
        if out.iter().any(|c| c.url == new[*j].url) {
            continue;
        }
        out.push(Candidate { url: new[*j].url.clone(), score: to_conf(*s), reason: "fuzzy".into(), breakdown: b.clone() });
    }
    out
}

// ---------------------------------------------------------------- pattern suggestions

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternSuggestion {
    pub regex: String,
    pub replacement: String,
    /// Mapped pairs that follow this directory move.
    pub support: usize,
    /// Old URLs the pattern would match.
    pub affected: usize,
    pub examples: Vec<(String, String)>,
}

fn segments(clean: &str) -> Vec<&str> {
    clean.split('/').filter(|s| !s.is_empty()).collect()
}

/// Finds directory moves (`/urunler/x` → `/magaza/x`, `/blog/2024/05/x` → `/makale/x`)
/// shared by at least `min_support` mapped pairs and proposes them as regex patterns.
pub fn suggest_patterns(pairs: &[(String, String)], old_urls: &[String], existing: &[Pattern], min_support: usize) -> Vec<PatternSuggestion> {
    let mut groups: HashMap<(String, String), (usize, Vec<(String, String)>)> = HashMap::new();
    for (old, new) in pairs {
        let oc = norm::clean_path(&norm::parts(old).path);
        let nc = norm::clean_path(&norm::parts(new).path);
        let (os, ns) = (segments(&oc), segments(&nc));
        let mut k = 0;
        while k < os.len() && k < ns.len() && os[os.len() - 1 - k].eq_ignore_ascii_case(ns[ns.len() - 1 - k]) {
            k += 1;
        }
        if k == 0 {
            continue;
        }
        let (op, np) = (&os[..os.len() - k], &ns[..ns.len() - k]);
        if op.is_empty() || op == np {
            continue;
        }
        let gen: Vec<String> = op
            .iter()
            .map(|s| if s.chars().all(|c| c.is_ascii_digit()) { format!("\\d{{{}}}", s.len()) } else { norm::regex_escape(s) })
            .collect();
        let regex = format!("^/{}/(.+)$", gen.join("/"));
        let replacement = if np.is_empty() { "/$1".to_string() } else { format!("/{}/$1", np.join("/")) };
        let entry = groups.entry((regex, replacement)).or_insert_with(|| (0, Vec::new()));
        entry.0 += 1;
        if entry.1.len() < 3 {
            entry.1.push((old.clone(), new.clone()));
        }
    }
    let cleaned: Vec<String> = old_urls.iter().map(|u| norm::clean_path(&norm::parts(u).path)).collect();
    let mut out: Vec<PatternSuggestion> = groups
        .into_iter()
        .filter(|((re, rep), (n, _))| *n >= min_support && !existing.iter().any(|p| &p.regex == re && &p.replacement == rep))
        .filter_map(|((regex, replacement), (support, examples))| {
            let re = regex::Regex::new(&regex).ok()?;
            let affected = cleaned.iter().filter(|c| re.is_match(c)).count();
            Some(PatternSuggestion { regex, replacement, support, affected, examples })
        })
        .collect();
    out.sort_by(|a, b| b.support.cmp(&a.support).then(b.affected.cmp(&a.affected)).then(a.regex.cmp(&b.regex)));
    out.truncate(10);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(id: i64, side: &str, u: &str, title: Option<&str>) -> UrlRow {
        UrlRow { id, side: side.into(), url: u.into(), title: title.map(str::to_string), ..Default::default() }
    }

    fn run_simple(old: &[UrlRow], new: &[UrlRow], patterns: &[Pattern], opts: &MatchOptions) -> HashMap<i64, MatchResult> {
        let fixed = HashMap::new();
        let input = MatchInput { old, new, patterns, options: opts, new_base: "https://yeni.com", fixed: &fixed };
        run(&input).unwrap().into_iter().map(|r| (r.old_id, r)).collect()
    }

    fn pattern(position: i64, regex: &str, replacement: &str) -> Pattern {
        Pattern { id: position, plan_id: 1, position, regex: regex.into(), replacement: replacement.into(), enabled: true, note: None }
    }

    #[test]
    fn stage1_patterns_win_with_confidence_100() {
        let old = vec![url(1, "old", "https://eski.com/blog/2024/05/kahve-rehberi/", None), url(2, "old", "https://eski.com/blog/2023/01/yok-olan/", None)];
        let new = vec![url(10, "new", "https://yeni.com/makale/kahve-rehberi/", None)];
        let pats = vec![pattern(1, r"^/blog/\d{4}/\d{2}/(.+)$", "/makale/$1")];
        let r = run_simple(&old, &new, &pats, &MatchOptions::default());
        assert_eq!(r[&1].confidence, 100);
        assert_eq!(r[&1].status, "auto");
        assert_eq!(r[&1].reason, "pattern:1");
        assert_eq!(r[&1].new_url.as_deref(), Some("https://yeni.com/makale/kahve-rehberi/"));
        // Target not in the new set: the pattern output is still proposed for review.
        assert_eq!(r[&2].reason, "pattern_missing:1");
        assert_eq!(r[&2].confidence, 70);
        assert_eq!(r[&2].new_url.as_deref(), Some("https://yeni.com/makale/yok-olan"));
    }

    #[test]
    fn stage2_same_path_and_keep() {
        let old = vec![
            url(1, "old", "https://eski.com/Hakkimizda/", None),
            url(2, "old", "https://eski.com/iletisim/index.html", None),
            url(3, "old", "https://yeni.com/ayni/", None),
        ];
        let new = vec![url(10, "new", "https://yeni.com/hakkimizda", None), url(11, "new", "https://yeni.com/iletisim/", None), url(12, "new", "https://yeni.com/ayni/", None)];
        let r = run_simple(&old, &new, &[], &MatchOptions::default());
        assert_eq!((r[&1].confidence, r[&1].reason.as_str()), (98, "same_path"));
        assert_eq!(r[&2].new_url.as_deref(), Some("https://yeni.com/iletisim/"));
        assert_eq!(r[&3].status, "keep");
        // Case-sensitive comparison turns the first one into a non-path match.
        let strict = MatchOptions { case_insensitive: false, ..Default::default() };
        let r = run_simple(&old, &new, &[], &strict);
        assert_ne!(r[&1].reason, "same_path");
    }

    #[test]
    fn stage3_same_turkish_slug() {
        let old = vec![url(1, "old", "https://eski.com/hizmetler/Kurumsal-SEO-Danışmanlığı/", None), url(2, "old", "https://eski.com/x/%C3%A7ay-%C5%9Fekeri", None)];
        let new = vec![url(10, "new", "https://yeni.com/services/kurumsal-seo-danismanligi", None), url(11, "new", "https://yeni.com/urun/cay-sekeri", None)];
        let r = run_simple(&old, &new, &[], &MatchOptions::default());
        assert_eq!((r[&1].confidence, r[&1].reason.as_str()), (92, "same_slug"));
        assert_eq!(r[&2].new_url.as_deref(), Some("https://yeni.com/urun/cay-sekeri"));
        assert_eq!(r[&2].reason, "same_slug");
    }

    #[test]
    fn stage3_ambiguous_slug_goes_to_scoring() {
        let old = vec![url(1, "old", "https://eski.com/kadin/ayakkabi/", Some("Kadın Ayakkabı | Eski"))];
        let new = vec![
            url(10, "new", "https://yeni.com/erkek/ayakkabi", Some("Erkek Ayakkabı | Yeni")),
            url(11, "new", "https://yeni.com/kadin/ayakkabi-modelleri", Some("Kadın Ayakkabı | Yeni")),
            url(12, "new", "https://yeni.com/kadin-giyim/ayakkabi", Some("Kadın Ayakkabı Modelleri | Yeni")),
        ];
        let r = run_simple(&old, &new, &[], &MatchOptions::default());
        assert_ne!(r[&1].reason, "same_slug");
        assert!(r[&1].candidates.len() >= 2);
    }

    #[test]
    fn stage4_same_title_and_h1_without_brand() {
        let mut o1 = url(1, "old", "https://eski.com/p?id=17", Some("Yaz Kampanyası Detayları | Eski Marka"));
        o1.h1 = Some("Kampanya".into());
        let mut o2 = url(2, "old", "https://eski.com/p?id=18", Some("Sayfa | Eski Marka"));
        o2.h1 = Some("Kış İndirimleri Başladı".into());
        let o3 = url(3, "old", "https://eski.com/p?id=19", Some("Diğer | Eski Marka"));
        let n1 = url(10, "new", "https://yeni.com/kampanyalar/yaz-2026", Some("Yaz Kampanyası Detayları — Yeni Marka"));
        let mut n2 = url(11, "new", "https://yeni.com/kis", Some("Kış | Yeni Marka"));
        n2.h1 = Some("Kış indirimleri başladı".into());
        let n3 = url(12, "new", "https://yeni.com/diger", Some("Başka | Yeni Marka"));
        let strict = MatchOptions { ignore_query: false, ..Default::default() };
        let r = run_simple(&[o1, o2, o3], &[n1, n2, n3], &[], &strict);
        assert_eq!((r[&1].confidence, r[&1].reason.as_str()), (90, "same_title"));
        assert_eq!(r[&1].new_url.as_deref(), Some("https://yeni.com/kampanyalar/yaz-2026"));
        assert_eq!((r[&2].confidence, r[&2].reason.as_str()), (90, "same_h1"));
    }

    #[test]
    fn stage5_fuzzy_finds_renamed_page_with_breakdown() {
        let old = vec![url(1, "old", "https://eski.com/blog/teknik-seo-rehberi-2024/", Some("Teknik SEO Rehberi 2024 | Eski"))];
        let new = vec![
            url(10, "new", "https://yeni.com/makale/teknik-seo-rehberi/", Some("Teknik SEO Rehberi | Yeni")),
            url(11, "new", "https://yeni.com/makale/yerel-seo/", Some("Yerel SEO | Yeni")),
            url(12, "new", "https://yeni.com/iletisim/", Some("İletişim | Yeni")),
        ];
        let r = run_simple(&old, &new, &[], &MatchOptions::default());
        let m = &r[&1];
        assert_eq!(m.reason, "fuzzy");
        assert_eq!(m.new_url.as_deref(), Some("https://yeni.com/makale/teknik-seo-rehberi/"));
        assert!(m.confidence >= 60 && m.confidence <= 88, "confidence {}", m.confidence);
        assert_eq!(m.status, "review");
        assert!(m.candidates[0].breakdown.slug.unwrap() > 0.8);
        assert!(m.candidates[0].breakdown.title.unwrap() > 0.5);
        assert!(m.candidates.len() >= 2 && m.candidates.len() <= 3);
    }

    #[test]
    fn stage6_one_to_one_moves_the_weaker_match() {
        let old = vec![
            url(1, "old", "https://eski.com/urun/kirmizi-elbise-modeli", Some("Kırmızı Elbise Modeli")),
            url(2, "old", "https://eski.com/urun/kirmizi-elbise-modelleri", Some("Kırmızı Elbise Modelleri")),
        ];
        let new = vec![
            url(10, "new", "https://yeni.com/magaza/kirmizi-elbise-modeli-x", Some("Kırmızı Elbise Modeli")),
            url(11, "new", "https://yeni.com/magaza/elbise-kirmizi-modeller", Some("Elbise Modelleri Kırmızı")),
        ];
        let r = run_simple(&old, &new, &[], &MatchOptions::default());
        let t1 = r[&1].new_url.clone().unwrap();
        let t2 = r[&2].new_url.clone().unwrap();
        assert_ne!(t1, t2, "one-to-one keeps targets distinct: {r:?}");
        assert_eq!(t1, "https://yeni.com/magaza/kirmizi-elbise-modeli-x");
    }

    #[test]
    fn stage6_conflict_when_no_free_candidate() {
        let old = vec![url(1, "old", "https://eski.com/a/ozel-teklif", Some("Özel Teklif")), url(2, "old", "https://eski.com/b/ozel-teklifler", Some("Özel Teklifler"))];
        let new = vec![url(10, "new", "https://yeni.com/kampanya/ozel-teklif", Some("Özel Teklif"))];
        let r = run_simple(&old, &new, &[], &MatchOptions::default());
        let reasons: HashSet<&str> = r.values().map(|m| m.reason.as_str()).collect();
        assert!(reasons.contains("conflict"), "{r:?}");
        let conflict = r.values().find(|m| m.reason == "conflict").unwrap();
        assert!(conflict.confidence <= 65);
    }

    #[test]
    fn stage7_parent_homepage_and_gone() {
        let mut v = url(3, "old", "https://eski.com/zzz/qqq", None);
        v.value = Some(12.0);
        let old = vec![
            url(1, "old", "https://eski.com/urunler/eski-model-xyz", None),
            url(2, "old", "https://eski.com/kampanya/bitti", None),
            v,
            url(4, "old", "https://eski.com/kampanya/", None),
        ];
        let new = vec![url(10, "new", "https://yeni.com/urunler/", None), url(11, "new", "https://yeni.com/firsatlar/", None)];
        let pats = vec![pattern(1, "^/kampanya$", "/firsatlar")];
        let r = run_simple(&old, &new, &pats, &MatchOptions::default());
        assert_eq!((r[&1].confidence, r[&1].reason.as_str()), (40, "parent"));
        assert_eq!(r[&1].new_url.as_deref(), Some("https://yeni.com/urunler/"));
        // The parent's own mapping is reused.
        assert_eq!(r[&2].new_url.as_deref(), Some("https://yeni.com/firsatlar/"));
        assert_eq!((r[&3].confidence, r[&3].reason.as_str()), (10, "homepage"));
        assert_eq!(r[&3].status, "weak");
        let gone = MatchOptions { gone_without_value: true, ..Default::default() };
        let r = run_simple(&old, &new, &pats, &gone);
        assert_eq!((r[&1].status.as_str(), r[&1].http_status), ("gone", 410));
        assert_eq!(r[&3].reason, "homepage", "valued URLs are never sent to 410");
    }

    #[test]
    fn fixed_mappings_are_skipped_and_reserve_targets() {
        let old = vec![url(1, "old", "https://eski.com/a/kahve", None), url(2, "old", "https://eski.com/b/kahve", None)];
        let new = vec![url(10, "new", "https://yeni.com/kahve", None)];
        let fixed: HashMap<i64, Option<String>> = [(1, Some("https://yeni.com/kahve".to_string()))].into_iter().collect();
        let opts = MatchOptions::default();
        let input = MatchInput { old: &old, new: &new, patterns: &[], options: &opts, new_base: "https://yeni.com", fixed: &fixed };
        let r = run(&input).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].old_id, 2);
        assert_eq!(r[0].reason, "conflict");
    }

    #[test]
    fn invalid_pattern_is_reported() {
        let opts = MatchOptions::default();
        let fixed = HashMap::new();
        let pats = vec![pattern(1, "^/(unclosed", "/x")];
        let input = MatchInput { old: &[], new: &[], patterns: &pats, options: &opts, new_base: "https://yeni.com", fixed: &fixed };
        assert!(run(&input).unwrap_err().contains("not a valid regex"));
    }

    #[test]
    fn suggests_directory_patterns() {
        let pairs: Vec<(String, String)> = ["a", "b", "c", "d"]
            .iter()
            .map(|s| (format!("https://eski.com/blog/2024/0{}/{s}-yazi/", s.len()), format!("https://yeni.com/makale/{s}-yazi/")))
            .chain(["x", "y", "z"].iter().map(|s| (format!("https://eski.com/urunler/{s}"), format!("https://yeni.com/magaza/{s}"))))
            .collect();
        let olds: Vec<String> = pairs.iter().map(|p| p.0.clone()).chain(["https://eski.com/urunler/w".to_string()]).collect();
        let s = suggest_patterns(&pairs, &olds, &[], 3);
        assert_eq!(s[0].regex, r"^/blog/\d{4}/\d{2}/(.+)$");
        assert_eq!(s[0].replacement, "/makale/$1");
        assert_eq!(s[0].support, 4);
        let shop = s.iter().find(|x| x.regex == "^/urunler/(.+)$").unwrap();
        assert_eq!((shop.replacement.as_str(), shop.support, shop.affected), ("/magaza/$1", 3, 4));
    }

    /// 1,000 × 1,000 synthetic plan; acceptance: ≤ 5 s in release builds.
    #[test]
    fn perf_1000_by_1000() {
        let words = ["kahve", "cay", "fincan", "demlik", "termos", "kupa", "bardak", "makine", "filtre", "cekirdek", "ogutucu", "sut", "kopuk", "sogut", "set", "hediye"];
        let mut old = Vec::new();
        let mut new = Vec::new();
        for i in 0..1000i64 {
            let a = words[(i % 16) as usize];
            let b = words[((i / 16) % 16) as usize];
            let c = words[((i * 7) % 16) as usize];
            let title = format!("{} {} {} Modeli {} | Eski Mağaza", a, b, c, i);
            let mut o = url(i + 1, "old", &format!("https://eski.com/kategori-{}/{}-{}-{}-{}", i % 37, a, b, c, i), Some(&title));
            o.value = Some((i % 50) as f64);
            old.push(o);
            // Every third URL keeps its slug, the others get a renamed slug and a new title.
            let slug = if i % 3 == 0 { format!("{}-{}-{}-{}", a, b, c, i) } else { format!("{}-{}-model-{}", b, a, i) };
            let ntitle = if i % 3 == 0 { title.replace("Eski Mağaza", "Yeni") } else { format!("{} {} Model {} | Yeni", b, a, i) };
            new.push(url(10_000 + i, "new", &format!("https://yeni.com/magaza/{}/{}", words[(i % 5) as usize], slug), Some(&ntitle)));
        }
        let start = std::time::Instant::now();
        let r = run_simple(&old, &new, &[], &MatchOptions::default());
        let elapsed = start.elapsed();
        let auto = r.values().filter(|m| m.status == "auto").count();
        let correct = r.values().filter(|m| m.new_url.as_deref().map_or(false, |u| u.ends_with(&format!("-{}", m.old_id - 1)))).count();
        eprintln!("perf_1000_by_1000: {:?}, auto {}, correct targets {}/1000", elapsed, auto, correct);
        assert_eq!(r.len(), 1000);
        assert!(correct >= 900, "correct {correct}");
        let limit = if cfg!(debug_assertions) { 60 } else { 5 };
        assert!(elapsed.as_secs_f64() <= limit as f64, "matching took {elapsed:?}");
    }
}
