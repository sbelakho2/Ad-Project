//! Shared integration-test helpers: temp dirs, fixtures, request plumbing and
//! one pinned Adyen webhook HMAC vector.
//!
//! The Adyen vector (key `hmac_api_test`, the AUTHORISATION body below) was
//! computed independently with Python `hmac` + base64; tests use it as data
//! so the webhook path is exercised without a signing dependency.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use apex_api::{AppState, FileLedgerSource, JsonFileAuditSource};
use apex_core::PlatformId;
use apex_proof::{ChangeKind, Keypair, Ledger, LedgerRecord, VerifyingKey};
use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

/// The publisher materialised by [`write_pub1_ledger`].
pub const PUBLISHER: &str = "pub1";

/// Webhook HMAC key of the pinned vector.
pub const WEBHOOK_HMAC_KEY: &[u8] = b"hmac_api_test";
/// Exact raw body of the pinned webhook vector.
pub const WEBHOOK_BODY: &[u8] =
    br#"{"eventCode":"AUTHORISATION","pspReference":"9916934567890123","success":"true"}"#;
/// base64(HMAC-SHA256(key = `hmac_api_test`, body = WEBHOOK_BODY)).
pub const WEBHOOK_SIGNATURE: &str = "Ty6Oev1JYY5G0ANzwIBCb0AIQyaDsUWletqzDv5aOK4=";

/// The valid `Stripe-Signature` header for the pinned vector.
pub fn webhook_header() -> String {
    WEBHOOK_SIGNATURE.to_string()
}

/// Unique temporary directory removed on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let unique = format!(
            "apex-api-{tag}-{}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A captured response with the body already read as UTF-8 text.
pub struct TestResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: String,
}

impl TestResponse {
    pub fn content_type(&self) -> &str {
        self.headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|error| panic!("body is not JSON ({error}): {}", self.body))
    }
}

/// Drive the router with one request (infallible router, no sockets).
pub async fn send(app: &Router, request: Request<Body>) -> TestResponse {
    let response = app.clone().oneshot(request).await.expect("infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    TestResponse {
        status,
        headers,
        body: String::from_utf8(bytes.to_vec()).expect("utf-8 body"),
    }
}

pub fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

pub fn post(uri: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

pub fn post_json(uri: &str, body: &[u8]) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_vec()))
        .expect("request")
}

pub fn post_with_header(uri: &str, name: &str, value: &str, body: &[u8]) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(name, value)
        .body(Body::from(body.to_vec()))
        .expect("request")
}

/// Deterministic keypair from a seed byte.
pub fn seed_keypair(seed: u8) -> Keypair {
    Keypair::from_bytes(&[seed; 32])
}

/// The key allow-list for [`PUBLISHER`].
pub fn pub1_keys() -> HashMap<String, VerifyingKey> {
    HashMap::from([(PUBLISHER.to_string(), seed_keypair(7).verifying_key())])
}

/// File-backed state over `dir` (snapshots and ledgers share the directory in
/// tests) with the given key allow-list.
pub fn file_state(dir: &Path, keys: HashMap<String, VerifyingKey>) -> AppState {
    AppState::new(
        Arc::new(JsonFileAuditSource::new(dir)),
        Arc::new(FileLedgerSource::new(dir, keys)),
    )
}

