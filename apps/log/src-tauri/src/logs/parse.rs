//! Streaming log line parsers and per-file format detection.
//!
//! Supported: Apache/Nginx combined + common (with trailing extras such as vhost, request
//! time, upstream time), Nginx/Caddy JSON lines, Cloudflare Logpush JSON, IIS W3C extended,
//! AWS CloudFront standard logs and AWS ALB access logs. Parsers borrow from the input line
//! wherever possible so the combined path allocates almost nothing per line.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Combined,
    Common,
    Json,
    Cloudflare,
    Iis,
    Cloudfront,
    Alb,
}

impl Format {
    pub const ALL: [Format; 7] = [Format::Combined, Format::Common, Format::Json, Format::Cloudflare, Format::Iis, Format::Cloudfront, Format::Alb];

    pub fn as_str(&self) -> &'static str {
        match self {
            Format::Combined => "combined",
            Format::Common => "common",
            Format::Json => "json",
            Format::Cloudflare => "cloudflare",
            Format::Iis => "iis",
            Format::Cloudfront => "cloudfront",
            Format::Alb => "alb",
        }
    }

    /// Parses a format name; `auto` / empty → `None` (detect per file).
    pub fn from_name(s: &str) -> Result<Option<Format>, String> {
        let s = s.trim().to_ascii_lowercase();
        if s.is_empty() || s == "auto" {
            return Ok(None);
        }
        Format::ALL
            .iter()
            .copied()
            .find(|f| f.as_str() == s)
            .map(Some)
            .ok_or_else(|| format!("Unknown log format \"{s}\". Use auto, combined, common, json, cloudflare, iis, cloudfront or alb."))
    }
}

/// One parsed request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rec<'a> {
    pub ip: Cow<'a, str>,
    /// Unix seconds, UTC.
    pub ts: i64,
    pub method: Cow<'a, str>,
    pub host: Option<Cow<'a, str>>,
    /// Request target as logged: `/path?query` or an absolute URL.
    pub target: Cow<'a, str>,
    pub status: u16,
    pub bytes: Option<i64>,
    pub referer: Option<Cow<'a, str>>,
    pub ua: Cow<'a, str>,
    pub response_ms: Option<f64>,
    /// Requests this line stands for (1 for real logs; aggregated Cloudflare analytics rows
    /// carry an estimated count in `SeobifyCount`).
    pub weight: u32,
    /// Cloudflare verified this crawler (`SeobifyEdgeVerified` on Cloudflare analytics rows).
    pub edge_verified: bool,
}

#[derive(Debug, PartialEq)]
pub enum Line<'a> {
    Rec(Rec<'a>),
    /// Blank line or format directive (`#Fields:` …): not counted.
    Skip,
    /// Unparseable line with a short reason.
    Bad(&'static str),
}

const IIS_DEFAULT_FIELDS: &str = "date time s-ip cs-method cs-uri-stem cs-uri-query s-port cs-username c-ip cs(User-Agent) cs(Referer) sc-status sc-substatus sc-win32-status time-taken";
const CLOUDFRONT_DEFAULT_FIELDS: &str = "date time x-edge-location sc-bytes c-ip cs-method cs(Host) cs-uri-stem sc-status cs(Referer) cs(User-Agent) cs-uri-query cs(Cookie) x-edge-result-type x-edge-request-id x-host-header cs-protocol cs-bytes time-taken x-forwarded-for ssl-protocol ssl-cipher x-edge-response-result-type cs-protocol-version fle-status fle-encrypted-fields c-port time-to-first-byte x-edge-detailed-result-type sc-content-type sc-content-len sc-range-start sc-range-end";

/// Stateful line parser (W3C formats keep the column layout from `#Fields:`).
pub struct Parser {
    pub format: Format,
    fields: Vec<String>,
    tokens: Vec<(usize, usize)>,
}

impl Parser {
    pub fn new(format: Format) -> Self {
        let fields = match format {
            Format::Iis => split_fields(IIS_DEFAULT_FIELDS),
            Format::Cloudfront => split_fields(CLOUDFRONT_DEFAULT_FIELDS),
            _ => Vec::new(),
        };
        Parser { format, fields, tokens: Vec::with_capacity(40) }
    }

    pub fn parse<'a>(&mut self, line: &'a str) -> Line<'a> {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            return Line::Skip;
        }
        match self.format {
            Format::Combined | Format::Common => to_line(parse_combined(line)),
            Format::Json => to_line(parse_json(line)),
            Format::Cloudflare => to_line(parse_cloudflare(line)),
            Format::Iis | Format::Cloudfront => {
                if let Some(rest) = line.strip_prefix('#') {
                    if let Some(f) = rest.strip_prefix("Fields:") {
                        self.fields = split_fields(f);
                    }
                    return Line::Skip;
                }
                let sep = if self.format == Format::Cloudfront { '\t' } else { ' ' };
                to_line(parse_w3c(line, &self.fields, sep, self.format == Format::Cloudfront))
            }
            Format::Alb => {
                self.tokens.clear();
                split_quoted(line, &mut self.tokens);
                to_line(parse_alb(line, &self.tokens))
            }
        }
    }
}

