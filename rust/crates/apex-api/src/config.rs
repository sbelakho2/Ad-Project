//! Configuration helpers shared by the API binary: environment reads and the
//! `publisher=<hex key>` allow-list. They live in the library so they are
//! compiled once and unit tested.

use std::collections::HashMap;

use apex_proof::VerifyingKey;

use crate::is_valid_publisher;

/// A trimmed, non-empty environment value.
pub fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Parse `publisher=<64 hex>` entries into the key allow-list.
pub fn parse_keys(raw: &str) -> Result<HashMap<String, VerifyingKey>, String> {
    let mut keys = HashMap::new();
    for entry in raw.split(',').filter(|entry| !entry.trim().is_empty()) {
        let (publisher, hex_key) = entry.split_once('=').ok_or_else(|| {
            format!("APEX_LEDGER_KEYS entry {entry:?} is not publisher=<hex key>")
        })?;
        let publisher = publisher.trim();
        if !is_valid_publisher(publisher) {
            return Err(format!(
                "APEX_LEDGER_KEYS publisher {publisher:?} must match ^[A-Za-z0-9_-]{{1,64}}$"
            ));
        }
        if keys.contains_key(publisher) {
            return Err(format!(
                "APEX_LEDGER_KEYS lists publisher {publisher:?} twice"
            ));
        }
        keys.insert(publisher.to_string(), parse_key_hex(hex_key)?);
    }
    Ok(keys)
}

/// Decode a 64-hex ed25519 verifying key.
pub fn parse_key_hex(raw: &str) -> Result<VerifyingKey, String> {
    let raw = raw.trim();
    if raw.len() != 64 {
        return Err(format!(
            "ed25519 public key must be 64 hex characters, got {}",
            raw.len()
        ));
    }
    let mut bytes = [0u8; 32];
    let raw = raw.as_bytes();
    for (index, byte) in bytes.iter_mut().enumerate() {
        let high = hex_nibble(raw[2 * index])
            .ok_or_else(|| format!("public key has a non-hex character at byte {index}"))?;
        let low = hex_nibble(raw[2 * index + 1])
            .ok_or_else(|| format!("public key has a non-hex character at byte {index}"))?;
        *byte = (high << 4) | low;
    }
    VerifyingKey::from_bytes(&bytes).map_err(|error| format!("invalid ed25519 public key: {error}"))
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn key_hex_roundtrips_a_seeded_keypair() {
        let keypair = apex_proof::Keypair::from_bytes(&[7u8; 32]);
        let mut hex = String::new();
        for byte in keypair.verifying_key().to_bytes() {
            hex.push_str(&format!("{byte:02x}"));
        }
        let parsed = parse_key_hex(&hex).expect("parse");
        assert_eq!(parsed, keypair.verifying_key());
        assert_eq!(parse_key_hex(&hex.to_uppercase()).expect("upper"), parsed);
    }

    #[test]
    fn key_and_entry_errors_are_named() {
        assert!(parse_key_hex("ab").expect_err("short").contains("64 hex"));
        assert!(parse_key_hex(&"zz".repeat(32))
            .expect_err("non-hex")
            .contains("non-hex"));
        assert!(parse_keys("no-equals")
            .expect_err("shape")
            .contains("publisher=<hex"));
        assert!(parse_keys("../evil=00").expect_err("charset").contains("^"));
        let good = format!("pub1={}", "00".repeat(32));
        assert!(parse_keys(&good).is_ok());
        assert!(parse_keys(&format!("{good},{good}"))
            .expect_err("duplicate")
            .contains("twice"));
        // Empty entries are skipped.
        assert!(parse_keys(&format!(" ,{good}, ")).is_ok());
    }

    #[test]
    fn env_value_trims_and_refuses_blanks() {
        std::env::set_var("APEX_CONFIG_TEST", "value");
        assert_eq!(env_value("APEX_CONFIG_TEST").as_deref(), Some("value"));
        std::env::set_var("APEX_CONFIG_TEST", "   ");
        assert!(env_value("APEX_CONFIG_TEST").is_none());
        std::env::remove_var("APEX_CONFIG_TEST");
        assert!(env_value("APEX_CONFIG_TEST").is_none());
    }
}
