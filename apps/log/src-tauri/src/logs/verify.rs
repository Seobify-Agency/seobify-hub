//! Crawler identity verification.
//!
//! 1. Published IP ranges (Google, Bing, OpenAI, Perplexity, Apple JSON files), refreshed at
//!    most once a day and cached in `ip_ranges`.
//! 2. Reverse DNS + forward confirmation (Google's recommended method) for families with
//!    known hostnames; results are cached for 30 days in `ip_verifications`, keyed by a
//!    salted hash of the address so the cache never holds raw visitor IPs.

use super::bots::Family;
use super::ipnet::{hash_ip, Cidr};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct RangeSource {
    pub id: &'static str,
    pub owner: &'static str,
    pub url: &'static str,
    /// Disabled sources are skipped (all 11 URLs answered when verified on 2026-10-29).
    pub enabled: bool,
}

pub const SOURCES: &[RangeSource] = &[
    RangeSource { id: "google-googlebot", owner: "Google", url: "https://developers.google.com/static/search/apis/ipranges/googlebot.json", enabled: true },
    RangeSource { id: "google-special", owner: "Google", url: "https://developers.google.com/static/search/apis/ipranges/special-crawlers.json", enabled: true },
    RangeSource { id: "google-user-triggered", owner: "Google", url: "https://developers.google.com/static/search/apis/ipranges/user-triggered-fetchers.json", enabled: true },
    RangeSource { id: "google-user-triggered-google", owner: "Google", url: "https://developers.google.com/static/search/apis/ipranges/user-triggered-fetchers-google.json", enabled: true },
    RangeSource { id: "bing", owner: "Microsoft", url: "https://www.bing.com/toolbox/bingbot.json", enabled: true },
    RangeSource { id: "openai-gptbot", owner: "OpenAI", url: "https://openai.com/gptbot.json", enabled: true },
    RangeSource { id: "openai-searchbot", owner: "OpenAI", url: "https://openai.com/searchbot.json", enabled: true },
    RangeSource { id: "openai-chatgpt-user", owner: "OpenAI", url: "https://openai.com/chatgpt-user.json", enabled: true },
    RangeSource { id: "perplexity-bot", owner: "Perplexity", url: "https://www.perplexity.ai/perplexitybot.json", enabled: true },
    RangeSource { id: "perplexity-user", owner: "Perplexity", url: "https://www.perplexity.ai/perplexity-user.json", enabled: true },
    RangeSource { id: "apple", owner: "Apple", url: "https://search.developer.apple.com/applebot.json", enabled: true },
];

const RANGE_TTL_SECS: i64 = 24 * 3600;
const DNS_TTL_DAYS: i64 = 30;
pub const DNS_CONCURRENCY: usize = 16;

/// Extracts `ipv4Prefix` / `ipv6Prefix` values from a published ranges JSON.
pub fn parse_ranges_json(text: &str) -> Result<Vec<String>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("invalid JSON: {e}"))?;
    let prefixes = v.get("prefixes").and_then(Value::as_array).ok_or("no \"prefixes\" array")?;
    let out: Vec<String> = prefixes
        .iter()
        .filter_map(|p| p.get("ipv4Prefix").or_else(|| p.get("ipv6Prefix")).and_then(Value::as_str))
        .filter(|s| Cidr::parse(s).is_some())
        .map(str::to_string)
        .collect();
    if out.is_empty() {
        return Err("no usable prefixes".into());
    }
    Ok(out)
}

/// In-memory range lists by source id.
#[derive(Default, Clone)]
pub struct RangeSet {
    map: HashMap<String, Vec<Cidr>>,
}

impl RangeSet {
    pub fn insert(&mut self, source: &str, prefixes: &[String]) {
        self.map.insert(source.to_string(), prefixes.iter().filter_map(|p| Cidr::parse(p)).collect());
    }