fn to_line<'a>(r: Result<Rec<'a>, &'static str>) -> Line<'a> {
    match r {
        Ok(rec) => Line::Rec(rec),
        Err(e) => Line::Bad(e),
    }
}

fn split_fields(s: &str) -> Vec<String> {
    s.split_whitespace().map(|f| f.to_ascii_lowercase()).collect()
}

// ---------------------------------------------------------------- detection

/// Picks the format that parses the most sample lines (first ~50 lines of a file).
pub fn detect(sample: &[&str]) -> Option<Format> {
    let lines: Vec<&str> = sample.iter().map(|l| l.trim_end_matches(['\r', '\n'])).filter(|l| !l.trim().is_empty()).collect();
    if let Some(fields) = lines.iter().find_map(|l| l.strip_prefix("#Fields:")) {
        let f = fields.to_ascii_lowercase();
        return Some(if f.contains("x-edge-location") || f.contains("x-edge-request-id") { Format::Cloudfront } else { Format::Iis });
    }
    let data: Vec<&str> = lines.iter().copied().filter(|l| !l.starts_with('#')).take(50).collect();
    if data.is_empty() {
        return None;
    }
    if data[0].trim_start().starts_with('{') {
        let v: Value = serde_json::from_str(data[0]).ok()?;
        let cf = ["ClientRequestURI", "EdgeStartTimestamp", "ClientRequestHost", "EdgeResponseStatus"];
        return Some(if cf.iter().any(|k| v.get(k).is_some()) { Format::Cloudflare } else { Format::Json });
    }
    let mut best: Option<(Format, usize)> = None;
    for format in [Format::Combined, Format::Alb, Format::Cloudfront, Format::Iis] {
        let mut parser = Parser::new(format);
        let ok = data.iter().filter(|l| matches!(parser.parse(l), Line::Rec(_))).count();
        if ok > 0 && best.map_or(true, |(_, n)| ok > n) {
            best = Some((format, ok));
        }
    }
    let (format, _) = best?;
    if format == Format::Combined {
        let with_ua = data.iter().filter(|l| matches!(parse_combined(l), Ok(r) if !r.ua.is_empty())).count();
        let parsed = data.iter().filter(|l| parse_combined(l).is_ok()).count();
        if with_ua * 2 < parsed {
            return Some(Format::Common);
        }
    }
    Some(format)
}

// ---------------------------------------------------------------- combined / common

/// Apache/Nginx combined or common, optional `vhost:port` prefix, optional trailing extras.
pub fn parse_combined(line: &str) -> Result<Rec<'_>, &'static str> {
    let lb = line.find(" [").ok_or("missing [timestamp]")?;
    let prefix = &line[..lb];
    let mut parts = prefix.split(' ').filter(|p| !p.is_empty());
    let first = parts.next().ok_or("missing client address")?;
    let count = prefix.split(' ').filter(|p| !p.is_empty()).count();
    let (mut host, ip) = if count >= 4 {
        // vhost_combined: "example.com:443 1.2.3.4 - - [...]"
        let ip = parts.next().unwrap_or(first);
        (Some(Cow::Borrowed(strip_port(first))), ip)
    } else {
        (None, first)
    };
    let after_lb = &line[lb + 2..];
    let rb = after_lb.find(']').ok_or("missing ]")?;
    let ts = parse_clf_time(&after_lb[..rb]).ok_or("bad timestamp")?;
    let rest = after_lb[rb + 1..].trim_start();
    let (request, rest) = read_quoted(rest).ok_or("missing \"request\"")?;
    let rest = rest.trim_start();
    let (status_tok, rest) = next_token(rest);
    let status = parse_status(status_tok).ok_or("bad status")?;
    let (bytes_tok, rest) = next_token(rest.trim_start());
    let bytes = bytes_tok.parse::<i64>().ok();
    let (method, target) = split_request(request);

    let mut rec = Rec { ip: Cow::Borrowed(ip), ts, method: Cow::Borrowed(method), host: None, target: Cow::Borrowed(target), status, bytes, referer: None, ua: Cow::Borrowed(""), response_ms: None, weight: 1, edge_verified: false };
    let mut rest = rest.trim_start();
    if let Some((referer, r)) = read_quoted(rest) {
        rec.referer = non_dash(referer).map(Cow::Borrowed);
        rest = r.trim_start();
        if let Some((ua, r)) = read_quoted(rest) {
            rec.ua = Cow::Borrowed(if ua == "-" { "" } else { ua });
            rest = r.trim_start();
        }
    }
    // Trailing extras: "x-forwarded-for", rt=0.12, 0.120 (seconds), 1234 (µs), vhost.
    let mut forwarded: Option<&str> = None;
    while !rest.is_empty() {
        let (tok, r) = if rest.starts_with('"') {
            match read_quoted(rest) {
                Some((q, r)) => {
                    if forwarded.is_none() && q != "-" && looks_like_ip(q.split(',').next().unwrap_or("").trim()) {
                        forwarded = Some(q);
                    } else if host.is_none() && looks_like_host(q) {
                        host = Some(Cow::Borrowed(strip_port(q)));
                    }
                    ("", r)
                }
                None => break,
            }
        } else {
            next_token(rest)
        };
        rest = r.trim_start();
        if tok.is_empty() || tok == "-" {
            continue;
        }
        if let Some((k, v)) = tok.split_once('=') {
            let k = k.to_ascii_lowercase();
            match k.as_str() {
                "rt" | "request_time" | "rtime" => rec.response_ms = rec.response_ms.or_else(|| seconds_ms(v)),
                "urt" | "upstream_response_time" | "ut" => {
                    if rec.response_ms.is_none() {
                        rec.response_ms = seconds_ms(v);
                    }
                }
                "host" | "vhost" | "server" => host = host.or(Some(Cow::Borrowed(strip_port(v.trim_matches('"'))))),
                _ => {}
            }
            continue;
        }
        if rec.response_ms.is_none() {
            if tok.contains('.') && tok.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
                rec.response_ms = seconds_ms(tok);
                continue;
            }
            if tok.bytes().all(|b| b.is_ascii_digit()) {
                rec.response_ms = tok.parse::<f64>().ok().map(|us| us / 1000.0);
                continue;
            }
        }
        if host.is_none() && looks_like_host(tok) {
            host = Some(Cow::Borrowed(strip_port(tok)));
        }
    }
    if let Some(xff) = forwarded {
        let client = xff.split(',').next().unwrap_or("").trim();
        if is_private_ip(ip) && looks_like_ip(client) && !is_private_ip(client) {
            rec.ip = Cow::Borrowed(client);
        }
    }
    rec.host = host;
    Ok(rec)
}

