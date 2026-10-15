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
//! - `APEX_CREDENTIAL_ROOT`, `APEX_VAULT_KEY_HEX` and `APEX_LIVE_AUDIT_TOKEN`
//!   — when all three are set, the live read-only audit is enabled over
//!   vault-encrypted `<root>/<publisher>.blob` account files, and
//!   `POST /v1/audit/{platform}` requires `Authorization: Bearer <token>` and
//!   runs on the publisher's own platform keys; without them that route
//!   answers `503 live_audit_not_configured`;
//! - `APEX_ADYEN_HMAC_KEY` (and optionally `APEX_ADYEN_API_KEY`,
//!   `APEX_ADYEN_MERCHANT_ACCOUNT`, `APEX_ADYEN_CHECKOUT_BASE` to build the
//!   client) — the webhook verifies each `notificationItems` item's
//!   `additionalData.hmacSignature` against this key; without it the endpoint
//!   answers `503 adyen_webhook_not_configured`;
//! - `APEX_LEDGER_TOKENS_FILE` — a JSON object of publisher to ledger token;
//!   without it no ledger is readable over HTTP (the routes answer
//!   `503 ledger_auth_not_configured`);
//! - `APEX_COLLECTION_FILE` — the append-only collection log the Adyen
//!   webhook records accepted events into (replay-protected). Without it the
//!   webhook answers `503 collection_store_not_configured`;
//! - `APEX_LEADS_FILE` — the append-only lead log for `POST /v1/leads`;
//!   `APEX_MAIL_WEBHOOK` optionally forwards each lead to ApexMail.
//!
//! The binary holds no platform credentials by construction: it serves the
//! read-only deployment mode. An embedding service can build the same router
//! from the library and inject a write-capable `CycleRunner` and an Adyen
//! client.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use apex_api::config::{env_value, parse_keys};
use apex_api::{
    router, AppState, FileLedgerSource, JsonFileAuditSource, LiveAuditor, VaultedFileAccountStore,
};
use apex_connectors::transport::{ReqwestTransport, RetryTransport, TokioSleeper};

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
    match (
        env_value("APEX_ADYEN_API_KEY"),
        env_value("APEX_ADYEN_MERCHANT_ACCOUNT"),
        env_value("APEX_ADYEN_CHECKOUT_BASE"),
    ) {
        (Some(api_key), Some(merchant_account), Some(checkout_base)) => {
            let adyen = apex_billing::AdyenClient::new(
                api_key,
                merchant_account,
                env_value("APEX_ADYEN_HMAC_KEY")
                    .unwrap_or_default()
                    .into_bytes(),
                checkout_base,
            )
            .map_err(|error| format!("adyen credentials are invalid: {error}"))?;
            state = state.with_adyen(Arc::new(adyen));
        }
        (None, None, None) => {}
        _ => {
            return Err(
                "APEX_ADYEN_API_KEY, APEX_ADYEN_MERCHANT_ACCOUNT and APEX_ADYEN_CHECKOUT_BASE must be set together"
                    .into(),
            );
        }
    }
    if let Some(hmac_key) = env_value("APEX_ADYEN_HMAC_KEY") {
        state = state.with_adyen_hmac_key(hmac_key.into_bytes());
    }

    // Collection state: accepted Adyen events are recorded (and
    // replay-protected) at the path named by `APEX_COLLECTION_FILE`.
    if let Some(path) = env_value("APEX_COLLECTION_FILE") {
        let store = apex_billing::FileCollectionStore::open(std::path::Path::new(&path))
            .map_err(|error| format!("APEX_COLLECTION_FILE could not be opened: {error}"))?;
        state = state.with_collection(Arc::new(store));
    }

    // Lead capture: append-only JSONL at the path named by `APEX_LEADS_FILE`,
    // forwarded to `APEX_MAIL_WEBHOOK` (the ApexMail hook) when set.
    if let Some(path) = env_value("APEX_LEADS_FILE") {
        let transport = Arc::new(
            ReqwestTransport::new()
                .map_err(|error| format!("lead webhook transport could not be built: {error}"))?,
        );
        let store: Arc<dyn apex_api::LeadStore> = match env_value("APEX_MAIL_WEBHOOK") {
            Some(url) => {
                Arc::new(apex_api::FileLeadStore::new(path).with_mail_webhook(url, transport))
            }
            None => Arc::new(apex_api::FileLeadStore::new(path)),
        };
        state = state.with_leads(store);
    }

    // Per-publisher ledger tokens: a JSON object of publisher -> token, read
    // from the file named by `APEX_LEDGER_TOKENS_FILE`. Without it no ledger
    // is readable over HTTP (the routes answer 503).
    if let Some(path) = env_value("APEX_LEDGER_TOKENS_FILE") {
        let raw = std::fs::read_to_string(&path)
            .map_err(|error| format!("APEX_LEDGER_TOKENS_FILE could not be read: {error}"))?;
        let tokens: HashMap<String, String> = serde_json::from_str(&raw)
            .map_err(|error| format!("APEX_LEDGER_TOKENS_FILE is not a JSON object: {error}"))?;
        let tokens = tokens
            .into_iter()
            .map(|(publisher, token)| (publisher, token.into_bytes()))
            .collect();
        state = state.with_ledger_tokens(tokens);
    }

    match (
        env_value("APEX_CREDENTIAL_ROOT"),
        env_value("APEX_VAULT_KEY_HEX"),
        env_value("APEX_LIVE_AUDIT_TOKEN"),
    ) {
        (Some(root), Some(key_hex), Some(token)) => {
            let store =
                VaultedFileAccountStore::with_vault_key_hex(root, &key_hex).map_err(|error| {
                    format!("APEX_VAULT_KEY_HEX is not a valid vault key: {error:?}")
                })?;
            let transport = ReqwestTransport::new()
                .map_err(|error| format!("live transport could not be built: {error}"))?;
            // The retry wrapper is also the status classifier the clients and
            // the auditor expect; without it a 401 would be read as data.
            let transport = Arc::new(RetryTransport::new(
                Arc::new(transport),
                2,
                Arc::new(TokioSleeper),
            ));
            let auditor = LiveAuditor::new(Arc::new(store), transport, Arc::clone(&state.clock))
                .with_token(token);
            state = state.with_live_auditor(Arc::new(auditor));
        }
        (None, None, None) => {}
        _ => {
            return Err(
                "APEX_CREDENTIAL_ROOT, APEX_VAULT_KEY_HEX and APEX_LIVE_AUDIT_TOKEN must be set together".into(),
            );
        }
    }

    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!(
        "apex-api (read-only mode) listening on http://{bind} (adyen hmac configured: {})",
        state.adyen_hmac_key.is_some() || state.adyen.is_some()
    );
    axum::serve(listener, router(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_documented_values() {
        assert_eq!(DEFAULT_BIND, "127.0.0.1:8787");
        assert_eq!(DEFAULT_SNAPSHOT_ROOT, ".apex/snapshots");
        assert_eq!(DEFAULT_LEDGER_ROOT, ".apex/ledgers");
        assert!(
            env_value("APEX_ADYEN_HMAC_KEY").is_none()
                || !env_value("APEX_ADYEN_HMAC_KEY").unwrap().is_empty()
        );
    }
}
