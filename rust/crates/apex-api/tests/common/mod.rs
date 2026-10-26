//! Shared integration-test helpers: temp dirs, fixtures, request plumbing and
//! one pinned Mollie webhook HMAC vector.
//!
//! The Mollie vector (key `hmac_api_test`, the AUTHORISATION body below) was
//! computed independently with Python `hmac` + base64; tests use it as data
//! so the webhook path is exercised without a signing dependency.

#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
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

/// The second publisher the multi-session console tests materialise.
pub const PUBLISHER2: &str = "pub2";
/// The ledger token `/console/ledger` requires for [`PUBLISHER2`].
pub const LEDGER_TOKEN2: &str = "ledger-token-pub2";

/// The bearer token the `/v1/cycle` and `POST /v1/audit/{platform}` tests
/// configure (`APEX_LIVE_AUDIT_TOKEN` in the real binary).
pub const API_TOKEN: &str = "api-token-test";

/// The payment id the default webhook fixture names.
pub const MOLLIE_PAYMENT_ID: &str = "tr_api_test";

/// A fetched Mollie payment in the shape the webhook route validates.
pub fn mollie_payment(reference: &str, amount_minor_units: i64, status: &str) -> Value {
    let amount = format!(
        "{}.{:02}",
        amount_minor_units / 100,
        amount_minor_units % 100
    );
    json!({
        "resource": "payment",
        "id": MOLLIE_PAYMENT_ID,
        "status": status,
        "amount": {"currency": "USD", "value": amount},
        "metadata": {
            "reference": reference,
            "publisher": "publisher-pub1",
            "amount": amount
        },
        "customerId": "cst_api_test",
        "mandateId": "mdt_api_test",
        "sequenceType": "first"
    })
}

/// The payments the test transport can fetch, keyed by payment id. A webhook
/// carries only an id; this map is the "provider" side the route fetches from.
pub fn payments() -> &'static std::sync::Mutex<HashMap<String, Value>> {
    static PAYMENTS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Value>>> =
        std::sync::OnceLock::new();
    PAYMENTS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

pub fn register_payment(payment: Value) {
    let id = payment["id"].as_str().expect("payment id").to_string();
    payments().lock().expect("payments").insert(id, payment);
}

/// A form-encoded Mollie webhook body naming one payment.
pub fn webhook_body() -> Vec<u8> {
    let payment = mollie_payment("apex-fee:pub1:exp:2026-10", 1_000, "paid");
    register_payment(payment);
    webhook_body_with(MOLLIE_PAYMENT_ID)
}

pub fn webhook_body_with(payment_id: &str) -> Vec<u8> {
    format!("id={payment_id}").into_bytes()
}

/// The default fetched payment for a webhook naming `MOLLIE_PAYMENT_ID`.
pub fn default_payment() -> Value {
    mollie_payment("apex-fee:pub1:exp:2026-10", 1_000, "paid")
}

/// One transport that serves the Mollie endpoints the API calls in tests:
/// customer creation, first/recurring payments, payment links, and the
/// server-side webhook fetch. It reads [`payments`] so a test can register a
/// different amount or status for the same id.
#[derive(Debug, Default)]
pub struct MollieTransport;

fn json_response(status: u16, value: Value) -> apex_connectors::transport::HttpResponse {
    apex_connectors::transport::HttpResponse {
        status,
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        body: serde_json::to_vec(&value).expect("json"),
    }
}

/// Handle a Mollie API request, or return `None` when the path is not one.
pub fn mollie_api(
    request: &apex_connectors::transport::HttpRequest,
) -> Option<apex_connectors::transport::HttpResponse> {
    let path = request
        .url
        .split("api.mollie.test")
        .nth(1)
        .unwrap_or(request.url.as_str());
    if request.method == apex_connectors::transport::HttpMethod::Get {
        if let Some(id) = path.strip_prefix("/v2/payments/") {
            let payments = payments().lock().expect("payments");
            return Some(match payments.get(id) {
                Some(payment) => json_response(200, payment.clone()),
                None => json_response(404, json!({"status": 404, "title": "Not Found"})),
            });
        }
    }
    if request.method != apex_connectors::transport::HttpMethod::Post {
        return None;
    }
    let body = request
        .body
        .as_ref()
        .and_then(|body| match body {
            apex_connectors::transport::HttpBody::Json(value) => Some(value.clone()),
            _ => None,
        })
        .unwrap_or_else(|| json!({}));
    if path == "/v2/customers" {
        return Some(json_response(
            201,
            json!({"resource": "customer", "id": "cst_api_test"}),
        ));
    }
    if path == "/v2/payment-links" {
        return Some(json_response(
            201,
            json!({
                "resource": "payment-link",
                "id": "pl_api_test",
                "_links": {"paymentLink": {"href": "https://checkout.example/pl_api_test"}}
            }),
        ));
    }
    if path == "/v2/payments" {
        let amount = body
            .get("amount")
            .cloned()
            .unwrap_or_else(|| json!({"currency": "USD", "value": "0.00"}));
        let metadata = body.get("metadata").cloned().unwrap_or_else(|| json!({}));
        let sequence = body
            .get("sequenceType")
            .and_then(|item| item.as_str())
            .unwrap_or("oneoff");
        if sequence == "first" {
            let payment = json!({
                "resource": "payment",
                "id": "tr_first_api_test",
                "status": "open",
                "amount": amount,
                "metadata": metadata,
                "customerId": body.get("customerId").cloned().unwrap_or(Value::Null),
                "sequenceType": "first",
                "_links": {"checkout": {"href": "https://checkout.example/tr_first_api_test"}}
            });
            register_payment(payment.clone());
            return Some(json_response(201, payment));
        }
        let payment = json!({
            "resource": "payment",
            "id": "tr_recurring_api_test",
            "status": "paid",
            "amount": amount,
            "metadata": metadata,
            "customerId": body.get("customerId").cloned().unwrap_or(Value::Null),
            "mandateId": body.get("mandateId").cloned().unwrap_or(Value::Null),
            "sequenceType": "recurring"
        });
        register_payment(payment.clone());
        return Some(json_response(201, payment));
    }
    None
}