fn seconds_ms(v: &str) -> Option<f64> {
    // nginx may log "0.010, 0.020" for several upstreams; take the first.
    v.split(',').next()?.trim().parse::<f64>().ok().filter(|s| s.is_finite() && *s >= 0.0).map(|s| s * 1000.0)
}

fn parse_status(tok: &str) -> Option<u16> {
    let s: u16 = tok.parse().ok()?;
    (100..=999).contains(&s).then_some(s)
}

fn split_request(request: &str) -> (&str, &str) {
    let mut it = request.splitn(3, ' ');
    let method = it.next().unwrap_or("-");
    match it.next() {
        Some(target) if !target.is_empty() => (method, target),
        _ => ("-", if request.is_empty() { "-" } else { request }),
    }
}

fn next_token(s: &str) -> (&str, &str) {
    match s.find([' ', '\t']) {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    }
}

/// Reads a `"…"` token (backslash escapes kept raw). Returns `(inner, rest)`.
fn read_quoted(s: &str) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    if b.first() != Some(&b'"') {
        return None;
    }
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => return Some((&s[1..i], &s[i + 1..])),
            _ => i += 1,
        }
    }
    None
}

fn non_dash(s: &str) -> Option<&str> {
    let s = s.trim();
    (!s.is_empty() && s != "-").then_some(s)
}

fn strip_port(s: &str) -> &str {
    if s.starts_with('[') {
        return s.trim_start_matches('[').split(']').next().unwrap_or(s);
    }
    match s.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') && !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => s,
    }
}

fn looks_like_ip(s: &str) -> bool {
    s.parse::<std::net::IpAddr>().is_ok()
}

fn looks_like_host(s: &str) -> bool {
    let h = strip_port(s);
    h.contains('.') && !h.contains('/') && !h.contains(' ') && h.bytes().any(|b| b.is_ascii_alphabetic()) && h.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

/// RFC 1918 / loopback / link-local / unique-local addresses (proxy hops).
pub fn is_private_ip(s: &str) -> bool {
    match s.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        Ok(std::net::IpAddr::V6(v6)) => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80,
        Err(_) => false,
    }
}

// ---------------------------------------------------------------- JSON lines (nginx / caddy)

const J_IP: &[&str] = &["remote_addr", "client_ip", "remote_ip", "clientip", "ip", "request.client_ip", "request.remote_ip"];
const J_TIME: &[&str] = &["time_local", "time_iso8601", "time", "timestamp", "@timestamp", "ts", "msec", "date"];
const J_METHOD: &[&str] = &["method", "request_method", "request.method"];
const J_URI: &[&str] = &["request_uri", "uri", "request.uri", "path", "url"];
const J_ARGS: &[&str] = &["args", "query_string"];
const J_HOST: &[&str] = &["host", "http_host", "server_name", "vhost", "request.host"];
const J_STATUS: &[&str] = &["status", "response_status", "status_code"];
const J_BYTES: &[&str] = &["body_bytes_sent", "bytes_sent", "size", "response_size"];
const J_UA: &[&str] = &["http_user_agent", "user_agent", "useragent", "agent", "request.headers.User-Agent", "request.headers.user-agent"];
const J_REFERER: &[&str] = &["http_referer", "referer", "referrer", "request.headers.Referer", "request.headers.referer"];
const J_RTIME: &[&str] = &["request_time", "duration", "response_time", "upstream_response_time"];

fn jget<'v>(v: &'v Value, path: &str) -> Option<&'v Value> {
    if let Some(x) = v.get(path) {
        return Some(x);
    }
    if !path.contains('.') {
        return None;
    }
    path.split('.').try_fold(v, |acc, k| acc.get(k))
}

