#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests over the real router: sign-in, the capability matrix,
//! CSRF, idempotency, stale state, the journal and job formats, isolation,
//! the unconfigured deployment and the fixture sweep.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use apex_control::capabilities::{Capability, Role};
use apex_control::config::ControlConfig;
use apex_control::data::{ControlData, DurableReader, LiveData, OperationsEvidence};
use apex_control::fixture_render::{self, FIXTURE_PASSWORD, FIXTURE_TOTP_SECRET};
use apex_control::pages;
use apex_control::routes::router;
use apex_control::session::{self, SessionView};
use apex_control::state::{Clock, ControlState, StateError};
use apex_proof::{Keypair, Ledger, LedgerRecord};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use tower::ServiceExt;

/// A clock the tests can advance (step-up and expiry need time to move).
#[derive(Debug, Clone)]
struct TestClock(Arc<AtomicI64>);

impl TestClock {
    fn new(now: i64) -> Self {
        Self(Arc::new(AtomicI64::new(now)))
    }

    fn set(&self, now: i64) {
        self.0.store(now, Ordering::SeqCst);
    }

    fn advance(&self, delta_ms: i64) {
        self.set(self.0.load(Ordering::SeqCst) + delta_ms);
    }
}

impl Clock for TestClock {
    fn now_unix_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

struct Harness {
    state: ControlState,
    clock: TestClock,
    _work_dir: std::path::PathBuf,
}

fn harness() -> Harness {
    let state = fixture_render::fixture_state().expect("fixture state");
    let work_dir = state
        .jobs
        .dir()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    let clock = TestClock::new(apex_control::fixtures::DEMO_NOW_MS);
    let state = state.with_clock(Arc::new(clock.clone()));
    Harness {
        state,
        clock,
        _work_dir: work_dir,
    }
}

fn app(harness: &Harness) -> Router {
    router(harness.state.clone())
}

/// One live session with its cookie, so a form's CSRF always matches the
/// cookie that submits it.
struct Signed {
    view: SessionView,
    cookie: String,
}

fn open_session(state: &ControlState, operator_id: &str, role: Role) -> Signed {
    let opened = state.sessions.open(
        operator_id,
        operator_id,
        role,
        "test device",
        state.now_ms(),
    );
    let view = state
        .sessions
        .touch(&opened.id, state.now_ms())
        .expect("live session");
    Signed {
        view,
        cookie: format!("apex_operator_session={}", opened.id),
    }
}

async fn get(app: &Router, uri: &str, cookie: Option<&str>) -> (StatusCode, String) {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&body).to_string())
}

async fn post(
    app: &Router,
    uri: &str,
    cookie: Option<&str>,
    form: &str,
) -> (StatusCode, Option<String>, Option<String>, String) {
    post_with_headers(app, uri, cookie, form, &[]).await
}

async fn post_with_headers(
    app: &Router,
    uri: &str,
    cookie: Option<&str>,
    form: &str,
    extra: &[(&str, &str)],
) -> (StatusCode, Option<String>, Option<String>, String) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    for (name, value) in extra {
        builder = builder.header(*name, *value);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(form.to_string())).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let cookie_header = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    (
        status,
        location,
        cookie_header,
        String::from_utf8_lossy(&body).to_string(),
    )
}

fn hidden_value(html: &str, name: &str) -> String {
    let needle = format!("name=\"{name}\" value=\"");
    let start = html.find(&needle).expect("hidden field") + needle.len();
    let rest = &html[start..];
    let end = rest.find('"').expect("value end");
    rest[..end].to_string()
}

/// The hidden fields of the first form whose action posts `kind`.
fn form_fields_for(html: &str, kind: &str) -> String {
    let marker = format!("action=\"/control/actions/{kind}\"");
    let start = html.find(&marker).expect("form action");
    let rest = &html[start..];
    let end = rest.find("</form>").expect("form end");
    rest[..end].to_string()
}

fn fixture_code(state: &ControlState) -> String {
    let secret = apex_control::base32::decode(FIXTURE_TOTP_SECRET).expect("secret");
    apex_control::totp::code_at(&secret, state.now_ms() / 1000)
}

fn session_cookie_value(set_cookie: &str) -> String {
    set_cookie
        .split(';')
        .next()
        .expect("cookie pair")
        .to_string()
}

