//! Validation panel checks over the current mappings and chain collapsing.
//!
//! - `chain`: the target is itself redirected by another mapping (A→B, B→C),
//! - `loop`: following targets comes back to the source,
//! - `multi_source` (info): several sources share one target,
//! - `missing_target`: the target is not in the new URL set,
//! - `target_not_200`: the target's known status is not 200,
//! - `self_redirect`: source = target (should be `keep`),
//! - `redundant`: source and target differ only by letter case or the trailing slash.

use super::model::MatchOptions;
use super::norm;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

pub const KINDS: &[&str] = &["chain", "loop", "multi_source", "missing_target", "target_not_200", "self_redirect", "redundant"];

/// A mapping as seen by the validator.
#[derive(Debug, Clone)]
pub struct VMap {
    pub id: i64,
    pub source: String,
    pub target: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub kind: String,
    pub mapping_id: i64,
    pub source: String,
    pub target: Option<String>,
    /// Extra context: chain end, loop path, known status, number of sources…
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Validation {
    pub counts: BTreeMap<String, usize>,
    pub issues: Vec<Issue>,
    /// Problems that block a clean launch (everything except `multi_source`).
    pub blocking: usize,
}

fn redirects(m: &VMap) -> bool {
    m.target.is_some() && m.status != "keep" && m.status != "gone"
}

/// Follows targets from mapping `start`; returns the visited mapping indices and whether
/// the walk came back to a visited node (loop).
fn walk(start: usize, maps: &[VMap], by_source: &HashMap<String, usize>, opts: &MatchOptions) -> (Vec<usize>, bool) {
    let mut path = vec![start];
    let mut seen: HashSet<usize> = [start].into_iter().collect();
    let mut cur = start;
    while let Some(t) = maps[cur].target.as_deref() {
        let Some(&next) = by_source.get(&norm::site_key(t, opts)) else { break };
        if next == cur {
            break;
        }
        if !seen.insert(next) {
            path.push(next);
            return (path, true);
        }
        path.push(next);
        cur = next;
        if path.len() > 50 {
            return (path, true);
        }
    }
    (path, false)
}

/// `new_urls`: every URL of the new set with its known status code.
pub fn validate(maps: &[VMap], new_urls: &[(String, Option<i64>)], opts: &MatchOptions) -> Validation {
    let active: Vec<usize> = (0..maps.len()).filter(|&i| redirects(&maps[i])).collect();
    let mut by_source: HashMap<String, usize> = HashMap::new();
    for &i in &active {
        if norm::same_url(&maps[i].source, maps[i].target.as_deref().unwrap_or_default()) {
            continue;
        }
        by_source.entry(norm::site_key(&maps[i].source, opts)).or_insert(i);
    }
    let new_status: HashMap<String, Option<i64>> = new_urls.iter().map(|(u, s)| (norm::site_key(u, opts), *s)).collect();
    let mut issues = Vec::new();
    let mut per_target: HashMap<String, Vec<usize>> = HashMap::new();
    for &i in &active {
        let m = &maps[i];
        let target = m.target.as_deref().unwrap_or_default();
        let tkey = norm::site_key(target, opts);
        per_target.entry(tkey.clone()).or_default().push(i);
        let issue = |kind: &str, detail: Option<String>| Issue { kind: kind.into(), mapping_id: m.id, source: m.source.clone(), target: m.target.clone(), detail };
        if norm::same_url(&m.source, target) {
            issues.push(issue("self_redirect", None));
            continue;
        }
        if norm::differs_only_by_case_or_slash(&m.source, target) {
            issues.push(issue("redundant", None));
        }
        if by_source.get(&tkey).map_or(false, |&next| next != i) {
            let (path, looped) = walk(i, maps, &by_source, opts);
            if looped {
                let names: Vec<String> = path.iter().map(|&k| maps[k].source.clone()).collect();
                issues.push(issue("loop", Some(names.join(" → "))));
                continue;
            }
            let end = maps[*path.last().expect("path")].target.clone().unwrap_or_default();
            issues.push(issue("chain", Some(format!("{} hops → {end}", path.len()))));
        }
        match new_status.get(&tkey) {
            None => issues.push(issue("missing_target", None)),
            Some(Some(code)) if *code != 200 => issues.push(issue("target_not_200", Some(code.to_string()))),
            _ => {}
        }
    }
    let mut targets: Vec<(&String, &Vec<usize>)> = per_target.iter().filter(|(_, v)| v.len() > 1).collect();
    targets.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(b.0)));
    for (_, sources) in targets {
        let m = &maps[sources[0]];
        issues.push(Issue { kind: "multi_source".into(), mapping_id: m.id, source: m.source.clone(), target: m.target.clone(), detail: Some(sources.len().to_string()) });
    }
    let mut counts: BTreeMap<String, usize> = KINDS.iter().map(|k| (k.to_string(), 0)).collect();
    for i in &issues {
        *counts.entry(i.kind.clone()).or_default() += 1;
    }
    let blocking = issues.iter().filter(|i| i.kind != "multi_source").count();
    Validation { counts, issues, blocking }
}