fn jstr(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| match jget(v, k)? {
        Value::String(s) if !s.is_empty() && s != "-" => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Array(a) => a.first().and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string),
        _ => None,
    })
}

fn jnum(v: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| match jget(v, k)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.split(',').next()?.trim().parse::<f64>().ok(),
        _ => None,
    })
}

fn jtime(v: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|k| match jget(v, k)? {
        Value::Number(n) => n.as_f64().map(unix_any),
        Value::String(s) => parse_any_time(s),
        _ => None,
    })
}

fn parse_json(line: &str) -> Result<Rec<'static>, &'static str> {
    let v: Value = serde_json::from_str(line).map_err(|_| "invalid JSON")?;
    let ts = jtime(&v, J_TIME).ok_or("missing time")?;
    let status = jnum(&v, J_STATUS).map(|s| s as u16).filter(|s| (100..=999).contains(s)).ok_or("missing status")?;
    let (method, target) = match jstr(&v, &["request"]) {
        Some(req) if req.contains(' ') => {
            let (m, t) = split_request(&req);
            (m.to_string(), t.to_string())
        }
        _ => {
            let uri = jstr(&v, J_URI).ok_or("missing request/uri")?;
            let target = match jstr(&v, J_ARGS) {
                Some(args) if !uri.contains('?') => format!("{uri}?{args}"),
                _ => uri,
            };
            (jstr(&v, J_METHOD).unwrap_or_else(|| "GET".into()), target)
        }
    };
    let rtime = jnum(&v, J_RTIME).filter(|s| s.is_finite() && *s >= 0.0);
    Ok(Rec {
        ip: Cow::Owned(jstr(&v, J_IP).unwrap_or_default()),
        ts,
        method: Cow::Owned(method),
        host: jstr(&v, J_HOST).map(Cow::Owned),
        target: Cow::Owned(target),
        status,
        bytes: jnum(&v, J_BYTES).map(|b| b as i64),
        referer: jstr(&v, J_REFERER).map(Cow::Owned),
        ua: Cow::Owned(jstr(&v, J_UA).unwrap_or_default()),
        response_ms: rtime.map(|s| s * 1000.0),
        weight: 1,
        edge_verified: false,
    })
}

// ---------------------------------------------------------------- Cloudflare Logpush

fn parse_cloudflare(line: &str) -> Result<Rec<'static>, &'static str> {
    let v: Value = serde_json::from_str(line).map_err(|_| "invalid JSON")?;
    let ts = jtime(&v, &["EdgeStartTimestamp", "EdgeEndTimestamp"]).ok_or("missing EdgeStartTimestamp")?;
    let status = jnum(&v, &["EdgeResponseStatus", "OriginResponseStatus"]).map(|s| s as u16).filter(|s| (100..=999).contains(s)).ok_or("missing EdgeResponseStatus")?;
    let target = jstr(&v, &["ClientRequestURI", "ClientRequestPath"]).ok_or("missing ClientRequestURI")?;
    Ok(Rec {
        ip: Cow::Owned(jstr(&v, &["ClientIP"]).unwrap_or_default()),
        ts,
        method: Cow::Owned(jstr(&v, &["ClientRequestMethod"]).unwrap_or_else(|| "GET".into())),
        host: jstr(&v, &["ClientRequestHost"]).map(Cow::Owned),
        target: Cow::Owned(target),
        status,
        bytes: jnum(&v, &["EdgeResponseBytes", "EdgeResponseBodyBytes"]).map(|b| b as i64),
        referer: jstr(&v, &["ClientRequestReferer"]).map(Cow::Owned),
        ua: Cow::Owned(jstr(&v, &["ClientRequestUserAgent"]).unwrap_or_default()),
        response_ms: jnum(&v, &["EdgeTimeToFirstByteMs", "OriginResponseDurationMs"]),
        weight: jnum(&v, &["SeobifyCount"]).map(|n| n.clamp(1.0, 1_000_000.0) as u32).unwrap_or(1),
        edge_verified: jget(&v, "SeobifyEdgeVerified").and_then(Value::as_bool).unwrap_or(false),
    })
}

// ---------------------------------------------------------------- W3C (IIS, CloudFront)

