//! Text and URL helpers shared by hub tools (Turkish-aware).

/// Folds Turkish and common Latin diacritics to ASCII and lowercases (`İstanbul` → `istanbul`).
pub fn fold(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        let rep = match ch {
            'ç' | 'Ç' => "c",
            'ğ' | 'Ğ' => "g",
            'ı' | 'I' | 'İ' => "i",
            'ö' | 'Ö' => "o",
            'ş' | 'Ş' => "s",
            'ü' | 'Ü' => "u",
            'â' | 'Â' | 'à' | 'á' | 'ä' | 'ã' | 'å' => "a",
            'î' | 'Î' | 'ì' | 'í' | 'ï' => "i",
            'û' | 'Û' | 'ù' | 'ú' => "u",
            'é' | 'è' | 'ê' | 'ë' | 'É' => "e",
            'ó' | 'ò' | 'ô' | 'õ' => "o",
            'ñ' => "n",
            'ß' => "ss",
            _ => {
                out.extend(ch.to_lowercase());
                continue;
            }
        };
        out.push_str(rep);
    }
    out
}

/// URL slug: folded, ASCII alphanumerics joined by `-`, max 80 chars at a word boundary.
pub fn slugify(input: &str) -> String {
    let folded = fold(input);
    let mut slug = folded.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("-");
    if slug.len() > 80 {
        let cut = slug[..80].rfind('-').unwrap_or(80);
        slug.truncate(cut);
    }
    slug
}

/// Word tokens for similarity matching (folded, ≥2 chars).
pub fn tokens(input: &str) -> Vec<String> {
    fold(input).split(|c: char| !c.is_alphanumeric()).filter(|t| t.len() >= 2).map(str::to_string).collect()
}

/// `https://www.Example.com/path` → `example.com`.
pub fn normalize_domain(input: &str) -> String {
    let s = input.trim().to_lowercase();
    let s = s.split("://").nth(1).unwrap_or(&s).to_string();
    let host = s.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.rsplit('@').next().unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    host.trim_start_matches("www.").trim_end_matches('.').to_string()
}

/// Canonical form for comparing URLs: lowercase scheme/host, no fragment, no default port,
/// no `utm_*`/`gclid`/`fbclid` params, remaining params sorted. Path case is kept.
pub fn normalize_url(input: &str) -> Option<String> {
    let mut u = url::Url::parse(input.trim()).ok()?;
    u.set_fragment(None);
    let mut pairs: Vec<(String, String)> = u
        .query_pairs()
        .filter(|(k, _)| {
            let k = k.to_lowercase();
            !(k.starts_with("utm_") || k == "gclid" || k == "fbclid" || k == "msclkid")
        })
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    pairs.sort();
    if pairs.is_empty() {
        u.set_query(None);
    } else {
        let q: String = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish();
        u.set_query(Some(&q));
    }
    Some(u.to_string())
}

/// Path of a URL without trailing slash (`/` stays `/`), percent-decoded.
pub fn url_path(input: &str) -> String {
    let path = url::Url::parse(input.trim()).map(|u| u.path().to_string()).unwrap_or_else(|_| input.to_string());
    let decoded = percent_decode(&path);
    if decoded.len() > 1 {
        decoded.trim_end_matches('/').to_string()
    } else {
        decoded
    }
}

/// Byte-based percent-decoding (a `%` followed by a multi-byte char is kept as is).
pub fn percent_decode(s: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Escapes text for HTML output.
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_turkish() {
        assert_eq!(fold("İSTANBUL Çağrı Şükrü"), "istanbul cagri sukru");
        assert_eq!(slugify("Kurumsal SEO Danışmanlığı — Hizmet!"), "kurumsal-seo-danismanligi-hizmet");
        assert_eq!(tokens("Ön İnceleme: a SEO"), vec!["on", "inceleme", "seo"]);
    }

    #[test]
    fn normalizes_urls_and_domains() {
        assert_eq!(normalize_domain("https://WWW.Example.com:443/a?b"), "example.com");
        assert_eq!(normalize_domain("example.com"), "example.com");
        assert_eq!(normalize_url("HTTPS://Example.com:443/A?utm_source=x&b=2&a=1#top").unwrap(), "https://example.com/A?a=1&b=2");
        assert_eq!(url_path("https://a.com/hizmet/%C5%9Fey/"), "/hizmet/şey");
        assert_eq!(url_path("https://a.com/"), "/");
        assert_eq!(html_escape("<a href=\"x\">&</a>"), "&lt;a href=&quot;x&quot;&gt;&amp;&lt;/a&gt;");
    }

    #[test]
    fn percent_decode_never_splits_chars() {
        assert_eq!(percent_decode("/a%ş%C5%9F"), "/a%şş");
        assert_eq!(percent_decode("%ğğ"), "%ğğ");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%41"), "%zzA");
    }
}
