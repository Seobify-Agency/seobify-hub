//! Secrets (API keys, OAuth refresh tokens, service-account keys) live in the OS credential
//! store (Windows Credential Manager), never in SQLite, settings or logs.
//!
//! Windows limits one credential blob to 2560 bytes (1280 UTF-16 chars), so long values are
//! stored transparently in chunks: the main entry holds `chunked:<n>` and the parts live in
//! `<name>#1` … `<name>#n`.

const CHUNK_CHARS: usize = 1000;
const CHUNK_PREFIX: &str = "chunked:";

fn entry(name: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(&format!("Seobify/{}", crate::TOOL_KEY), name).map_err(|e| format!("keyring: {e}"))
}

fn raw_get(name: &str) -> Result<Option<String>, String> {
    match entry(name)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("keyring get {name}: {e}")),
    }
}

fn raw_delete(name: &str) -> Result<(), String> {
    match entry(name)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("keyring delete {name}: {e}")),
    }
}

/// Splits a value into credential-sized chunks on char boundaries.
fn split_chunks(value: &str) -> Vec<String> {
    let chars: Vec<char> = value.chars().collect();
    chars.chunks(CHUNK_CHARS).map(|c| c.iter().collect()).collect()
}

fn chunk_count(header: &str) -> Option<usize> {
    header.strip_prefix(CHUNK_PREFIX).and_then(|n| n.parse().ok())
}

pub fn set(name: &str, value: &str) -> Result<(), String> {
    delete(name)?;
    if value.chars().count() <= CHUNK_CHARS {
        return entry(name)?.set_password(value).map_err(|e| format!("keyring set {name}: {e}"));
    }
    let chunks = split_chunks(value);
    for (i, chunk) in chunks.iter().enumerate() {
        entry(&format!("{name}#{}", i + 1))?
            .set_password(chunk)
            .map_err(|e| format!("keyring set {name}#{}: {e}", i + 1))?;
    }
    entry(name)?
        .set_password(&format!("{CHUNK_PREFIX}{}", chunks.len()))
        .map_err(|e| format!("keyring set {name}: {e}"))
}

pub fn get(name: &str) -> Result<Option<String>, String> {
    let Some(head) = raw_get(name)? else { return Ok(None) };
    let Some(n) = chunk_count(&head) else { return Ok(Some(head)) };
    let mut out = String::new();
    for i in 1..=n {
        match raw_get(&format!("{name}#{i}"))? {
            Some(part) => out.push_str(&part),
            None => return Err(format!("keyring: secret {name} is missing part {i} of {n}")),
        }
    }
    Ok(Some(out))
}

pub fn delete(name: &str) -> Result<(), String> {
    if let Some(head) = raw_get(name)? {
        if let Some(n) = chunk_count(&head) {
            for i in 1..=n {
                raw_delete(&format!("{name}#{i}"))?;
            }
        }
    }
    raw_delete(name)
}

/// `true` when a secret is stored (for UI badges; never return the value to the UI).
pub fn exists(name: &str) -> bool {
    matches!(raw_get(name), Ok(Some(_)))
}

/// Masks a secret for display: `sk-a…9f2c`.
pub fn mask(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 8 {
        return "••••".into();
    }
    format!("{}…{}", chars[..4].iter().collect::<String>(), chars[chars.len() - 4..].iter().collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks() {
        assert_eq!(mask("short"), "••••");
        assert_eq!(mask("sk-abcdefgh1234"), "sk-a…1234");
    }

    #[test]
    fn chunks_long_values_on_char_boundaries() {
        let value: String = "ğ".repeat(2500) + "end";
        let parts = split_chunks(&value);
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.chars().count() <= CHUNK_CHARS));
        assert_eq!(parts.concat(), value);
        assert_eq!(chunk_count("chunked:3"), Some(3));
        assert_eq!(chunk_count("sk-plain"), None);
    }

    #[test]
    #[ignore = "writes to the real OS credential store"]
    fn live_roundtrip_long_secret() {
        let name = "hub-selftest-long";
        let value: String = "x".repeat(2999) + "ş";
        set(name, &value).unwrap();
        assert!(exists(name));
        assert_eq!(get(name).unwrap().as_deref(), Some(value.as_str()));
        set(name, "short").unwrap();
        assert_eq!(get(name).unwrap().as_deref(), Some("short"));
        assert!(raw_get(&format!("{name}#1")).unwrap().is_none(), "old chunks removed");
        delete(name).unwrap();
        assert!(!exists(name));
    }
}