fn parse_w3c<'a>(line: &'a str, fields: &[String], sep: char, cloudfront: bool) -> Result<Rec<'a>, &'static str> {
    let cols: Vec<&str> = if sep == ' ' { line.split(' ').filter(|c| !c.is_empty()).collect() } else { line.split(sep).collect() };
    if cols.len() < fields.len().min(8) {
        return Err("too few columns");
    }
    let get = |name: &str| -> Option<&'a str> { fields.iter().position(|f| f == name).and_then(|i| cols.get(i).copied()).filter(|c| !c.is_empty() && *c != "-") };
    let date = get("date").ok_or("missing date")?;
    let time = get("time").ok_or("missing time")?;
    let ts = parse_any_time(&format!("{date}T{time}Z")).ok_or("bad date/time")?;
    let status = get("sc-status").and_then(parse_status).ok_or("bad status")?;
    let stem = get("cs-uri-stem").ok_or("missing cs-uri-stem")?;
    let target = match get("cs-uri-query") {
        Some(q) => Cow::Owned(format!("{stem}?{q}")),
        None => Cow::Borrowed(stem),
    };
    let decode = |s: &'a str| -> Cow<'a, str> {
        if cloudfront {
            let once = percent_decode(s);
            if once.contains('%') { Cow::Owned(percent_decode(&once)) } else { Cow::Owned(once) }
        } else if s.contains('+') {
            Cow::Owned(s.replace('+', " "))
        } else {
            Cow::Borrowed(s)
        }
    };
    let taken = get("time-taken").and_then(|t| t.parse::<f64>().ok());
    Ok(Rec {
        ip: Cow::Borrowed(get("c-ip").unwrap_or("")),
        ts,
        method: Cow::Borrowed(get("cs-method").unwrap_or("GET")),
        host: get("x-host-header").or_else(|| get("cs-host")).or_else(|| get("cs(host)")).map(Cow::Borrowed),
        target,
        status,
        bytes: get("sc-bytes").and_then(|b| b.parse().ok()),
        referer: get("cs(referer)").map(decode),
        ua: get("cs(user-agent)").map(decode).unwrap_or(Cow::Borrowed("")),
        // IIS time-taken is milliseconds; CloudFront time-taken is seconds.
        response_ms: taken.map(|t| if cloudfront { t * 1000.0 } else { t }),
        weight: 1,
        edge_verified: false,
    })
}

// ---------------------------------------------------------------- AWS ALB

/// Splits on spaces; `"…"` tokens are returned without quotes. Pushes byte ranges.
fn split_quoted(line: &str, out: &mut Vec<(usize, usize)>) {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        if b[i] == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < b.len() && b[j] != b'"' {
                if b[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            out.push((start, j.min(b.len())));
            i = j + 1;
        } else {
            let start = i;
            while i < b.len() && b[i] != b' ' {
                i += 1;
            }
            out.push((start, i));
        }
    }
}

fn parse_alb<'a>(line: &'a str, tokens: &[(usize, usize)]) -> Result<Rec<'a>, &'static str> {
    if tokens.len() < 14 {
        return Err("too few ALB fields");
    }
    let t = |i: usize| -> &'a str { tokens.get(i).map(|&(s, e)| &line[s..e]).unwrap_or("") };
    let kind = t(0);
    if !matches!(kind, "http" | "https" | "h2" | "ws" | "wss" | "grpcs") {
        return Err("not an ALB entry");
    }
    let ts = parse_any_time(t(1)).ok_or("bad ALB time")?;
    let client = t(3);
    let ip = if let Some(rest) = client.strip_prefix('[') { rest.split(']').next().unwrap_or("") } else { client.rsplit_once(':').map(|(h, _)| h).unwrap_or(client) };
    let status = parse_status(t(8)).or_else(|| parse_status(t(9))).ok_or("bad ALB status")?;
    let times: Vec<f64> = (5..=7).filter_map(|i| t(i).parse::<f64>().ok()).collect();
    let response_ms = (times.len() == 3 && times.iter().all(|x| *x >= 0.0)).then(|| times.iter().sum::<f64>() * 1000.0);
    let (method, target) = split_request(t(12));
    let domain = t(18);
    Ok(Rec {
        ip: Cow::Borrowed(ip),
        ts,
        method: Cow::Borrowed(method),
        host: non_dash(domain).map(Cow::Borrowed),
        target: Cow::Borrowed(target),
        status,
        bytes: t(11).parse().ok(),
        referer: None,
        ua: Cow::Borrowed(if t(13) == "-" { "" } else { t(13) }),
        response_ms,
        weight: 1,
        edge_verified: false,
    })
}

// ---------------------------------------------------------------- targets and URLs

/// Splits a request target into `(host, path, query)`; absolute URLs (proxies, ALB) keep their host.
pub fn split_target(target: &str) -> (Option<&str>, &str, Option<&str>) {
    let (host, rest) = if let Some(r) = target.strip_prefix("http://").or_else(|| target.strip_prefix("https://")) {
        match r.find('/') {
            Some(i) => (Some(strip_port(&r[..i])), &r[i..]),
            None => (Some(strip_port(r.split('?').next().unwrap_or(r))), r.find('?').map(|i| &r[i..]).unwrap_or("/")),
        }
    } else {
        (None, target)
    };
    let rest = rest.split('#').next().unwrap_or(rest);
    match rest.split_once('?') {
        Some((p, q)) => (host, if p.is_empty() { "/" } else { p }, (!q.is_empty()).then_some(q)),
        None => (host, if rest.is_empty() { "/" } else { rest }, None),
    }
}

