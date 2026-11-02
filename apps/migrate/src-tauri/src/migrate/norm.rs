//! URL normalisation for matching and escaping helpers for rule output.
//!
//! Matching works on the *decoded* path (Turkish characters as text) without the trailing
//! slash and without index files; rule exporters re-encode as each server expects.

use super::model::MatchOptions;
use crate::hub::text;
use std::collections::{HashMap, HashSet};

const INDEX_FILES: &[&str] = &["index.html", "index.htm", "index.php", "index.asp", "index.aspx", "default.asp", "default.aspx", "default.htm"];
const PAGE_EXTS: &[&str] = &[".html", ".htm", ".php", ".aspx", ".asp", ".jsp", ".cfm"];

/// Percent-decodes `%XX` sequences (invalid sequences are kept; invalid UTF-8 is replaced).
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() && bytes[i + 1].is_ascii_hexdigit() && bytes[i + 2].is_ascii_hexdigit() {
            let hex = |b: u8| (b as char).to_digit(16).unwrap_or(0) as u8;
            out.push(hex(bytes[i + 1]) * 16 + hex(bytes[i + 2]));
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parts of a URL as a server sees them.
#[derive(Debug, Clone, PartialEq)]
pub struct Parts {
    pub scheme: String,
    /// Lowercase host (without port).
    pub host: String,
    /// Decoded path exactly as requested (trailing slash kept).
    pub path: String,
    /// Encoded path exactly as requested.
    pub raw_path: String,
    /// Raw query string (without `?`), if any.
    pub query: Option<String>,
}

pub fn parts(url: &str) -> Parts {
    match url::Url::parse(url.trim()) {
        Ok(u) => Parts {
            scheme: u.scheme().to_string(),
            host: u.host_str().unwrap_or_default().to_lowercase(),
            path: percent_decode(u.path()),
            raw_path: u.path().to_string(),
            query: u.query().filter(|q| !q.is_empty()).map(str::to_string),
        },
        Err(_) => {
            let s = url.trim();
            let (p, q) = match s.split_once('?') {
                Some((p, q)) => (p, Some(q)),
                None => (s, None),
            };
            let p = if p.starts_with('/') { p.to_string() } else { format!("/{p}") };
            Parts { scheme: String::new(), host: String::new(), path: percent_decode(&p), raw_path: p, query: q.filter(|q| !q.is_empty()).map(str::to_string) }
        }
    }
}

/// Decoded path without trailing slash, with duplicate slashes collapsed and a trailing
/// index file (`index.html`, `default.aspx`…) removed. Case is kept. Root is `/`.
pub fn clean_path(decoded: &str) -> String {
    let mut segs: Vec<&str> = decoded.split('/').filter(|s| !s.is_empty()).collect();
    if let Some(last) = segs.last() {
        if INDEX_FILES.contains(&last.to_ascii_lowercase().as_str()) {
            segs.pop();
        }
    }
    if segs.is_empty() {
        "/".into()
    } else {
        format!("/{}", segs.join("/"))
    }
}

pub fn is_tracking_param(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.starts_with("utm_") || matches!(k.as_str(), "gclid" | "fbclid" | "msclkid" | "yclid" | "_ga")
}

/// Query without tracking parameters, pairs sorted; `None` when nothing is left.
pub fn sorted_query(query: Option<&str>) -> Option<String> {
    let q = query?;
    let mut pairs: Vec<(String, String)> = url::form_urlencoded::parse(q.as_bytes())
        .filter(|(k, _)| !is_tracking_param(k))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if pairs.is_empty() {
        return None;
    }
    pairs.sort();
    Some(pairs.iter().map(|(k, v)| if v.is_empty() { k.clone() } else { format!("{k}={v}") }).collect::<Vec<_>>().join("&"))
}

/// Path key used to compare URLs across the old and the new site (host not included).
pub fn match_key(url: &str, opts: &MatchOptions) -> String {
    let p = parts(url);
    let mut key = clean_path(&p.path);
    if opts.case_insensitive {
        key = key.to_lowercase();
    }
    if !opts.ignore_query {
        if let Some(q) = sorted_query(p.query.as_deref()) {
            key.push('?');
            key.push_str(&q);
        }
    }
    key
}

/// Host (without `www.`) + path key: identifies a page across schemes.
pub fn site_key(url: &str, opts: &MatchOptions) -> String {
    format!("{}{}", text::normalize_domain(url), match_key(url, opts))
}

/// `true` when two URLs are the same resource (scheme, host, decoded path, sorted query).
pub fn same_url(a: &str, b: &str) -> bool {
    let (pa, pb) = (parts(a), parts(b));
    pa.scheme == pb.scheme
        && pa.host == pb.host
        && pa.path == pb.path
        && sorted_query(pa.query.as_deref()) == sorted_query(pb.query.as_deref())
}

/// Same host and the paths differ only by letter case or the trailing slash.
pub fn differs_only_by_case_or_slash(a: &str, b: &str) -> bool {
    let (pa, pb) = (parts(a), parts(b));
    pa.scheme == pb.scheme
        && pa.host == pb.host
        && clean_path(&pa.path).to_lowercase() == clean_path(&pb.path).to_lowercase()
        && sorted_query(pa.query.as_deref()) == sorted_query(pb.query.as_deref())
        && pa.path != pb.path
}

/// Last path segment as a Turkish-folded slug (page extension removed). Root → empty.
pub fn slug_of(clean: &str) -> String {
    let last = clean.rsplit('/').next().unwrap_or_default();
    let mut stem = last;
    for ext in PAGE_EXTS {
        if last.len() > ext.len() {
            let cut = last.len() - ext.len();
            if last.is_char_boundary(cut) && last[cut..].eq_ignore_ascii_case(ext) {
                stem = &last[..cut];
                break;
            }
        }
    }
    text::slugify(stem)
}

/// Parent directory of a cleaned path (`/a/b` → `/a`, `/a` → `/`, `/` → empty).
pub fn dir_of(clean: &str) -> String {
    if clean == "/" || clean.is_empty() {
        return String::new();
    }
    match clean.rfind('/') {
        Some(0) => "/".into(),
        Some(i) => clean[..i].to_string(),
        None => String::new(),
    }
}

/// Ancestor directories, nearest first, root excluded (`/a/b/c` → `/a/b`, `/a`).
pub fn ancestors(clean: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = dir_of(clean);
    while !cur.is_empty() && cur != "/" {
        out.push(cur.clone());
        cur = dir_of(&cur);
    }
    out
}

pub fn depth(clean: &str) -> usize {
    clean.split('/').filter(|s| !s.is_empty()).count()
}

/// Resolves a pattern output or a user-typed target against a base URL.
/// Absolute URLs are kept; `/path` and `path` are joined to the base origin.
pub fn resolve(target: &str, base: &str) -> String {
    let t = target.trim();
    if t.starts_with("http://") || t.starts_with("https://") {
        return url::Url::parse(t).map(|u| u.to_string()).unwrap_or_else(|_| t.to_string());
    }
    let base = if base.trim().is_empty() { "https://example.invalid/" } else { base.trim() };
    match url::Url::parse(base) {
        Ok(b) => {
            let path = if t.starts_with('/') { t.to_string() } else { format!("/{t}") };
            b.join(&path).map(|u| u.to_string()).unwrap_or_else(|_| format!("{}{}", base.trim_end_matches('/'), path))
        }
        Err(_) => format!("{}/{}", base.trim_end_matches('/'), t.trim_start_matches('/')),
    }
}

/// Origin (`scheme://host[:port]`) of a base URL; empty when it cannot be parsed.
pub fn origin(base: &str) -> String {
    url::Url::parse(base.trim()).map(|u| u.origin().ascii_serialization()).unwrap_or_default()
}

// ---------------------------------------------------------------- titles

const TITLE_SEPARATORS: &[&str] = &[" | ", "|", " – ", " — ", " - ", " · ", " :: "];

/// Splits `Prefix | Suffix` at the last separator.
fn split_suffix(title: &str) -> Option<(&str, &str)> {
    let mut best: Option<(usize, usize)> = None;
    for sep in TITLE_SEPARATORS {
        if let Some(i) = title.rfind(sep) {
            if best.map_or(true, |(b, _)| i > b) {
                best = Some((i, sep.len()));
            }
        }
    }
    let (i, len) = best?;
    let prefix = title[..i].trim();
    let suffix = title[i + len..].trim();
    (!prefix.is_empty() && !suffix.is_empty()).then_some((prefix, suffix))
}

/// Brand suffixes (`| Seobify`) of a site: suffixes repeated on ≥ 2 and ≥ 20 % of titles.
pub fn brand_suffixes<'a>(titles: impl Iterator<Item = &'a str>) -> HashSet<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut total = 0usize;
    for t in titles {
        let t = t.trim();
        if t.is_empty() {
            continue;
        }
        total += 1;
        if let Some((_, suffix)) = split_suffix(t) {
            *counts.entry(text::fold(suffix)).or_default() += 1;
        }
    }
    counts.into_iter().filter(|(_, c)| *c >= 2 && c * 5 >= total).map(|(s, _)| s).collect()
}