#[async_trait::async_trait]
impl apex_connectors::transport::Transport for MollieTransport {
    async fn request(
        &self,
        request: apex_connectors::transport::HttpRequest,
    ) -> Result<apex_connectors::transport::HttpResponse, apex_connectors::transport::TransportError>
    {
        mollie_api(&request).ok_or_else(|| {
            apex_connectors::transport::TransportError::Network(format!(
                "MollieTransport has no answer for {}",
                request.url
            ))
        })
    }
}

/// A test AppState method: the state the binary builds when the Mollie
/// variables are configured, over [`MollieTransport`].
pub trait MollieTestState {
    fn with_mollie_test(self) -> Self;
    fn with_mollie_payment(self, payment: Value) -> Self;
}

impl MollieTestState for AppState {
    fn with_mollie_test(self) -> Self {
        let client = apex_billing::MollieClient::new(
            "test_key",
            "https://api.mollie.test",
            "https://apex.test/v1/mollie/webhook",
            "https://apex.test/watch/return",
        )
        .expect("mollie client");
        self.with_mollie(Arc::new(client))
            .with_billing_transport(Arc::new(MollieTransport))
    }

    fn with_mollie_payment(self, payment: Value) -> Self {
        register_payment(payment);
        self.with_mollie_test()
    }
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

/// A GET carrying `Authorization: Bearer <token>`.
pub fn get_with_bearer(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
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

/// A POST carrying an `application/x-www-form-urlencoded` byte body.
pub fn post_form_bytes(uri: &str, body: &[u8]) -> Request<Body> {
    post_with_header(
        uri,
        "content-type",
        "application/x-www-form-urlencoded",
        body,
    )
}

/// A POST carrying an `application/x-www-form-urlencoded` body.
pub fn post_form(uri: &str, body: &str) -> Request<Body> {
    post_with_header(
        uri,
        "content-type",
        "application/x-www-form-urlencoded",
        body.as_bytes(),
    )
}

/// A POST carrying `Authorization: Bearer <token>`.
pub fn post_with_bearer(uri: &str, token: &str, body: &[u8]) -> Request<Body> {
    post_with_header(uri, "authorization", &format!("Bearer {token}"), body)
}

/// A JSON POST carrying `Authorization: Bearer <token>`.
pub fn post_json_with_bearer(uri: &str, token: &str, body: &[u8]) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_vec()))
        .expect("request")
}

/// A GET carrying one `Cookie` header.
pub fn get_with_cookie(uri: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .expect("request")
}

/// A form POST carrying one `Cookie` header.
pub fn post_form_with_cookie(uri: &str, body: &str, cookie: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::COOKIE, cookie)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body.to_string()))
        .expect("request")
}

