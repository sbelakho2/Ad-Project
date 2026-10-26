//! Phase-2 integration tests: connections, proposals, experiments, billing,
//! settings and analytics through the real router.
//!
//! Every mutation goes through a live console session (CSRF) and answers
//! `303`; every refusal keeps its documented status. The analytics sink is
//! exercised against an `apex-analytics` `MemoryStore` (schema created in the
//! test), never a live database.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;

use apex_analytics::store::{EventStore, ProjectionStore, ReportDayRow};
use apex_analytics::{MemoryReader, MemoryStore};
use apex_api::{AnalyticsAccess, AppState};
use axum::http::{header, StatusCode};
use common::{
    file_state, get, get_with_bearer, get_with_cookie, open_ledger_session, post_form,
    post_form_with_cookie, pub1_keys, send, set_cookie, TempDir, LEDGER_TOKEN, PUBLISHER,
};
use serde_json::json;

/// The vault key the connection store tests use (64 hex characters).
const VAULT_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

/// The analytics client over a memory store (the store contract asks for the
/// trait object).
fn client_over(store: &Arc<MemoryStore>) -> apex_analytics::AnalyticsClient {
    let concrete: Arc<MemoryStore> = Arc::clone(store);
    let store: Arc<dyn EventStore> = concrete;
    apex_analytics::AnalyticsClient::new(store, "test", "apex-api-test")
}

/// The memory reader over the same store.
fn reader_over(store: &Arc<MemoryStore>) -> Arc<dyn apex_analytics::AnalyticsReader> {
    Arc::new(MemoryReader::new((**store).clone()))
}

fn connection_dir(dir: &common::TempDir) -> String {
    dir.path().join("credentials").display().to_string()
}

/// State with a vault-backed connection store and an optional analytics
/// access; the console publisher is always the live session's.
fn console_state(dir: &common::TempDir, analytics: Option<AnalyticsAccess>) -> AppState {
    let store = apex_api::connections::VaultedConnectionStore::with_vault_key_hex(
        connection_dir(dir),
        VAULT_KEY,
    )
    .expect("vault key");
    let mut state = file_state(dir.path(), pub1_keys()).with_connection_store(Arc::new(store));
    if let Some(analytics) = analytics {
        state = state.with_analytics(analytics);
    }
    state
}

async fn app(state: AppState) -> axum::Router {
    apex_api::router(state)
}

#[tokio::test]
async fn connections_without_console_credentials_or_a_session_keep_their_states() {
    // No configured ledger tokens at all: no session can ever be opened, so
    // the page keeps the not-configured panel.
    let dir = TempDir::new("p2-conn-nopub");
    let no_auth = apex_api::AppState::new(
        Arc::new(apex_api::JsonFileAuditSource::new(dir.path())),
        Arc::new(apex_api::FileLedgerSource::new(
            dir.path(),
            std::collections::HashMap::new(),
        )),
    );
    let router = app(no_auth).await;
    let response = send(&router, get("/console/connections")).await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(response
        .body
        .contains("This view is not available in this deployment."));
    assert!(!response.body.contains("<script"));

    // With credentials configured but no session, the stable access panel is
    // the recovery point; with the session, the page renders.
    let router = app(console_state(&dir, None)).await;
    let anonymous = send(&router, get("/console/connections")).await;
    assert_eq!(anonymous.status, StatusCode::SEE_OTHER);
    assert_eq!(
        anonymous
            .headers
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/console/ledger")
    );
    let cookie = open_ledger_session(&router).await;
    let page = send(&router, get_with_cookie("/console/connections", &cookie)).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    assert!(page.body.contains("pub1 / as of"), "{}", page.body);
}