/// Comparable title: brand suffix removed, Turkish-folded word tokens joined by spaces.
pub fn title_key(title: &str, brands: &HashSet<String>) -> String {
    let t = title.trim();
    let base = match split_suffix(t) {
        Some((prefix, suffix)) if brands.contains(&text::fold(suffix)) => prefix,
        _ => t,
    };
    text::tokens(base).join(" ")
}

// ---------------------------------------------------------------- output escaping

/// Percent-encodes a decoded path (non-ASCII, spaces, `%`, `$`, quotes and other unsafe bytes).
pub fn encode_path(decoded: &str) -> String {
    let mut out = String::with_capacity(decoded.len());
    for b in decoded.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/!*'(),;:@=+&".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// ASCII-only target URL: path re-encoded from its decoded form so `%c5%9f`, `ş` and
/// `%C5%9F` all become `%C5%9F`; `$` is encoded too (it is a variable sigil in nginx).
pub fn encode_url(url: &str) -> String {
    let p = parts(url);
    let path = encode_path(&p.path);
    let query = p.query.map(|q| format!("?{}", q.replace('$', "%24").replace(' ', "%20").replace('"', "%22"))).unwrap_or_default();
    if p.host.is_empty() {
        return format!("{path}{query}");
    }
    let port = url::Url::parse(url.trim()).ok().and_then(|u| u.port()).map(|p| format!(":{p}")).unwrap_or_default();
    format!("{}://{}{}{}{}", p.scheme, p.host, port, path, query)
}

/// Escapes regex metacharacters (PCRE, .NET and Rust regex compatible).
pub fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if "\\.+*?()|[]{}^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A piece of a pattern replacement: literal text or a capture-group reference.
#[derive(Debug, Clone, PartialEq)]
pub enum RepPart {
    Lit(String),
    Group(usize),
}

/// Parses `$1`, `${1}`, `${name}` and `$$` in a replacement (Apache/PCRE semantics:
/// `$1abc` is group 1 followed by `abc`). Named groups are resolved with `names`.
pub fn parse_replacement(rep: &str, names: &[Option<String>]) -> Vec<RepPart> {
    let mut parts = Vec::new();
    let mut lit = String::new();
    let chars: Vec<char> = rep.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '$' && i + 1 < chars.len() {
            let n = chars[i + 1];
            if n == '$' {
                lit.push('$');
                i += 2;
                continue;
            }
            if let Some(d) = n.to_digit(10) {
                if !lit.is_empty() {
                    parts.push(RepPart::Lit(std::mem::take(&mut lit)));
                }
                parts.push(RepPart::Group(d as usize));
                i += 2;
                continue;
            }
            if n == '{' {
                if let Some(end) = chars[i + 2..].iter().position(|&ch| ch == '}') {
                    let name: String = chars[i + 2..i + 2 + end].iter().collect();
                    let idx = name.parse::<usize>().ok().or_else(|| names.iter().position(|nm| nm.as_deref() == Some(name.as_str())));
                    if let Some(idx) = idx {
                        if !lit.is_empty() {
                            parts.push(RepPart::Lit(std::mem::take(&mut lit)));
                        }
                        parts.push(RepPart::Group(idx));
                        i += 3 + end;
                        continue;
                    }
                }
            }
        }
        lit.push(c);
        i += 1;
    }
    if !lit.is_empty() {
        parts.push(RepPart::Lit(lit));
    }
    parts
}