/// The first `Set-Cookie` value of a response, if any.
pub fn set_cookie(response: &TestResponse) -> Option<String> {
    response
        .headers
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// Exchange [`LEDGER_TOKEN`] for a session and return the cookie pair
/// (`name=value`) to send on later requests.
pub async fn open_ledger_session(app: &Router) -> String {
    let response = send(
        app,
        post_form(
            "/console/ledger",
            &format!("publisher={PUBLISHER}&token={LEDGER_TOKEN}"),
        ),
    )
    .await;
    assert_eq!(
        response.status,
        StatusCode::SEE_OTHER,
        "token exchange: {}",
        response.body
    );
    let cookie = set_cookie(&response).expect("the exchange sets a cookie");
    cookie
        .split(';')
        .next()
        .expect("cookie pair before the first attribute")
        .to_string()
}

/// The CSRF token of the session-end form rendered on a ledger page.
pub fn extract_csrf(html: &str) -> String {
    const ANCHOR: &str = "name=\"csrf_token\" value=\"";
    let start = html.find(ANCHOR).expect("a csrf hidden input") + ANCHOR.len();
    let end = start + html[start..].find('"').expect("the closing quote");
    html[start..end].to_string()
}

/// Deterministic keypair from a seed byte.
pub fn seed_keypair(seed: u8) -> Keypair {
    Keypair::from_bytes(&[seed; 32])
}

/// The key allow-list for [`PUBLISHER`].
pub fn pub1_keys() -> HashMap<String, VerifyingKey> {
    HashMap::from([(PUBLISHER.to_string(), seed_keypair(7).verifying_key())])
}

/// The key allow-list for both test publishers.
pub fn two_publisher_keys() -> HashMap<String, VerifyingKey> {
    HashMap::from([
        (PUBLISHER.to_string(), seed_keypair(7).verifying_key()),
        (PUBLISHER2.to_string(), seed_keypair(9).verifying_key()),
    ])
}

/// A live auditor over an empty account store and a transport that refuses
/// every call: the constructor needs concrete plumbing, but the bearer-token
/// gate on `/v1/cycle` and `POST /v1/audit/{platform}` never reaches it.
pub fn token_auditor(token: &str) -> Arc<apex_api::LiveAuditor> {
    #[derive(Debug)]
    struct RefusingTransport;

    #[async_trait::async_trait]
    impl apex_connectors::transport::Transport for RefusingTransport {
        async fn request(
            &self,
            _request: apex_connectors::transport::HttpRequest,
        ) -> Result<
            apex_connectors::transport::HttpResponse,
            apex_connectors::transport::TransportError,
        > {
            Err(apex_connectors::transport::TransportError::Network(
                "the token-only test transport refuses every call".to_string(),
            ))
        }
    }

    #[derive(Debug)]
    struct NoAccounts;

    #[async_trait::async_trait]
    impl apex_api::LiveAccountStore for NoAccounts {
        async fn account(
            &self,
            _publisher: &str,
        ) -> Result<apex_api::LiveAccount, apex_api::ApiError> {
            Err(apex_api::ApiError::NotFound)
        }
    }

    Arc::new(
        apex_api::LiveAuditor::new(
            Arc::new(NoAccounts),
            Arc::new(RefusingTransport),
            Arc::new(apex_api::FixedClock::new(1_700_000_000)),
        )
        .with_token(token),
    )
}

/// A DNS resolver that answers one fixed public address, so the guarded
/// app-ads proof can fetch in tests without touching the real resolver.
#[derive(Debug)]
pub struct PublicResolver;

#[async_trait::async_trait]
impl apex_api::app_ads_proof::DomainResolver for PublicResolver {
    async fn resolve(&self, _domain: &str) -> Result<Vec<std::net::IpAddr>, String> {
        Ok(vec!["93.184.216.34"
            .parse()
            .expect("a public test address")])
    }
}

/// A transport that counts every request and answers `body` with `200`, used
/// to prove how many guarded app-ads fetches a request performs.
#[derive(Debug)]
pub struct CountingAppAdsTransport {
    calls: AtomicUsize,
    body: String,
}

impl CountingAppAdsTransport {
    pub fn new(body: impl Into<String>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            body: body.into(),
        }
    }

    /// How many requests this transport has answered.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl apex_api::app_ads_proof::PinnedTransport for CountingAppAdsTransport {
    async fn request(
        &self,
        _domain: &str,
        _addresses: &[std::net::SocketAddr],
        request: apex_connectors::transport::HttpRequest,
    ) -> Result<apex_connectors::transport::HttpResponse, apex_connectors::transport::TransportError>
    {
        assert!(
            request.url.ends_with("/app-ads.txt"),
            "the proof fetches app-ads.txt, not {}",
            request.url
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(apex_connectors::transport::HttpResponse {
            status: 200,
            headers: vec![],
            body: self.body.as_bytes().to_vec(),
        })
    }
}

/// The ledger token `/v1/ledger/{PUBLISHER}` and `/console/ledger` require.
pub const LEDGER_TOKEN: &str = "ledger-token-pub1";

/// File-backed state over `dir` (snapshots and ledgers share the directory in
/// tests) with the given key allow-list and the publisher's ledger token.
pub fn file_state(dir: &Path, keys: HashMap<String, VerifyingKey>) -> AppState {
    AppState::new(
        Arc::new(JsonFileAuditSource::new(dir)),
        Arc::new(FileLedgerSource::new(dir, keys)),
    )
    .with_ledger_token(PUBLISHER, LEDGER_TOKEN)
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

/// Write `<dir>/<publisher>/<platform>.json` and return the path.
pub fn write_snapshot_for(
    dir: &Path,
    publisher: &str,
    platform: &str,
    snapshot: &Value,
) -> PathBuf {
    let path = dir.join(publisher).join(format!("{platform}.json"));
    std::fs::create_dir_all(path.parent().expect("snapshot parent")).expect("snapshot directory");
    let bytes = serde_json::to_vec_pretty(snapshot).expect("serialize snapshot");
    std::fs::write(&path, bytes).expect("write snapshot");
    path
}

/// Write [`PUBLISHER`]'s `<platform>.json` into the publisher-scoped layout
/// (`<dir>/<publisher>/<platform>.json`) and return the path.
pub fn write_snapshot(dir: &Path, platform: &str, snapshot: &Value) -> PathBuf {
    write_snapshot_for(dir, PUBLISHER, platform, snapshot)
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
