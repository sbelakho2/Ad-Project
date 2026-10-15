#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Integration tests over the real router: sign-in, the capability matrix,
//! CSRF, idempotency, stale state, the journal and job formats, isolation,
//! the unconfigured deployment and the fixture sweep.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use apex_control::capabilities::{Capability, Role};
use apex_control::config::ControlConfig;
use apex_control::data::ControlData;
use apex_control::fixture_render::{self, FIXTURE_PASSWORD, FIXTURE_TOTP_SECRET};
use apex_control::pages;
use apex_control::routes::router;
use apex_control::session::{self, SessionView};
use apex_control::state::{Clock, ControlState, StateError};
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
        .parent()
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
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
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
        "csrf_token={}&idempotency_key={}&version_token={}&target={}&return_path={}&confirmation={}&reason={}&evidence={}&source_document={}",
        enc(&spec.csrf),
        enc(key),
        enc(&spec.version_token),
        enc(&spec.target),
        enc(&spec.return_path),
        enc(confirmation),
        enc("integration reason"),
        enc("confirming read"),
        enc("doc-1"),
    )
}

/// The target used for one capability in the matrix.
fn target_for(capability: Capability, state: &ControlState) -> String {
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
            state.jobs.write(&job).expect("job").job().job_id.clone()
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
        Capability::OperatorActionReview => {
            let read = state.journal.read().expect("journal");
            let entry = read.entries.first().expect("fixture journal entry");
            apex_control::actions::action_target_key(entry)
        }
    }
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
            let target = target_for(capability, &harness.state);
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
                "action_class" => {
                    apex_metrics::vocab::ActionClass::VALUES.contains(&value.as_str())
                }
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
    let jobs = harness.state.jobs.list().expect("jobs");
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
    let job_rows = harness.state.jobs.list().expect("jobs").len();
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
    assert_eq!(harness.state.jobs.list().expect("jobs").len(), job_rows);
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
    harness.state.jobs.write(&stale).expect("stale job");
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
        harness.state.jobs.list().expect("jobs").len(),
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
    let jobs = harness.state.jobs.list().expect("jobs");
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
    let jobs = harness.state.jobs.list().expect("jobs");
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
    assert_eq!(job.kind.as_str(), "restore_reconcile_request");
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
        environment: "TEST".to_string(),
        ledger_root: work.join("ledgers"),
        invoice_register: work.join("invoices.jsonl"),
        operations_path: work.join("operations.json"),
        policies_path: work.join("policies.jsonl"),
        interventions_path: work.join("interventions.jsonl"),
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
        environment: "TEST".to_string(),
        ledger_root: work.join("ledgers"),
        invoice_register: work.join("invoices.jsonl"),
        operations_path: work.join("operations.json"),
        policies_path: work.join("policies.jsonl"),
        interventions_path: work.join("interventions.jsonl"),
        clickhouse: None,
    };
    let error = ControlState::new(config, ControlData::fixtures()).expect_err("refused");
    assert!(matches!(error, StateError::Config(_)));
    let _ = std::fs::remove_dir_all(&work);
}
