//! IPv4/IPv6 CIDR matching and GDPR helpers (masking, salted hashing).

use sha2::{Digest, Sha256};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    v6: bool,
    net: u128,
    mask: u128,
}

fn bits(ip: &IpAddr) -> (bool, u128) {
    match ip {
        IpAddr::V4(v4) => (false, u32::from(*v4) as u128),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => (false, u32::from(v4) as u128),
            None => (true, u128::from(*v6)),
        },
    }
}

impl Cidr {
    /// `66.249.64.0/27`, `2001:4860:4801:10::/64` or a bare address (host route).
    pub fn parse(s: &str) -> Option<Cidr> {
        let s = s.trim();
        let (addr, prefix) = match s.split_once('/') {
            Some((a, p)) => (a, Some(p.parse::<u8>().ok()?)),
            None => (s, None),
        };
        let ip: IpAddr = addr.parse().ok()?;
        let (v6, value) = bits(&ip);
        let width = if v6 { 128 } else { 32 };
        let prefix = prefix.unwrap_or(width);
        if prefix > width {
            return None;
        }
        let mask = if prefix == 0 {
            0
        } else {
            let all: u128 = if v6 { u128::MAX } else { u32::MAX as u128 };
            (all << (width - prefix)) & all
        };
        Some(Cidr { v6, net: value & mask, mask })
    }

    pub fn contains(&self, ip: &IpAddr) -> bool {
        let (v6, value) = bits(ip);
        v6 == self.v6 && value & self.mask == self.net
    }
}

/// Masks an address for storage: IPv4 last octet → 0, IPv6 last 80 bits → 0 (keeps /48).
pub fn mask_ip(ip: &str) -> String {
    match ip.trim().parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            let o = v4.octets();
            Ipv4Addr::new(o[0], o[1], o[2], 0).to_string()
        }
        Ok(IpAddr::V6(v6)) => Ipv6Addr::from(u128::from(v6) & (u128::MAX << 80)).to_string(),
        Err(_) => "—".into(),
    }
}

/// Salted SHA-256 pseudonym of an address (`h:` + 24 hex chars).
pub fn hash_ip(ip: &str, salt: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update([0u8]);
    h.update(ip.trim().as_bytes());
    let hex: String = h.finalize().iter().take(12).map(|b| format!("{b:02x}")).collect();
    format!("h:{hex}")
}

/// Replaces anything that parses as an IP address inside free text (error samples).
pub fn redact_ips(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        if !token.is_empty() {
            let bare = token.split(']').next().unwrap_or(token).trim_start_matches('[');
            let host = match bare.rsplit_once(':') {
                Some((h, p)) if !h.contains(':') && p.bytes().all(|b| b.is_ascii_digit()) => h,
                _ => bare,
            };
            if host.parse::<IpAddr>().is_ok() {
                out.push_str(&mask_ip(host));
            } else {
                out.push_str(token);
            }
            token.clear();
        }
    };
    for c in text.chars() {
        if c.is_ascii_hexdigit() || c == '.' || c == ':' || c == '[' || c == ']' {
            token.push(c);
        } else {
            flush(&mut token, &mut out);
            out.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn ipv4_cidr() {
        let c = Cidr::parse("66.249.64.0/27").unwrap();
        assert!(c.contains(&ip("66.249.64.1")));
        assert!(c.contains(&ip("66.249.64.31")));
        assert!(!c.contains(&ip("66.249.64.32")));
        assert!(!c.contains(&ip("2001:4860:4801:10::1")));
        assert!(Cidr::parse("10.0.0.0/8").unwrap().contains(&ip("10.200.1.1")));
        assert!(Cidr::parse("0.0.0.0/0").unwrap().contains(&ip("8.8.8.8")));
        assert!(Cidr::parse("107.20.236.150").unwrap().contains(&ip("107.20.236.150")));
        assert!(Cidr::parse("1.2.3.0/33").is_none());
        assert!(Cidr::parse("nope/24").is_none());
        assert!(c.contains(&ip("::ffff:66.249.64.5")), "IPv4-mapped IPv6 matches IPv4 ranges");
    }

    #[test]
    fn ipv6_cidr() {
        let c = Cidr::parse("2001:4860:4801:10::/64").unwrap();
        assert!(c.contains(&ip("2001:4860:4801:10::1")));
        assert!(c.contains(&ip("2001:4860:4801:10:ffff:ffff:ffff:ffff")));
        assert!(!c.contains(&ip("2001:4860:4801:11::1")));
        assert!(!c.contains(&ip("66.249.64.1")));
        assert!(Cidr::parse("2600:1f00::/24").unwrap().contains(&ip("2600:1fff::1")));
    }

    #[test]
    fn gdpr_helpers() {
        assert_eq!(mask_ip("203.0.113.77"), "203.0.113.0");
        assert_eq!(mask_ip("2001:db8:abcd:12:34:56:78:9a"), "2001:db8:abcd::");
        assert_eq!(mask_ip("garbage"), "—");
        let a = hash_ip("203.0.113.77", "salt-1");
        assert!(a.starts_with("h:") && a.len() == 26);
        assert_eq!(a, hash_ip("203.0.113.77", "salt-1"));
        assert_ne!(a, hash_ip("203.0.113.77", "salt-2"));
        assert!(!a.contains("203.0.113"));
        let red = redact_ips("bad line from 203.0.113.77 and [2001:db8::5]:443 via 10.0.0.1:8080 at 12:30");
        assert!(!red.contains("203.0.113.77") && red.contains("203.0.113.0"));
        assert!(!red.contains("2001:db8::5") && !red.contains("10.0.0.1"));
        assert!(red.contains("12:30"));
    }
}