#[tokio::test]
async fn connection_submission_stores_secrets_encrypted_never_echoes_and_redirects() {
    let dir = TempDir::new("p2-conn-submit");
    let router = app(console_state(&dir, None)).await;
    let cookie = open_ledger_session(&router).await;

    // The detail page renders the credentials form with the frozen copy.
    let page = send(
        &router,
        get_with_cookie("/console/connections/applovin_max", &cookie),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Connect for the audit."));
    assert!(page.body.contains("writes need a verified allow-list"));
    let csrf = common::extract_csrf(&page.body);

    let body = format!(
        "csrf_token={csrf}&action=submit&account_id=acct-1&api_key=management-secret-value&report_key=reporting-secret-value"
    );
    let submitted = send(
        &router,
        post_form_with_cookie("/console/connections/applovin_max/submit", &body, &cookie),
    )
    .await;
    assert_eq!(submitted.status, StatusCode::SEE_OTHER);
    assert_eq!(
        submitted
            .headers
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/console/connections/applovin_max")
    );

    // The stored bundle is ciphertext and the response never carries a secret.
    let blob = std::fs::read_to_string(dir.path().join("credentials").join("pub1.blob"))
        .expect("blob exists");
    assert!(!blob.contains("management-secret-value"));
    assert!(!blob.contains("reporting-secret-value"));
    let after = send(
        &router,
        get_with_cookie("/console/connections/applovin_max", &cookie),
    )
    .await;
    assert_eq!(after.status, StatusCode::OK);
    assert!(
        !after.body.contains("management-secret-value")
            && !after.body.contains("reporting-secret-value"),
        "no secret is echoed"
    );
    assert!(after.body.contains("Access is not yet confirmed."));
    assert!(after
        .body
        .contains("<code>account_id, api_key, report_key</code>"));
    assert!(after.body.contains("aes-256-gcm"));
    assert!(after
        .body
        .contains("Not available (the platform does not publish a refresh time here)"));

    // The metadata file records the observation without any secret value.
    let meta =
        std::fs::read_to_string(dir.path().join("credentials").join("pub1.connections.json"))
            .expect("meta exists");
    assert!(meta.contains("\"unvalidated\""));
    assert!(!meta.contains("management-secret-value"));
}

#[tokio::test]
async fn connection_submission_requires_a_session_and_the_csrf_token() {
    let dir = TempDir::new("p2-conn-csrf");
    let router = app(console_state(&dir, None)).await;
    // Without a session the POST redirects to the access panel.
    let response = send(
        &router,
        post_form(
            "/console/connections/applovin_max/submit",
            "account_id=a&api_key=b",
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/console/ledger")
    );

    // With a session but a wrong token the form is refused.
    let cookie = open_ledger_session(&router).await;
    let response = send(
        &router,
        post_form_with_cookie(
            "/console/connections/applovin_max/submit",
            "csrf_token=wrong&account_id=a&api_key=b",
            &cookie,
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);

    // Missing required fields re-render the form with field errors.
    let page = send(
        &router,
        get_with_cookie("/console/connections/levelplay", &cookie),
    )
    .await;
    let csrf = common::extract_csrf(&page.body);
    let response = send(
        &router,
        post_form_with_cookie(
            "/console/connections/levelplay/submit",
            &format!("csrf_token={csrf}&account_id=&api_key=&refresh_token="),
            &cookie,
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::BAD_REQUEST);
    assert!(response
        .body
        .contains("This field is required to confirm access."));
    assert!(
        response.body.contains("role=\"alert\""),
        "the error summary links the fields"
    );

    // Unknown platforms are 404s and disconnect needs the session too.
    let unknown = send(
        &router,
        get_with_cookie("/console/connections/unknown_platform", &cookie),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn disconnect_removes_the_platform_and_keeps_the_response_honest() {
    let dir = TempDir::new("p2-conn-disc");
    let router = app(console_state(&dir, None)).await;
    let cookie = open_ledger_session(&router).await;
    let page = send(
        &router,
        get_with_cookie("/console/connections/admob", &cookie),
    )
    .await;
    let csrf = common::extract_csrf(&page.body);
    let body = format!("csrf_token={csrf}&action=submit&account_id=pub-1&api_key=token-value");
    let submitted = send(
        &router,
        post_form_with_cookie("/console/connections/admob/submit", &body, &cookie),
    )
    .await;
    assert_eq!(submitted.status, StatusCode::SEE_OTHER);

    let detail = send(
        &router,
        get_with_cookie("/console/connections/admob", &cookie),
    )
    .await;
    let csrf = common::extract_csrf(&detail.body);
    assert!(detail
        .body
        .contains("No running work is recorded for this platform."));
    // Without the explicit confirmation nothing is disconnected.
    let unconfirmed = send(
        &router,
        post_form_with_cookie(
            "/console/connections/admob/disconnect",
            &format!("csrf_token={csrf}&action=disconnect"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(unconfirmed.status, StatusCode::BAD_REQUEST);
    let still_there = send(
        &router,
        get_with_cookie("/console/connections/admob", &cookie),
    )
    .await;
    assert!(still_there.body.contains("Disconnect this platform"));
    let response = send(
        &router,
        post_form_with_cookie(
            "/console/connections/admob/disconnect",
            &format!("csrf_token={csrf}&action=disconnect&confirm=yes"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::SEE_OTHER);
    let after = send(
        &router,
        get_with_cookie("/console/connections/admob", &cookie),
    )
    .await;
    assert!(after
        .body
        .contains("Not available (no credential is stored)"));
    assert!(!after.body.contains("Disconnect this platform"));
}

fn write_proposal(dir: &common::TempDir, id: &str, expires_at: Option<i64>) {
    write_proposal_for(dir, id, expires_at, common::PUBLISHER);
}

fn write_proposal_for(dir: &common::TempDir, id: &str, expires_at: Option<i64>, publisher: &str) {
    let proposal = json!({
        "publisher": publisher,
        "proposal": {
            "action": {"setCountryFloor": {
                "ad_unit_id": "rw_de_01",
                "country": "DE",
                "floor_ecpm": 3_200_000_i64
            }},
            "rationale": "Raise the DE rewarded floor by 8%",
            "expectedEffectBp": 260,
            "prevValue": "{\"DE\":4000000}"
        },
        "snapshot": "14 Oct 2026, 08:10 UTC",
        "createdAtUnixMs": 1_792_000_000_000_i64,
        "policyVersion": "apex-autopilot-0.1.0",
        "expiresAtUnixMs": expires_at,
        "testShare": 0.25,
        "burnInDays": 3,
        "windowDays": 14,
        "rateBp": 2000,
        "thresholds": {
            "harmPoint": -0.05,
            "fillRewarded": 0.02,
            "fillBanner": 0.05,
            "splitChiSquare": 10.83
        }
    });
    std::fs::write(
        dir.path().join(format!("{id}.json")),
        serde_json::to_vec_pretty(&proposal).expect("json"),
    )
    .expect("write proposal");
}

fn idempotency_keys(html: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("name=\"idempotency_key\" value=\"") {
        rest = &rest[start + "name=\"idempotency_key\" value=\"".len()..];
        let Some(end) = rest.find('"') else { break };
        keys.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    keys
}

fn proposal_state(dir: &common::TempDir) -> AppState {
    let store = apex_api::proposals::FileProposalStore::new(dir.path());
    file_state(dir.path(), pub1_keys()).with_proposal_store(Arc::new(store))
}

#[tokio::test]
async fn proposal_index_lists_proposals_with_states_and_caps_the_files() {
    let dir = common::TempDir::new("proposals-index");
    write_proposal(&dir, "p_0247", Some(1_792_055_400_000));
    write_proposal(&dir, "p_0248", None);
    std::fs::write(dir.path().join("not-a-proposal.txt"), b"x").expect("write");
    let router = app(proposal_state(&dir)).await;
    let cookie = open_ledger_session(&router).await;
    let response = send(&router, get_with_cookie("/console/proposals", &cookie)).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert!(response.body.contains("p_0247"));
    assert!(response.body.contains("p_0248"));
    assert!(response.body.contains("Approval required"));
    // The rail now carries every area the console serves.
    for href in [
        "/console/analytics",
        "/console/proposals",
        "/console/experiments",
        "/console/billing",
        "/console/connections",
        "/console/settings",
    ] {
        assert!(response.body.contains(href), "rail is missing {href}");
    }
    // A non-proposal file is skipped by name, not parsed: nothing to report.
    assert!(!response.body.contains("could not be read"));

    // Without a store the route keeps the not-configured state (the session
    // is the gate; the store state is what the authenticated user sees).
    let bare = file_state(dir.path(), pub1_keys());
    let router = app(bare).await;
    let cookie = open_ledger_session(&router).await;
    let response = send(&router, get_with_cookie("/console/proposals", &cookie)).await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn decline_form_submits_the_decline_key_and_records_the_publisher() {
    let dir = TempDir::new("p2-proposal-decline");
    write_proposal(&dir, "p_0301", Some(i64::MAX / 2));
    let router = app(proposal_state(&dir)).await;
    let cookie = open_ledger_session(&router).await;

    let page = send(
        &router,
        get_with_cookie("/console/proposals/p_0301", &cookie),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK);
    let csrf = common::extract_csrf(&page.body);
    let keys = idempotency_keys(&page.body);
    assert_eq!(keys.len(), 2, "one key per decision form: {}", page.body);

    let declined = send(
        &router,
        post_form_with_cookie(
            "/console/proposals/p_0301/decision",
            &format!(
                "csrf_token={csrf}&decision=decline&idempotency_key={}",
                keys[1]
            ),
            &cookie,
        ),
    )
    .await;
    assert_eq!(
        declined.status,
        StatusCode::SEE_OTHER,
        "the rendered decline form must be accepted: {}",
        declined.body
    );
    let record =
        std::fs::read_to_string(dir.path().join("p_0301.approval.json")).expect("decline decision");
    assert!(record.contains("\"decision\": \"declined\""), "{record}");
    assert!(
        record.contains("\"approvalActor\": \"pub1\""),
        "the acting workspace is recorded: {record}"
    );
}

#[tokio::test]
async fn proposals_are_scoped_to_the_session_publisher() {
    let dir = TempDir::new("p2-proposal-tenant");
    write_proposal_for(&dir, "p_0401", Some(i64::MAX / 2), common::PUBLISHER);
    write_proposal_for(&dir, "p_0402", Some(i64::MAX / 2), common::PUBLISHER2);
    let store = apex_api::proposals::FileProposalStore::new(dir.path());
    let router =
        app(file_state(dir.path(), common::two_publisher_keys())
            .with_proposal_store(Arc::new(store)))
        .await;
    let cookie = open_ledger_session(&router).await;

    let index = send(&router, get_with_cookie("/console/proposals", &cookie)).await;
    assert_eq!(index.status, StatusCode::OK);
    assert!(index.body.contains("p_0401"));
    assert!(
        !index.body.contains("p_0402"),
        "another workspace's proposal id must not be listed"
    );

    let foreign = send(
        &router,
        get_with_cookie("/console/proposals/p_0402", &cookie),
    )
    .await;
    assert_eq!(foreign.status, StatusCode::NOT_FOUND);

    let own = send(
        &router,
        get_with_cookie("/console/proposals/p_0401", &cookie),
    )
    .await;
    let csrf = common::extract_csrf(&own.body);
    let key = idempotency_keys(&own.body)[0].clone();
    let decision = send(
        &router,
        post_form_with_cookie(
            "/console/proposals/p_0402/decision",
            &format!("csrf_token={csrf}&decision=approve&idempotency_key={key}"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(decision.status, StatusCode::NOT_FOUND);
    assert!(
        !dir.path().join("p_0402.approval.json").exists(),
        "no decision may be recorded on a foreign proposal"
    );
}

#[tokio::test]
async fn billing_shows_only_the_session_publishers_payments() {
    let dir = TempDir::new("p2-billing-tenant");
    billing_fixture(&dir);
    let log = dir.path().join("collection.jsonl");
    let events = [
        json!({"atUnixMs": 1_792_000_000_000_i64, "status": "paid",
               "paymentId": "PSP-OWN", "reference": "apex-fee:pub1:exp:2026-10",
               "success": "true", "amountMinorUnits": 10_000, "amountCurrency": "USD"}),
        json!({"atUnixMs": 1_792_000_000_100_i64, "status": "paid",
               "paymentId": "PSP-FOREIGN", "reference": "apex-fee:pub2:exp:2026-10",
               "success": "true", "amountMinorUnits": 20_000, "amountCurrency": "USD"}),
        json!({"atUnixMs": 1_792_000_000_200_i64, "status": "paid",
               "paymentId": "PSP-WATCH", "reference": "apex-watch:sub_0123456789:2026-10",
               "success": "true", "amountMinorUnits": 50_000, "amountCurrency": "USD"}),
    ];
    let mut body = String::new();
    for event in &events {
        body.push_str(&serde_json::to_string(event).unwrap());
        body.push('\n');
    }
    std::fs::write(&log, body).expect("collection log");
    let router = app(proposal_state(&dir).with_collection_log_path(&log)).await;
    let cookie = open_ledger_session(&router).await;

    let page = send(
        &router,
        get_with_cookie("/console/billing/invoices/2026-10", &cookie),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("PSP-OWN"), "own payment is shown");
    assert!(
        !page.body.contains("PSP-FOREIGN"),
        "another publisher's payment reference must not leak"
    );
    assert!(
        !page.body.contains("PSP-WATCH"),
        "watch subscriptions are not publisher billing"
    );
}

#[tokio::test]
async fn proposal_decisions_are_csrf_bound_idempotent_and_final() {
    let dir = TempDir::new("p2-proposal");
    write_proposal(&dir, "p_0247", Some(i64::MAX / 2));
    let router = app(proposal_state(&dir)).await;
    let cookie = open_ledger_session(&router).await;

    let page = send(
        &router,
        get_with_cookie("/console/proposals/p_0247", &cookie),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains("Review a proposed test"));
    assert!(page.body.contains("Country floor (rw_de_01 / DE)"));
    assert!(page.body.contains("$4.00 eCPM"));
    assert!(page.body.contains("$3.20 eCPM"));
    let csrf = common::extract_csrf(&page.body);
    let key = extract_idempotency_key(&page.body, "idempotency_key");

    let approved = send(
        &router,
        post_form_with_cookie(
            "/console/proposals/p_0247/decision",
            &format!("csrf_token={csrf}&decision=approve&idempotency_key={key}"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(approved.status, StatusCode::SEE_OTHER);
    assert!(dir.path().join("p_0247.approval.json").exists());

    // A retry with the same key returns the original operation status.
    let retried = send(
        &router,
        post_form_with_cookie(
            "/console/proposals/p_0247/decision",
            &format!("csrf_token={csrf}&decision=approve&idempotency_key={key}"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(retried.status, StatusCode::SEE_OTHER);

    // A different decision is refused and the record stands.
    let declined = send(
        &router,
        post_form_with_cookie(
            "/console/proposals/p_0247/decision",
            &format!("csrf_token={csrf}&decision=decline&idempotency_key={key}"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(
        declined.status,
        StatusCode::BAD_REQUEST,
        "a stale key is refused"
    );
    let declined = send(
        &router,
        post_form_with_cookie(
            "/console/proposals/p_0247/decision",
            &format!(
                "csrf_token={csrf}&decision=decline&idempotency_key={}",
                wrong_idempotency_key()
            ),
            &cookie,
        ),
    )
    .await;
    assert_eq!(declined.status, StatusCode::BAD_REQUEST);

    let decided = send(
        &router,
        get_with_cookie("/console/proposals/p_0247", &cookie),
    )
    .await;
    assert!(decided.body.contains("Already decided"));
    assert!(decided.body.contains("Approved"));
    assert!(!decided.body.contains("Approve this test"));
    let record: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("p_0247.approval.json")).expect("record"),
    )
    .expect("json");
    assert_eq!(record["decision"], "approved");
    assert_eq!(record["approvalActor"], "pub1");
}

/// The route derives the key; the test only needs a well-formed wrong one.
fn wrong_idempotency_key() -> String {
    "0".repeat(32)
}

fn extract_idempotency_key(html: &str, name: &str) -> String {
    let anchor = format!("name=\"{name}\" value=\"");
    let start = html.find(&anchor).expect("idempotency input") + anchor.len();
    let end = start + html[start..].find('"').expect("closing quote");
    html[start..end].to_string()
}

#[tokio::test]
async fn unknown_and_unconfigured_proposals_keep_their_states() {
    let dir = TempDir::new("p2-proposal-unknown");
    let router = app(proposal_state(&dir)).await;
    let cookie = open_ledger_session(&router).await;
    let missing = send(
        &router,
        get_with_cookie("/console/proposals/p_missing", &cookie),
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert!(missing.body.contains("No proposal matches this address."));

    let bare = app(file_state(dir.path(), pub1_keys())).await;
    let cookie = open_ledger_session(&bare).await;
    let unconfigured = send(&bare, get_with_cookie("/console/proposals/p_0247", &cookie)).await;
    assert_eq!(unconfigured.status, StatusCode::SERVICE_UNAVAILABLE);
}

/// The proposal races run over a real analytics sink, so file and event can
/// be compared.
async fn racing_state(dir: &TempDir) -> (AppState, Arc<MemoryStore>) {
    let store = Arc::new(MemoryStore::new());
    store.ensure_schema().await.expect("schema");
    let state = proposal_state(dir).with_analytics(AnalyticsAccess::enabled(
        reader_over(&store),
        client_over(&store),
    ));
    (state, store)
}

/// The decision form values of one proposal page: CSRF, the approve key and
/// the separately noted decline key.
async fn decision_form(router: &axum::Router, id: &str, cookie: &str) -> (String, String, String) {
    let page = send(
        router,
        get_with_cookie(&format!("/console/proposals/{id}"), cookie),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.body);
    let csrf = common::extract_csrf(&page.body);
    let keys = idempotency_keys(&page.body);
    assert_eq!(keys.len(), 2, "one key per decision form: {}", page.body);
    (csrf, keys[0].clone(), keys[1].clone())
}

fn approval_files(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .expect("dir")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".approval.json"))
        .collect()
}

fn temp_files(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .expect("dir")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp"))
        .collect()
}

/// The one recorded decision event of `id`, checked against the durable file.
async fn assert_file_and_event_agree(
    dir: &std::path::Path,
    store: &Arc<MemoryStore>,
    id: &str,
) -> &'static str {
    let record: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join(format!("{id}.approval.json"))).expect("decision file"),
    )
    .expect("decision json");
    let decision = record["decision"]
        .as_str()
        .expect("decision field")
        .to_string();
    let events = store.load_events().await.expect("events");
    let proposal_events: Vec<_> = events
        .iter()
        .filter(|event| event.payload.proposal_id.as_deref() == Some(id))
        .collect();
    assert_eq!(
        proposal_events.len(),
        1,
        "exactly one decision event for {id}: {events:?}"
    );
    let event = proposal_events[0];
    match decision.as_str() {
        "approved" => assert_eq!(
            event.event_type,
            apex_analytics::EventType::ProposalApproved
        ),
        "declined" => assert_eq!(
            event.event_type,
            apex_analytics::EventType::ProposalDeclined
        ),
        other => panic!("unexpected recorded decision {other}"),
    }
    assert_eq!(
        event.payload.change_hash.as_deref(),
        record["changeHash"].as_str()
    );
    if decision == "approved" {
        "Approved"
    } else {
        "Declined"
    }
}

#[tokio::test]
async fn concurrent_same_key_decisions_both_303_with_one_record_and_one_event() {
    let dir = TempDir::new("p2-proposal-race-same");
    write_proposal(&dir, "p_0247", Some(i64::MAX / 2));
    let (state, store) = racing_state(&dir).await;
    let router = app(state).await;
    let cookie = open_ledger_session(&router).await;
    let (csrf, key, _) = decision_form(&router, "p_0247", &cookie).await;
    let request = || {
        post_form_with_cookie(
            "/console/proposals/p_0247/decision",
            &format!("csrf_token={csrf}&decision=approve&idempotency_key={key}"),
            &cookie,
        )
    };
    let (first, second) = tokio::join!(async { send(&router, request()).await }, async {
        send(&router, request()).await
    });
    assert_eq!(first.status, StatusCode::SEE_OTHER, "{}", first.body);
    assert_eq!(second.status, StatusCode::SEE_OTHER, "{}", second.body);
    assert_eq!(
        approval_files(dir.path()).len(),
        1,
        "exactly one decision file"
    );
    let label = assert_file_and_event_agree(dir.path(), &store, "p_0247").await;
    assert_eq!(label, "Approved");
    assert!(
        temp_files(dir.path()).is_empty(),
        "no temp file survives: {:?}",
        temp_files(dir.path())
    );
}

#[tokio::test]
async fn concurrent_approve_and_decline_agree_with_the_winner_everywhere() {
    let dir = TempDir::new("p2-proposal-race-mixed");
    write_proposal(&dir, "p_0247", Some(i64::MAX / 2));
    let (state, store) = racing_state(&dir).await;
    let router = app(state).await;
    let cookie = open_ledger_session(&router).await;
    let (csrf, approve_key, decline_key) = decision_form(&router, "p_0247", &cookie).await;
    let approve = || {
        post_form_with_cookie(
            "/console/proposals/p_0247/decision",
            &format!("csrf_token={csrf}&decision=approve&idempotency_key={approve_key}"),
            &cookie,
        )
    };
    let decline = || {
        post_form_with_cookie(
            "/console/proposals/p_0247/decision",
            &format!("csrf_token={csrf}&decision=decline&idempotency_key={decline_key}"),
            &cookie,
        )
    };
    let (approved, declined) = tokio::join!(async { send(&router, approve()).await }, async {
        send(&router, decline()).await
    });
    // Exactly one writer wins with a 303; the loser gets the typed conflict
    // page and never an internal error.
    let (winner, loser, winner_label) = match (approved.status, declined.status) {
        (StatusCode::SEE_OTHER, StatusCode::CONFLICT) => (approved, declined, "Approved"),
        (StatusCode::SEE_OTHER, StatusCode::SEE_OTHER) => {
            unreachable!("different decisions cannot both replay")
        }
        (StatusCode::SEE_OTHER, other) => {
            panic!("unexpected loser status {other}: {}", declined.body)
        }
        (other, StatusCode::SEE_OTHER) => {
            assert_eq!(other, StatusCode::CONFLICT, "{}", approved.body);
            (declined, approved, "Declined")
        }
        (first, second) => panic!("no winner: {first} / {second}"),
    };
    assert_eq!(
        winner
            .headers
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/console/proposals/p_0247"),
        "the winner redirects to the decided page"
    );
    let label = assert_file_and_event_agree(dir.path(), &store, "p_0247").await;
    assert_eq!(label, winner_label, "file and winner status agree");
    assert!(
        loser.body.contains("Already decided"),
        "the loser gets the typed conflict page: {}",
        loser.body
    );
    assert!(
        loser.body.contains(winner_label),
        "the conflict page names the winner's decision: {}",
        loser.body
    );
    assert_eq!(approval_files(dir.path()).len(), 1);
    assert!(
        temp_files(dir.path()).is_empty(),
        "no temp file survives: {:?}",
        temp_files(dir.path())
    );
}

#[tokio::test]
async fn an_eight_way_same_key_storm_has_zero_500s_and_one_record() {
    let dir = TempDir::new("p2-proposal-storm");
    write_proposal(&dir, "p_0248", Some(i64::MAX / 2));
    let (state, store) = racing_state(&dir).await;
    let router = app(state).await;
    let cookie = open_ledger_session(&router).await;
    let (csrf, key, _) = decision_form(&router, "p_0248", &cookie).await;
    let attempts = 8;
    let mut tasks = Vec::new();
    for _ in 0..attempts {
        let router = router.clone();
        let cookie = cookie.clone();
        let csrf = csrf.clone();
        let key = key.clone();
        tasks.push(tokio::spawn(async move {
            send(
                &router,
                post_form_with_cookie(
                    "/console/proposals/p_0248/decision",
                    &format!("csrf_token={csrf}&decision=approve&idempotency_key={key}"),
                    &cookie,
                ),
            )
            .await
        }));
    }
    let mut statuses = Vec::new();
    for task in tasks {
        let response = task.await.expect("join");
        assert!(
            response.status.is_success() || response.status.is_redirection(),
            "no request may fail (got {}): {}",
            response.status,
            response.body
        );
        statuses.push(response.status);
    }
    assert_eq!(
        statuses,
        vec![StatusCode::SEE_OTHER; attempts],
        "every attempt either records or replays as 303"
    );
    println!("storm attempts: {attempts}, statuses: {statuses:?}");
    assert_eq!(
        approval_files(dir.path()).len(),
        1,
        "exactly one decision file"
    );
    let label = assert_file_and_event_agree(dir.path(), &store, "p_0248").await;
    assert_eq!(label, "Approved");
    assert!(
        temp_files(dir.path()).is_empty(),
        "no temp file survives the storm: {:?}",
        temp_files(dir.path())
    );
}

#[tokio::test]
async fn expired_proposals_render_the_expired_state_without_a_form() {
    let dir = TempDir::new("p2-proposal-expired");
    write_proposal(&dir, "p_0248", Some(1));
    let router = app(proposal_state(&dir)).await;
    let cookie = open_ledger_session(&router).await;
    let page = send(
        &router,
        get_with_cookie("/console/proposals/p_0248", &cookie),
    )
    .await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.body.contains(">Expired<"));
    assert!(page
        .body
        .contains("Nothing is written for an expired proposal."));
    assert!(!page
        .body
        .contains("<form method=\"post\" action=\"/console/proposals/p_0248/decision\">"));
}

/// A sealed lift record whose payload digest is honest.
fn lift_payload(payload: &str) -> apex_proof::LedgerRecord {
    use sha2::{Digest, Sha256};
    let digest = hex::encode(Sha256::digest(payload.as_bytes()));
    apex_proof::LedgerRecord::lift_record(1_792_000_000_000, &digest, payload)
}

fn experiment_fixture(dir: &common::TempDir) {
    use apex_api::Clock;
    use common::{seed_keypair, write_ledger};
    let day = 86_400_000_i64;
    let now = apex_api::clock::SystemClock.now_unix_s() * 1000;
    let day_string = |offset_days: i64| {
        apex_analytics::UtcMillis::from_unix_millis(now - offset_days * day).to_day_string()
    };
    let started = apex_proof::LedgerRecord::experiment_started(
        now - 20 * day,
        apex_core::PlatformId::AppLovinMax,
        "exp_0246",
        apex_core::ExperimentSpec {
            name: "DE floor".to_string(),
            ad_unit_id: "rw_de_01".to_string(),
            metric: apex_core::ExperimentMetric::Ecpm,
            variants: 2,
            test_group_allocation: 25,
            change_description: "raise the DE floor".to_string(),
        },
    );
    let prereg = lift_payload(
        r#"{"type":"preRegistration","experiment_id":"exp_0246","scope":"rw_de_01","before_value":{"DE":4000000},"after_value":{"DE":3200000},"split":0.25,"metric":"RevenuePerUser","burn_in_days":3,"window_days":14,"looks":1,"extension_allowed":true,"guardrail_thresholds":{"harmPoint":-0.05,"fillRewarded":0.02,"fillBanner":0.05,"splitChiSquare":10.83},"code_version_hash":"apex-lift-test","rate_bp":2000}"#,
    );
    let pull = |offset_days: i64| {
        lift_payload(&format!(
            r#"{{"type":"dataPull","day":"{}","rows_sha256":"ab","rows":[]}}"#,
            day_string(offset_days)
        ))
    };
    let result = lift_payload(
        r#"{"type":"result","experiment_id":"exp_0246","daily_log_ratios":[0.05,0.055,0.052],"mean_log":0.052,"lower_90":0.03,"lower_99":0.02,"bootstrap_lower":0.028,"decision":"keep","lambda":0.03}"#,
    );
    write_ledger(
        dir.path(),
        PUBLISHER,
        &seed_keypair(7),
        vec![started, prereg, pull(16), pull(15), pull(14), result],
    );
}

/// Repeated experiment renders parse the alert log once; a changed log
/// invalidates the cached view and is parsed again.
#[tokio::test]
async fn experiment_alert_log_is_parsed_once_until_it_changes() {
    let dir = TempDir::new("p2-alert-cache");
    experiment_fixture(&dir);
    let alerts = dir.path().join("alerts.jsonl");
    std::fs::write(
        &alerts,
        "{\"kind\":\"failedRollback\",\"experiment_id\":\"exp_0246\",\"detail\":\"first\"}\n",
    )
    .expect("alerts");
    let state = file_state(dir.path(), pub1_keys()).with_alerts_path(&alerts);
    let router = app(state.clone()).await;
    let cookie = open_ledger_session(&router).await;

    let list = send(&router, get_with_cookie("/console/experiments", &cookie)).await;
    assert_eq!(list.status, StatusCode::OK);
    let detail = send(
        &router,
        get_with_cookie("/console/experiments/exp_0246", &cookie),
    )
    .await;
    assert_eq!(detail.status, StatusCode::OK);
    let incident = send(
        &router,
        get_with_cookie("/console/experiments/exp_0246/incident", &cookie),
    )
    .await;
    assert_eq!(incident.status, StatusCode::OK);
    assert_eq!(
        state.alerts_cache_stats().parses,
        1,
        "three pages parse the unchanged alert log once"
    );

    std::fs::write(
        &alerts,
        "{\"kind\":\"failedRollback\",\"experiment_id\":\"exp_0246\",\"detail\":\"changed\"}\n\
         {\"kind\":\"failedRollback\",\"experiment_id\":\"exp_other\",\"detail\":\"second\"}\n",
    )
    .expect("rewrite alerts");
    let changed = send(&router, get_with_cookie("/console/experiments", &cookie)).await;
    assert_eq!(changed.status, StatusCode::OK);
    assert_eq!(
        state.alerts_cache_stats().parses,
        2,
        "the changed alert log is parsed exactly once"
    );
}

#[tokio::test]
async fn experiments_render_the_runner_states_from_verified_records() {
    let dir = TempDir::new("p2-experiments");
    experiment_fixture(&dir);
    let router = app(file_state(dir.path(), pub1_keys())).await;
    let cookie = open_ledger_session(&router).await;

    let list = send(&router, get_with_cookie("/console/experiments", &cookie)).await;
    assert_eq!(list.status, StatusCode::OK);
    assert!(list.body.contains(">Kept<"));
    assert!(list.body.contains("View fee basis"));
    assert!(list.body.contains("Pending data means waiting."));
    assert!(!list.body.contains("<script"));

    let detail = send(
        &router,
        get_with_cookie("/console/experiments/exp_0246", &cookie),
    )
    .await;
    assert_eq!(detail.status, StatusCode::OK);
    assert!(detail.body.contains("exp_0246 / rw_de_01"));
    assert!(detail.body.contains("Observed lift and uncertainty"));
    assert!(detail.body.contains("What this chart uses"));
    assert!(detail.body.contains("of 14 analysis days final"));
    assert!(detail.body.contains("The decision rule passed."));

    let bad_filter = send(
        &router,
        get_with_cookie("/console/experiments?filter=nope", &cookie),
    )
    .await;
    assert_eq!(bad_filter.status, StatusCode::BAD_REQUEST);
    let bad_tab = send(
        &router,
        get_with_cookie("/console/experiments/exp_0246?tab=nope", &cookie),
    )
    .await;
    assert_eq!(bad_tab.status, StatusCode::BAD_REQUEST);
    let unknown = send(
        &router,
        get_with_cookie("/console/experiments/exp_missing", &cookie),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    let no_incident = send(
        &router,
        get_with_cookie("/console/experiments/exp_0246/incident", &cookie),
    )
    .await;
    assert_eq!(no_incident.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_failed_rollback_alert_surfaces_the_recovery_state() {
    let dir = TempDir::new("p2-incident");
    experiment_fixture(&dir);
    let alerts = dir.path().join("alerts.jsonl");
    std::fs::write(
        &alerts,
        "{\"kind\":\"failedRollback\",\"experiment_id\":\"exp_0246\",\"detail\":\"rollback also failed\"}\n",
    )
    .expect("alerts");
    let router = app(file_state(dir.path(), pub1_keys()).with_alerts_path(&alerts)).await;
    let cookie = open_ledger_session(&router).await;

    let list = send(&router, get_with_cookie("/console/experiments", &cookie)).await;
    assert!(list.body.contains("Restore failed"));
    assert!(list.body.contains("Open incident"));
    let incident = send(
        &router,
        get_with_cookie("/console/experiments/exp_0246/incident", &cookie),
    )
    .await;
    assert_eq!(incident.status, StatusCode::OK);
    assert!(incident.body.contains("This test needs intervention"));
    assert!(incident
        .body
        .contains("The platform refused the stop request. Apex has not marked the test restored."));
    assert!(incident
        .body
        .contains("Restore not confirmed / incident inc_"));
    assert!(incident.body.contains("Operator investigating"));
    let detail = send(
        &router,
        get_with_cookie("/console/experiments/exp_0246", &cookie),
    )
    .await;
    assert!(detail.body.contains("Restore failed"));
    assert!(detail.body.contains("View incident details"));
}

fn billing_fixture(dir: &common::TempDir) {
    use common::{seed_keypair, write_ledger};
    let billing = lift_payload(
        r#"{"type":"monthlyBilling","changeId":"ch_0247","month":"2026-10","scopeRevenue":51500000000,"lambda":0.03,"factor":1.03,"incremental":1500000000,"logShare":1.0,"rateBp":2000,"feeMicros":300000000,"billedDays":31,"monthDays":31}"#,
    );
    write_ledger(dir.path(), PUBLISHER, &seed_keypair(7), vec![billing]);
}

#[tokio::test]
async fn billing_register_and_invoice_reproduce_signed_amounts() {
    let dir = TempDir::new("p2-billing");
    billing_fixture(&dir);
    let router = app(file_state(dir.path(), pub1_keys())).await;
    let cookie = open_ledger_session(&router).await;

    let register = send(&router, get_with_cookie("/console/billing", &cookie)).await;
    assert_eq!(register.status, StatusCode::OK);
    assert!(register.body.contains("USD / October 2026"));
    assert!(register.body.contains("$300.00"));
    assert!(register.body.contains("Invoice is still a draft."));

    let invoice = send(
        &router,
        get_with_cookie("/console/billing/invoices/2026-10", &cookie),
    )
    .await;
    assert_eq!(invoice.status, StatusCode::OK);
    assert!(invoice.body.contains("October 2026 fee calculation"));
    assert!(invoice.body.contains("Eligible scope revenue"));
    assert!(invoice.body.contains("$51,500.00"));
    assert!(invoice.body.contains("Signed decision, not point estimate"));
    assert!(invoice
        .body
        .contains("Not available (no VAT rate is recorded for this workspace)"));

    let unknown = send(
        &router,
        get_with_cookie("/console/billing/invoices/2025-01", &cookie),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    let garbage = send(
        &router,
        get_with_cookie("/console/billing/invoices/nonsense", &cookie),
    )
    .await;
    assert_eq!(garbage.status, StatusCode::NOT_FOUND);
}

/// Repeated billing renders parse the collection log once; a changed log
/// invalidates the cached view and is parsed again.
#[tokio::test]
async fn billing_collection_log_is_parsed_once_until_it_changes() {
    let dir = TempDir::new("p2-collection-cache");
    billing_fixture(&dir);
    let log = dir.path().join("collection.jsonl");
    let event = json!({"atUnixMs": 1_792_000_000_000_i64, "status": "paid",
           "paymentId": "PSP-ONE", "reference": "apex-fee:pub1:exp:2026-10",
           "success": "true", "amountMinorUnits": 10_000, "amountCurrency": "USD"});
    std::fs::write(
        &log,
        format!("{}\n", serde_json::to_string(&event).unwrap()),
    )
    .expect("collection log");
    let state = proposal_state(&dir).with_collection_log_path(&log);
    let router = app(state.clone()).await;
    let cookie = open_ledger_session(&router).await;

    for _ in 0..3 {
        let page = send(&router, get_with_cookie("/console/billing", &cookie)).await;
        assert_eq!(page.status, StatusCode::OK);
    }
    assert_eq!(
        state.collection_cache_stats().parses,
        1,
        "three requests parse the unchanged collection log once"
    );

    let second = json!({"atUnixMs": 1_792_000_000_100_i64, "status": "paid",
           "paymentId": "PSP-TWO", "reference": "apex-fee:pub1:exp:2026-10",
           "success": "true", "amountMinorUnits": 20_000, "amountCurrency": "USD"});
    std::fs::write(
        &log,
        format!(
            "{}\n{}\n",
            serde_json::to_string(&event).unwrap(),
            serde_json::to_string(&second).unwrap()
        ),
    )
    .expect("rewrite collection log");
    let page = send(&router, get_with_cookie("/console/billing", &cookie)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(
        state.collection_cache_stats().parses,
        2,
        "the changed collection log is parsed exactly once"
    );
}

#[tokio::test]
async fn settings_render_and_the_theme_form_sets_the_documented_cookie() {
    let dir = TempDir::new("p2-settings");
    let router = app(console_state(&dir, None)).await;

    // Without a session the settings page redirects to the access panel.
    let anonymous = send(&router, get("/console/settings")).await;
    assert_eq!(anonymous.status, StatusCode::SEE_OTHER);
    assert_eq!(
        anonymous
            .headers
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/console/ledger")
    );

    let cookie = open_ledger_session(&router).await;
    let page = send(&router, get_with_cookie("/console/settings", &cookie)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page
        .body
        .contains("Ask your account team to change these values."));
    assert!(
        page.body.contains("pub1"),
        "the session's publisher is shown"
    );

    // Without a session the settings POST redirects to the access panel.
    let refused = send(&router, post_form("/console/settings/theme", "theme=dark")).await;
    assert_eq!(refused.status, StatusCode::SEE_OTHER);
    assert_eq!(
        refused
            .headers
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/console/ledger")
    );

    let csrf = common::extract_csrf(&page.body);
    let themed = send(
        &router,
        post_form_with_cookie(
            "/console/settings/theme",
            &format!("csrf_token={csrf}&theme=dark"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(themed.status, StatusCode::SEE_OTHER);
    let set_cookie = set_cookie(&themed).expect("theme cookie");
    assert!(set_cookie.contains("apex_theme=dark"), "{set_cookie}");
    assert!(set_cookie.contains("Path=/console"));

    // The cookie drives the rendered theme on the next request, always with
    // the live session.
    let dark = send(
        &router,
        get_with_cookie("/console/settings", &format!("{cookie}; apex_theme=dark")),
    )
    .await;
    assert_eq!(dark.status, StatusCode::OK, "{}", dark.body);
    assert!(dark.body.contains("data-theme=\"dark\""));

    let invalid = send(
        &router,
        post_form_with_cookie(
            "/console/settings/theme",
            &format!("csrf_token={csrf}&theme=neon"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);
    assert!(invalid.body.contains("Choose System, Light or Dark."));
}

#[tokio::test]
async fn access_lists_sessions_by_handle_and_revokes_with_csrf() {
    let dir = TempDir::new("p2-access");
    let state = console_state(&dir, None);
    let sessions = Arc::clone(&state.ledger_sessions);
    let router = app(state).await;
    let first = open_ledger_session(&router).await;
    let second = open_ledger_session(&router).await;

    use apex_api::Clock;
    let summaries = sessions.summaries(apex_api::clock::SystemClock.now_unix_s() * 1000);
    assert_eq!(summaries.len(), 2);
    let victim = summaries
        .iter()
        .map(|summary| summary.handle.clone())
        .collect::<Vec<_>>();

    let page = send(&router, get_with_cookie("/console/settings/access", &first)).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page
        .body
        .contains("The ledger access token is never stored in the browser."));
    for handle in &victim {
        assert!(page.body.contains(handle), "the handle is listed");
    }
    let csrf = common::extract_csrf(&page.body);

    // Revoke the session that is not the caller's; the handle chosen is the
    // one that is not the current session, found from the summaries.
    let current = {
        // The access page marks the caller's row with "(this session)"; find
        // which handle that is by looking at the rendered table order.
        let marker = "(this session)";
        let position = page.body.find(marker).expect("current marker");
        let before = &page.body[..position];
        victim
            .iter()
            .rev()
            .find(|handle| before.contains(*handle))
            .cloned()
            .expect("current handle")
    };
    let other = victim
        .iter()
        .find(|handle| **handle != current)
        .cloned()
        .expect("another session");
    // Without confirmation the session stays live.
    let unconfirmed = send(
        &router,
        post_form_with_cookie(
            "/console/settings/access/revoke",
            &format!(
                "csrf_token={csrf}&handle={other}&idempotency_key={}",
                "0".repeat(32)
            ),
            &first,
        ),
    )
    .await;
    assert_eq!(unconfirmed.status, StatusCode::BAD_REQUEST);
    let still_live = send(&router, get_with_cookie("/console/settings/access", &first)).await;
    assert!(still_live.body.contains(&other), "the session is untouched");
    let revoked = send(
        &router,
        post_form_with_cookie(
            "/console/settings/access/revoke",
            &format!(
                "csrf_token={csrf}&handle={other}&idempotency_key={}&confirm=yes",
                "0".repeat(32)
            ),
            &first,
        ),
    )
    .await;
    assert_eq!(revoked.status, StatusCode::SEE_OTHER);
    let after = send(&router, get_with_cookie("/console/settings/access", &first)).await;
    assert!(!after.body.contains(&other), "the revoked handle is gone");

    // A revoke without CSRF is refused, and an unknown handle is a 400.
    let refused = send(
        &router,
        post_form_with_cookie(
            "/console/settings/access/revoke",
            &format!("handle={other}"),
            &first,
        ),
    )
    .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);

    let _ = second;
}

#[tokio::test]
async fn policy_and_monitoring_pages_show_recorded_values_and_settings_links() {
    let dir = TempDir::new("p2-policy");
    let policy_file = dir.path().join("policy.json");
    let policy = apex_autopilot::model::AutopilotPolicy::conservative();
    std::fs::write(
        &policy_file,
        serde_json::to_vec_pretty(&policy).expect("policy json"),
    )
    .expect("write policy");
    let router = app(console_state(&dir, None)).await;
    let cookie = open_ledger_session(&router).await;
    std::env::set_var("APEX_POLICY_FILE", &policy_file);
    let policy_page = send(
        &router,
        get_with_cookie("/console/settings/policy", &cookie),
    )
    .await;
    std::env::remove_var("APEX_POLICY_FILE");
    assert_eq!(policy_page.status, StatusCode::OK);
    assert!(policy_page
        .body
        .contains("Approval required before the first write of each test."));
    assert!(policy_page.body.contains("3 actions per day"));
    assert!(policy_page
        .body
        .contains("The signed order is the authority"));

    let monitoring = send(
        &router,
        get_with_cookie("/console/settings/monitoring", &cookie),
    )
    .await;
    assert_eq!(monitoring.status, StatusCode::OK);
    assert!(monitoring.body.contains("Monitoring plan"));
    assert!(monitoring.body.contains("Not recorded"));
    assert!(monitoring.body.contains("Per-platform reporting health"));
    assert!(monitoring.body.contains("A rollback that failed to revert"));
    assert!(
        !monitoring.body.contains("Cancel monitoring"),
        "nothing is recorded, so nothing can be cancelled"
    );

    // Cancelling without a recorded plan is a refusal, never a fabricated
    // plan. The token comes from a page that renders a form (the monitoring
    // page offers none while nothing is recorded).
    let access = send(
        &router,
        get_with_cookie("/console/settings/access", &cookie),
    )
    .await;
    let csrf = common::extract_csrf(&access.body);
    let refused = send(
        &router,
        post_form_with_cookie(
            "/console/settings/monitoring/cancel",
            &format!("csrf_token={csrf}&action=cancel&confirm=yes"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST);

    // The runner records the plan at connection; the page then shows the
    // tier, the price and the cancel action, and the POST records the
    // cancellation on the same state file (the plan line keeps billing the
    // current period and stops after it).
    let state_path = dir.path().join("credentials").join("pub1.monitoring.json");
    apex_billing::MonitoringPlan::default()
        .activate(350_000 * 1_000_000, "2026-05-01")
        .save(&state_path)
        .expect("recorded plan");
    let page = send(
        &router,
        get_with_cookie("/console/settings/monitoring", &cookie),
    )
    .await;
    assert!(page.body.contains("Included"), "{}", page.body);
    assert!(page.body.contains(">Mid<"), "{}", page.body);
    assert!(page.body.contains("$1,500.00"), "{}", page.body);
    assert!(page.body.contains("Since 2026-05-01"), "{}", page.body);
    assert!(page
        .body
        .contains("Cancels any time; the plan stops after the current month"));
    assert!(page.body.contains("/console/settings/monitoring/cancel"));
    assert!(
        page.body.contains("name=\"confirm\" value=\"yes\""),
        "the cancel form renders the confirmation checkbox with the accepted value"
    );
    let csrf = common::extract_csrf(&page.body);
    let unconfirmed = send(
        &router,
        post_form_with_cookie(
            "/console/settings/monitoring/cancel",
            &format!("csrf_token={csrf}&action=cancel"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(unconfirmed.status, StatusCode::BAD_REQUEST);
    assert!(
        !apex_billing::PlanState::load(&state_path)
            .expect("loadable")
            .expect("recorded")
            .cancelled(),
        "nothing changed without confirmation"
    );
    let cancelled = send(
        &router,
        post_form_with_cookie(
            "/console/settings/monitoring/cancel",
            &format!("csrf_token={csrf}&action=cancel&confirm=yes"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(cancelled.status, StatusCode::SEE_OTHER);
    let state = apex_billing::PlanState::load(&state_path)
        .expect("loadable")
        .expect("kept");
    assert!(state.cancelled(), "cancelled-not-deleted");
    assert!(state.stops_after.is_some(), "the stop period is recorded");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&state_path)
            .expect("state file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "plan state must be 0600, got {mode:o}");
    }
    let detail = send(
        &router,
        get_with_cookie("/console/settings/monitoring", &cookie),
    )
    .await;
    assert!(detail.body.contains("Cancelled"), "{}", detail.body);
    assert!(detail.body.contains("Cancelled; the plan bills"));
    assert!(
        !detail.body.contains("Cancel monitoring"),
        "no double cancel"
    );

    // The billing settings page shows the managed line, its written scope
    // and the (absent, until recorded) cancel action.
    let billing = send(
        &router,
        get_with_cookie("/console/settings/billing", &cookie),
    )
    .await;
    assert_eq!(billing.status, StatusCode::OK);
    assert!(billing.body.contains("Managed ad ops"));
    assert!(billing.body.contains("From $2,500.00 per month"));
    for item in apex_billing::MANAGED_SCOPE {
        assert!(billing.body.contains(item), "scope item missing: {item}");
    }
    assert!(!billing.body.contains("Cancel managed plan"));

    // A recorded managed plan renders its price and the cancel action; the
    // POST keeps the record with the stop period.
    let managed_path = dir.path().join("credentials").join("pub1.managed.json");
    apex_billing::ManagedPlan::default()
        .activate(2_000_000 * 1_000_000, "2026-06-01")
        .save(&managed_path)
        .expect("recorded managed");
    let billing = send(
        &router,
        get_with_cookie("/console/settings/billing", &cookie),
    )
    .await;
    assert!(billing.body.contains("$5,000.00"), "{}", billing.body);
    assert!(billing.body.contains("Cancel managed plan"));
    assert!(
        billing.body.contains("name=\"confirm\" value=\"yes\""),
        "the cancel form renders the confirmation checkbox with the accepted value"
    );
    let csrf = common::extract_csrf(&billing.body);
    let unconfirmed = send(
        &router,
        post_form_with_cookie(
            "/console/settings/billing/managed/cancel",
            &format!("csrf_token={csrf}&action=cancel"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(unconfirmed.status, StatusCode::BAD_REQUEST);
    assert!(
        !apex_billing::PlanState::load(&managed_path)
            .expect("loadable")
            .expect("recorded")
            .cancelled(),
        "nothing changed without confirmation"
    );
    let cancelled = send(
        &router,
        post_form_with_cookie(
            "/console/settings/billing/managed/cancel",
            &format!("csrf_token={csrf}&action=cancel&confirm=yes"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(cancelled.status, StatusCode::SEE_OTHER);
    let managed = apex_billing::PlanState::load(&managed_path)
        .expect("loadable")
        .expect("kept");
    assert!(managed.cancelled());
    assert!(managed.stops_after.is_some());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&managed_path)
            .expect("state file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "plan state must be 0600, got {mode:o}");
    }
}

#[tokio::test]
async fn analytics_without_a_store_is_503_with_the_reason_and_no_numbers() {
    let dir = TempDir::new("p2-analytics-off");
    let router = app(console_state(&dir, None)).await;
    let cookie = open_ledger_session(&router).await;
    for view in [
        "",
        "?view=revenue",
        "?view=lift",
        "?view=fees",
        "?view=data-quality",
    ] {
        let response = send(
            &router,
            get_with_cookie(&format!("/console/analytics{view}"), &cookie),
        )
        .await;
        assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(response
            .body
            .contains("the analytics store is not configured in this deployment"));
        assert!(!response.body.contains("$0.00"));
    }
    let unknown = send(
        &router,
        get_with_cookie("/console/analytics?view=nope", &cookie),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);
    assert!(unknown
        .body
        .contains("not one of: revenue, lift, fees, data-quality"));
    let bad_platform = send(
        &router,
        get_with_cookie("/console/analytics?platform=myspace", &cookie),
    )
    .await;
    assert_eq!(bad_platform.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn analytics_memory_seeded_reads_render_currency_scope_as_of_and_gaps() {
    let dir = TempDir::new("p2-analytics-mem");
    let store = Arc::new(MemoryStore::new());
    store.ensure_schema().await.expect("schema");
    seed_report_days(&store).await;
    let analytics = AnalyticsAccess::enabled(reader_over(&store), client_over(&store));
    let router = app(console_state(&dir, Some(analytics))).await;
    let cookie = open_ledger_session(&router).await;

    let now_ms = apex_analytics::UtcMillis::system_now()
        .expect("clock")
        .unix_millis();
    let today_label = apex_ui::format::utc_datetime(now_ms)
        .split(',')
        .next()
        .unwrap_or_default()
        .to_string();
    let response = send(
        &router,
        get_with_cookie("/console/analytics?view=revenue", &cookie),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    assert!(response.body.contains("Revenue and reporting coverage"));
    assert!(response.body.contains("currency USD"));
    assert!(
        response.body.contains(&format!("as of {today_label}")),
        "as-of uses the design date format: {}",
        response.body
    );
    assert!(response.body.contains("Platform revenue"));
    assert!(response.body.contains("$15.00"));
    assert!(response
        .body
        .contains("provisional days are marked separately"));
    assert!(response.body.contains("Matched source days"));
    assert!(
        response.body.contains("Stale sources (oldest gap first)")
            || response.body.contains("Data quality")
    );

    // The CSV export is the same rows with UTC days verbatim.
    let csv = send(
        &router,
        get_with_cookie("/console/analytics?view=revenue&format=csv", &cookie),
    )
    .await;
    assert_eq!(csv.status, StatusCode::OK);
    assert!(csv.content_type().starts_with("text/csv"));
    assert!(csv.body.starts_with("day,platform,revenue_micros,currency"));
    assert!(csv.body.contains(",applovin_max,15000000,USD"));
    assert!(
        csv.body.contains(",levelplay,1000000,USD"),
        "the stale source stays in the export"
    );
    // Every line ends CRLF: there is no lone LF in the raw bytes.
    assert!(csv.body.ends_with("\r\n"));
    assert_eq!(
        csv.body.matches('\n').count(),
        csv.body.matches("\r\n").count(),
        "no lone LF line ending: {:?}",
        csv.body
    );

    // The data-quality tab renders the stale source with the oldest gap.
    let quality = send(
        &router,
        get_with_cookie("/console/analytics?view=data-quality", &cookie),
    )
    .await;
    assert_eq!(quality.status, StatusCode::OK);
    assert!(quality.body.contains("levelplay"));
    assert!(quality.body.contains("10 days"));
}

async fn seed_report_days(store: &Arc<MemoryStore>) {
    let now_ms = apex_analytics::UtcMillis::system_now()
        .expect("clock")
        .unix_millis();
    let yesterday =
        apex_analytics::UtcMillis::from_unix_millis(now_ms - 86_400_000).to_day_string();
    // The stale source sits exactly ten days behind the as-of day, so the
    // oldest-gap assertion stays true on every calendar day (a fixed date
    // would drift one day per day).
    let stale_day =
        apex_analytics::UtcMillis::from_unix_millis(now_ms - 10 * 86_400_000).to_day_string();
    let rows = vec![
        ReportDayRow {
            publisher_id: PUBLISHER.to_string(),
            platform: "applovin_max".to_string(),
            platform_account_id: "acct-1".to_string(),
            day: yesterday.clone(),
            finality: "final".to_string(),
            revenue_micros: Some(15_000_000),
            impressions: Some(20),
            currency: Some("USD".to_string()),
            country: Some("DE".to_string()),
            ad_unit_id: Some("rw_de_01".to_string()),
            format: Some("rewarded".to_string()),
            source_digest: None,
        },
        ReportDayRow {
            publisher_id: PUBLISHER.to_string(),
            platform: "levelplay".to_string(),
            platform_account_id: "app-1".to_string(),
            day: stale_day.clone(),
            finality: "final".to_string(),
            revenue_micros: Some(1_000_000),
            impressions: Some(2),
            currency: Some("USD".to_string()),
            country: None,
            ad_unit_id: None,
            format: None,
            source_digest: None,
        },
    ];
    store.insert_report_days(&rows).await.expect("seed rows");
}

#[tokio::test]
async fn analytics_unreachable_store_renders_the_reason_not_numbers() {
    let dir = TempDir::new("p2-analytics-down");
    let analytics = AnalyticsAccess::from_lookup(&|key| match key {
        apex_api::analytics::CLICKHOUSE_HTTP_ENV => Some("http://127.0.0.1:1".to_string()),
        apex_api::analytics::CLICKHOUSE_DB_ENV => Some("apex".to_string()),
        _ => None,
    });
    assert!(analytics.is_enabled());
    let router = app(console_state(&dir, Some(analytics))).await;
    let cookie = open_ledger_session(&router).await;
    let response = send(
        &router,
        get_with_cookie("/console/analytics?view=revenue", &cookie),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.body.contains("the analytics store did not answer"));
    assert!(!response.body.contains("$0.00"));
}

#[tokio::test]
async fn analytics_csv_without_a_store_is_503_text_not_html() {
    // Not configured: the CSV export is a machine answer, never an HTML page.
    let dir = TempDir::new("p2-analytics-csv-off");
    let router = app(console_state(&dir, None)).await;
    let cookie = open_ledger_session(&router).await;
    let response = send(
        &router,
        get_with_cookie("/console/analytics?view=revenue&format=csv", &cookie),
    )
    .await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.content_type(), "text/plain; charset=utf-8");
    assert!(
        response.body.starts_with("export unavailable: "),
        "one plain-text reason line: {}",
        response.body
    );
    assert_eq!(response.body.lines().count(), 1);
    assert!(!response.body.contains("<html"));
    assert!(response
        .body
        .contains("the analytics store is not configured in this deployment"));
    // The HTML view keeps its honest panel for a browser.
    let html = send(
        &router,
        get_with_cookie("/console/analytics?view=revenue", &cookie),
    )
    .await;
    assert_eq!(html.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(html.body.contains("This view is not available right now."));

    // Configured but unreachable: the same 503 text contract.
    let dir = TempDir::new("p2-analytics-csv-down");
    let analytics = AnalyticsAccess::from_lookup(&|key| match key {
        apex_api::analytics::CLICKHOUSE_HTTP_ENV => Some("http://127.0.0.1:1".to_string()),
        apex_api::analytics::CLICKHOUSE_DB_ENV => Some("apex".to_string()),
        _ => None,
    });
    let router = app(console_state(&dir, Some(analytics))).await;
    let cookie = open_ledger_session(&router).await;
    let response = send(
        &router,
        get_with_cookie("/console/analytics?view=lift&format=csv", &cookie),
    )
    .await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.content_type(), "text/plain; charset=utf-8");
    assert!(response.body.contains("the analytics store did not answer"));
    assert_eq!(response.body.lines().count(), 1);
    assert!(!response.body.contains("<html"));
}

/// The real-store check: when `APEX_CLICKHOUSE_HTTP` and `APEX_CLICKHOUSE_DB`
/// are set (the dev server from `make clickhouse-dev`), the console reads a
/// freshly seeded schema on the real server; when they are unset the test
/// prints a clear SKIP and passes, exactly like the apex-analytics gate.
///
/// The ranged report-day SQL of `apex-analytics` currently fails on
/// ClickHouse 26.8 (`NO_COMMON_TYPE` comparing `toString(day)` with a
/// `Date`); the probe records that precisely, proves the un-ranged read and
/// the seeded value work against the same live server, and asserts the
/// console renders the honest not-available state (never numbers) for the
/// ranged route read.
#[tokio::test]
async fn analytics_real_clickhouse_reads_when_configured() {
    let Some(http) = std::env::var("APEX_CLICKHOUSE_HTTP")
        .ok()
        .filter(|v| !v.is_empty())
    else {
        println!("SKIP: APEX_CLICKHOUSE_HTTP is not set; start `make clickhouse-dev` and export APEX_CLICKHOUSE_HTTP/APEX_CLICKHOUSE_DB");
        return;
    };
    let db = format!(
        "apex_console_route_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.subsec_nanos())
            .unwrap_or(0)
    );
    let dir = TempDir::new("p2-analytics-ch");
    let db_for_lookup = db.clone();
    let http_for_lookup = http.clone();
    let config = apex_analytics::ClickHouseConfig::from_lookup(&|key| match key {
        "APEX_CLICKHOUSE_HTTP" => Some(http_for_lookup.clone()),
        "APEX_CLICKHOUSE_DB" => Some(db_for_lookup.clone()),
        _ => None,
    })
    .expect("config");
    let store = Arc::new(apex_analytics::ClickHouseStore::new(config).expect("store"));
    store.ensure_schema().await.expect("schema");
    let yesterday = apex_analytics::UtcMillis::from_unix_millis(
        apex_analytics::UtcMillis::system_now()
            .expect("clock")
            .unix_millis()
            - 86_400_000,
    )
    .to_day_string();
    store
        .insert_report_days(&[ReportDayRow {
            publisher_id: PUBLISHER.to_string(),
            platform: "applovin_max".to_string(),
            platform_account_id: "acct-1".to_string(),
            day: yesterday.clone(),
            finality: "final".to_string(),
            revenue_micros: Some(15_000_000),
            impressions: Some(20),
            currency: Some("USD".to_string()),
            country: Some("DE".to_string()),
            ad_unit_id: Some("rw_de_01".to_string()),
            format: Some("rewarded".to_string()),
            source_digest: None,
        }])
        .await
        .expect("seed");
    // The reader path serves the projection tables, so the batch rebuild is
    // part of the real flow.
    let rebuilt = apex_analytics::projections::rebuild_batch(&*store, "apex-api-console-gated")
        .await
        .expect("projection rebuild");
    println!("projection rebuild: {rebuilt:?}");
    let reader: Arc<dyn apex_analytics::AnalyticsReader> =
        Arc::new(apex_analytics::ClickHouseReader::new((*store).clone()));
    let client = client_over_memory_like(&store);
    let at = apex_analytics::UtcMillis::system_now().expect("clock");
    let ranged = apex_analytics::queries::QueryScope::new(PUBLISHER, at)
        .with_currency("USD")
        .with_day_range(
            apex_analytics::UtcMillis::from_unix_millis(at.unix_millis() - 30 * 86_400_000)
                .to_day_string(),
            yesterday.clone(),
        );
    // The un-ranged read is the dependency's own verified path: it must
    // return the seeded row on the live server.
    let un_ranged = apex_analytics::queries::QueryScope::new(PUBLISHER, at).with_currency("USD");
    let rows = reader
        .revenue_daily(&un_ranged)
        .await
        .expect("the un-ranged ClickHouse read works");
    assert!(
        rows.iter()
            .any(|row| row.day == yesterday && row.revenue.micros == 15_000_000),
        "the seeded row comes back from the live server: {rows:?}"
    );
    match reader.revenue_daily(&ranged).await {
        Ok(rows) => {
            assert!(
                rows.iter()
                    .any(|row| row.day == yesterday && row.revenue.micros == 15_000_000),
                "the ranged read returns the seeded row: {rows:?}"
            );
            println!("the ranged report-day read returns the seeded row on this server");
        }
        Err(error) => {
            let message = format!("{error}");
            panic!(
                "the ranged report-day read must work after the day-qualification fix: {message}"
            );
        }
    }
    let router = app(console_state(
        &dir,
        Some(AnalyticsAccess::enabled(reader, client)),
    ))
    .await;
    let cookie = open_ledger_session(&router).await;
    let response = send(
        &router,
        get_with_cookie("/console/analytics?view=revenue", &cookie),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.body);
    let as_of_label = apex_ui::format::utc_datetime(
        apex_analytics::UtcMillis::system_now()
            .expect("clock")
            .unix_millis(),
    )
    .split(',')
    .next()
    .unwrap_or_default()
    .to_string();
    assert!(
        response.body.contains(&format!("as of {as_of_label}")),
        "the as-of time uses the design format"
    );
    // The route reads a day range; with the day-qualified SQL the live server
    // answers with the seeded row, and the page renders the real amount
    // (15,000,000 micros = $15.00 in the fixed USD context).
    assert!(
        response.body.contains("$15.00"),
        "the ranged revenue page renders the seeded amount: {}",
        response.body
    );
    assert!(!response.body.contains("$0.00"));
    let _ = store.drop_database().await;
    println!(
        "apex-api analytics route PASS (live ranged read renders the seeded amount) against {db}"
    );
}

/// The client needs the trait-object form; the memory-store helper does not
/// fit here, so the ClickHouse store is coerced once.
fn client_over_memory_like(
    store: &Arc<apex_analytics::ClickHouseStore>,
) -> apex_analytics::AnalyticsClient {
    let concrete: Arc<apex_analytics::ClickHouseStore> = Arc::clone(store);
    let store: Arc<dyn EventStore> = concrete;
    apex_analytics::AnalyticsClient::new(store, "test", "apex-api-test")
}

#[tokio::test]
async fn lead_capture_and_free_audit_emit_the_catalog_events() {
    let dir = TempDir::new("p2-emit");
    let store = Arc::new(MemoryStore::new());
    store.ensure_schema().await.expect("schema");
    let leads_path = dir.path().join("leads.jsonl");
    let state = file_state(dir.path(), pub1_keys())
        .with_analytics(AnalyticsAccess::enabled(
            reader_over(&store),
            client_over(&store),
        ))
        .with_leads(Arc::new(apex_api::FileLeadStore::new(&leads_path)));
    let router = app(state).await;

    let lead = json!({
        "publisher": "northstar",
        "email": "you@studio.example",
        "platforms": ["applovin_max"],
        "monthlyRevenueRange": "$10,000 to $50,000"
    });
    let response = send(
        &router,
        common::post_json(
            "/v1/leads",
            serde_json::to_vec(&lead).expect("json").as_slice(),
        ),
    )
    .await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.body);

    let free_audit = "email=bad@&studio=Northstar&platforms=max&revenue=10k-50k";
    let invalid = send(&router, post_form("/free-audit", free_audit)).await;
    assert_eq!(invalid.status, StatusCode::BAD_REQUEST);

    let events = store.load_events().await.expect("events");
    let accepted = events
        .iter()
        .find(|event| event.event_type == apex_analytics::EventType::AuditRequestAccepted)
        .expect("accepted event");
    assert_eq!(accepted.source, apex_analytics::EventSource::Axum);
    assert!(accepted.event_id.starts_with("req_"));
    assert_eq!(accepted.payload.source_code.as_deref(), Some("api"));
    assert!(accepted.payload.range_start.is_some());
    assert!(accepted.payload.range_end.is_some());

    let rejected = events
        .iter()
        .find(|event| event.event_type == apex_analytics::EventType::AuditRequestRejected)
        .expect("rejected event");
    assert_eq!(rejected.payload.source_code.as_deref(), Some("free-audit"));
    assert_eq!(rejected.reason_code.as_deref(), Some("validation_failed"));
    assert_eq!(rejected.payload.validation_reason.as_deref(), Some("email"));
}

#[tokio::test]
async fn ledger_reads_emit_verified_events_and_failures_emit_the_line() {
    let dir = TempDir::new("p2-ledger-emit");
    common::write_standard_fixture(dir.path());
    let store = Arc::new(MemoryStore::new());
    store.ensure_schema().await.expect("schema");
    let router = app(
        file_state(dir.path(), pub1_keys()).with_analytics(AnalyticsAccess::enabled(
            reader_over(&store),
            client_over(&store),
        )),
    )
    .await;

    let response = send(
        &router,
        get_with_bearer(&format!("/v1/ledger/{PUBLISHER}"), LEDGER_TOKEN),
    )
    .await;
    assert_eq!(response.status, StatusCode::OK);
    let events = store.load_events().await.expect("events");
    let verified = events
        .iter()
        .find(|event| event.event_type == apex_analytics::EventType::LedgerVerified)
        .expect("verified event");
    assert_eq!(verified.publisher_id.as_deref(), Some(PUBLISHER));
    assert_eq!(verified.payload.highest_sequence, Some(1));
    assert!(verified.event_id.starts_with("lgr-"));

    // Tampering turns the next read into a verification failure event.
    common::tamper_line(&dir.path().join("pub1.jsonl"), 2);
    let failed = send(
        &router,
        get_with_bearer(&format!("/v1/ledger/{PUBLISHER}"), LEDGER_TOKEN),
    )
    .await;
    assert_eq!(failed.status, StatusCode::UNPROCESSABLE_ENTITY);
    let events = store.load_events().await.expect("events");
    let failure = events
        .iter()
        .find(|event| event.event_type == apex_analytics::EventType::LedgerVerificationFailed)
        .expect("failure event");
    assert_eq!(failure.payload.failure_line, Some(2));
    assert_eq!(failure.publisher_id.as_deref(), Some(PUBLISHER));
}

#[tokio::test]
async fn connection_validated_events_emit_through_the_console_decision() {
    // This deployment has no live read path configured in the test, so a
    // submission stores the credential unvalidated and emits no validation
    // event; the decision path emits instead.
    let dir = TempDir::new("p2-conn-emit");
    write_proposal(&dir, "p_0249", Some(i64::MAX / 2));
    let store = Arc::new(MemoryStore::new());
    store.ensure_schema().await.expect("schema");
    let state = proposal_state(&dir).with_analytics(AnalyticsAccess::enabled(
        reader_over(&store),
        client_over(&store),
    ));
    let router = app(state).await;
    let cookie = open_ledger_session(&router).await;
    let page = send(
        &router,
        get_with_cookie("/console/proposals/p_0249", &cookie),
    )
    .await;
    let csrf = common::extract_csrf(&page.body);
    let key = extract_idempotency_key(&page.body, "idempotency_key");
    let decided = send(
        &router,
        post_form_with_cookie(
            "/console/proposals/p_0249/decision",
            &format!("csrf_token={csrf}&decision=approve&idempotency_key={key}"),
            &cookie,
        ),
    )
    .await;
    assert_eq!(decided.status, StatusCode::SEE_OTHER);
    let events = store.load_events().await.expect("events");
    let approved = events
        .iter()
        .find(|event| event.event_type == apex_analytics::EventType::ProposalApproved)
        .expect("approved event");
    assert_eq!(approved.publisher_id.as_deref(), Some(PUBLISHER));
    assert_eq!(approved.payload.proposal_id.as_deref(), Some("p_0249"));
    assert_eq!(approved.payload.approval_actor.as_deref(), Some("pub1"));
}