/// Applies a compiled pattern to a path; `None` when it does not match.
pub fn apply_pattern(re: &regex::Regex, replacement: &str, path: &str) -> Option<String> {
    let caps = re.captures(path)?;
    let names: Vec<Option<String>> = re.capture_names().map(|n| n.map(str::to_string)).collect();
    let mut out = String::new();
    for part in parse_replacement(replacement, &names) {
        match part {
            RepPart::Lit(s) => out.push_str(&s),
            RepPart::Group(i) => out.push_str(caps.get(i).map(|m| m.as_str()).unwrap_or_default()),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_normalise_case_slash_index_and_query() {
        let o = MatchOptions::default();
        assert_eq!(match_key("https://a.com/Blog/Yazi/", &o), "/blog/yazi");
        assert_eq!(match_key("https://a.com/blog/index.html", &o), "/blog");
        assert_eq!(match_key("https://a.com/x?utm_source=1&b=2", &o), "/x");
        let strict = MatchOptions { case_insensitive: false, ignore_query: false, ..o };
        assert_eq!(match_key("https://a.com/X?b=2&a=1&utm_medium=z", &strict), "/X?a=1&b=2");
        assert_eq!(match_key("https://a.com/hizmet/%C5%9Fey/", &o), "/hizmet/şey");
        assert_eq!(site_key("http://www.a.com/x/", &o), "a.com/x");
    }

    #[test]
    fn slugs_dirs_and_resolve() {
        assert_eq!(slug_of("/hizmetler/Kurumsal-SEO-Danışmanlığı.html"), "kurumsal-seo-danismanligi");
        assert_eq!(slug_of("/"), "");
        assert_eq!(dir_of("/a/b/c"), "/a/b");
        assert_eq!(dir_of("/a"), "/");
        assert_eq!(ancestors("/a/b/c"), vec!["/a/b".to_string(), "/a".to_string()]);
        assert_eq!(resolve("/makale/x", "https://yeni.com"), "https://yeni.com/makale/x");
        assert_eq!(resolve("https://b.com/y", "https://yeni.com"), "https://b.com/y");
        assert_eq!(origin("https://yeni.com/alt/"), "https://yeni.com");
    }

    #[test]
    fn brand_suffix_is_detected_and_removed() {
        let titles = ["SEO Danışmanlığı | Seobify", "Hakkımızda | Seobify", "E-Ticaret - Mağaza", "Blog | Seobify"];
        let brands = brand_suffixes(titles.iter().copied());
        assert!(brands.contains("seobify"));
        assert!(!brands.contains("magaza"));
        assert_eq!(title_key("SEO Danışmanlığı | Seobify", &brands), "seo danismanligi");
        assert_eq!(title_key("E-Ticaret - Mağaza", &brands), "ticaret magaza");
    }

    #[test]
    fn encodes_and_escapes() {
        assert_eq!(encode_path("/hizmet/şey güzel/"), "/hizmet/%C5%9Fey%20g%C3%BCzel/");
        assert_eq!(encode_url("https://a.com/%c5%9fey?x=1"), "https://a.com/%C5%9Fey?x=1");
        assert_eq!(encode_url("https://a.com/fiyat$/"), "https://a.com/fiyat%24/");
        assert_eq!(regex_escape("/a.b(c)/"), "/a\\.b\\(c\\)/");
        assert!(same_url("https://A.com/x?b=1&a=2", "https://a.com/x?a=2&b=1"));
        assert!(!same_url("http://a.com/x", "https://a.com/x"));
        assert!(differs_only_by_case_or_slash("https://a.com/X/", "https://a.com/x"));
    }

    #[test]
    fn replacements_follow_pcre_rules() {
        let re = regex::Regex::new(r"^/blog/(\d{4})/(?P<slug>.+)$").unwrap();
        assert_eq!(apply_pattern(&re, "/makale/$2", "/blog/2024/yazi").as_deref(), Some("/makale/yazi"));
        assert_eq!(apply_pattern(&re, "/m/${slug}-$1x$$", "/blog/2024/yazi").as_deref(), Some("/m/yazi-2024x$"));
        assert_eq!(apply_pattern(&re, "/x", "/urun/1"), None);
    }
}