    pub fn load(conn: &Connection) -> Result<RangeSet, String> {
        let mut set = RangeSet::default();
        let mut stmt = conn.prepare("SELECT source, json FROM ip_ranges WHERE json IS NOT NULL").map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(|e| e.to_string())?;
        for row in rows.flatten() {
            if SOURCES.iter().any(|s| s.id == row.0 && s.enabled) {
                if let Ok(list) = serde_json::from_str::<Vec<String>>(&row.1) {
                    set.insert(&row.0, &list);
                }
            }
        }
        Ok(set)
    }

    /// `Some(true/false)` when at least one of the sources is loaded; `None` when none is.
    pub fn check(&self, sources: &[&str], ip: &IpAddr) -> Option<bool> {
        let mut any = false;
        for s in sources {
            if let Some(list) = self.map.get(*s) {
                any = true;
                if list.iter().any(|c| c.contains(ip)) {
                    return Some(true);
                }
            }
        }
        any.then_some(false)
    }

    pub fn loaded(&self) -> usize {
        self.map.len()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RangeStatus {
    pub source: String,
    pub owner: String,
    pub url: String,
    pub enabled: bool,
    pub fetched_at: Option<String>,
    pub prefixes: i64,
    pub error: Option<String>,
}

pub fn range_status(conn: &Connection) -> Result<Vec<RangeStatus>, String> {
    let mut out = Vec::new();
    for s in SOURCES {
        let row: Option<(Option<String>, i64, Option<String>)> = conn
            .query_row("SELECT fetched_at, prefixes, error FROM ip_ranges WHERE source = ?1", [s.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()
            .map_err(|e| e.to_string())?;
        let (fetched_at, prefixes, error) = row.unwrap_or((None, 0, None));
        out.push(RangeStatus { source: s.id.into(), owner: s.owner.into(), url: s.url.into(), enabled: s.enabled, fetched_at, prefixes, error });
    }
    Ok(out)
}

/// `true` when every enabled source has a cached list (of any age) or was tried in the last day.
pub fn has_all_ranges(conn: &Connection) -> Result<bool, String> {
    let day_ago = (chrono::Utc::now() - chrono::Duration::days(1)).format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let cached: i64 = conn
        .query_row("SELECT COUNT(*) FROM ip_ranges WHERE json IS NOT NULL OR checked_at >= ?1", [day_ago], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    Ok(cached as usize >= SOURCES.iter().filter(|s| s.enabled).count())
}

/// Refreshes stale (older than a day) or missing range lists; `force` refetches all.
/// Network failures keep the previous list and record the error.
pub async fn refresh_ranges(force: bool) -> Result<Vec<RangeStatus>, String> {
    let stale: Vec<&RangeSource> = crate::hub::mcp::blocking(move || {
        let conn = crate::hub::db::open()?;
        let now = chrono::Utc::now().timestamp();
        let mut stale = Vec::new();
        for s in SOURCES.iter().filter(|s| s.enabled) {
            let fetched: Option<String> = conn.query_row("SELECT fetched_at FROM ip_ranges WHERE source = ?1", [s.id], |r| r.get(0)).optional().map_err(|e| e.to_string())?.flatten();
            let age = fetched.and_then(|f| super::parse::parse_any_time(&f)).map(|t| now - t);
            if force || age.map_or(true, |a| a > RANGE_TTL_SECS) {
                stale.push(s.id);
            }
        }
        Ok(stale)
    })
    .await?
    .into_iter()
    .filter_map(|id| SOURCES.iter().find(|s| s.id == id))
    .collect();
    if !stale.is_empty() {
        let client = crate::hub::http::builder().timeout(Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
        // One task per host, URLs of the same host sequentially (openai.com / perplexity.ai
        // drop parallel connections), one retry after a short pause.
        let mut by_host: Vec<(String, Vec<&'static RangeSource>)> = Vec::new();
        for s in stale {
            let host = url::Url::parse(s.url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
            match by_host.iter_mut().find(|(h, _)| *h == host) {
                Some((_, list)) => list.push(s),
                None => by_host.push((host, vec![s])),
            }
        }
        let mut tasks = Vec::new();
        for (_, sources) in by_host {
            let client = client.clone();
            tasks.push(tokio::spawn(async move {
                let mut out = Vec::new();
                for s in sources {
                    let mut result = fetch_ranges(&client, s.url).await;
                    if result.is_err() {
                        tokio::time::sleep(Duration::from_millis(1200)).await;
                        result = fetch_ranges(&client, s.url).await;
                    }
                    out.push((s.id, s.url, result));
                }
                out
            }));
        }
        let mut results = Vec::new();
        for t in tasks {
            if let Ok(r) = t.await {
                results.extend(r);
            }
        }
        crate::hub::mcp::blocking(move || {
            let conn = crate::hub::db::open()?;
            let now = crate::hub::now_iso();
            for (id, url, result) in results {
                match result {
                    Ok(list) => conn.execute(
                        "INSERT INTO ip_ranges (source, url, fetched_at, prefixes, json, error, checked_at) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?3)
                         ON CONFLICT(source) DO UPDATE SET url = excluded.url, fetched_at = excluded.fetched_at, prefixes = excluded.prefixes, json = excluded.json, error = NULL, checked_at = excluded.checked_at",
                        params![id, url, now, list.len() as i64, serde_json::to_string(&list).unwrap_or_default()],
                    ),
                    Err(e) => conn.execute(
                        "INSERT INTO ip_ranges (source, url, error, checked_at) VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT(source) DO UPDATE SET error = excluded.error, checked_at = excluded.checked_at",
                        params![id, url, e, now],
                    ),
                }
                .map_err(|e| e.to_string())?;
            }
            Ok(())
        })
        .await?;
    }
    crate::hub::mcp::blocking(|| range_status(&crate::hub::db::open()?)).await
}

async fn fetch_ranges(client: &reqwest::Client, url: &str) -> Result<Vec<String>, String> {
    let resp = client.get(url).send().await.map_err(|e| error_chain(&e))?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let text = resp.text().await.map_err(|e| error_chain(&e))?;
    parse_ranges_json(&text)
}

/// `error sending request (… : connection reset)`: reqwest errors with their causes.
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut msg = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        msg.push_str(&format!(": {c}"));
        cur = c.source();
    }
    msg
}

// ---------------------------------------------------------------- verdicts

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Verified { method: &'static str, hostname: Option<String> },
    Fake { method: &'static str },
    Unverifiable,
    /// Needs a reverse-DNS check after parsing.
    Pending,
}

impl Verdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            Verdict::Verified { .. } => "verified",
            Verdict::Fake { .. } => "fake",
            Verdict::Unverifiable => "unverifiable",
            Verdict::Pending => "pending",
        }
    }
}

/// Cloudflare edge ranges (https://www.cloudflare.com/ips-v4 and ips-v6, fetched 2026-10-31).
/// An origin server behind Cloudflare logs these instead of the visitor's address unless it
/// restores `CF-Connecting-IP`; such hits cannot be verified, but they are not fake either.
const CLOUDFLARE_EDGE: &[&str] = &[
    "173.245.48.0/20", "103.21.244.0/22", "103.22.200.0/22", "103.31.4.0/22", "141.101.64.0/18", "108.162.192.0/18", "190.93.240.0/20", "188.114.96.0/20",
    "197.234.240.0/22", "198.41.128.0/17", "162.158.0.0/15", "104.16.0.0/13", "104.24.0.0/14", "172.64.0.0/13", "131.0.72.0/22",
    "2400:cb00::/32", "2606:4700::/32", "2803:f800::/32", "2405:b500::/32", "2405:8100::/32", "2a06:98c0::/29", "2c0f:f248::/32",
];

pub fn is_cloudflare_edge(ip: &IpAddr) -> bool {
    CLOUDFLARE_EDGE.iter().filter_map(|c| Cidr::parse(c)).any(|c| c.contains(ip))
}

/// Fast, offline verification: published ranges, then the 30-day DNS cache.
pub struct Verifier {
    pub ranges: RangeSet,
    pub salt: String,
    pub use_dns: bool,
    /// Distinct Cloudflare edge addresses seen with a crawler user agent (origin behind Cloudflare).
    pub cloudflare_edge_ips: usize,
    memo: HashMap<Family, HashMap<String, Verdict>>,
}

impl Verifier {
    pub fn new(ranges: RangeSet, salt: String, use_dns: bool) -> Self {
        Verifier { ranges, salt, use_dns, cloudflare_edge_ips: 0, memo: HashMap::new() }
    }

    pub fn quick(&mut self, conn: &Connection, ip: &str, family: Family) -> Verdict {
        if family == Family::None {
            return Verdict::Unverifiable;
        }
        if let Some(v) = self.memo.get(&family).and_then(|m| m.get(ip)) {
            return v.clone();
        }
        let v = self.compute(conn, ip, family);
        self.memo.entry(family).or_default().insert(ip.to_string(), v.clone());
        v
    }

    fn compute(&mut self, conn: &Connection, ip: &str, family: Family) -> Verdict {
        let Ok(addr) = ip.trim().parse::<IpAddr>() else { return Verdict::Unverifiable };
        let spec = family.spec();
        let in_range = self.ranges.check(spec.sources, &addr);
        if in_range == Some(true) {
            return Verdict::Verified { method: "ip_range", hostname: None };
        }
        if is_cloudflare_edge(&addr) {
            self.cloudflare_edge_ips += 1;
            return Verdict::Unverifiable;
        }
        if let Some(cached) = cache_get(conn, &hash_ip(ip, &self.salt), family) {
            return cached;
        }
        if spec.dns.is_empty() {
            return if in_range == Some(false) { Verdict::Fake { method: "ip_range" } } else { Verdict::Unverifiable };
        }
        if self.use_dns {
            Verdict::Pending
        } else if in_range == Some(false) {
            Verdict::Fake { method: "ip_range" }
        } else {
            Verdict::Unverifiable
        }
    }

    /// Records DNS outcomes in the memo (so later lines of the same import reuse them).
    pub fn remember(&mut self, ip: &str, family: Family, verdict: Verdict) {
        self.memo.entry(family).or_default().insert(ip.to_string(), verdict);
    }
}

fn cache_get(conn: &Connection, key: &str, family: Family) -> Option<Verdict> {
    let cutoff = (chrono::Utc::now() - chrono::Duration::days(DNS_TTL_DAYS)).format("%Y-%m-%dT%H:%M:%SZ").to_string();
    conn.query_row(
        "SELECT verified, method, hostname FROM ip_verifications WHERE ip_key = ?1 AND bot_family = ?2 AND checked_at >= ?3",
        params![key, family.as_str(), cutoff],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)),
    )
    .optional()
    .ok()
    .flatten()
    .map(|(ok, _method, host)| if ok == 1 { Verdict::Verified { method: "dns", hostname: host } } else { Verdict::Fake { method: "dns" } })
}

pub fn cache_put(conn: &Connection, ip: &str, salt: &str, family: Family, verdict: &Verdict) -> Result<(), String> {
    let (ok, method, host) = match verdict {
        Verdict::Verified { method, hostname } => (1, *method, hostname.clone()),
        Verdict::Fake { method } => (0, *method, None),
        _ => return Ok(()),
    };
    conn.execute(
        "INSERT INTO ip_verifications (ip_key, bot_family, verified, method, hostname, checked_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(ip_key, bot_family) DO UPDATE SET verified = excluded.verified, method = excluded.method, hostname = excluded.hostname, checked_at = excluded.checked_at",
        params![hash_ip(ip, salt), family.as_str(), ok, method, host, crate::hub::now_iso()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------- reverse DNS

/// `true` when `host` equals or ends with `.suffix` for one of the suffixes.
pub fn host_matches(host: &str, suffixes: &[&str]) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    suffixes.iter().any(|s| h == *s || h.ends_with(&format!(".{s}")))
}

/// Reverse lookup (PTR), suffix check, forward confirmation (A/AAAA must contain the IP).
/// A missing PTR record means `fake`; resolver failures mean `unverifiable` (not cached).
pub fn dns_verify(ip: &IpAddr, family: Family) -> Verdict {
    let suffixes = family.spec().dns;
    let host = match resolver::ptr(ip) {
        Ok(Some(h)) => h,
        Ok(None) => return Verdict::Fake { method: "dns" },
        Err(_) => return Verdict::Unverifiable,
    };
    if !host_matches(&host, suffixes) {
        return Verdict::Fake { method: "dns" };
    }
    match resolver::forward(&host) {
        Ok(addrs) if addrs.contains(ip) => Verdict::Verified { method: "dns", hostname: Some(host) },
        Ok(_) => Verdict::Fake { method: "dns" },
        Err(_) => Verdict::Unverifiable,
    }
}

/// Checks that the system resolver answers at all (a known Googlebot address).
pub fn resolver_available() -> bool {
    let probe: IpAddr = "66.249.66.1".parse().expect("static ip");
    matches!(resolver::ptr(&probe), Ok(Some(h)) if h.contains("googlebot"))
}

/// PTR / forward lookups through the operating system resolver.
mod resolver {
    use std::net::IpAddr;

    /// `in-addr.arpa` / `ip6.arpa` name of an address.
    pub fn reverse_name(ip: &IpAddr) -> String {
        match ip {
            IpAddr::V4(v4) => {
                let o = v4.octets();
                format!("{}.{}.{}.{}.in-addr.arpa", o[3], o[2], o[1], o[0])
            }
            IpAddr::V6(v6) => {
                let mut s = String::new();
                for b in v6.octets().iter().rev() {
                    s.push_str(&format!("{:x}.{:x}.", b & 0xf, b >> 4));
                }
                s.push_str("ip6.arpa");
                s
            }
        }
    }

    fn clean(host: &str) -> String {
        host.trim_end_matches('.').to_ascii_lowercase()
    }

    // Windows: DnsQuery_W follows CNAME-delegated reverse zones (RFC 2317, used by e.g.
    // Yandex); getnameinfo() returns the bare in-addr.arpa name for those.
    #[cfg(windows)]
    mod win {
        use std::ffi::c_void;

        #[repr(C)]
        struct DnsRecordW {
            next: *mut DnsRecordW,
            name: *mut u16,
            kind: u16,
            data_length: u16,
            flags: u32,
            ttl: u32,
            reserved: u32,
            data: [u8; 16],
        }

        #[link(name = "dnsapi")]
        extern "system" {
            fn DnsQuery_W(name: *const u16, kind: u16, options: u32, extra: *mut c_void, results: *mut *mut DnsRecordW, reserved: *mut c_void) -> i32;
            fn DnsFree(data: *mut c_void, free_type: i32);
        }

        pub const PTR: u16 = 12;
        pub const A: u16 = 1;
        pub const AAAA: u16 = 28;
        const NAME_ERROR: i32 = 9003;
        const NO_RECORDS: i32 = 9501;

        pub enum Rdata {
            Name(String),
            V4([u8; 4]),
            V6([u8; 16]),
        }

        unsafe fn wide(p: *const u16) -> String {
            if p.is_null() {
                return String::new();
            }
            let mut len = 0;
            while *p.add(len) != 0 && len < 1024 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
        }

        /// Records of `kind` for `name`; `Ok(empty)` for NXDOMAIN / no data.
        pub fn query(name: &str, kind: u16) -> Result<Vec<Rdata>, String> {
            let wname: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            let mut results: *mut DnsRecordW = std::ptr::null_mut();
            // SAFETY: valid NUL-terminated name and out-pointer; the record list is only read
            // while alive and released with DnsFree(DnsFreeRecordList = 1).
            unsafe {
                let status = DnsQuery_W(wname.as_ptr(), kind, 0, std::ptr::null_mut(), &mut results, std::ptr::null_mut());
                if status != 0 {
                    if !results.is_null() {
                        DnsFree(results as *mut c_void, 1);
                    }
                    return if status == NAME_ERROR || status == NO_RECORDS { Ok(Vec::new()) } else { Err(format!("DnsQuery error {status}")) };
                }
                let mut out = Vec::new();
                let mut cur = results;
                while !cur.is_null() {
                    let rec = &*cur;
                    if rec.kind == kind {
                        match kind {
                            PTR => {
                                let target = std::ptr::read_unaligned(rec.data.as_ptr() as *const *const u16);
                                out.push(Rdata::Name(wide(target)));
                            }
                            A => out.push(Rdata::V4([rec.data[0], rec.data[1], rec.data[2], rec.data[3]])),
                            AAAA => out.push(Rdata::V6(rec.data)),
                            _ => {}
                        }
                    }
                    cur = rec.next;
                }
                DnsFree(results as *mut c_void, 1);
                Ok(out)
            }
        }
    }

    /// Host name of an address (`Ok(None)` when it has no PTR record). A PTR answer that
    /// points back into `*.arpa` is a broken resolver answer (seen with some home-router DNS
    /// proxies on CNAME-delegated reverse zones) and is reported as an error, not as "fake".
    pub fn ptr(ip: &IpAddr) -> Result<Option<String>, String> {
        #[cfg(windows)]
        {
            let recs = win::query(&reverse_name(ip), win::PTR)?;
            let names: Vec<String> = recs
                .into_iter()
                .filter_map(|r| match r {
                    win::Rdata::Name(n) if !n.is_empty() => Some(clean(&n)),
                    _ => None,
                })
                .collect();
            if !names.is_empty() && names.iter().all(|n| n.ends_with(".arpa")) {
                return Err("resolver returned an in-addr.arpa name as PTR target".into());
            }
            Ok(names.into_iter().find(|n| !n.ends_with(".arpa")))
        }
        #[cfg(not(windows))]
        {
            use dns_lookup::LookupErrorKind;
            match dns_lookup::lookup_addr(ip) {
                Ok(h) if h.ends_with(".arpa") => Err("resolver returned an in-addr.arpa name as PTR target".into()),
                Ok(h) if h.parse::<IpAddr>().is_ok() => Ok(None),
                Ok(h) => Ok(Some(clean(&h))),
                Err(e) if matches!(e.kind(), LookupErrorKind::NoName | LookupErrorKind::NoData) => Ok(None),
                Err(e) => Err(format!("{e:?}")),
            }
        }
    }

    /// Addresses of a host (A + AAAA).
    pub fn forward(host: &str) -> Result<Vec<IpAddr>, String> {
        #[cfg(windows)]
        {
            let mut out = Vec::new();
            for kind in [win::A, win::AAAA] {
                for r in win::query(host, kind)? {
                    match r {
                        win::Rdata::V4(b) => out.push(IpAddr::from(b)),
                        win::Rdata::V6(b) => out.push(IpAddr::from(b)),
                        win::Rdata::Name(_) => {}
                    }
                }
            }
            if out.is_empty() {
                // Fall back to getaddrinfo (hosts file, unusual resolvers).
                if let Ok(addrs) = dns_lookup::lookup_host(host) {
                    out.extend(addrs);
                }
            }
            Ok(out)
        }
        #[cfg(not(windows))]
        {
            use dns_lookup::LookupErrorKind;
            match dns_lookup::lookup_host(host) {
                Ok(addrs) => Ok(addrs.collect()),
                Err(e) if matches!(e.kind(), LookupErrorKind::NoName | LookupErrorKind::NoData) => Ok(Vec::new()),
                Err(e) => Err(format!("{e:?}")),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn reverse_names() {
            assert_eq!(super::reverse_name(&"66.249.66.1".parse().unwrap()), "1.66.249.66.in-addr.arpa");
            let v6 = super::reverse_name(&"2001:db8::1".parse().unwrap());
            assert!(v6.starts_with("1.0.0.0.0.0.0.0.") && v6.ends_with("8.b.d.0.1.0.0.2.ip6.arpa"), "{v6}");
        }
    }
}

/// Verifies many `(ip, family)` pairs with at most `DNS_CONCURRENCY` lookups in flight.
pub fn dns_verify_batch(items: Vec<(String, Family)>) -> Vec<(String, Family, Verdict)> {
    if items.is_empty() {
        return Vec::new();
    }
    let queue = Arc::new(Mutex::new(items));
    let results = Arc::new(Mutex::new(Vec::new()));
    let workers = DNS_CONCURRENCY.min(queue.lock().map(|q| q.len()).unwrap_or(1)).max(1);
    let mut handles = Vec::new();
    for _ in 0..workers {
        let queue = Arc::clone(&queue);
        let results = Arc::clone(&results);
        handles.push(std::thread::spawn(move || loop {
            let next = queue.lock().ok().and_then(|mut q| q.pop());
            let Some((ip, family)) = next else { break };
            let verdict = match ip.parse::<IpAddr>() {
                Ok(addr) => dns_verify(&addr, family),
                Err(_) => Verdict::Unverifiable,
            };
            if let Ok(mut r) = results.lock() {
                r.push((ip, family, verdict));
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    Arc::try_unwrap(results).ok().and_then(|m| m.into_inner().ok()).unwrap_or_default()
}

/// Per-install salt for IP pseudonyms. Kept in the OS credential store (not in the DB), so a
/// copied database alone cannot be brute-forced back to addresses; falls back to `kv`.
pub fn ip_salt(conn: &Connection) -> String {
    const KEY: &str = "ip-salt";
    if let Ok(Some(s)) = crate::hub::secrets::get(KEY) {
        return s;
    }
    let fresh = new_salt();
    if crate::hub::secrets::set(KEY, &fresh).is_ok() {
        return fresh;
    }
    if let Ok(Some(s)) = crate::hub::db::kv_get(conn, "ip_salt") {
        return s;
    }
    let _ = crate::hub::db::kv_set(conn, "ip_salt", &fresh);
    fresh
}

fn new_salt() -> String {
    use sha2::{Digest, Sha256};
    use std::hash::{BuildHasher, Hasher};
    let mut h = Sha256::new();
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.update(hasher.finish().to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    let addr = format!("{:p}", &hasher);
    h.update(addr.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn test_ranges() -> RangeSet {
        let mut r = RangeSet::default();
        r.insert("google-googlebot", &["66.249.64.0/27".into(), "2001:4860:4801:10::/64".into()]);
        r.insert("openai-gptbot", &["132.196.86.0/24".into()]);
        r
    }

    #[test]
    fn parses_published_json() {
        let text = r#"{"creationTime":"2026-10-28T15:46:02","prefixes":[{"ipv6Prefix":"2001:4860:4801:10::/64"},{"ipv4Prefix":"66.249.64.0/27"},{"ipv4Prefix":"bogus"}]}"#;
        assert_eq!(parse_ranges_json(text).unwrap(), vec!["2001:4860:4801:10::/64", "66.249.64.0/27"]);
        assert!(parse_ranges_json("{}").is_err());
        assert!(parse_ranges_json("<html>").is_err());
    }

    #[test]
    fn range_and_cache_verdicts() {
        let conn = crate::hub::db::open_memory().unwrap();
        let mut v = Verifier::new(test_ranges(), "s".into(), true);
        assert_eq!(v.quick(&conn, "66.249.64.5", Family::Google).as_str(), "verified");
        assert_eq!(v.quick(&conn, "2001:4860:4801:10::abc", Family::Google).as_str(), "verified");
        assert_eq!(v.quick(&conn, "203.0.113.9", Family::Google), Verdict::Pending, "outside ranges → reverse DNS");
        assert_eq!(v.quick(&conn, "203.0.113.9", Family::OpenAiBot).as_str(), "fake", "OpenAI has no DNS method");
        assert_eq!(v.quick(&conn, "132.196.86.10", Family::OpenAiBot).as_str(), "verified");
        assert_eq!(v.quick(&conn, "1.2.3.4", Family::PerplexityBot).as_str(), "unverifiable", "list not loaded");
        assert_eq!(v.quick(&conn, "1.2.3.4", Family::None).as_str(), "unverifiable");
        assert_eq!(v.quick(&conn, "not-an-ip", Family::Google).as_str(), "unverifiable");
        cache_put(&conn, "198.51.100.7", "s", Family::Yandex, &Verdict::Fake { method: "dns" }).unwrap();
        assert_eq!(v.quick(&conn, "198.51.100.7", Family::Yandex).as_str(), "fake", "served from the DNS cache");
        let stored: String = conn.query_row("SELECT ip_key FROM ip_verifications", [], |r| r.get(0)).unwrap();
        assert!(!stored.contains("198.51.100"), "cache keys are salted hashes");
        let mut offline = Verifier::new(test_ranges(), "s".into(), false);
        assert_eq!(offline.quick(&conn, "203.0.113.9", Family::Google).as_str(), "fake", "not in ranges, DNS off");
        assert_eq!(offline.quick(&conn, "203.0.113.9", Family::Baidu).as_str(), "unverifiable");
    }

    #[test]
    fn cloudflare_edge_addresses_are_unverifiable_not_fake() {
        let conn = crate::hub::db::open_memory().unwrap();
        for use_dns in [true, false] {
            let mut v = Verifier::new(test_ranges(), "s".into(), use_dns);
            assert_eq!(v.quick(&conn, "172.70.1.2", Family::Google).as_str(), "unverifiable");
            assert_eq!(v.quick(&conn, "2a06:98c0::1", Family::OpenAiBot).as_str(), "unverifiable");
            assert_eq!(v.quick(&conn, "172.70.1.2", Family::Google).as_str(), "unverifiable", "memoised");
            assert_eq!(v.cloudflare_edge_ips, 2);
        }
        assert!(!is_cloudflare_edge(&"66.249.64.5".parse().unwrap()));
    }

    #[test]
    fn hostname_suffixes() {
        assert!(host_matches("crawl-66-249-66-1.googlebot.com.", &["googlebot.com"]));
        assert!(host_matches("rate-limited-proxy-66-249-90-77.google.com", &["googlebot.com", "google.com"]));
        assert!(!host_matches("googlebot.com.evil.net", &["googlebot.com"]));
        assert!(!host_matches("evilgooglebot.com", &["googlebot.com"]));
        assert!(host_matches("msnbot-157-55-39-1.search.msn.com", &["search.msn.com"]));
    }

    /// Live check against a real Googlebot address. Run: cargo test live_dns -- --ignored
    #[test]
    #[ignore]
    fn live_dns_googlebot() {
        let ip: IpAddr = "66.249.66.1".parse().unwrap();
        let v = dns_verify(&ip, Family::Google);
        println!("66.249.66.1 → {v:?}");
        assert_eq!(v.as_str(), "verified");
        let fake: IpAddr = "203.0.113.50".parse().unwrap();
        assert_eq!(dns_verify(&fake, Family::Google).as_str(), "fake");
        assert!(resolver_available());
    }

    #[test]
    #[ignore]
    fn live_dns_yandex() {
        for ip in ["5.255.231.100", "213.180.203.10"] {
            let addr: IpAddr = ip.parse().unwrap();
            println!("ptr={:?}", resolver::ptr(&addr));
            if let Ok(Some(h)) = resolver::ptr(&addr) { println!("fwd={:?}", resolver::forward(&h)); }
            let v = dns_verify(&addr, Family::Yandex);
            println!("{ip} → {v:?}");
            // Some router DNS proxies break CNAME-delegated reverse zones: never "fake" then.
            assert_ne!(v.as_str(), "fake");
        }
    }
}
