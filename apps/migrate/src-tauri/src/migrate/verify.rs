//! Verification runs.
//!
//! - `post` (after launch): every exported rule's old URL is requested without following
//!   redirects. `ok` = one 301/308 hop to the expected target, which answers 2xx. Otherwise
//!   `wrong_target`, `chain` (> 1 hop), `temporary` (302/303/307), `not_redirected` (the old
//!   URL answered itself), `target_error` (target 4xx/5xx), `loop` or `error` (network).
//!   410 rules expect 410 (404 is accepted).
//! - `staging` (before launch): each distinct target, moved onto the staging host, must
//!   answer 2xx directly (`chain` when it redirects, `target_error` for 4xx/5xx).
//!
//! At most 4 requests run at once; each worker waits `delay_ms` before a request and backs
//! off on 429/503 (exponential, honours a short Retry-After).

use super::{norm, Progress};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const OUTCOMES: &[&str] = &["ok", "wrong_target", "chain", "temporary", "not_redirected", "target_error", "loop", "error"];
pub const MAX_CONCURRENCY: usize = 4;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Hop {
    pub url: String,
    pub status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Trace {
    pub hops: Vec<Hop>,
    pub looped: bool,
    pub error: Option<String>,
}

async fn get_with_backoff(client: &reqwest::Client, url: &str) -> Result<reqwest::Response, String> {
    let mut wait = Duration::from_millis(1000);
    for attempt in 0..3 {
        let resp = client.get(url).send().await.map_err(|e| short_error(&e))?;
        let status = resp.status().as_u16();
        if (status == 429 || status == 503) && attempt < 2 {
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .filter(|s| *s <= 10)
                .map(Duration::from_secs);
            tokio::time::sleep(retry_after.unwrap_or(wait)).await;
            wait *= 2;
            continue;
        }
        return Ok(resp);
    }
    unreachable!("the last attempt always returns")
}

fn short_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "timeout".into()
    } else if e.is_connect() {
        format!("connection failed: {e}")
    } else {
        e.to_string()
    }
}

/// Follows redirects manually (up to `max_hops` responses) and records every hop.
pub async fn trace(client: &reqwest::Client, url: &str, max_hops: usize) -> Trace {
    let mut t = Trace::default();
    let mut current = url.to_string();
    let mut seen: HashSet<String> = HashSet::new();
    for _ in 0..max_hops {
        seen.insert(current.clone());
        let resp = match get_with_backoff(client, &current).await {
            Ok(r) => r,
            Err(e) => {
                t.error = Some(e);
                return t;
            }
        };
        let status = resp.status().as_u16();
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|loc| url::Url::parse(&current).ok()?.join(loc).ok())
            .map(|u| u.to_string());
        drop(resp);
        t.hops.push(Hop { url: current.clone(), status, location: location.clone() });
        match (status, location) {
            (300..=399, Some(next)) => {
                if seen.contains(&next) {
                    t.looped = true;
                    return t;
                }
                current = next;
            }
            _ => return t,
        }
    }
    t.error = Some(format!("more than {max_hops} redirects"));
    t
}

/// Same scheme, host and query; paths equal ignoring the trailing slash.
fn reached(final_url: &str, expected: &str) -> bool {
    let (a, b) = (norm::parts(final_url), norm::parts(expected));
    a.scheme == b.scheme && a.host == b.host && norm::clean_path(&a.path) == norm::clean_path(&b.path) && norm::sorted_query(a.query.as_deref()) == norm::sorted_query(b.query.as_deref())
}

pub fn classify_post(expected: Option<&str>, expect_gone: bool, t: &Trace) -> &'static str {
    if t.looped {
        return "loop";
    }
    if t.hops.is_empty() || (t.error.is_some() && t.hops.last().map_or(true, |h| (300..=399).contains(&h.status))) {
        return if t.error.as_deref().map_or(false, |e| e.starts_with("more than")) { "loop" } else { "error" };
    }
    let first = &t.hops[0];
    if expect_gone {
        return match first.status {
            404 | 410 => "ok",
            300..=399 => "wrong_target",
            _ => "not_redirected",
        };
    }
    if !(300..=399).contains(&first.status) {
        return "not_redirected";
    }
    let last = t.hops.last().expect("hops");
    let redirects = t.hops.iter().filter(|h| (300..=399).contains(&h.status)).count();
    if !expected.map_or(false, |e| reached(&last.url, e)) {
        return "wrong_target";
    }
    if last.status >= 400 {
        return "target_error";
    }
    if redirects > 1 {
        return "chain";
    }
    if matches!(first.status, 302 | 303 | 307) {
        return "temporary";
    }
    if (200..=299).contains(&last.status) {
        "ok"
    } else {
        "target_error"
    }
}