/// A minimal snapshot for one or more ad units:
///
/// - each unit is enabled with an `EE` floor, earns 0 micros over 100_000
///   impressions there (a Critical stale-floor finding), and earns
///   10_000_000 micros over 100_000 impressions in `US` (the eCPM basis);
/// - platform and mediation totals agree, so no reporting-gap finding fires.
pub fn minimal_snapshot(unit_ids: &[&str]) -> Value {
    let mut ad_units = Vec::new();
    let mut country_revenue = Vec::new();
    for unit_id in unit_ids {
        ad_units.push(json!({
            "platformUnitId": unit_id,
            "name": format!("unit {unit_id}"),
            "format": "banner",
            "enabled": true,
            "countryFloors": [{"country": "EE", "floorEcpm": 2_500_000}]
        }));
        country_revenue.push(json!({
            "country": "EE",
            "adUnitId": unit_id,
            "revenue30d": 0,
            "impressions30d": 100_000
        }));
        country_revenue.push(json!({
            "country": "US",
            "adUnitId": unit_id,
            "revenue30d": 10_000_000,
            "impressions30d": 100_000
        }));
    }
    json!({
        "platform": "admob",
        "fetchedAtUnixMs": 1_700_000_000_000i64,
        "adUnits": ad_units,
        "waterfall": [],
        "mediationGroups": [],
        "bidderAccounts": [],
        "countryRevenue": country_revenue,
        "appAdsTxt": {"present": true, "lines": [], "expectedNetworks": []},
        "platformTotals30d": {"revenue30d": 10_000_000, "impressions30d": 100_000},
        "mediationTotals30d": {"revenue30d": 10_000_000, "impressions30d": 100_000}
    })
}

/// Write `<platform>.json` into `dir` and return the path.
pub fn write_snapshot(dir: &Path, platform: &str, snapshot: &Value) -> PathBuf {
    let path = dir.join(format!("{platform}.json"));
    let bytes = serde_json::to_vec_pretty(snapshot).expect("serialize snapshot");
    std::fs::write(&path, bytes).expect("write snapshot");
    path
}

/// One signed `changeApplied` record (usable with [`Ledger::append`]).
pub fn change_record(index: u64) -> LedgerRecord {
    LedgerRecord::change_applied(
        1_700_000_000_000 + index as i64,
        PlatformId::Admob,
        &format!("u{index}"),
        ChangeKind::CountryFloorSet {
            country: "EE".into(),
            floor_ecpm: 2_500_000,
        },
        &format!("admob-floor:u{index}:EE"),
        "0",
    )
}

/// Write and sign a ledger file at `<dir>/<publisher>.jsonl`.
pub fn write_ledger(
    dir: &Path,
    publisher: &str,
    keypair: &Keypair,
    records: Vec<LedgerRecord>,
) -> PathBuf {
    let path = dir.join(format!("{publisher}.jsonl"));
    let mut ledger = Ledger::open(&path, keypair.clone()).expect("open ledger");
    for record in records {
        ledger.append(record).expect("append record");
    }
    path
}

/// The standard fixture: `admob.json` with one unit and a two-record ledger for
/// [`PUBLISHER`], both in `dir`.
pub fn write_standard_fixture(dir: &Path) {
    write_snapshot(dir, "admob", &minimal_snapshot(&["u1"]));
    write_ledger(
        dir,
        PUBLISHER,
        &seed_keypair(7),
        vec![change_record(1), change_record(2)],
    );
}

/// Rewrite the JSON payload of one 1-based ledger line, leaving the signature
/// and chain untouched (i.e. producing a tampered line exactly as an attacker
/// with disk access would).
pub fn tamper_line(path: &Path, line_number: usize) {
    let raw = std::fs::read_to_string(path).expect("read ledger");
    let mut lines: Vec<String> = raw.lines().map(str::to_string).collect();
    let mut value: Value =
        serde_json::from_str(&lines[line_number - 1]).expect("parse ledger line");
    value["record"]["idempotencyKey"] = json!("tampered-by-test");
    lines[line_number - 1] = serde_json::to_string(&value).expect("serialize ledger line");
    std::fs::write(path, format!("{}\n", lines.join("\n"))).expect("write tampered ledger");
}

/// Run the audit engine over a snapshot fixture exactly as the server would.
pub fn report_for(snapshot: &Value) -> apex_audit::AuditReport {
    let parsed: apex_audit::Snapshot =
        serde_json::from_value(snapshot.clone()).expect("snapshot fixture");
    apex_audit::run(&parsed)
}