/// Percent-decodes (lossy UTF-8). `+` is left as is (paths).
pub fn percent_decode(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Host of a referer URL (`https://www.google.com/search?q=…` → `www.google.com`).
pub fn referer_host(referer: &str) -> Option<String> {
    let r = referer.strip_prefix("https://").or_else(|| referer.strip_prefix("http://"))?;
    let host = r.split(['/', '?', '#']).next()?;
    let host = strip_port(host).to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

// ---------------------------------------------------------------- time

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn day_string(ts: i64) -> String {
    let (y, m, d) = civil_from_days(ts.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn iso_string(ts: i64) -> String {
    let (y, m, d) = civil_from_days(ts.div_euclid(86_400));
    let s = ts.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, (s / 60) % 60, s % 60)
}

/// Unix days for `YYYY-MM-DD`.
pub fn parse_day(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    Some(days_from_civil(num(&b[0..4])? as i64, num(&b[5..7])?, num(&b[8..10])?))
}

fn num(b: &[u8]) -> Option<u32> {
    if b.is_empty() || !b.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(b.iter().fold(0u32, |acc, c| acc * 10 + (c - b'0') as u32))
}

fn month(b: &[u8]) -> Option<u32> {
    Some(match b {
        b"Jan" => 1,
        b"Feb" => 2,
        b"Mar" => 3,
        b"Apr" => 4,
        b"May" => 5,
        b"Jun" => 6,
        b"Jul" => 7,
        b"Aug" => 8,
        b"Sep" => 9,
        b"Oct" => 10,
        b"Nov" => 11,
        b"Dec" => 12,
        _ => return None,
    })
}

/// `10/Oct/2000:13:55:36 -0700` → unix seconds (UTC).
pub fn parse_clf_time(s: &str) -> Option<i64> {
    let b = s.trim().as_bytes();
    if b.len() < 20 || b[2] != b'/' || b[6] != b'/' || b[11] != b':' {
        return None;
    }
    let d = num(&b[0..2])?;
    let mo = month(&b[3..6])?;
    let y = num(&b[7..11])? as i64;
    let h = num(&b[12..14])? as i64;
    let mi = num(&b[15..17])? as i64;
    let sec = num(&b[18..20])? as i64;
    let mut offset = 0i64;
    if b.len() >= 26 && (b[21] == b'+' || b[21] == b'-') {
        let oh = num(&b[22..24])? as i64;
        let om = num(&b[24..26])? as i64;
        offset = (oh * 3600 + om * 60) * if b[21] == b'-' { -1 } else { 1 };
    }
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + sec - offset)
}

/// RFC 3339 / ISO 8601 (`2024-05-01T12:00:00.123+03:00`, `…Z`, `2024-05-01 12:00:00`), CLF, or unix digits.
pub fn parse_any_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() >= 19 && b[4] == b'-' && b[7] == b'-' && (b[10] == b'T' || b[10] == b' ') && b[13] == b':' {
        let days = days_from_civil(num(&b[0..4])? as i64, num(&b[5..7])?, num(&b[8..10])?);
        let secs = num(&b[11..13])? as i64 * 3600 + num(&b[14..16])? as i64 * 60 + num(&b[17..19])? as i64;
        let mut i = 19;
        if i < b.len() && (b[i] == b'.' || b[i] == b',') {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        let mut offset = 0i64;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            let rest: Vec<u8> = b[i + 1..].iter().copied().filter(|c| *c != b':').collect();
            if rest.len() >= 4 {
                offset = (num(&rest[0..2])? as i64 * 3600 + num(&rest[2..4])? as i64 * 60) * if b[i] == b'-' { -1 } else { 1 };
            }
        }
        return Some(days * 86_400 + secs - offset);
    }
    if let Some(t) = parse_clf_time(s) {
        return Some(t);
    }
    s.parse::<f64>().ok().map(unix_any)
}

/// Unix timestamp in s, ms, µs or ns (by magnitude) → seconds.
pub fn unix_any(n: f64) -> i64 {
    let a = n.abs();
    (if a >= 1e17 {
        n / 1e9
    } else if a >= 1e14 {
        n / 1e6
    } else if a >= 1e11 {
        n / 1e3
    } else {
        n
    }) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMBINED: &str = r#"66.249.66.1 - - [10/Oct/2024:13:55:36 +0300] "GET /hizmetlerimiz/?utm_source=x HTTP/1.1" 200 5120 "https://www.google.com/" "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)""#;

    fn rec(format: Format, line: &str) -> Rec<'_> {
        match Parser::new(format).parse(line) {
            Line::Rec(r) => r,
            other => panic!("{format:?} did not parse: {other:?} — {line}"),
        }
    }

    #[test]
    fn time_helpers_roundtrip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
        assert_eq!(parse_clf_time("10/Oct/2000:13:55:36 -0700"), Some(971_211_336));
        assert_eq!(parse_any_time("2000-10-10T20:55:36Z"), Some(971_211_336));
        assert_eq!(parse_any_time("2000-10-10T23:55:36.250+03:00"), Some(971_211_336));
        assert_eq!(parse_any_time("971211336"), Some(971_211_336));
        assert_eq!(unix_any(971_211_336_000_000_000.0), 971_211_336);
        assert_eq!(iso_string(971_211_336), "2000-10-10T20:55:36Z");
        assert_eq!(day_string(971_211_336), "2000-10-10");
        assert_eq!(parse_day("2000-10-10"), Some(971_211_336 / 86_400));
    }

    #[test]
    fn combined_line() {
        let r = rec(Format::Combined, COMBINED);
        assert_eq!(r.ip, "66.249.66.1");
        assert_eq!(r.method, "GET");
        assert_eq!(r.target, "/hizmetlerimiz/?utm_source=x");
        assert_eq!(r.status, 200);
        assert_eq!(r.bytes, Some(5120));
        assert_eq!(r.referer.as_deref(), Some("https://www.google.com/"));
        assert!(r.ua.contains("Googlebot/2.1"));
        assert_eq!(iso_string(r.ts), "2024-10-10T10:55:36Z");
    }

    #[test]
    fn combined_with_extras_and_vhost() {
        let line = r#"seobify.com:443 10.0.0.5 - - [10/Oct/2024:13:55:36 +0000] "GET /a HTTP/2.0" 404 0 "-" "curl/8.0" "203.0.113.9" rt=0.250 urt=0.200"#;
        let r = rec(Format::Combined, line);
        assert_eq!(r.host.as_deref(), Some("seobify.com"));
        assert_eq!(r.ip, "203.0.113.9", "private proxy hop replaced by X-Forwarded-For");
        assert_eq!(r.referer, None);
        assert_eq!(r.response_ms, Some(250.0));
        let line2 = r#"1.2.3.4 - - [10/Oct/2024:13:55:36 +0000] "GET /b HTTP/1.1" 200 12 "-" "Mozilla/5.0" 0.042 0.040 www.seobify.com"#;
        let r2 = rec(Format::Combined, line2);
        assert_eq!(r2.response_ms, Some(42.0));
        assert_eq!(r2.host.as_deref(), Some("www.seobify.com"));
        let line3 = r#"1.2.3.4 - - [10/Oct/2024:13:55:36 +0000] "GET /c HTTP/1.1" 200 12 "-" "Mozilla/5.0" 15300"#;
        assert_eq!(rec(Format::Combined, line3).response_ms, Some(15.3), "Apache %D is microseconds");
    }

    #[test]
    fn common_line_and_detection() {
        let line = r#"192.0.2.10 - frank [10/Oct/2024:13:55:36 -0700] "GET /apache_pb.gif HTTP/1.0" 200 2326"#;
        let r = rec(Format::Common, line);
        assert_eq!(r.ua, "");
        assert_eq!(r.status, 200);
        assert_eq!(detect(&[line, line]), Some(Format::Common));
        assert_eq!(detect(&[COMBINED, COMBINED]), Some(Format::Combined));
    }

    #[test]
    fn bad_lines_are_reported() {
        let mut p = Parser::new(Format::Combined);
        assert!(matches!(p.parse("garbage without fields"), Line::Bad(_)));
        assert!(matches!(p.parse(r#"1.2.3.4 - - [99/Foo/2024:13:55:36 +0000] "GET / HTTP/1.1" 200 1 "-" "x""#), Line::Bad("bad timestamp")));
        assert!(matches!(p.parse(r#"1.2.3.4 - - [10/Oct/2024:13:55:36 +0000] "GET / HTTP/1.1" abc 1"#), Line::Bad("bad status")));
        assert_eq!(p.parse("   "), Line::Skip);
    }

    #[test]
    fn nginx_and_caddy_json() {
        let nginx = r#"{"time_local":"10/Oct/2024:13:55:36 +0300","remote_addr":"66.249.66.1","request":"GET /a?b=1 HTTP/1.1","status":"301","body_bytes_sent":"0","http_referer":"-","http_user_agent":"Googlebot/2.1","request_time":"0.012"}"#;
        let r = rec(Format::Json, nginx);
        assert_eq!((r.ip.as_ref(), r.target.as_ref(), r.status), ("66.249.66.1", "/a?b=1", 301));
        assert_eq!(r.referer, None);
        assert_eq!(r.response_ms, Some(12.0));
        let split = r#"{"ts":"2024-10-10T10:55:36Z","client_ip":"1.2.3.4","method":"HEAD","uri":"/x","args":"q=1","status":200,"size":10,"user_agent":"bingbot/2.0"}"#;
        let r = rec(Format::Json, split);
        assert_eq!((r.method.as_ref(), r.target.as_ref()), ("HEAD", "/x?q=1"));
        let caddy = r#"{"level":"info","ts":1728557736.52,"logger":"http.log.access","request":{"remote_ip":"1.2.3.4","client_ip":"5.6.7.8","method":"GET","host":"seobify.com","uri":"/blog/","headers":{"User-Agent":["GPTBot/1.1"]}},"duration":0.0021,"size":1500,"status":200}"#;
        let r = rec(Format::Json, caddy);
        assert_eq!((r.ip.as_ref(), r.host.as_deref(), r.ua.as_ref()), ("5.6.7.8", Some("seobify.com"), "GPTBot/1.1"));
        assert_eq!(iso_string(r.ts), "2024-10-10T10:55:36Z");
        assert_eq!(detect(&[caddy]), Some(Format::Json));
        assert!(matches!(Parser::new(Format::Json).parse("{not json"), Line::Bad("invalid JSON")));
    }

    #[test]
    fn cloudflare_logpush() {
        let ns = r#"{"ClientIP":"66.249.66.1","ClientRequestHost":"seobify.com","ClientRequestMethod":"GET","ClientRequestURI":"/a?x=1","ClientRequestUserAgent":"Googlebot/2.1","EdgeResponseStatus":200,"EdgeResponseBytes":999,"EdgeStartTimestamp":1728557736000000000,"ClientRequestReferer":""}"#;
        let r = rec(Format::Cloudflare, ns);
        assert_eq!((r.host.as_deref(), r.target.as_ref(), r.bytes), (Some("seobify.com"), "/a?x=1", Some(999)));
        assert_eq!(iso_string(r.ts), "2024-10-10T10:55:36Z");
        let rfc = ns.replace("1728557736000000000", "\"2024-10-10T10:55:36Z\"");
        assert_eq!(rec(Format::Cloudflare, &rfc).ts, r.ts);
        assert_eq!(detect(&[ns]), Some(Format::Cloudflare));
    }

    #[test]
    fn iis_w3c() {
        let header = "#Fields: date time s-ip cs-method cs-uri-stem cs-uri-query s-port cs-username c-ip cs(User-Agent) cs(Referer) sc-status sc-substatus sc-win32-status time-taken";
        let line = "2024-10-10 10:55:36 10.0.0.1 GET /urun/liste page=2 443 - 157.55.39.1 Mozilla/5.0+(compatible;+bingbot/2.0;++http://www.bing.com/bingbot.htm) - 200 0 0 31";
        let mut p = Parser::new(Format::Iis);
        assert_eq!(p.parse("#Software: Microsoft Internet Information Services 10.0"), Line::Skip);
        assert_eq!(p.parse(header), Line::Skip);
        let Line::Rec(r) = p.parse(line) else { panic!("iis") };
        assert_eq!((r.ip.as_ref(), r.target.as_ref(), r.status, r.response_ms), ("157.55.39.1", "/urun/liste?page=2", 200, Some(31.0)));
        assert!(r.ua.contains("bingbot/2.0"));
        assert_eq!(detect(&[header, line]), Some(Format::Iis));
    }

    #[test]
    fn cloudfront_standard() {
        let line = "2024-10-10\t10:55:36\tIST50-C1\t5120\t66.249.66.1\tGET\td111.cloudfront.net\t/blog/yazi/\t200\t-\tMozilla/5.0%20(compatible;%20Googlebot/2.1;%20+http://www.google.com/bot.html)\tutm_source=x\t-\tHit\tabc==\tseobify.com\thttps\t300\t0.002\t-\tTLSv1.3\tTLS_AES_128_GCM_SHA256\tHit\tHTTP/2.0\t-\t-\t51000\t0.002\tHit\ttext/html\t5000\t-\t-";
        let r = rec(Format::Cloudfront, line);
        assert_eq!((r.host.as_deref(), r.target.as_ref()), (Some("seobify.com"), "/blog/yazi/?utm_source=x"));
        assert_eq!(r.ua, "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)");
        assert_eq!(r.response_ms, Some(2.0));
        assert_eq!(detect(&["#Version: 1.0", "#Fields: date time x-edge-location sc-bytes c-ip", line]), Some(Format::Cloudfront));
        assert_eq!(detect(&[line]), Some(Format::Cloudfront));
    }

    #[test]
    fn aws_alb() {
        let line = r#"https 2024-10-10T10:55:36.186641Z app/my-lb/50dc6c495c0c9188 66.249.66.1:2817 10.0.0.1:80 0.001 0.048 0.001 200 200 34 366 "GET https://seobify.com:443/iletisim/?a=1 HTTP/1.1" "Mozilla/5.0 (compatible; Googlebot/2.1)" ECDHE-RSA-AES128-GCM-SHA256 TLSv1.2 arn:aws:elasticloadbalancing:eu-central-1:1:targetgroup/t/1 "Root=1-58337262" "seobify.com" "arn:aws:acm:eu-central-1:1:certificate/1" 0 2024-10-10T10:55:36.138000Z "forward" "-" "-" "10.0.0.1:80" "200" "-" "-""#;
        let r = rec(Format::Alb, line);
        assert_eq!((r.ip.as_ref(), r.status, r.host.as_deref()), ("66.249.66.1", 200, Some("seobify.com")));
        assert_eq!(split_target(&r.target), (Some("seobify.com"), "/iletisim/", Some("a=1")));
        assert_eq!(r.response_ms.map(|v| v.round()), Some(50.0));
        assert_eq!(detect(&[line]), Some(Format::Alb));
    }

    #[test]
    fn targets_and_referers() {
        assert_eq!(split_target("/a/b?x=1#f"), (None, "/a/b", Some("x=1")));
        assert_eq!(split_target("/?"), (None, "/", None));
        assert_eq!(split_target("http://seobify.com"), (Some("seobify.com"), "/", None));
        assert_eq!(percent_decode("/hizmet/%C5%9Fey"), "/hizmet/şey");
        assert_eq!(percent_decode("/bad%2"), "/bad%2");
        assert_eq!(referer_host("https://www.Google.com:443/search?q=1").as_deref(), Some("www.google.com"));
        assert_eq!(referer_host("-"), None);
        assert_eq!(Format::from_name("auto"), Ok(None));
        assert_eq!(Format::from_name("IIS"), Ok(Some(Format::Iis)));
        assert!(Format::from_name("xml").is_err());
    }
}