/// Complete the full two-factor sign-in and return the session cookie.
async fn sign_in(app: &Router, harness: &Harness, operator: &str) -> String {
    let code = fixture_code(&harness.state);
    let (status, _, _, html) = post(
        app,
        "/control/login",
        None,
        &format!("operator={operator}&password={FIXTURE_PASSWORD}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{html}");
    let pending = hidden_value(&html, "pending");
    let (status, _, cookie, html) = post(
        app,
        "/control/login/code",
        None,
        &format!("pending={pending}&code={code}"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{html}");
    session_cookie_value(&cookie.expect("session cookie"))
}

fn enc(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Build a valid action form for `capability` with a fresh idempotency key.
async fn form_for(
    state: &ControlState,
    signed: &Signed,
    capability: Capability,
    target: &str,
    key: &str,
) -> String {
    let policy = serde_json::to_string(&apex_autopilot::model::AutopilotPolicy::conservative())
        .expect("policy json");
    form_with(state, signed, capability, target, key, &policy, "0", "").await
}

/// Build an action form with explicit policy/revenue/tier values.
#[allow(clippy::too_many_arguments)]
async fn form_with(
    state: &ControlState,
    signed: &Signed,
    capability: Capability,
    target: &str,
    key: &str,
    policy_json: &str,
    revenue_30d_micros: &str,
    tier: &str,
) -> String {
    let spec = apex_control::routes::build_form_spec(
        state,
        &signed.view,
        capability,
        target,
        "/control/",
        &[],
    )
    .await
    .unwrap_or_else(|| panic!("form spec missing for {capability:?} target {target}"));
    let confirmation = capability.typed_confirmation().unwrap_or("");
    format!(
        "csrf_token={}&idempotency_key={}&version_token={}&target={}&return_path={}&confirmation={}&reason={}&evidence={}&source_document={}&policy_json={}&revenue_30d_micros={}&tier={}",
        enc(&spec.csrf),
        enc(key),
        enc(&spec.version_token),
        enc(&spec.target),
        enc(&spec.return_path),
        enc(confirmation),
        enc("integration reason"),
        enc("confirming read"),
        enc("doc-1"),
        enc(policy_json),
        enc(revenue_30d_micros),
        enc(tier),
    )
}

/// The target used for one capability in the matrix.
async fn target_for(capability: Capability, state: &ControlState) -> String {
    match capability {
        Capability::IncidentAcknowledge | Capability::IncidentResolve | Capability::OwnerAssign => {
            "inc_018".to_string()
        }
        Capability::RetryRead => "northstar:applovin_max:acct_northstar_max_01".to_string(),
        Capability::RequestReconnection => "northstar".to_string(),
        Capability::RestoreReconcileRequest => "exp_0246".to_string(),
        Capability::RecheckRequest => "ch_0246".to_string(),
        Capability::JobRetry => {
            let job = apex_control::jobs::OperatorJob::queued(
                apex_control::jobs::JobKind::RecheckRequest,
                "maya",
                state.now_ms(),
                "ch_0246",
                serde_json::json!({}),
                "0123456789abcdef0123456789abcdef",
                "corr-job",
            );
            // The write is idempotent by key: repeated calls resolve to the
            // one durable job, whose id is the target.
            state
                .jobs
                .write(&job)
                .await
                .expect("job")
                .job()
                .job_id
                .clone()
        }
        Capability::FleetPauseScheduling | Capability::FleetResumeScheduling => {
            "fleet:scheduling".to_string()
        }
        Capability::InvoiceHold
        | Capability::InvoiceRelease
        | Capability::CollectionRetry
        | Capability::DisputeMark
        | Capability::ManualReconcile => "INV-2026-10-northstar".to_string(),
        Capability::SessionRevoke => {
            let opened = state
                .sessions
                .open("sam", "Sam", Role::Finance, "device", state.now_ms());
            opened.id[..8].to_string()
        }
        Capability::PolicyDraft => "policy:draft".to_string(),
        Capability::PolicyActivate => {
            let record = apex_control::governance::PolicyRecord::draft(
                "maya",
                state.now_ms(),
                serde_json::json!({}),
                serde_json::json!({}),
                serde_json::json!({}),
                "future proposals",
                1,
                "corr-matrix-policy",
            );
            let policy_id = record.policy_id.clone();
            state.governance.append_policy(&record).expect("policy");
            policy_id
        }
        Capability::PolicySet => "policy:effective".to_string(),
        Capability::ManagedPlanActivate => "northstar".to_string(),
        Capability::OperatorActionReview => {
            let read = state.journal.read().expect("journal");
            let entry = read.entries.first().expect("fixture journal entry");
            apex_control::actions::action_target_key(entry)
        }
    }
}

fn ledger_key() -> Keypair {
    Keypair::from_bytes(&[9u8; 32])
}

fn write_signed_ledger(path: &std::path::Path, key: &Keypair, records: Vec<LedgerRecord>) {
    let mut ledger = Ledger::open(path, key.clone()).expect("open ledger");
    for record in records {
        ledger.append(record).expect("append record");
    }
}

fn write_snapshot(
    root: &std::path::Path,
    publisher: &str,
    platform: &str,
    fetched_at_unix_ms: i64,
    ad_units: usize,
) {
    let dir = root.join(publisher);
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let units: Vec<serde_json::Value> = (0..ad_units)
        .map(|index| serde_json::json!({"platformUnitId": format!("u{index}")}))
        .collect();
    std::fs::write(
        dir.join(format!("{platform}.json")),
        serde_json::to_vec(&serde_json::json!({
            "fetchedAtUnixMs": fetched_at_unix_ms,
            "adUnits": units,
        }))
        .expect("snapshot json"),
    )
    .expect("snapshot write");
}

#[allow(clippy::too_many_arguments)]
fn live_state(
    work: &std::path::Path,
    keys: std::collections::HashMap<String, apex_proof::VerifyingKey>,
    configured: bool,
    notify_sink: Option<&str>,
    metrics_textfile: Option<std::path::PathBuf>,
    create_dirs: bool,
) -> ControlState {
    if create_dirs {
        for path in [
            work.join("jobs"),
            work.join("ledgers"),
            work.join("snapshots"),
        ] {
            std::fs::create_dir_all(path).expect("state dir");
        }
    }
    let config = ControlConfig {
        bind: "127.0.0.1:0".parse().expect("addr"),
        operators_path: None,
        journal_path: work.join("journal.jsonl"),
        jobs_dir: work.join("jobs"),
        jobs_remote: None,
        sync_marker: None,
        environment: "TEST".to_string(),
        ledger_root: work.join("ledgers"),
        ledger_keys: keys.clone(),
        snapshot_root: work.join("snapshots"),
        notify_sink: notify_sink.map(str::to_string),
        metrics_textfile,
        invoice_register: work.join("invoices.jsonl"),
        totp_state_path: work.join("totp-steps.json"),
        operations_path: work.join("operations.json"),
        policies_path: work.join("policies.jsonl"),
        interventions_path: work.join("interventions.jsonl"),
        effective_policy_path: work.join("effective-policy.json"),
        connections_root: work.join("connections"),
        clickhouse: configured.then(apex_analytics::ClickHouseConfig::default),
    };
    let durable = DurableReader::new(work.join("ledgers"), work.join("invoices.jsonl"))
        .with_ledger_keys(keys)
        .with_snapshot_root(work.join("snapshots"));
    let data = ControlData::Live(Box::new(LiveData {
        analytics: None,
        durable,
        operations: Ok(OperationsEvidence::default()),
        sync_marker: None,
    }));
    ControlState::new(config, data)
        .expect("live state")
        .with_clock(Arc::new(apex_control::state::FixedClock::new(
            apex_control::fixtures::DEMO_NOW_MS,
        )))
        .with_operators(fixture_render::fixture_operators().expect("operators"))
}

// ---------------------------------------------------------------------------
// Sign-in
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sign_in_happy_path_requires_both_factors() {
    let harness = harness();
    let app = app(&harness);
    // The route module's operator-identity lookup resolves configured ids.
    assert_eq!(
        apex_control::routes::role_of(&harness.state, "maya"),
        Some(Role::Operator)
    );
    assert_eq!(
        apex_control::routes::role_of(&harness.state, "nobody"),
        None
    );
    let cookie = sign_in(&app, &harness, "maya").await;
    assert!(cookie.starts_with("apex_operator_session="));
    let (status, html) = get(&app, "/control/", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("What needs attention"));
    assert!(html.contains("Maya"));
    assert!(html.contains("(Operator)"));
    assert!(html.contains("OPERATOR PLANE"));
}

#[tokio::test]
async fn wrong_password_and_wrong_code_use_the_frozen_copy() {
    let harness = harness();
    let app = app(&harness);
    let (status, _, _, html) = post(
        &app,
        "/control/login",
        None,
        "operator=maya&password=wrong-password",
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(html.contains("We could not verify those access details."));
    assert!(html.contains("value=\"maya\""));
    assert!(!html.contains("wrong-password"));
    let (status, _, _, html) = post(
        &app,
        "/control/login",
        None,
        &format!("operator=maya&password={FIXTURE_PASSWORD}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let pending = hidden_value(&html, "pending");
    let (status, _, _, html) = post(
        &app,
        "/control/login/code",
        None,
        &format!("pending={pending}&code=000000"),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(html.contains("We could not verify those access details."));
    let code = fixture_code(&harness.state);
    let (status, _, cookie, _) = post(
        &app,
        "/control/login/code",
        None,
        &format!("pending={pending}&code={code}"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(cookie.is_some());
}

#[tokio::test]
async fn login_posts_reject_cross_origin_and_accept_same_origin() {
    let harness = harness();
    let app = app(&harness);
    let form = format!("operator=maya&password={FIXTURE_PASSWORD}");
    let same_origin = [
        ("host", "control.example"),
        ("origin", "https://control.example"),
        ("x-forwarded-proto", "https"),
    ];
    let cross_origin = [
        ("host", "control.example"),
        ("origin", "https://evil.example"),
        ("x-forwarded-proto", "https"),
    ];

    // The password step never mints a session on a cross-origin request.
    let (status, _, cookie, html) =
        post_with_headers(&app, "/control/login", None, &form, &cross_origin).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{html}");
    assert!(cookie.is_none(), "a refused origin mints no cookie");

    // The browser's own cross-site hint is refused even without an Origin.
    let (status, _, cookie, _) = post_with_headers(
        &app,
        "/control/login",
        None,
        &form,
        &[
            ("host", "control.example"),
            ("sec-fetch-site", "cross-site"),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(cookie.is_none());

    // Same origin reaches the second factor.
    let (status, _, cookie, html) =
        post_with_headers(&app, "/control/login", None, &form, &same_origin).await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(cookie.is_none(), "the password step sets no session cookie");
    let pending = hidden_value(&html, "pending");

    let code = fixture_code(&harness.state);
    let code_form = format!("pending={pending}&code={code}");
    let (status, _, cookie, html) =
        post_with_headers(&app, "/control/login/code", None, &code_form, &cross_origin).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{html}");
    assert!(cookie.is_none(), "a refused origin mints no cookie");

    // The pending step was not consumed by the refused request.
    let (status, _, cookie, html) =
        post_with_headers(&app, "/control/login/code", None, &code_form, &same_origin).await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{html}");
    assert!(cookie.is_some(), "the verified step mints the session");
}

#[tokio::test]
async fn rate_limit_and_lockout_after_repeated_failures() {
    let harness = harness();
    let app = app(&harness);
    for _ in 0..5 {
        let (status, _, _, _) =
            post(&app, "/control/login", None, "operator=maya&password=wrong").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, _, _, html) =
        post(&app, "/control/login", None, "operator=maya&password=wrong").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(html.contains("Too many attempts."));
    assert!(html.contains("Wait for the lockout to end."));
    let (status, _, _, _) = post(
        &app,
        "/control/login",
        None,
        &format!("operator=maya&password={FIXTURE_PASSWORD}"),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn session_expiry_returns_the_safe_recovery_page() {
    let harness = harness();
    let app = app(&harness);
    let cookie = sign_in(&app, &harness, "maya").await;
    harness.clock.advance(31 * 60 * 1000);
    let (status, html) = get(&app, "/control/", Some(&cookie)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(html.contains("Your session ended."));
    assert!(html.contains("Sign in again"));
}

#[tokio::test]
async fn step_up_requires_a_fresh_sign_in_before_the_action() {
    let harness = harness();
    let app = app(&harness);
    let cookie = sign_in(&app, &harness, "maya").await;
    harness.clock.advance(11 * 60 * 1000);
    // The incident page carries the resolve form for an Operator.
    let (status, html) = get(&app, "/control/incidents/inc_018", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    let form = form_fields_for(&html, "incident_resolve");
    let request = format!(
        "csrf_token={}&idempotency_key={}&version_token={}&target={}&return_path={}&reason=x&evidence=y",
        hidden_value(&form, "csrf_token"),
        hidden_value(&form, "idempotency_key"),
        hidden_value(&form, "version_token"),
        hidden_value(&form, "target"),
        hidden_value(&form, "return_path"),
    );
    let (status, location, _, _) = post(
        &app,
        "/control/actions/incident_resolve",
        Some(&cookie),
        &request,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = location.expect("redirect");
    assert!(location.starts_with("/control/login?reauth=1&next="));
    // The requirement was journaled, not executed.
    let read = harness.state.journal.read().expect("journal");
    assert!(
        read.entries
            .iter()
            .any(|entry| entry.outcome == "step_up_required"
                && entry.capability == "incident_resolve")
    );
    // Re-authenticate with the same cookie; the code step refreshes the
    // session instead of minting a second one.
    let next = location.split("next=").nth(1).expect("next");
    let code = fixture_code(&harness.state);
    let (status, _, _, html) = post(
        &app,
        "/control/login",
        Some(&cookie),
        &format!("operator=maya&password={FIXTURE_PASSWORD}&reauth=1&next={next}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{html}");
    let pending = hidden_value(&html, "pending");
    let (status, location, _, _) = post(
        &app,
        "/control/login/code",
        Some(&cookie),
        &format!("pending={pending}&code={code}"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location.as_deref(), Some("/control/incidents/inc_018"));
    // The same cookie now passes the freshness gate.
    let (_, html) = get(&app, "/control/incidents/inc_018", Some(&cookie)).await;
    let form = form_fields_for(&html, "incident_resolve");
    let request = format!(
        "csrf_token={}&idempotency_key={}&version_token={}&target={}&return_path={}&reason=x&evidence=y",
        hidden_value(&form, "csrf_token"),
        hidden_value(&form, "idempotency_key"),
        hidden_value(&form, "version_token"),
        hidden_value(&form, "target"),
        hidden_value(&form, "return_path"),
    );
    let (status, location, _, html) = post(
        &app,
        "/control/actions/incident_resolve",
        Some(&cookie),
        &request,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{html}");
    assert!(location.expect("result").contains("/result/res_"));
}

// ---------------------------------------------------------------------------
// Capability matrix
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_role_times_every_capability_is_permitted_or_denied_with_exact_copy() {
    let harness = harness();
    let app = app(&harness);
    let mut denied_posts = 0usize;
    for (role_index, role) in Role::ALL.into_iter().enumerate() {
        for (capability_index, capability) in Capability::ALL.into_iter().enumerate() {
            let signed = open_session(&harness.state, &format!("probe{role_index}"), role);
            let target = target_for(capability, &harness.state).await;
            let key = format!(
                "{:x}{:x}",
                role_index as u128 * 1_000 + capability_index as u128,
                capability_index as u128 + 1
            );
            let key = format!("{key:0<32}").chars().take(32).collect::<String>();
            let form = form_for(&harness.state, &signed, capability, &target, &key).await;
            let (status, _, _, html) = post(
                &app,
                &format!("/control/actions/{}", capability.as_str()),
                Some(&signed.cookie),
                &form,
            )
            .await;
            if role.allows(capability) {
                assert_eq!(
                    status,
                    StatusCode::SEE_OTHER,
                    "{role:?} {capability:?} should be permitted: {html}"
                );
            } else {
                denied_posts += 1;
                assert_eq!(
                    status,
                    StatusCode::FORBIDDEN,
                    "{role:?} {capability:?} should be denied"
                );
                assert!(html.contains("Your role does not permit this action."));
                assert!(html.contains("No operation was requested."));
                assert!(html.contains("Return to the object."));
            }
        }
    }
    let family = harness
        .state
        .metrics
        .family_snapshot(apex_control::telemetry::OPERATOR_ACTIONS_TOTAL.name)
        .expect("the denied posts recorded the bounded series");
    let denied_total: u64 = family
        .series
        .iter()
        .filter(|series| {
            series
                .labels
                .iter()
                .any(|(name, value)| *name == "outcome" && value == "denied")
        })
        .map(|series| series.counter.unwrap_or(0))
        .sum();
    assert_eq!(denied_total as usize, denied_posts, "{family:?}");
    assert!(denied_posts > 50, "the matrix denies the majority");
    // Every recorded dimension is inside the declared bounded vocabularies.
    for series in &family.series {
        for (name, value) in &series.labels {
            let bounded = match *name {
                "capability_class" => {
                    apex_metrics::vocab::CapabilityClass::VALUES.contains(&value.as_str())
                }
                "action_class" => apex_control::telemetry::ACTION_CLASSES.contains(&value.as_str()),
                "outcome" => apex_metrics::vocab::Outcome::VALUES.contains(&value.as_str()),
                other => panic!("unexpected metric label {other}"),
            };
            assert!(bounded, "unbounded {name}={value}");
        }
    }
}

#[tokio::test]
async fn an_allowed_action_counts_confirmed_with_a_bounded_capability_class() {
    let harness = harness();
    let app = app(&harness);
    // Maya's operator role holds incident_acknowledge; the action executes
    // locally and records the page-64 `confirmed` result.
    let signed = open_session(&harness.state, "maya", Role::Operator);
    assert!(Role::Operator.allows(Capability::IncidentAcknowledge));
    let form = form_for(
        &harness.state,
        &signed,
        Capability::IncidentAcknowledge,
        "inc_018",
        "5eed0000000000000000000000000001",
    )
    .await;
    let (status, _, _, html) = post(
        &app,
        "/control/actions/incident_acknowledge",
        Some(&signed.cookie),
        &form,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{html}");
    let labels = [
        ("capability_class", "incident"),
        ("action_class", "incident_acknowledge"),
        ("outcome", "confirmed"),
    ];
    assert_eq!(
        harness
            .state
            .metrics
            .counter_value(&apex_control::telemetry::OPERATOR_ACTIONS_TOTAL, &labels)
            .expect("counter readable"),
        1
    );
}

#[tokio::test]
async fn a_denied_action_counts_denied_and_a_job_action_counts_requested() {
    let harness = harness();
    let app = app(&harness);
    // An observer holds no capabilities: the attempt is denied and counted
    // with the page-97 outcome, and no object changes.
    let signed = open_session(&harness.state, "ravi", Role::Observer);
    assert!(!Role::Observer.allows(Capability::IncidentAcknowledge));
    let form = form_for(
        &harness.state,
        &signed,
        Capability::IncidentAcknowledge,
        "inc_018",
        "de7e0000000000000000000000000001",
    )
    .await;
    let (status, _, _, html) = post(
        &app,
        "/control/actions/incident_acknowledge",
        Some(&signed.cookie),
        &form,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{html}");
    let denied = [
        ("capability_class", "incident"),
        ("action_class", "incident_acknowledge"),
        ("outcome", "denied"),
    ];
    assert_eq!(
        harness
            .state
            .metrics
            .counter_value(&apex_control::telemetry::OPERATOR_ACTIONS_TOTAL, &denied)
            .expect("counter readable"),
        1
    );
    // A permitted action that writes a durable job records the `requested`
    // result state.
    let operator = open_session(&harness.state, "maya", Role::Operator);
    assert!(Role::Operator.allows(Capability::RetryRead));
    let form = form_for(
        &harness.state,
        &operator,
        Capability::RetryRead,
        "northstar:applovin_max:acct_northstar_max_01",
        "0b1e0000000000000000000000000001",
    )
    .await;
    let (status, _, _, html) = post(
        &app,
        "/control/actions/retry_read",
        Some(&operator.cookie),
        &form,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{html}");
    let requested = [
        ("capability_class", "ingestion"),
        ("action_class", "retry_read"),
        ("outcome", "requested"),
    ];
    assert_eq!(
        harness
            .state
            .metrics
            .counter_value(&apex_control::telemetry::OPERATOR_ACTIONS_TOTAL, &requested)
            .expect("counter readable"),
        1
    );
}

#[tokio::test]
async fn control_http_series_use_route_templates_and_the_in_flight_gauge_drains() {
    let harness = harness();
    let app = app(&harness);
    let (status, _) = get(&app, "/control/", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let labels = [
        ("service", "apex-control"),
        ("route_template", "/control/"),
        ("method", "GET"),
        ("status_class", "4xx"),
    ];
    assert_eq!(
        harness
            .state
            .metrics
            .counter_value(&apex_control::telemetry::HTTP_REQUESTS_TOTAL, &labels)
            .expect("counter readable"),
        1
    );
    let family = harness
        .state
        .metrics
        .family_snapshot(apex_control::telemetry::HTTP_REQUESTS_IN_FLIGHT.name)
        .expect("the guard wrote the series");
    assert_eq!(family.series[0].gauge, Some(0.0), "{family:?}");
}

#[tokio::test]
async fn an_observer_cannot_submit_a_mutation_and_nothing_is_requested() {
    let harness = harness();
    let app = app(&harness);
    let signed = open_session(&harness.state, "ravi", Role::Observer);
    let before = harness.state.journal.read().expect("journal").entries.len();
    let form = form_for(
        &harness.state,
        &signed,
        Capability::IncidentAcknowledge,
        "inc_018",
        "0b5e12ae000000000000000000000001",
    )
    .await;
    let (status, _, _, html) = post(
        &app,
        "/control/actions/incident_acknowledge",
        Some(&signed.cookie),
        &form,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(html.contains("Your role does not permit this action. No operation was requested."));
    let read = harness.state.journal.read().expect("journal");
    assert_eq!(read.entries.len(), before + 1, "exactly the denial row");
    assert_eq!(read.entries.last().expect("row").outcome, "denied");
}

// ---------------------------------------------------------------------------
// CSRF, idempotency, stale state
// ---------------------------------------------------------------------------

#[tokio::test]
async fn csrf_failure_is_refused_without_a_journal_entry() {
    let harness = harness();
    let app = app(&harness);
    let signed = open_session(&harness.state, "maya", Role::Operator);
    let before = harness.state.journal.read().expect("journal").entries.len();
    let form = format!(
        "csrf_token=deadbeefdeadbeefdeadbeefdeadbeef&idempotency_key=0123456789abcdef0123456789abcdef&version_token=v1%3A{}&target=inc_018&return_path=%2Fcontrol%2F&reason=x",
        "0".repeat(64)
    );
    let (status, _, _, html) = post(
        &app,
        "/control/actions/incident_acknowledge",
        Some(&signed.cookie),
        &form,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(html.contains("no action was requested"));
    assert_eq!(
        harness.state.journal.read().expect("journal").entries.len(),
        before
    );
}

#[tokio::test]
async fn cross_origin_action_and_logout_posts_are_refused_before_any_effect() {
    let harness = harness();
    let app = app(&harness);
    let signed = open_session(&harness.state, "maya", Role::Operator);
    let before = harness.state.journal.read().expect("journal").entries.len();
    let form = form_for(
        &harness.state,
        &signed,
        Capability::IncidentAcknowledge,
        "inc_018",
        "0f0e0000000000000000000000000001",
    )
    .await;
    let (status, _, _, html) = post_with_headers(
        &app,
        "/control/actions/incident_acknowledge",
        Some(&signed.cookie),
        &form,
        &[("origin", "https://evil.example")],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{html}");
    assert_eq!(
        harness.state.journal.read().expect("journal").entries.len(),
        before,
        "a cross-origin action writes nothing"
    );

    let (_, page) = get(&app, "/control/", Some(&signed.cookie)).await;
    let csrf = hidden_value(&page, "csrf_token");
    let (status, _, _, html) = post_with_headers(
        &app,
        "/control/logout",
        Some(&signed.cookie),
        &format!("csrf_token={csrf}"),
        &[("origin", "https://evil.example")],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{html}");
    let (status, _) = get(&app, "/control/", Some(&signed.cookie)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the session survives a refused sign-out"
    );
}

#[tokio::test]
async fn a_result_page_under_the_wrong_capability_is_404() {
    let harness = harness();
    let app = app(&harness);
    let signed = open_session(&harness.state, "maya", Role::Operator);
    let form = form_for(
        &harness.state,
        &signed,
        Capability::RetryRead,
        "northstar:applovin_max:acct_northstar_max_01",
        "1de00000000000000000000000000002",
    )
    .await;
    let (status, location, _, _) = post(
        &app,
        "/control/actions/retry_read",
        Some(&signed.cookie),
        &form,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let location = location.expect("location");
    let result_id = location.rsplit('/').next().expect("result id");
    let (status, _) = get(
        &app,
        &format!("/control/actions/incident_acknowledge/result/{result_id}"),
        Some(&signed.cookie),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the kind is part of the identity"
    );
    let (status, body) = get(&app, &location, Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(result_id));
}

#[tokio::test]
async fn duplicate_submits_return_the_same_result_with_one_journal_entry_and_one_job() {
    let harness = harness();
    let app = app(&harness);
    let signed = open_session(&harness.state, "maya", Role::Operator);
    let form = form_for(
        &harness.state,
        &signed,
        Capability::RetryRead,
        "northstar:applovin_max:acct_northstar_max_01",
        "1de00000000000000000000000000001",
    )
    .await;
    let (first_status, first_location, _, _) = post(
        &app,
        "/control/actions/retry_read",
        Some(&signed.cookie),
        &form,
    )
    .await;
    let (second_status, second_location, _, _) = post(
        &app,
        "/control/actions/retry_read",
        Some(&signed.cookie),
        &form,
    )
    .await;
    assert_eq!(first_status, StatusCode::SEE_OTHER);
    assert_eq!(second_status, StatusCode::SEE_OTHER);
    assert_eq!(first_location, second_location);
    let location = first_location.expect("location");
    assert!(location.contains("/control/actions/retry_read/result/res_"));
    let read = harness.state.journal.read().expect("journal");
    let fingerprint =
        apex_control::signing::idempotency_fingerprint("1de00000000000000000000000000001");
    assert_eq!(
        read.entries
            .iter()
            .filter(|entry| entry.idempotency_key == fingerprint)
            .count(),
        1
    );
    let jobs = harness.state.jobs.list().await.expect("jobs");
    let matching = jobs
        .iter()
        .filter(|job| match job {
            apex_control::jobs::JobRead::Parsed(job) => {
                job.idempotency_key == "1de00000000000000000000000000001"
            }
            apex_control::jobs::JobRead::StateUnknown { .. } => false,
        })
        .count();
    assert_eq!(matching, 1);
    let (status, html) = get(&app, &location, Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Requested"));
    assert!(html.contains("opj_"));
    assert!(html.contains("Confirming read"));
}

#[tokio::test]
async fn a_reused_key_for_a_different_target_is_a_conflict_and_changes_nothing() {
    let harness = harness();
    let app = app(&harness);
    let signed = open_session(&harness.state, "maya", Role::Operator);
    let key = "1de0000000000000000000000000feed";
    let first = form_for(
        &harness.state,
        &signed,
        Capability::RetryRead,
        "northstar:applovin_max:acct_northstar_max_01",
        key,
    )
    .await;
    let (status, _, _, _) = post(
        &app,
        "/control/actions/retry_read",
        Some(&signed.cookie),
        &first,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let journal_rows = harness.state.journal.read().expect("journal").entries.len();
    let job_rows = harness.state.jobs.list().await.expect("jobs").len();
    // The same key now targets a different account: a fresh, valid form that
    // would execute if the key were not bound to the first action.
    let second = form_for(
        &harness.state,
        &signed,
        Capability::RetryRead,
        "northstar:applovin_max:acct_northstar_max_02",
        key,
    )
    .await;
    let (status, location, _, html) = post(
        &app,
        "/control/actions/retry_read",
        Some(&signed.cookie),
        &second,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{html}");
    assert!(location.is_none());
    assert!(
        html.contains("This request key was already used for a different action."),
        "{html}"
    );
    assert!(html.contains("Return to the object"));
    // No state change: no new journal entry and no second durable job.
    assert_eq!(
        harness.state.journal.read().expect("journal").entries.len(),
        journal_rows
    );
    assert_eq!(
        harness.state.jobs.list().await.expect("jobs").len(),
        job_rows
    );
}

#[tokio::test]
async fn a_durable_job_claim_for_a_different_action_is_a_conflict() {
    let harness = harness();
    let app = app(&harness);
    let signed = open_session(&harness.state, "maya", Role::Operator);
    let key = "1de0000000000000000000000000cafe";
    // A restart between the job write and the journal append leaves a
    // durable job under the key with no remembered result in the index.
    let stale = apex_control::jobs::OperatorJob::queued(
        apex_control::jobs::JobKind::RetryRead,
        "maya",
        harness.state.now_ms(),
        "northstar:applovin_max:acct_northstar_max_01",
        serde_json::json!({}),
        key,
        "corr-stale",
    );
    harness.state.jobs.write(&stale).await.expect("stale job");
    let journal_rows = harness.state.journal.read().expect("journal").entries.len();
    let form = form_for(
        &harness.state,
        &signed,
        Capability::RetryRead,
        "northstar:applovin_max:acct_northstar_max_02",
        key,
    )
    .await;
    let (status, _, _, html) = post(
        &app,
        "/control/actions/retry_read",
        Some(&signed.cookie),
        &form,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{html}");
    assert!(
        html.contains("This request key was already used for a different action."),
        "{html}"
    );
    assert_eq!(
        harness.state.journal.read().expect("journal").entries.len(),
        journal_rows,
        "the conflict writes no journal entry"
    );
    assert_eq!(
        harness.state.jobs.list().await.expect("jobs").len(),
        1,
        "the durable job claim is unchanged"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_submits_of_one_key_produce_one_job_and_one_journal_entry() {
    let harness = harness();
    let signed = open_session(&harness.state, "maya", Role::Operator);
    let key = "c0ffeec0ffeec0ffeec0ffeec0ffee01";
    let build_request = |target: &str| {
        let state = harness.state.clone();
        let view = signed.view.clone();
        let key = key.to_string();
        let target = target.to_string();
        async move {
            let spec = apex_control::routes::build_form_spec(
                &state,
                &view,
                Capability::RetryRead,
                &target,
                "/control/ingestion",
                &[],
            )
            .await
            .expect("spec");
            apex_control::actions::ActionRequest {
                target: spec.target.clone(),
                reason: "concurrency check".to_string(),
                source_document: String::new(),
                evidence: String::new(),
                confirmation: String::new(),
                idempotency_key: key,
                version_token: spec.version_token.clone(),
                csrf_token: view.csrf.clone(),
                return_path: "/control/ingestion".to_string(),
                payload: serde_json::json!({}),
                customer_population: String::new(),
                expected_effect: String::new(),
                platform: None,
                policy_id: String::new(),
                policy_json: String::new(),
                revenue_30d_micros: String::new(),
                tier: String::new(),
            }
        }
    };
    let request = build_request("northstar:applovin_max:acct_northstar_max_01").await;
    // A third submission shares the key but targets another account: it must
    // either execute (and then both same-key v1 submits conflict) or conflict
    // itself, never return the other action's result.
    let other = build_request("northstar:applovin_max:acct_northstar_max_02").await;
    // All three tasks are released together on a two-worker runtime, so the
    // duplicate check genuinely races.
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let submit_once = |state: ControlState, request: apex_control::actions::ActionRequest| {
        let barrier = Arc::clone(&barrier);
        tokio::spawn(async move {
            barrier.wait().await;
            apex_control::actions::submit(
                &state,
                Capability::RetryRead,
                &request,
                "maya",
                Role::Operator,
                &request.csrf_token,
                true,
            )
            .await
        })
    };
    let (first, second, third) = tokio::join!(
        submit_once(harness.state.clone(), request.clone()),
        submit_once(harness.state.clone(), request.clone()),
        submit_once(harness.state.clone(), other.clone())
    );
    let outcomes = [
        first.expect("join"),
        second.expect("join"),
        third.expect("join"),
    ];
    let mut executed: Vec<(usize, String)> = Vec::new();
    let mut duplicated = 0usize;
    let mut conflicted = 0usize;
    for (index, outcome) in outcomes.iter().enumerate() {
        match outcome {
            apex_control::actions::SubmitOutcome::Result { result_id: id, .. } => {
                executed.push((index, id.clone()));
            }
            apex_control::actions::SubmitOutcome::Duplicate { .. } => duplicated += 1,
            apex_control::actions::SubmitOutcome::Conflict { .. } => conflicted += 1,
            other => panic!("unexpected outcome: {other:?}"),
        }
    }
    assert_eq!(
        executed.len(),
        1,
        "exactly one submission executes: {outcomes:?}"
    );
    let (executor, result_id) = &executed[0];
    if *executor == 2 {
        // The different-action submission won the race: both same-payload
        // duplicates are key conflicts.
        assert_eq!(duplicated, 0, "{outcomes:?}");
        assert_eq!(conflicted, 2, "{outcomes:?}");
    } else {
        // A same-payload submission executed: the twin is a duplicate and
        // the different-action submission is a conflict.
        assert_eq!(duplicated, 1, "{outcomes:?}");
        assert_eq!(conflicted, 1, "{outcomes:?}");
    }
    for outcome in &outcomes {
        if let apex_control::actions::SubmitOutcome::Duplicate { result_id: id } = outcome {
            assert_eq!(id, result_id, "the duplicate returns the original page");
        }
    }
    // Exactly one journal entry and one durable job carry the key.
    let fingerprint = apex_control::signing::idempotency_fingerprint(key);
    let read = harness.state.journal.read().expect("journal");
    assert_eq!(
        read.entries
            .iter()
            .filter(|entry| entry.idempotency_key == fingerprint)
            .count(),
        1,
        "one journal entry"
    );
    let jobs = harness.state.jobs.list().await.expect("jobs");
    let durable = jobs
        .iter()
        .filter(|job| match job {
            apex_control::jobs::JobRead::Parsed(job) => job.idempotency_key == key,
            apex_control::jobs::JobRead::StateUnknown { .. } => false,
        })
        .count();
    assert_eq!(durable, 1, "one durable job");
}

#[tokio::test]
async fn a_stale_state_token_is_409_and_changes_nothing() {
    let harness = harness();
    let app = app(&harness);
    let signed = open_session(&harness.state, "maya", Role::Operator);
    let form = form_for(
        &harness.state,
        &signed,
        Capability::IncidentAcknowledge,
        "inc_018",
        "5a1e0000000000000000000000000001",
    )
    .await;
    harness.state.versions.bump("incident:inc_018");
    let (status, _, _, html) = post(
        &app,
        "/control/actions/incident_acknowledge",
        Some(&signed.cookie),
        &form,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(html.contains("This object changed while the form was open."));
    assert!(html.contains("no operation was requested"));
    assert!(html.contains("Return to the object"));
}

// ---------------------------------------------------------------------------
// Journal and job formats
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_journal_line_carries_the_format_and_redacts_restricted_values() {
    let harness = harness();
    let app = app(&harness);
    let signed = open_session(&harness.state, "maya", Role::Operator);
    let spec = apex_control::routes::build_form_spec(
        &harness.state,
        &signed.view,
        Capability::RecheckRequest,
        "ch_0246",
        "/control/finance",
        &[],
    )
    .await
    .expect("spec");
    let form = format!(
        "csrf_token={}&idempotency_key={}&version_token={}&target={}&return_path=%2Fcontrol%2Ffinance&reason=token%3A+0123456789abcdef0123456789abcdef+checking",
        enc(&spec.csrf),
        enc(&spec.idempotency_key),
        enc(&spec.version_token),
        enc(&spec.target)
    );
    let (status, _, _, _) = post(
        &app,
        "/control/actions/recheck_request",
        Some(&signed.cookie),
        &form,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let raw = std::fs::read_to_string(harness.state.journal.path()).expect("journal file");
    let line = raw.lines().last().expect("last line");
    let value: serde_json::Value = serde_json::from_str(line).expect("json line");
    for field in [
        "atMs",
        "actor",
        "role",
        "capability",
        "capabilityClass",
        "target",
        "outcome",
        "reason",
        "beforeDigest",
        "afterDigest",
        "idempotencyKey",
        "resultId",
        "correlationId",
    ] {
        assert!(value.get(field).is_some(), "missing field {field}: {line}");
    }
    assert_eq!(value["actor"], "maya");
    assert_eq!(value["outcome"], "requested");
    assert_eq!(value["capability"], "recheck_request");
    let reason = value["reason"].as_str().expect("reason");
    assert!(reason.contains("[withheld]"), "reason: {reason}");
    assert!(!reason.contains("0123456789abcdef"), "reason: {reason}");
    assert!(value["idempotencyKey"]
        .as_str()
        .expect("key")
        .starts_with("sha256:"));
    assert_eq!(value["beforeDigest"].as_str().expect("digest").len(), 64);
}

#[tokio::test]
async fn the_job_file_matches_the_frozen_schema() {
    let harness = harness();
    let app = app(&harness);
    let signed = open_session(&harness.state, "maya", Role::Operator);
    let form = form_for(
        &harness.state,
        &signed,
        Capability::RestoreReconcileRequest,
        "exp_0246",
        "10b00000000000000000000000000001",
    )
    .await;
    let (status, _, _, _) = post(
        &app,
        "/control/actions/restore_reconcile_request",
        Some(&signed.cookie),
        &form,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let jobs = harness.state.jobs.list().await.expect("jobs");
    let job = jobs
        .iter()
        .find_map(|job| match job {
            apex_control::jobs::JobRead::Parsed(job)
                if job.idempotency_key == "10b00000000000000000000000000001" =>
            {
                Some(job)
            }
            _ => None,
        })
        .expect("job row");
    assert!(job.job_id.starts_with("opj_"));
    assert_eq!(job.job_id.len(), 16);
    assert_eq!(job.kind.as_str(), "restore_reconcile");
    assert_eq!(job.requested_by, "maya");
    assert_eq!(job.state, "queued");
    assert_eq!(job.state_history.len(), 1);
    assert_eq!(job.state_history[0].state, "queued");
    assert!(job.outcome.is_none());
    assert_eq!(job.target, "exp_0246");
    assert!(!job.correlation_id.is_empty());
    let raw = std::fs::read_to_string(
        harness
            .state
            .jobs
            .dir()
            .expect("local jobs dir")
            .join(format!("{}.json", job.job_id)),
    )
    .expect("job file");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("json");
    for field in [
        "job_id",
        "kind",
        "requested_by",
        "requested_at_ms",
        "target",
        "payload",
        "idempotency_key",
        "state",
        "state_history",
        "outcome",
        "correlation_id",
    ] {
        assert!(value.get(field).is_some(), "missing field {field}");
    }
    assert_eq!(
        value["kind"], "restore_reconcile",
        "the runner's canonical wire kind, not the capability spelling"
    );
}

// ---------------------------------------------------------------------------
// Isolation, credentials, no manual keep
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_console_session_cookie_grants_nothing_here() {
    let harness = harness();
    let app = app(&harness);
    let (status, html) = get(&app, "/control/", Some("apex_ledger_session=deadbeef")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(html.contains("Your session ended."));
    assert_eq!(session::SESSION_COOKIE, "apex_operator_session");
    assert_ne!(session::SESSION_COOKIE, "apex_ledger_session");
    let cookie = sign_in(&app, &harness, "maya").await;
    for uri in [
        "/control/",
        "/control/customers",
        "/control/ingestion",
        "/control/finance",
        "/control/access",
        "/control/system/policies",
    ] {
        let (status, html) = get(&app, uri, Some(&cookie)).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert!(!html.contains("apex_ledger_session"), "{uri}");
        assert!(!html.contains("Argon2"), "{uri}");
        assert!(!html.contains("BEGIN PRIVATE KEY"), "{uri}");
        assert!(!html.contains("Authorization: Bearer"), "{uri}");
    }
}

#[tokio::test]
async fn there_is_no_manual_experiment_keep() {
    let harness = harness();
    let app = app(&harness);
    let cookie = sign_in(&app, &harness, "maya").await;
    assert!(Capability::parse("manual_keep").is_none());
    assert!(Capability::parse("experiment_keep").is_none());
    let (status, _, _, _) = post(
        &app,
        "/control/actions/manual_keep",
        Some(&cookie),
        "csrf_token=x",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, html) = get(&app, "/control/experiments/exp_0246", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("An operator cannot type a better result."));
    assert!(!html.contains("manual_keep"));
    assert!(!html.contains("Keep experiment"));
}

// ---------------------------------------------------------------------------
// Unconfigured deployment
// ---------------------------------------------------------------------------

fn unconfigured_state() -> ControlState {
    let work = std::env::temp_dir().join(format!(
        "apex-control-unconfigured-{}-{}",
        std::process::id(),
        apex_control::signing::random_hex_32()
    ));
    let config = ControlConfig {
        bind: "127.0.0.1:0".parse().expect("addr"),
        operators_path: None,
        journal_path: work.join("journal.jsonl"),
        jobs_dir: work.join("jobs"),
        jobs_remote: None,
        sync_marker: None,
        environment: "TEST".to_string(),
        ledger_root: work.join("ledgers"),
        ledger_keys: std::collections::HashMap::new(),
        snapshot_root: work.join("snapshots"),
        notify_sink: None,
        metrics_textfile: None,
        invoice_register: work.join("invoices.jsonl"),
        totp_state_path: work.join("totp-steps.json"),
        operations_path: work.join("operations.json"),
        policies_path: work.join("policies.jsonl"),
        interventions_path: work.join("interventions.jsonl"),
        effective_policy_path: work.join("effective-policy.json"),
        connections_root: work.join("connections"),
        clickhouse: None,
    };
    ControlState::new(config, ControlData::fixtures()).expect("state")
}

#[tokio::test]
async fn an_unconfigured_deployment_explains_and_mints_nothing() {
    let state = unconfigured_state();
    assert!(matches!(
        state.unconfigured_reason.as_deref(),
        Some(reason) if reason.contains("APEX_CONTROL_OPERATORS")
    ));
    let app = router(state.clone());
    let (status, html) = get(&app, "/control/login", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(html.contains("This control plane is not configured."));
    assert!(html.contains("Set APEX_CONTROL_OPERATORS"));
    let (status, _, _, html) = post(
        &app,
        "/control/login",
        None,
        &format!("operator=maya&password={FIXTURE_PASSWORD}"),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(html.contains("This control plane is not configured."));
    let (status, html) = get(&app, "/control/", None).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(html.contains("no session can be issued"));
    assert!(state.sessions.is_empty(), "no session may be minted");
}

// ---------------------------------------------------------------------------
// KPI parity and the fixture sweep
// ---------------------------------------------------------------------------

#[tokio::test]
async fn overview_kpis_equal_their_row_populations() {
    let harness = harness();
    let app = app(&harness);
    let cookie = sign_in(&app, &harness, "maya").await;
    let (status, html) = get(&app, "/control/", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    let demo = apex_control::fixtures::DemoData::new();
    let queues = demo.queues();
    let mut publishers: Vec<String> = queues
        .incidents
        .iter()
        .map(|row| row.publisher_id.clone())
        .collect();
    publishers.sort();
    publishers.dedup();
    assert_eq!(publishers.len(), 3);
    assert!(html.contains(">3</a>") || html.contains(">3</p>"));
    assert!(html.contains(">2</a>") || html.contains(">2</p>"));
    let rows = pages::overview::priority_rows(&queues);
    assert!(html.contains(&format!("{} unresolved item(s)", rows.len())));
    assert!(html.contains("USD 500.00"));
    assert!(html.contains("2 held fee line(s)"));
}

#[tokio::test]
async fn every_fixture_page_is_script_free_with_one_h1_and_no_credentials() {
    let dir = std::env::temp_dir().join(format!(
        "apex-control-acceptance-{}-{}",
        std::process::id(),
        apex_control::signing::random_hex_32()
    ));
    let files = fixture_render::render_all(&dir).expect("render");
    assert!(files.len() >= 28, "pages: {}", files.len());
    for file in &files {
        let html = std::fs::read_to_string(dir.join(file)).expect("read");
        assert_eq!(html.matches("<h1").count(), 1, "{file}: one h1");
        assert!(!html.contains("<script"), "{file}: script free");
        assert!(
            !html.contains("apex_operator_session="),
            "{file}: session handle leaked"
        );
        assert!(!html.contains(FIXTURE_TOTP_SECRET), "{file}: totp secret");
        assert!(!html.contains(FIXTURE_PASSWORD), "{file}: password");
        assert!(html.contains("OPERATOR PLANE"), "{file}: operator area");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn fixture_pages_scale_percentages_exactly_once() {
    let dir = std::env::temp_dir().join(format!(
        "apex-control-percent-{}-{}",
        std::process::id(),
        apex_control::signing::random_hex_32()
    ));
    fixture_render::render_all(&dir).expect("render");
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).expect("fixture page");
    // 18 of 30 valid settled kept = 60.0%; 18 of 40 started = 45.0%.
    let effectiveness = read("experiments-effectiveness.html");
    assert!(effectiveness.contains("60.0%"), "18 of 30 kept is 60.0%");
    assert!(effectiveness.contains("45.0%"), "18 of 40 started is 45.0%");
    assert!(!effectiveness.contains("6000.0%"));
    assert!(!effectiveness.contains("4500.0%"));
    // 540 bp observed lift = 5.4%; 300 bp billable bound = 3.0%.
    let detail = read("experiment-detail.html");
    assert!(detail.contains("5.4%"), "540 bp is 5.4%");
    assert!(detail.contains("3.0%"), "300 bp is 3.0%");
    assert!(!detail.contains("540.0%"));
    // The growth funnel is a share of requested: 35 of 52 = 67.3%,
    // 8 of 52 = 15.4%.
    let growth = read("growth.html");
    assert!(growth.contains("67.3%"), "35 of 52 is 67.3%");
    assert!(growth.contains("15.4%"), "8 of 52 is 15.4%");
    assert!(!growth.contains("6730.8%"));
    // Cost coverage 2026-01: 2.4B over 6.3B = 38.1%.
    let analytics = read("finance-analytics.html");
    assert!(analytics.contains("38.1%"), "2.4 of 6.3 is 38.1%");
    assert!(!analytics.contains("3810.0%"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn the_system_page_shows_the_copied_ledger_sync_age_or_the_honest_absence() {
    let dir = std::env::temp_dir().join(format!(
        "apex-control-syncpage-{}-{}",
        std::process::id(),
        apex_control::signing::random_hex_32()
    ));
    std::fs::create_dir_all(&dir).expect("dir");
    let marker = dir.join("sync-marker.json");
    let as_of = apex_control::fixtures::DEMO_NOW_MS;
    std::fs::write(
        &marker,
        format!(
            "{{\"synced_at_unix_ms\":{},\"source\":\"ledger\"}}",
            as_of - 3 * 3_600_000
        ),
    )
    .expect("write");

    let live_with_marker = |marker: Option<std::path::PathBuf>| {
        ControlData::Live(Box::new(LiveData {
            analytics: None,
            durable: DurableReader::new(dir.join("ledgers"), dir.join("invoices.jsonl")),
            operations: Ok(OperationsEvidence::default()),
            sync_marker: marker,
        }))
    };

    // Recorded: the exact line, with the fixed as-of clock.
    let state = fixture_render::fixture_state()
        .expect("fixture state")
        .with_data(live_with_marker(Some(marker.clone())));
    let signed = open_session(&state, "maya", Role::Operator);
    let app = router(state);
    let (status, html) = get(&app, "/control/system", Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK);
    let expected = format!(
        "Ledger copy synced: {} (age 3 h)",
        apex_ui::format::utc_datetime(as_of - 3 * 3_600_000)
    );
    assert!(
        html.contains(&expected),
        "expected the sync line '{expected}' in: {html}"
    );

    // Unset and absent both render the honest not-recorded state.
    for marker in [None, Some(dir.join("absent.json"))] {
        let state = fixture_render::fixture_state()
            .expect("fixture state")
            .with_data(live_with_marker(marker));
        let signed = open_session(&state, "maya", Role::Operator);
        let app = router(state);
        let (status, html) = get(&app, "/control/system", Some(&signed.cookie)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            html.contains("Ledger copy synced: not recorded"),
            "expected the honest absent state in: {html}"
        );
    }

    // A malformed marker is a typed unavailable, never a panic: the page
    // still serves and explains the absence instead of inventing an age.
    std::fs::write(&marker, "{ not json").expect("write");
    let state = fixture_render::fixture_state()
        .expect("fixture state")
        .with_data(live_with_marker(Some(marker)));
    let signed = open_session(&state, "maya", Role::Operator);
    let app = router(state);
    let (status, html) = get(&app, "/control/system", Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains("Ledger copy synced: not recorded (the sync marker did not parse"),
        "expected the typed unreadable reason in: {html}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn live_customers_and_audits_render_verified_sources_without_fixture_data() {
    let work = std::env::temp_dir().join(format!(
        "apex-control-live-sources-{}-{}",
        std::process::id(),
        apex_control::signing::random_hex_32()
    ));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(work.join("ledgers")).expect("ledgers");
    let key = ledger_key();
    write_signed_ledger(
        &work.join("ledgers").join("northstar.jsonl"),
        &key,
        vec![
            LedgerRecord::lift_record(1_792_000_000_000, &"aa".repeat(32), "{\"lift\":1}"),
            LedgerRecord::lift_record(1_792_010_000_000, &"bb".repeat(32), "{\"lift\":2}"),
        ],
    );
    write_snapshot(
        &work.join("snapshots"),
        "northstar",
        "applovin_max",
        apex_control::fixtures::DEMO_NOW_MS - 7_200_000,
        3,
    );
    write_snapshot(
        &work.join("snapshots"),
        "northstar",
        "levelplay",
        apex_control::fixtures::DEMO_NOW_MS - 86_400_000,
        2,
    );
    let mut keys = std::collections::HashMap::new();
    keys.insert("northstar".to_string(), key.verifying_key());
    let state = live_state(&work, keys, false, None, None, true);
    let signed = open_session(&state, "maya", Role::Operator);
    let app = router(state);

    let (status, customers) = get(&app, "/control/customers", Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(customers.contains("northstar"), "{customers}");
    assert!(
        customers.contains("2 verified record(s)"),
        "the list shows the verified count: {customers}"
    );
    assert!(!customers.contains("ILLUSTRATIVE DATA"));
    assert!(!customers.contains("no account projection source"));

    let (status, detail) = get(&app, "/control/customers/northstar", Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(detail.contains("Verified ledger records"), "{detail}");
    assert!(detail.contains("liftRecord"), "{detail}");
    assert!(!detail.contains("ILLUSTRATIVE DATA"));

    let (status, audits) = get(&app, "/control/audits", Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(audits.contains("northstar"), "{audits}");
    assert!(audits.contains("applovin_max"), "{audits}");
    assert!(audits.contains("levelplay"), "{audits}");
    assert!(audits.contains("Eligible inputs"), "{audits}");
    assert!(audits.contains("2h"), "per-platform snapshot age: {audits}");
    assert!(audits.contains("1d"), "per-platform snapshot age: {audits}");
    assert!(!audits.contains("ILLUSTRATIVE DATA"));
    let _ = std::fs::remove_dir_all(&work);
}

#[tokio::test]
async fn live_customer_paths_that_are_not_ledger_publishers_are_404() {
    let work = std::env::temp_dir().join(format!(
        "apex-control-live-idor-{}-{}",
        std::process::id(),
        apex_control::signing::random_hex_32()
    ));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(work.join("ledgers")).expect("ledgers");
    let key = ledger_key();
    write_signed_ledger(
        &work.join("ledgers").join("northstar.jsonl"),
        &key,
        vec![LedgerRecord::lift_record(
            1_792_000_000_000,
            &"aa".repeat(32),
            "{}",
        )],
    );
    let mut keys = std::collections::HashMap::new();
    keys.insert("northstar".to_string(), key.verifying_key());
    let state = live_state(&work, keys, false, None, None, true);
    let signed = open_session(&state, "maya", Role::Operator);
    let app = router(state);
    for path in [
        "/control/customers/..%2Fescape",
        "/control/customers/%2e%2e%2fescape",
        "/control/customers/other-publisher",
        "/control/customers/northstar%00",
    ] {
        let (status, html) = get(&app, path, Some(&signed.cookie)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {html}");
        assert!(html.contains("Not found."), "{path}");
    }
    let _ = std::fs::remove_dir_all(&work);
}

#[tokio::test]
async fn an_empty_live_deployment_keeps_the_honest_unavailable_reason() {
    let work = std::env::temp_dir().join(format!(
        "apex-control-live-empty-{}-{}",
        std::process::id(),
        apex_control::signing::random_hex_32()
    ));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("work");
    let state = live_state(
        &work,
        std::collections::HashMap::new(),
        false,
        None,
        None,
        false,
    );
    let signed = open_session(&state, "maya", Role::Operator);
    let app = router(state);

    let (status, customers) = get(&app, "/control/customers", Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        customers.contains("no account projection source is available"),
        "{customers}"
    );
    assert!(
        customers.contains("no ledger root is present"),
        "{customers}"
    );
    assert!(!customers.contains("ILLUSTRATIVE DATA"));
    assert!(!customers.contains("Northstar Media"));

    let (status, audits) = get(&app, "/control/audits", Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        audits.contains("no runner snapshot root is present"),
        "{audits}"
    );
    assert!(!audits.contains("ILLUSTRATIVE DATA"));
    assert!(!audits.contains("Stale floor"), "{audits}");
    let _ = std::fs::remove_dir_all(&work);
}

#[tokio::test]
async fn system_components_report_the_real_configured_signals() {
    let work = std::env::temp_dir().join(format!(
        "apex-control-system-signals-{}-{}",
        std::process::id(),
        apex_control::signing::random_hex_32()
    ));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("work");
    let metrics = work.join("metrics.prom");
    let state = live_state(
        &work,
        std::collections::HashMap::new(),
        true,
        Some("email"),
        Some(metrics.clone()),
        true,
    );
    let signed = open_session(&state, "maya", Role::Operator);
    let app = router(state);
    let (status, html) = get(&app, "/control/system", Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Analytics store"), "{html}");
    assert!(html.contains("Configured (ClickHouse)"), "{html}");
    assert!(html.contains("Notification sink"), "{html}");
    assert!(html.contains("Configured (email)"), "{html}");
    assert!(html.contains("Runner state directories"), "{html}");
    assert!(html.contains("3/3 present"), "{html}");
    assert!(html.contains("Metrics textfile"), "{html}");
    assert!(
        html.contains(&metrics.display().to_string()),
        "the configured export path is shown: {html}"
    );

    let unconfigured = live_state(
        &work,
        std::collections::HashMap::new(),
        false,
        None,
        None,
        false,
    );
    let signed = open_session(&unconfigured, "maya", Role::Operator);
    let app = router(unconfigured);
    let (status, html) = get(&app, "/control/system", Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        html.contains("Not configured (APEX_CLICKHOUSE_HTTP/DB are unset)"),
        "{html}"
    );
    assert!(
        html.contains("Not configured (APEX_NOTIFY_SINK is unset)"),
        "{html}"
    );
    assert!(
        html.contains("Not configured (APEX_METRICS_TEXTFILE is unset)"),
        "{html}"
    );
    let _ = std::fs::remove_dir_all(&work);
}

#[tokio::test]
async fn growth_without_an_analytics_source_never_offers_a_write() {
    let work = std::env::temp_dir().join(format!(
        "apex-control-growth-honest-{}-{}",
        std::process::id(),
        apex_control::signing::random_hex_32()
    ));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).expect("work");
    let state = live_state(
        &work,
        std::collections::HashMap::new(),
        false,
        None,
        None,
        false,
    );
    let signed = open_session(&state, "maya", Role::Operator);
    let app = router(state);
    let (status, html) = get(&app, "/control/growth", Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("The cohort read is not available"), "{html}");
    assert!(!html.contains("action=\"/control/actions/"), "{html}");
    assert!(!html.contains("ILLUSTRATIVE DATA"));
    let _ = std::fs::remove_dir_all(&work);
}

#[tokio::test]
async fn font_assets_are_served_from_the_router() {
    let harness = harness();
    let app = app(&harness);
    for name in apex_ui::fonts::FONT_NAMES {
        // The stylesheet URLs carry the extension; that is the request the
        // browser actually makes, so the route must answer it.
        let (status, body) = get(&app, &format!("/assets/fonts/{name}.woff2"), None).await;
        assert_eq!(status, StatusCode::OK, "{name}.woff2");
        assert!(
            body.starts_with("wOF2"),
            "{name}.woff2 lacks the WOFF2 magic"
        );
        let (status, _) = get(&app, &format!("/assets/fonts/{name}"), None).await;
        assert_eq!(status, StatusCode::OK, "{name} (bare stem)");
    }
    let (status, body) = get(&app, "/assets/fonts/manrope-999", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.is_empty(), "asset 404 carries no body");
}

#[tokio::test]
async fn unknown_action_routes_and_objects_are_404_pages() {
    let harness = harness();
    let app = app(&harness);
    let cookie = sign_in(&app, &harness, "maya").await;
    let (status, html) = get(
        &app,
        "/control/actions/not_a_kind/result/res_x",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(html.contains("Not found."));
    let (status, html) = get(&app, "/control/customers/acct_missing", Some(&cookie)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(html.contains("Not found."));
    let (status, html) = get(&app, "/control/incidents/inc_missing", Some(&cookie)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(html.contains("Not found."));
}

#[tokio::test]
async fn a_configured_but_invalid_operator_file_refuses_startup() {
    let work = std::env::temp_dir().join(format!(
        "apex-control-badfile-{}-{}",
        std::process::id(),
        apex_control::signing::random_hex_32()
    ));
    std::fs::create_dir_all(&work).expect("dir");
    std::fs::write(work.join("operators.json"), "{not json}").expect("write");
    let config = ControlConfig {
        bind: "127.0.0.1:0".parse().expect("addr"),
        operators_path: Some(work.join("operators.json")),
        journal_path: work.join("journal.jsonl"),
        jobs_dir: work.join("jobs"),
        jobs_remote: None,
        sync_marker: None,
        environment: "TEST".to_string(),
        ledger_root: work.join("ledgers"),
        ledger_keys: std::collections::HashMap::new(),
        snapshot_root: work.join("snapshots"),
        notify_sink: None,
        metrics_textfile: None,
        invoice_register: work.join("invoices.jsonl"),
        totp_state_path: work.join("totp-steps.json"),
        operations_path: work.join("operations.json"),
        policies_path: work.join("policies.jsonl"),
        interventions_path: work.join("interventions.jsonl"),
        effective_policy_path: work.join("effective-policy.json"),
        connections_root: work.join("connections"),
        clickhouse: None,
    };
    let error = ControlState::new(config, ControlData::fixtures()).expect_err("refused");
    assert!(matches!(error, StateError::Config(_)));
    let _ = std::fs::remove_dir_all(&work);
}

#[test]
fn the_hash_password_entry_point_mints_a_verifiable_operator_hash() {
    // The binary path is exercised through the library entry point (no
    // process spawn): one password line in, one PHC string out.
    let mut output = Vec::new();
    apex_control::cli::hash_password_line(
        &mut std::io::Cursor::new("operator-password\n"),
        &mut output,
    )
    .expect("hash");
    let phc = String::from_utf8(output).expect("utf8");
    assert_eq!(phc.matches('\n').count(), 1);
    let phc = phc.trim_end();
    assert!(apex_control::passwords::phc_is_argon2id(phc).is_ok());
    assert!(apex_control::passwords::verify("operator-password", phc));
    assert!(!apex_control::passwords::verify("some-other-password", phc));
    assert!(!phc.contains("operator-password"));
}

// ---------------------------------------------------------------------------
// Governance wiring: effective policy, managed plan, job-kind and forms
// ---------------------------------------------------------------------------

#[tokio::test]
async fn policy_set_writes_the_effective_file_the_runner_reads_and_refuses_invalid_documents() {
    use std::os::unix::fs::PermissionsExt as _;

    let harness = harness();
    let app = app(&harness);
    let signed = open_session(&harness.state, "ada", Role::Security);
    // A valid document is written atomically to the effective path.
    let form = form_for(
        &harness.state,
        &signed,
        Capability::PolicySet,
        "policy:effective",
        "90000000000000000000000000000001",
    )
    .await;
    let (status, location, _, html) = post(
        &app,
        "/control/actions/policy_set",
        Some(&signed.cookie),
        &form,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{html}");
    assert!(location
        .expect("result")
        .contains("/policy_set/result/res_"));
    let path = harness.state.config.effective_policy_path.clone();
    let policy = apex_autopilot::model::load_policy_file(&path).expect("runner reads it");
    assert_eq!(
        policy,
        apex_autopilot::model::AutopilotPolicy::conservative()
    );
    assert_eq!(
        std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let active = harness.state.governance.active_policies().expect("active");
    assert_eq!(active.len(), 1);
    assert!(active[0]
        .caps
        .get("hash")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|hash| hash.len() == 64));
    // No temp file survives the atomic rename.
    let leftovers: Vec<String> = std::fs::read_dir(path.parent().expect("parent"))
        .expect("dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    // Invalid JSON and out-of-bounds values refuse before any write.
    let _ = std::fs::remove_file(&path);
    let bad = form_with(
        &harness.state,
        &signed,
        Capability::PolicySet,
        "policy:effective",
        "90000000000000000000000000000002",
        "{not json}",
        "0",
        "",
    )
    .await;
    let (status, _, _, _) = post(
        &app,
        "/control/actions/policy_set",
        Some(&signed.cookie),
        &bad,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(!path.exists(), "invalid JSON writes nothing");

    let mut invalid = apex_autopilot::model::AutopilotPolicy::conservative();
    invalid.max_step_bp = 5_000;
    let bad = form_with(
        &harness.state,
        &signed,
        Capability::PolicySet,
        "policy:effective",
        "90000000000000000000000000000003",
        &serde_json::to_string(&invalid).expect("json"),
        "0",
        "",
    )
    .await;
    let (status, _, _, _) = post(
        &app,
        "/control/actions/policy_set",
        Some(&signed.cookie),
        &bad,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(!path.exists(), "an out-of-bounds policy writes nothing");
}

#[tokio::test]
async fn managed_plan_activation_is_idempotent_and_cancel_is_visible_to_the_shared_plan_state() {
    let harness = harness();
    let app = app(&harness);
    let signed = open_session(&harness.state, "sam", Role::Finance);
    let path = harness
        .state
        .config
        .connections_root
        .join("northstar.managed.json");
    let form = form_with(
        &harness.state,
        &signed,
        Capability::ManagedPlanActivate,
        "northstar",
        "a0000000000000000000000000000001",
        "",
        "2000000000",
        "",
    )
    .await;
    let (status, _, _, html) = post(
        &app,
        "/control/actions/managed_plan_activate",
        Some(&signed.cookie),
        &form,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER, "{html}");
    let activated = apex_billing::PlanState::load(&path)
        .expect("load")
        .expect("state");
    assert!(activated.active);
    assert_eq!(activated.plan, apex_billing::PlanId::Managed);
    assert_eq!(activated.monthly_fee_micros, 2_500 * 1_000_000);
    assert_eq!(activated.revenue_30d, Some(2_000_000_000));
    let since = activated.since.clone();
    assert!(activated.monthly_line(1, "2026-10").is_some());

    // A duplicate activation is idempotent: the recorded start does not move
    // and the file keeps one answer.
    let duplicate = form_with(
        &harness.state,
        &signed,
        Capability::ManagedPlanActivate,
        "northstar",
        "a0000000000000000000000000000002",
        "",
        "2000000000",
        "top",
    )
    .await;
    let (status, _, _, _) = post(
        &app,
        "/control/actions/managed_plan_activate",
        Some(&signed.cookie),
        &duplicate,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let unchanged = apex_billing::PlanState::load(&path)
        .expect("load")
        .expect("state");
    assert_eq!(
        unchanged.since, since,
        "duplicate activation does not re-date"
    );
    assert_eq!(unchanged.tier, None, "a duplicate does not change the plan");
    assert_eq!(
        unchanged.monthly_line(2, "2026-10").unwrap().proven_lift,
        2_500 * 1_000_000
    );

    // The console's cancel writes the same file; the runner's billing path
    // (PlanState::monthly_line) sees the cancellation for the next period.
    let mut cancelled = unchanged;
    cancelled.cancel("2026-10");
    cancelled.save(&path).expect("cancel");
    let read_back = apex_billing::PlanState::load(&path)
        .expect("load")
        .expect("state");
    assert!(read_back.cancelled());
    assert!(read_back.monthly_line(3, "2026-10").is_some());
    assert!(
        read_back.monthly_line(4, "2026-11").is_none(),
        "the runner bills no managed line after the stop period"
    );

    // Invalid publisher and tier are refused before any file is written.
    let invalid_tier = form_with(
        &harness.state,
        &signed,
        Capability::ManagedPlanActivate,
        "orbit",
        "a0000000000000000000000000000003",
        "",
        "0",
        "platinum",
    )
    .await;
    let (status, _, _, _) = post(
        &app,
        "/control/actions/managed_plan_activate",
        Some(&signed.cookie),
        &invalid_tier,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(!harness
        .state
        .config
        .connections_root
        .join("orbit.managed.json")
        .exists());
    let invalid_publisher = form_with(
        &harness.state,
        &signed,
        Capability::ManagedPlanActivate,
        "../escape",
        "a0000000000000000000000000000004",
        "",
        "0",
        "",
    )
    .await;
    let (status, _, _, _) = post(
        &app,
        "/control/actions/managed_plan_activate",
        Some(&signed.cookie),
        &invalid_publisher,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(!harness
        .state
        .config
        .connections_root
        .join("escape.managed.json")
        .exists());
}

#[tokio::test]
async fn the_formerly_missing_capabilities_now_render_real_forms() {
    let harness = harness();
    let app = app(&harness);

    // Reconnection: Operator, customer Connections tab.
    let signed = open_session(&harness.state, "maya", Role::Operator);
    let (status, html) = get(
        &app,
        "/control/customers/acct_northstar?tab=connections",
        Some(&signed.cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("action=\"/control/actions/request_reconnection\""));

    // Managed activation: Finance, customer Finance tab.
    let signed = open_session(&harness.state, "sam", Role::Finance);
    let (status, html) = get(
        &app,
        "/control/customers/acct_northstar?tab=finance",
        Some(&signed.cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("action=\"/control/actions/managed_plan_activate\""));
    assert!(html.contains("name=\"revenue_30d_micros\""));
    assert!(html.contains("name=\"tier\""));

    // Re-check: Operator, experiment detail.
    let signed = open_session(&harness.state, "maya", Role::Operator);
    let (status, html) = get(&app, "/control/experiments/exp_0246", Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("action=\"/control/actions/recheck_request\""));

    // Review: Security, access page.
    let signed = open_session(&harness.state, "ada", Role::Security);
    let (status, html) = get(&app, "/control/access", Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("action=\"/control/actions/operator_action_review\""));

    // Effective policy: Security, system policies page.
    let (status, html) = get(&app, "/control/system/policies", Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("action=\"/control/actions/policy_set\""));
    assert!(html.contains("name=\"policy_json\""));
}

#[tokio::test]
async fn a_terminal_job_renders_a_real_retry_form_on_the_system_page() {
    let harness = harness();
    std::fs::create_dir_all(&harness.state.config.jobs_dir).expect("jobs dir");
    let job = serde_json::json!({
        "job_id": "opj_000000000777",
        "kind": "retry_read",
        "requested_by": "maya",
        "requested_at_ms": 1_700_000_000_000_i64,
        "target": "exp_0247",
        "payload": {},
        "idempotency_key": "f0000000000000000000000000000001",
        "state": "completed",
        "state_history": [{
            "state": "completed",
            "at_ms": 1_700_000_001_000_i64,
            "note": "state re-read",
        }],
        "outcome": "state re-read",
        "correlation_id": "corr-retry-form",
    });
    std::fs::write(
        harness.state.config.jobs_dir.join("opj_000000000777.json"),
        serde_json::to_vec(&job).expect("json"),
    )
    .expect("write");
    let app = app(&harness);
    let signed = open_session(&harness.state, "maya", Role::Operator);
    let (status, html) = get(&app, "/control/system", Some(&signed.cookie)).await;
    assert_eq!(status, StatusCode::OK, "{html}");
    assert!(html.contains("action=\"/control/actions/job_retry\""));
    assert!(html.contains("Retry opj_000000000777"));
}
