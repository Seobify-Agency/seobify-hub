//! Optional content fetching for URLs without title/H1: downloads the page (max 4 at once,
//! polite delay), extracts title, H1 and the main text, and computes a 64-bit simhash
//! fingerprint used by the fuzzy matcher.

use super::Progress;
use crate::hub::text;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const MAX_BODY: usize = 2 * 1024 * 1024;
const SKIP: &[&str] = &["script", "style", "noscript", "template", "svg", "nav", "header", "footer", "aside", "form"];

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fetched {
    pub status: Option<i64>,
    pub title: Option<String>,
    pub h1: Option<String>,
    pub simhash: Option<String>,
    pub word_count: Option<i64>,
    pub error: Option<String>,
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Title, first H1 and main text (from `<main>`/`<article>` when present, else `<body>`,
/// without scripts, navigation, header and footer).
pub fn extract(html: &str) -> (Option<String>, Option<String>, String) {
    let doc = scraper::Html::parse_document(html);
    let sel = |s: &str| scraper::Selector::parse(s).expect("static selector");
    let title = doc.select(&sel("title")).next().map(|e| squash(&e.text().collect::<String>())).filter(|s| !s.is_empty());
    let h1 = doc.select(&sel("h1")).next().map(|e| squash(&e.text().collect::<String>())).filter(|s| !s.is_empty());
    let root = doc.select(&sel("main")).next().or_else(|| doc.select(&sel("article")).next()).or_else(|| doc.select(&sel("body")).next());
    let mut body = String::new();
    if let Some(root) = root {
        for node in root.descendants() {
            if let Some(t) = node.value().as_text() {
                let skipped = node.ancestors().any(|a| a.value().as_element().map_or(false, |el| SKIP.contains(&el.name())));
                if !skipped {
                    body.push_str(t);
                    body.push(' ');
                }
            }
        }
    }
    (title, h1, squash(&body))
}

fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// 64-bit simhash over folded word tokens and word bigrams (weighted by frequency).
pub fn simhash(text_in: &str) -> Option<u64> {
    let toks = text::tokens(text_in);
    if toks.len() < 5 {
        return None;
    }
    let mut weights: HashMap<String, i64> = HashMap::new();
    for t in &toks {
        *weights.entry(t.clone()).or_default() += 1;
    }
    for w in toks.windows(2) {
        *weights.entry(format!("{} {}", w[0], w[1])).or_default() += 1;
    }
    let mut v = [0i64; 64];
    for (feature, w) in &weights {
        let h = fnv1a(feature);
        for (bit, slot) in v.iter_mut().enumerate() {
            if h >> bit & 1 == 1 {
                *slot += w;
            } else {
                *slot -= w;
            }
        }
    }
    let mut out = 0u64;
    for (bit, slot) in v.iter().enumerate() {
        if *slot > 0 {
            out |= 1 << bit;
        }
    }
    Some(out)
}

pub async fn fetch_one(client: &reqwest::Client, url: &str) -> Fetched {
    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => return Fetched { error: Some(e.to_string()), ..Default::default() },
    };
    let status = resp.status().as_u16() as i64;
    let is_html = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map_or(true, |ct| ct.contains("html"));
    if !is_html {
        return Fetched { status: Some(status), error: Some("not an HTML page".into()), ..Default::default() };
    }
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return Fetched { status: Some(status), error: Some(e.to_string()), ..Default::default() },
    };
    let html = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_BODY)]).into_owned();
    let (title, h1, body) = extract(&html);
    Fetched {
        status: Some(status),
        title,
        h1,
        word_count: Some(body.split_whitespace().count() as i64),
        simhash: simhash(&body).map(|h| format!("{h:016x}")),
        error: None,
    }
}

/// Fetches `(id, url)` pairs (max 4 at once) and returns `(id, result)` in input order.
pub async fn fetch_many(items: Vec<(i64, String)>, delay_ms: u64, progress: Option<Progress>) -> Vec<(i64, Fetched)> {
    let client = crate::hub::http::client();
    let total = items.len();
    let sem = Arc::new(tokio::sync::Semaphore::new(4));
    let done = Arc::new(AtomicUsize::new(0));
    let mut set = tokio::task::JoinSet::new();
    for (idx, (id, url)) in items.into_iter().enumerate() {
        let permit = sem.clone().acquire_owned().await.expect("semaphore open");
        let (client, done, progress) = (client.clone(), done.clone(), progress.clone());
        set.spawn(async move {
            let _permit = permit;
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            let f = fetch_one(&client, &url).await;
            let n = done.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some(p) = &progress {
                p(n, total);
            }
            (idx, id, f)
        });
    }
    let mut out = Vec::with_capacity(total);
    while let Some(Ok(item)) = set.join_next().await {
        out.push(item);
    }
    out.sort_by_key(|(i, _, _)| *i);
    out.into_iter().map(|(_, id, f)| (id, f)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_title_h1_and_main_text() {
        let html = r#"<html><head><title> Kahve  Rehberi | Marka </title><style>.x{}</style></head>
            <body><nav>Menü Ana Sayfa</nav><main><h1>Kahve Rehberi</h1><p>Türk kahvesi nasıl pişirilir?</p><script>var x=1;</script></main><footer>©</footer></body></html>"#;
        let (title, h1, body) = extract(html);
        assert_eq!(title.as_deref(), Some("Kahve Rehberi | Marka"));
        assert_eq!(h1.as_deref(), Some("Kahve Rehberi"));
        assert!(body.contains("Türk kahvesi"));
        assert!(!body.contains("var x") && !body.contains("Menü"));
    }

    #[test]
    fn simhash_is_close_for_similar_texts() {
        let a = "Türk kahvesi cezvede kısık ateşte pişirilir köpüğü bol olur ve fincana yavaşça dökülür şekerli ya da sade içilir";
        let b = "Türk kahvesi cezvede kısık ateşte pişirilir köpüğü bol olur ve fincana yavaşça dökülür sade ya da şekerli tercih edilir";
        let c = "Yerel SEO için Google İşletme Profili eksiksiz doldurulmalı yorumlar yanıtlanmalı ve NAP bilgileri tutarlı olmalıdır";
        let (ha, hb, hc) = (simhash(a).unwrap(), simhash(b).unwrap(), simhash(c).unwrap());
        assert!((ha ^ hb).count_ones() < (ha ^ hc).count_ones());
        assert!((ha ^ hb).count_ones() <= 16);
        assert_eq!(simhash("çok kısa"), None);
    }
}
