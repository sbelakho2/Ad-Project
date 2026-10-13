//! `apex-api` binary: serve the JSON API and the read-only console.
//!
//! Configuration (all optional; documented defaults apply):
//!
//! - `APEX_API_BIND` — listen address, default `127.0.0.1:8787`;
//! - `APEX_SNAPSHOT_ROOT` — snapshot directory, default `.apex/snapshots`
//!   (reads `<root>/<platform>.json`);
//! - `APEX_LEDGER_ROOT` — ledger directory, default `.apex/ledgers` (reads
//!   `<root>/<publisher>.jsonl`);
//! - `APEX_LEDGER_KEYS` — comma-separated `publisher=<64-hex-ed25519-pubkey>`
//!   allow-list of onboarded publishers;
//! - `APEX_WEBHOOK_SECRET` — Stripe webhook signing secret; without it the
//!   webhook endpoint answers `503 webhook_not_configured`;
//! - `APEX_STRIPE_TOLERANCE_SECS` — accepted signature age, default 300.
//!
//! The binary holds no platform credentials by construction: it serves the
//! read-only deployment mode. An embedding service can build the same router
//! from the library and inject a write-capable `CycleRunner` and a Stripe
//! client.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use apex_api::{is_valid_publisher, router, AppState, FileLedgerSource, JsonFileAuditSource};
use apex_proof::VerifyingKey;

const DEFAULT_BIND: &str = "127.0.0.1:8787";
const DEFAULT_SNAPSHOT_ROOT: &str = ".apex/snapshots";
const DEFAULT_LEDGER_ROOT: &str = ".apex/ledgers";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bind: SocketAddr = env_value("APEX_API_BIND")
        .unwrap_or_else(|| DEFAULT_BIND.to_string())
        .parse()
        .map_err(|error| format!("APEX_API_BIND is not a socket address: {error}"))?;
    let snapshot_root =
        env_value("APEX_SNAPSHOT_ROOT").unwrap_or_else(|| DEFAULT_SNAPSHOT_ROOT.to_string());
    let ledger_root =
        env_value("APEX_LEDGER_ROOT").unwrap_or_else(|| DEFAULT_LEDGER_ROOT.to_string());
    let keys = match env_value("APEX_LEDGER_KEYS") {
        Some(raw) => parse_keys(&raw)?,
        None => HashMap::new(),
    };

    let mut state = AppState::new(
        Arc::new(JsonFileAuditSource::new(snapshot_root)),
        Arc::new(FileLedgerSource::new(ledger_root, keys)),
    );
    if let Some(secret) = env_value("APEX_WEBHOOK_SECRET") {
        state = state.with_webhook_secret(secret.into_bytes());
    }
    if let Some(raw) = env_value("APEX_STRIPE_TOLERANCE_SECS") {
        let secs: u64 = raw
            .parse()
            .map_err(|error| format!("APEX_STRIPE_TOLERANCE_SECS is not a number: {error}"))?;
        state = state.with_stripe_tolerance_secs(secs);
    }

    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!(
        "apex-api (read-only mode) listening on http://{bind} (webhook secret configured: {})",
        state.webhook_secret.is_some()
    );
    axum::serve(listener, router(state)).await?;
    Ok(())
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Parse `publisher=<64 hex>` entries into the key allow-list.
fn parse_keys(raw: &str) -> Result<HashMap<String, VerifyingKey>, String> {
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

fn parse_key_hex(raw: &str) -> Result<VerifyingKey, String> {
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
    use apex_api::DEFAULT_STRIPE_TOLERANCE_SECS;

    use super::*;

    #[test]
    fn key_hex_roundtrips_a_seeded_keypair() {
        let keypair = apex_proof::Keypair::from_bytes(&[7u8; 32]);
        let hex = {
            let mut out = String::new();
            for byte in keypair.verifying_key().to_bytes() {
                out.push_str(&format!("{byte:02x}"));
            }
            out
        };
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
    }

    #[test]
    fn defaults_are_documented_values() {
        assert_eq!(DEFAULT_BIND, "127.0.0.1:8787");
        assert_eq!(DEFAULT_SNAPSHOT_ROOT, ".apex/snapshots");
        assert_eq!(DEFAULT_LEDGER_ROOT, ".apex/ledgers");
        assert_eq!(DEFAULT_STRIPE_TOLERANCE_SECS, 300);
    }
}