pub fn classify_staging(t: &Trace) -> &'static str {
    if t.looped {
        return "loop";
    }
    match t.hops.first() {
        None => "error",
        Some(h) if (200..=299).contains(&h.status) => "ok",
        Some(h) if (300..=399).contains(&h.status) => "chain",
        Some(_) => "target_error",
    }
}

/// Moves a target onto the staging origin (`https://yeni.com/x?q` → `https://staging.yeni.com/x?q`).
pub fn rebase(target: &str, new_base: &str, staging_base: &str) -> Option<String> {
    let (t, n) = (norm::parts(target), norm::parts(new_base));
    if t.host.is_empty() || t.host.trim_start_matches("www.") != n.host.trim_start_matches("www.") {
        return None;
    }
    let origin = norm::origin(staging_base);
    if origin.is_empty() {
        return None;
    }
    Some(format!("{origin}{}{}", norm::encode_path(&t.path), t.query.map(|q| format!("?{q}")).unwrap_or_default()))
}

#[derive(Debug, Clone)]
pub struct Job {
    pub mapping_ids: Vec<i64>,
    pub source_url: String,
    pub expected_url: Option<String>,
    pub check_url: String,
    pub expect_gone: bool,
}

#[derive(Debug, Clone)]
pub struct JobResult {
    pub job: Job,
    pub outcome: String,
    pub trace: Trace,
}

/// Runs the jobs with bounded concurrency and returns results in input order.
pub async fn run(jobs: Vec<Job>, staging: bool, concurrency: usize, delay_ms: u64, progress: Option<Progress>) -> Vec<JobResult> {
    run_with_client(crate::hub::http::client_no_redirect(), jobs, staging, concurrency, delay_ms, progress).await
}

pub async fn run_with_client(client: reqwest::Client, jobs: Vec<Job>, staging: bool, concurrency: usize, delay_ms: u64, progress: Option<Progress>) -> Vec<JobResult> {
    let total = jobs.len();
    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency.clamp(1, MAX_CONCURRENCY)));
    let done = Arc::new(AtomicUsize::new(0));
    if let Some(p) = &progress {
        p(0, total);
    }
    let mut set = tokio::task::JoinSet::new();
    for (idx, job) in jobs.into_iter().enumerate() {
        let permit = sem.clone().acquire_owned().await.expect("semaphore open");
        let client = client.clone();
        let done = done.clone();
        let progress = progress.clone();
        set.spawn(async move {
            let _permit = permit;
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            let t = trace(&client, &job.check_url, 10).await;
            let outcome = if staging { classify_staging(&t) } else { classify_post(job.expected_url.as_deref(), job.expect_gone, &t) };
            let n = done.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some(p) = &progress {
                p(n, total);
            }
            (idx, JobResult { job, outcome: outcome.to_string(), trace: t })
        });
    }
    let mut out: Vec<(usize, JobResult)> = Vec::with_capacity(total);
    while let Some(res) = set.join_next().await {
        if let Ok(item) = res {
            out.push(item);
        }
    }
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
pub mod test_server {
    //! Minimal HTTP/1.1 server with fixed redirect scenarios (tests only).
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn route(path: &str) -> (u16, Option<&'static str>) {
        match path {
            "/ok" => (301, Some("/new-ok")),
            "/perm308" => (308, Some("/new-ok")),
            "/new-ok" | "/new-temp" | "/new-chain" | "/other" => (200, None),
            "/temp" => (302, Some("/new-temp")),
            "/chain" => (301, Some("/mid")),
            "/mid" => (301, Some("/new-chain")),
            "/loop-a" => (301, Some("/loop-b")),
            "/loop-b" => (301, Some("/loop-a")),
            "/wrong" => (301, Some("/other")),
            "/broken" => (301, Some("/gone-target")),
            "/removed" => (410, None),
            "/still-here" => (200, None),
            _ => (404, None),
        }
    }

