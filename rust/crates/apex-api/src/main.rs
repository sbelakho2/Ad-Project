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
//! - `APEX_CREDENTIAL_ROOT` and `APEX_VAULT_KEY_HEX` — when both are set, the
//!   live read-only audit path is enabled over vault-encrypted
//!   `<root>/<publisher>.blob` account files: the console connection pages
//!   store and validate credentials against this vault, and (with
//!   `APEX_LIVE_AUDIT_TOKEN` also set) `POST /v1/audit/{platform}` requires
//!   `Authorization: Bearer <token>` and runs on the publisher's own platform
//!   keys. Without the token the JSON route stays disabled while connection
//!   validation still works; without the vault the console connection pages
//!   answer the not-configured panel;
//! - `APEX_ADYEN_HMAC_KEY` (and optionally `APEX_ADYEN_API_KEY`,
//!   `APEX_ADYEN_MERCHANT_ACCOUNT`, `APEX_ADYEN_CHECKOUT_BASE` to build the
//!   client) — the webhook verifies each `notificationItems` item's
//!   `additionalData.hmacSignature` against this key; without it the endpoint
//!   answers `503 adyen_webhook_not_configured`;
//! - `APEX_LEDGER_TOKENS_FILE` — a JSON object of publisher to ledger token;
//!   without it no ledger is readable over HTTP (the routes answer
//!   `503 ledger_auth_not_configured`);
//! - `APEX_COLLECTION_FILE` — the append-only collection log the Adyen
//!   webhook records accepted events into (replay-protected); the console
//!   billing pages read the same file for payment facts. Without it the
//!   webhook answers `503 collection_store_not_configured`;
//! - `APEX_LEADS_FILE` — the append-only lead log behind `POST /v1/leads` and
//!   the free-audit form; `APEX_MAIL_WEBHOOK` optionally forwards each lead to
//!   ApexMail, and `APEX_LEAD_ID_SALT` salts the stable, personal-data-free
//!   `req_…` confirmation ids. Without the env salt the file store generates
//!   32 random bytes once into `<APEX_LEADS_FILE>.salt` (mode 0600) and reuses
//!   them, so ids stay stable across restarts and per deployment;
//! - `APEX_CONSOLE_PUBLISHER_NAME` (default `Publisher workspace`) and
//!   `APEX_CONSOLE_CURRENCY` (default `USD`) — the console's workspace label
//!   and the currency every micros amount is rendered in;
//! - `APEX_CONSOLE_PUBLISHER` — the workspace publisher the phase-2 console
//!   surfaces (connections, proposals, experiments, billing, analytics) read;
//!   without it those pages answer the not-configured panel;
//! - `APEX_PROPOSALS_ROOT` — the directory of the runner's proposal files
//!   (`<id>.json`) and the console's decision records (`<id>.approval.json`);
//! - `APEX_ALERT_FILE` — the runner's alert log (`FileAlertSink` JSONL); a
//!   `failedRollback` alert renders the restore-failed incident state. The
//!   historical `APEX_ALERTS_FILE` is read as a fallback (with one logged
//!   line) when only that name is set;
//! - `APEX_POLICY_FILE` — the runner's recorded `AutopilotPolicy` JSON (or
//!   `{"policy": {...}}`), read-only behind `/console/settings/policy`;
//! - `APEX_CONSOLE_CONTACT`, `APEX_CONSOLE_TIMEZONE` (default `UTC`) and
//!   `APEX_RETENTION_MONTHS` — the account facts shown on `/console/settings`;
//! - `APEX_BILLING_VAT_RATE_BP`, `APEX_MONITORING_FEE_MICROS` and
//!   `APEX_MONITORING_SINCE` — the recorded VAT rate and monitoring plan the
//!   billing pages show (unset means "not recorded"/"Not selected", never a
//!   fabricated zero);
//! - `APEX_CLICKHOUSE_HTTP` and `APEX_CLICKHOUSE_DB` (plus
//!   `APEX_ANALYTICS_ENV`, default `production`) — the analytics reader and
//!   `apex_analytics::AnalyticsClient` sink; when unset the sink logs and
//!   `/console/analytics` renders the honest not-available state.
//!
//! Metrics: `GET /metrics` serves the hand-rolled counters plus the bounded
//! `apex-metrics` registry render (route-template request series), and when
//! `APEX_METRICS_TEXTFILE` names a path, a background task exports the
//! registry to that path atomically every 15 seconds (and once more after
//! the graceful shutdown signal below), for node_exporter's textfile
//! collector. The periodic write bounds both the export cost and the file's
//! staleness; per-request writes would couple latency to disk I/O.
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
    // Operational log lines (tracing warn/error) must be visible in
    // production: initialize the stderr subscriber once, tolerating an
    // already-installed global subscriber.
    let _ = tracing_subscriber::fmt()
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();

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
    // forwarded to `APEX_MAIL_WEBHOOK` (the ApexMail hook) when set. The
    // `req_…` confirmation ids are salted with `APEX_LEAD_ID_SALT` when the
    // deployment names one; without it the store generates a persistent salt
    // beside the leads file on first use.
    if let Some(path) = env_value("APEX_LEADS_FILE") {
        let transport = Arc::new(
            ReqwestTransport::new()
                .map_err(|error| format!("lead webhook transport could not be built: {error}"))?,
        );
        let mut store = apex_api::FileLeadStore::new(path);
        if let Some(salt) = env_value("APEX_LEAD_ID_SALT") {
            store = store.with_id_salt(salt);
        }
        let store: Arc<dyn apex_api::LeadStore> = match env_value("APEX_MAIL_WEBHOOK") {
            Some(url) => Arc::new(store.with_mail_webhook(url, transport)),
            None => Arc::new(store),
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
        (Some(root), Some(key_hex), token) => {
            let live_store =
                VaultedFileAccountStore::with_vault_key_hex(&root, &key_hex).map_err(|error| {
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
            let mut auditor =
                LiveAuditor::new(Arc::new(live_store), transport, Arc::clone(&state.clock));
            if let Some(token) = token {
                auditor = auditor.with_token(token);
            }
            let auditor = Arc::new(auditor);
            // The console connection routes validate submitted credentials
            // through the same read path; the JSON route keeps its bearer
            // token gate (`with_live_auditor` is only set when one exists).
            let connection_store =
                apex_api::connections::VaultedConnectionStore::with_vault_key_hex(&root, &key_hex)
                    .map_err(|error| {
                        format!("APEX_VAULT_KEY_HEX is not a valid vault key: {error:?}")
                    })?;
            state = state
                .with_connection_store(Arc::new(connection_store))
                .with_connection_validator(Arc::clone(&auditor));
            let requires_token = env_value("APEX_LIVE_AUDIT_TOKEN").is_some();
            if requires_token {
                state = state.with_live_auditor(auditor);
            }
        }
        (None, None, None) => {}
        _ => {
            return Err(
                "APEX_CREDENTIAL_ROOT and APEX_VAULT_KEY_HEX must be set together (APEX_LIVE_AUDIT_TOKEN is optional and enables the JSON live-audit route)".into(),
            );
        }
    }

    // The workspace publisher the phase-2 console surfaces read
    // (`APEX_CONSOLE_PUBLISHER`), and the proposal files the runner writes
    // (`APEX_PROPOSALS_ROOT`) plus the runner's alert log
    // (`APEX_ALERT_FILE`, with `APEX_ALERTS_FILE` as a fallback).
    if let Some(publisher) = env_value("APEX_CONSOLE_PUBLISHER") {
        state = state.with_workspace_publisher(publisher);
    }
    if let Some(path) = env_value(apex_api::proposals::PROPOSALS_ROOT_ENV) {
        state =
            state.with_proposal_store(Arc::new(apex_api::proposals::FileProposalStore::new(path)));
    }
    if let Some(path) = apex_api::experiments::alerts_file_from_env(&env_value) {
        state = state.with_alerts_path(path);
    }
    if let Some(path) = env_value("APEX_COLLECTION_FILE") {
        state = state.with_collection_log_path(path);
    }
    if let Some(salt) = env_value("APEX_LEAD_ID_SALT") {
        state = state.with_lead_id_salt(salt);
    }

    // The analytics sink and reader share the ClickHouse settings; when they
    // are unset the sink logs and the console renders the honest
    // not-available state.
    state = state.with_analytics(apex_api::AnalyticsAccess::from_env());
    // Boot creates the schema once (idempotent) so a fresh deployment starts
    // writing instead of spooling against a missing database. A failure is
    // logged and left visible; the spool keeps events durable meanwhile.
    if let Some(client) = state.analytics.client() {
        if let Err(error) = client.ensure_schema().await {
            eprintln!("apex-api: analytics schema setup failed: {error}");
        }
    }

    // With `APEX_METRICS_TEXTFILE` set, a background task exports the
    // registry to that file every 15 seconds; the graceful shutdown below
    // writes the final file before the process exits.
    let metrics_registry = Arc::clone(&state.metrics_registry);
    apex_api::metrics::spawn_textfile_exporter(Arc::clone(&metrics_registry));

    let listener = tokio::net::TcpListener::bind(bind).await?;
    println!(
        "apex-api (read-only mode) listening on http://{bind} (adyen hmac configured: {})",
        state.adyen_hmac_key.is_some() || state.adyen.is_some()
    );
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    apex_api::metrics::export_textfile(&metrics_registry);
    Ok(())
}

/// Resolve when the process is asked to stop (Ctrl-C, or SIGTERM on Unix),
/// so the final metrics textfile export runs before the process exits.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
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
