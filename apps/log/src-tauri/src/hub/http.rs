//! Shared HTTP clients with an honest, identifiable user agent.

use std::time::Duration;

pub fn user_agent() -> String {
    format!("Mozilla/5.0 (compatible; {}/{}; +https://seobify.com)", crate::PRODUCT_NAME.replace(' ', ""), crate::VERSION)
}

/// Default client: follows up to 10 redirects, 30 s timeout, gzip/brotli.
pub fn client() -> reqwest::Client {
    builder().redirect(reqwest::redirect::Policy::limited(10)).build().expect("http client")
}

/// Client that never follows redirects (redirect-chain tracing: Watch, Migrate).
pub fn client_no_redirect() -> reqwest::Client {
    builder().redirect(reqwest::redirect::Policy::none()).build().expect("http client")
}

pub fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(user_agent())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .gzip(true)
        .brotli(true)
        .deflate(true)
}

/// One hop of a redirect chain.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hop {
    pub url: String,
    pub status: u16,
    pub location: Option<String>,
}

/// Follows redirects manually (max 10 hops) and returns every hop; the last hop is final.
pub async fn trace_redirects(url: &str) -> Result<Vec<Hop>, String> {
    let client = client_no_redirect();
    let mut hops = Vec::new();
    let mut current = url.to_string();
    for _ in 0..10 {
        let resp = client.get(&current).send().await.map_err(|e| format!("{current}: {e}"))?;
        let status = resp.status().as_u16();
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|loc| url::Url::parse(&current).ok()?.join(loc).ok())
            .map(|u| u.to_string());
        hops.push(Hop { url: current.clone(), status, location: location.clone() });
        match (status, location) {
            (300..=399, Some(next)) => {
                if hops.iter().any(|h| h.url == next) {
                    return Err(format!("redirect loop at {next}"));
                }
                current = next;
            }
            _ => return Ok(hops),
        }
    }
    Err(format!("more than 10 redirects starting at {url}"))
}