    /// Starts the server on a random local port and returns its origin.
    pub async fn start() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let path = req.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/").to_string();
                    let (status, location) = route(&path);
                    let reason = match status {
                        200 => "OK",
                        301 => "Moved Permanently",
                        302 => "Found",
                        308 => "Permanent Redirect",
                        404 => "Not Found",
                        410 => "Gone",
                        _ => "X",
                    };
                    let body = format!("{status}");
                    let mut resp = format!("HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
                    if let Some(l) = location {
                        resp.push_str(&format!("Location: {l}\r\n"));
                    }
                    resp.push_str("\r\n");
                    resp.push_str(&body);
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        format!("http://{addr}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(base: &str, src: &str, expected: Option<&str>, gone: bool) -> Job {
        Job { mapping_ids: vec![1], source_url: format!("{base}{src}"), expected_url: expected.map(|e| format!("{base}{e}")), check_url: format!("{base}{src}"), expect_gone: gone }
    }

    #[tokio::test]
    async fn post_launch_scenarios_on_a_local_server() {
        let base = test_server::start().await;
        let jobs = vec![
            job(&base, "/ok", Some("/new-ok"), false),
            job(&base, "/perm308", Some("/new-ok"), false),
            job(&base, "/temp", Some("/new-temp"), false),
            job(&base, "/chain", Some("/new-chain"), false),
            job(&base, "/missing", Some("/new-missing"), false),
            job(&base, "/still-here", Some("/new-ok"), false),
            job(&base, "/loop-a", Some("/new-ok"), false),
            job(&base, "/wrong", Some("/new-ok"), false),
            job(&base, "/broken", Some("/gone-target"), false),
            job(&base, "/removed", None, true),
            job(&base, "/ok", None, true),
        ];
        let calls = Arc::new(AtomicUsize::new(0));
        let c2 = calls.clone();
        let progress: Progress = Arc::new(move |_, _| {
            c2.fetch_add(1, Ordering::SeqCst);
        });
        let results = run(jobs, false, 4, 0, Some(progress)).await;
        let outcomes: Vec<&str> = results.iter().map(|r| r.outcome.as_str()).collect();
        assert_eq!(
            outcomes,
            vec!["ok", "ok", "temporary", "chain", "not_redirected", "not_redirected", "loop", "wrong_target", "target_error", "ok", "wrong_target"]
        );
        assert_eq!(results[3].trace.hops.len(), 3);
        assert_eq!(results[3].trace.hops[0].status, 301);
        assert_eq!(calls.load(Ordering::SeqCst), 12, "initial call + one per job");
    }

    #[tokio::test]
    async fn staging_scenarios_on_a_local_server() {
        let base = test_server::start().await;
        let jobs = vec![job(&base, "/new-ok", None, false), job(&base, "/mid", None, false), job(&base, "/nope", None, false)];
        let results = run(jobs, true, 2, 5, None).await;
        let outcomes: Vec<&str> = results.iter().map(|r| r.outcome.as_str()).collect();
        assert_eq!(outcomes, vec!["ok", "chain", "target_error"]);
    }

    #[tokio::test]
    async fn network_errors_are_reported() {
        // Port 9 (discard) on localhost is closed on CI and dev machines.
        let jobs = vec![Job { mapping_ids: vec![1], source_url: "http://127.0.0.1:9/x".into(), expected_url: Some("http://127.0.0.1:9/y".into()), check_url: "http://127.0.0.1:9/x".into(), expect_gone: false }];
        let r = run(jobs, false, 1, 0, None).await;
        assert_eq!(r[0].outcome, "error");
        assert!(r[0].trace.error.is_some());
    }

    #[test]
    fn rebases_targets_onto_staging() {
        assert_eq!(rebase("https://yeni.com/hizmet/çay?x=1", "https://yeni.com", "https://staging.yeni.com").as_deref(), Some("https://staging.yeni.com/hizmet/%C3%A7ay?x=1"));
        assert_eq!(rebase("https://baska.com/x", "https://yeni.com", "https://staging.yeni.com"), None);
    }
}