/// For every chain (not loop) returns `(mapping id, final target)` so A→B→C becomes A→C.
pub fn collapse(maps: &[VMap], opts: &MatchOptions) -> Vec<(i64, String)> {
    let mut by_source: HashMap<String, usize> = HashMap::new();
    for (i, m) in maps.iter().enumerate() {
        if redirects(m) && !norm::same_url(&m.source, m.target.as_deref().unwrap_or_default()) {
            by_source.entry(norm::site_key(&m.source, opts)).or_insert(i);
        }
    }
    let mut out = Vec::new();
    for (i, m) in maps.iter().enumerate() {
        if !redirects(m) || by_source.get(&norm::site_key(m.target.as_deref().unwrap_or_default(), opts)).map_or(true, |&next| next == i) {
            continue;
        }
        let (path, looped) = walk(i, maps, &by_source, opts);
        if looped || path.len() < 2 {
            continue;
        }
        if let Some(end) = maps[*path.last().expect("path")].target.clone() {
            if !norm::same_url(&end, m.target.as_deref().unwrap_or_default()) {
                out.push((m.id, end));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(id: i64, s: &str, t: Option<&str>) -> VMap {
        VMap { id, source: s.into(), target: t.map(str::to_string), status: "auto".into() }
    }

    #[test]
    fn detects_chain_loop_and_collapses() {
        let opts = MatchOptions::default();
        let maps = vec![
            m(1, "https://a.com/a", Some("https://a.com/b")),
            m(2, "https://a.com/b", Some("https://a.com/c")),
            m(3, "https://a.com/x", Some("https://a.com/y")),
            m(4, "https://a.com/y", Some("https://a.com/x")),
        ];
        let news = vec![("https://a.com/c".to_string(), Some(200))];
        let v = validate(&maps, &news, &opts);
        assert_eq!(v.counts["chain"], 1);
        assert_eq!(v.counts["loop"], 2);
        let chain = v.issues.iter().find(|i| i.kind == "chain").unwrap();
        assert_eq!(chain.mapping_id, 1);
        assert!(chain.detail.as_deref().unwrap().ends_with("https://a.com/c"));
        let fixes = collapse(&maps, &opts);
        assert_eq!(fixes, vec![(1, "https://a.com/c".to_string())]);
    }

    #[test]
    fn longer_chains_collapse_to_the_last_target() {
        let opts = MatchOptions::default();
        let maps = vec![
            m(1, "https://a.com/1", Some("https://a.com/2/")),
            m(2, "https://a.com/2", Some("https://a.com/3")),
            m(3, "https://a.com/3", Some("https://a.com/4")),
        ];
        let fixes = collapse(&maps, &opts);
        assert!(fixes.contains(&(1, "https://a.com/4".to_string())));
        assert!(fixes.contains(&(2, "https://a.com/4".to_string())));
    }

    #[test]
    fn missing_non200_self_redundant_and_multi_source() {
        let opts = MatchOptions::default();
        let mut keep = m(9, "https://a.com/k", Some("https://a.com/k"));
        keep.status = "keep".into();
        let maps = vec![
            m(1, "https://a.com/eski1", Some("https://b.com/yok")),
            m(2, "https://a.com/eski2", Some("https://b.com/taslak")),
            m(3, "https://b.com/ayni", Some("https://b.com/ayni")),
            m(4, "https://b.com/Hakkimizda/", Some("https://b.com/hakkimizda")),
            m(5, "https://a.com/eski5", Some("https://b.com/ortak")),
            m(6, "https://a.com/eski6", Some("https://b.com/ortak")),
            keep,
        ];
        let news = vec![
            ("https://b.com/taslak".to_string(), Some(404)),
            ("https://b.com/ortak".to_string(), Some(200)),
            ("https://b.com/hakkimizda".to_string(), Some(200)),
            ("https://b.com/ayni".to_string(), None),
        ];
        let v = validate(&maps, &news, &opts);
        assert_eq!(v.counts["missing_target"], 1);
        assert_eq!(v.counts["target_not_200"], 1);
        assert_eq!(v.counts["self_redirect"], 1);
        assert_eq!(v.counts["redundant"], 1);
        assert_eq!(v.counts["multi_source"], 1);
        assert_eq!(v.issues.iter().find(|i| i.kind == "multi_source").unwrap().detail.as_deref(), Some("2"));
        assert_eq!(v.counts["chain"] + v.counts["loop"], 0);
        assert_eq!(v.blocking, 4);
    }
}
