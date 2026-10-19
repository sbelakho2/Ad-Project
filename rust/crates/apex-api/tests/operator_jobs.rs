#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The operator-jobs receiver: `POST /v1/operator/jobs` and
//! `GET /v1/operator/jobs/{job_id}`.
//!
//! The tests pin the frozen contract the control plane and the runner share:
//! the posted bytes land in the runner's jobs folder verbatim (atomic,
//! idempotent by `job_id`), the runner's own consumer parses and consumes
//! the file, and the bearer gate mirrors the live-audit semantics (`503` with
//! no configured token, `401` for a missing or wrong one).
//!
//! `APEX_OPERATOR_JOBS_TOKEN` / `APEX_OPERATOR_JOBS_DIR` are process-wide, so
//! every test in this binary serializes on one environment lock.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use apex_api::{router, AppState};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use serde::Serialize;
use tower::ServiceExt;

const TOKEN_ENV: &str = "APEX_OPERATOR_JOBS_TOKEN";
const DIR_ENV: &str = "APEX_OPERATOR_JOBS_DIR";
const INTERVENTIONS_ENV: &str = "APEX_CONTROL_INTERVENTIONS";
const POLICY_ENV: &str = "APEX_EFFECTIVE_POLICY";

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Serializes one test over the two job env vars and removes both on drop.
struct EnvGuard {
    _lock: MutexGuard<'static, ()>,
}

fn with_jobs_env(token: Option<&str>, dir: Option<&Path>) -> EnvGuard {
    let lock = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match token {
        Some(token) => std::env::set_var(TOKEN_ENV, token),
        None => std::env::remove_var(TOKEN_ENV),
    }
    match dir {
        Some(dir) => std::env::set_var(DIR_ENV, dir),
        None => std::env::remove_var(DIR_ENV),
    }
    EnvGuard { _lock: lock }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        std::env::remove_var(TOKEN_ENV);
        std::env::remove_var(DIR_ENV);
        std::env::remove_var(INTERVENTIONS_ENV);
        std::env::remove_var(POLICY_ENV);
    }
}

/// A unique temp directory removed (best effort) on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "apex-api-operator-jobs-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn app() -> Router {
    router(AppState::new(
        std::sync::Arc::new(apex_api::JsonFileAuditSource::new(std::env::temp_dir())),
        std::sync::Arc::new(apex_api::FileLedgerSource::new(
            std::env::temp_dir(),
            std::collections::HashMap::new(),
        )),
    ))
}

/// One job in the exact frozen shape the control plane writes: a serde struct
/// in the control plane's field order, serialized with the same pretty
/// printer, including the control plane's `payload_fingerprint` field.
#[derive(Serialize)]
struct FrozenStateChange {
    state: String,
    at_ms: i64,
    note: String,
}

#[derive(Serialize)]
struct FrozenJob {
    job_id: String,
    kind: String,
    requested_by: String,
    requested_at_ms: i64,
    target: String,
    payload: serde_json::Value,
    idempotency_key: String,
    payload_fingerprint: String,
    state: String,
    state_history: Vec<FrozenStateChange>,
    outcome: Option<String>,
    correlation_id: String,
}

fn frozen_job(job_id: &str, kind: &str) -> FrozenJob {
    let requested_at_ms = 1_792_000_000_000i64;
    FrozenJob {
        job_id: job_id.to_string(),
        kind: kind.to_string(),
        requested_by: "maya".to_string(),
        requested_at_ms,
        target: "exp_0247".to_string(),
        payload: serde_json::json!({"platform": "applovin_max"}),
        idempotency_key: "0123456789abcdef0123456789abcdef".to_string(),
        payload_fingerprint: "sha256:2f7d2a0f0f14fede".to_string(),
        state: "queued".to_string(),
        state_history: vec![FrozenStateChange {
            state: "queued".to_string(),
            at_ms: requested_at_ms,
            note: "requested by maya from the operator control plane".to_string(),
        }],
        outcome: None,
        correlation_id: "corr-1234".to_string(),
    }
}

fn frozen_bytes(job: &FrozenJob) -> Vec<u8> {
    serde_json::to_vec_pretty(job).expect("frozen job json")
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Vec<u8>) {
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    (status, body.to_vec())
}

async fn post_job(app: &Router, authorization: Option<&str>, body: &[u8]) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/v1/operator/jobs")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(value) = authorization {
        builder = builder.header(header::AUTHORIZATION, value);
    }
    send(
        app,
        builder.body(Body::from(body.to_vec())).expect("request"),
    )
    .await
}

async fn get_job(app: &Router, authorization: Option<&str>, job_id: &str) -> (StatusCode, Vec<u8>) {
    let mut builder = Request::builder()
        .method("GET")
        .uri(format!("/v1/operator/jobs/{job_id}"));
    if let Some(value) = authorization {
        builder = builder.header(header::AUTHORIZATION, value);
    }
    send(app, builder.body(Body::empty()).expect("request")).await
}

fn error_code(body: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(body).expect("json body")["error"]
        .as_str()
        .expect("error code")
        .to_string()
}

/// A runner ops stub that knows one experiment, exactly like the operator
/// jobs consumer's own tests.
fn runner_ops() -> std::sync::Arc<apex_autopilot::RecordingOps> {
    let ops = std::sync::Arc::new(apex_autopilot::RecordingOps::new());
    ops.put_experiment(
        "exp_0247",
        apex_core::ExperimentResult {
            experiment_id: "exp_0247".into(),
            state: apex_core::ExperimentState::Running,
            results: vec![],
            lift_percent_bp: None,
            confidence: None,
            winner_variant_id: None,
        },
    );
    ops
}

#[tokio::test]
async fn posted_job_lands_in_the_runner_folder_in_frozen_bytes_and_is_consumed() {
    let dir = TempDir::new("consume");
    let _env = with_jobs_env(Some("jobs-token"), Some(&dir.path));
    let app = app();
    let job = frozen_job("opj_0000000000a1", "retry_read");
    let body = frozen_bytes(&job);

    let (status, response) = post_job(&app, Some("Bearer jobs-token"), &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        response, body,
        "the created-job response is the frozen bytes verbatim"
    );
    let path = dir.path.join("opj_0000000000a1.json");
    assert_eq!(
        std::fs::read(&path).expect("job file"),
        body,
        "the file on disk is the posted body byte for byte"
    );
    // No temp file survives the atomic write.
    let leftovers: Vec<String> = std::fs::read_dir(&dir.path)
        .expect("dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name != "opj_0000000000a1.json")
        .collect();
    assert!(leftovers.is_empty(), "leftovers: {leftovers:?}");

    // The runner's own daily-pass consumer parses the frozen shape and
    // executes the job; nothing about the API's copy is special-cased.
    let ops = runner_ops();
    let report = apex_autopilot::consume_operator_jobs(
        &dir.path,
        ops.as_ref(),
        &|_| false,
        &|_| true,
        None,
        None,
        "northstar",
        1_792_000_900_000,
    )
    .await
    .expect("consume");
    assert_eq!(report.completed, 1);
    assert_eq!(report.failed, 0);
    let consumed: apex_autopilot::OperatorJob =
        serde_json::from_slice(&std::fs::read(&path).expect("job file")).expect("job json");
    assert_eq!(consumed.kind, "retry_read");
    assert_eq!(consumed.state, "completed");
    assert_eq!(
        consumed.state_history.len(),
        3,
        "queued, running, completed"
    );
    assert_eq!(consumed.state_history[1].state, "running");
    assert!(consumed.state_history[1].note.contains("northstar"));
    assert!(consumed
        .outcome
        .as_str()
        .expect("outcome note")
        .contains("state re-read"));
}

#[tokio::test]
async fn duplicate_post_returns_the_existing_job_and_rewrites_nothing() {
    let dir = TempDir::new("duplicate");
    let _env = with_jobs_env(Some("jobs-token"), Some(&dir.path));
    let app = app();
    let first = frozen_bytes(&frozen_job("opj_0000000000b2", "recheck_request"));
    let (status, _) = post_job(&app, Some("Bearer jobs-token"), &first).await;
    assert_eq!(status, StatusCode::OK);

    // The same job id with different bytes (a corrupted replay) still returns
    // the durable record and leaves the file untouched.
    let mut replay = frozen_job("opj_0000000000b2", "mystery_kind");
    replay.correlation_id = "corr-replay".to_string();
    let replay = frozen_bytes(&replay);
    assert_ne!(first, replay);
    let (status, response) = post_job(&app, Some("Bearer jobs-token"), &replay).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        response, first,
        "the existing job is returned, not the replay"
    );
    let path = dir.path.join("opj_0000000000b2.json");
    assert_eq!(std::fs::read(&path).expect("job file"), first);
    let job_files = std::fs::read_dir(&dir.path)
        .expect("dir")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .count();
    assert_eq!(job_files, 1, "one durable job for one job id");
}

#[tokio::test]
async fn the_bearer_gate_mirrors_the_live_audit_semantics() {
    let dir = TempDir::new("bearer");
    let body = frozen_bytes(&frozen_job("opj_0000000000c3", "retry_read"));
    let app = app();

    // Unset token: both routes fail closed with 503 and write nothing.
    {
        let _env = with_jobs_env(None, Some(&dir.path));
        let (status, response) = post_job(&app, Some("Bearer anything"), &body).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error_code(&response), "operator_jobs_not_configured");
        let (status, response) = get_job(&app, Some("Bearer anything"), "opj_0000000000c3").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error_code(&response), "operator_jobs_not_configured");
    }
    assert!(!dir.path.join("opj_0000000000c3.json").exists());

    // Configured-but-empty token: every request is 401, never open.
    {
        let _env = with_jobs_env(Some(""), Some(&dir.path));
        let (status, response) = post_job(&app, Some("Bearer "), &body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(error_code(&response), "unauthorized");
    }

    // A configured token: missing, wrong and wrong-scheme bearers are 401;
    // the exact bearer is accepted on both routes.
    let _env = with_jobs_env(Some("jobs-token"), Some(&dir.path));
    for authorization in [None, Some("Bearer wrong"), Some("Token jobs-token")] {
        let (status, response) = post_job(&app, authorization, &body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{authorization:?}");
        assert_eq!(error_code(&response), "unauthorized");
    }
    let (status, _) = post_job(&app, Some("Bearer jobs-token"), &body).await;
    assert_eq!(status, StatusCode::OK);
    for authorization in [None, Some("Bearer wrong")] {
        let (status, _) = get_job(&app, authorization, "opj_0000000000c3").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{authorization:?}");
    }
    let (status, response) = get_job(&app, Some("Bearer jobs-token"), "opj_0000000000c3").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response, body);
}

#[tokio::test]
async fn get_reads_the_folder_and_refuses_unknown_or_malformed_ids() {
    let dir = TempDir::new("get");
    let _env = with_jobs_env(Some("jobs-token"), Some(&dir.path));
    let app = app();
    let job = frozen_job("opj_0000000000d4", "restore_reconcile_request");
    let body = frozen_bytes(&job);
    let (status, _) = post_job(&app, Some("Bearer jobs-token"), &body).await;
    assert_eq!(status, StatusCode::OK);

    let (status, response) = get_job(&app, Some("Bearer jobs-token"), "opj_0000000000d4").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response, body, "GET returns the stored bytes");

    let (status, response) = get_job(&app, Some("Bearer jobs-token"), "opj_ffffffffffff").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&response), "not_found");

    // Malformed ids (and traversal attempts) never touch the filesystem.
    for id in [
        "job_0000000000d4",
        "opj_short",
        "..%2Fledger.json",
        "opj_00000000000Z",
    ] {
        let (status, response) = get_job(&app, Some("Bearer jobs-token"), id).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{id}");
        assert_eq!(error_code(&response), "bad_request", "{id}");
    }
}

#[tokio::test]
async fn bodies_outside_the_frozen_shape_are_refused() {
    let dir = TempDir::new("shape");
    let _env = with_jobs_env(Some("jobs-token"), Some(&dir.path));
    let app = app();
    let valid = frozen_bytes(&frozen_job("opj_0000000000e5", "retry_read"));

    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("not json", b"not json".to_vec()),
        ("empty object", b"{}".to_vec()),
        (
            "job id shape",
            frozen_bytes(&frozen_job("job_0000000000e5", "retry_read")),
        ),
        ("empty kind", {
            let mut job = frozen_job("opj_0000000000e6", "retry_read");
            job.kind = String::new();
            frozen_bytes(&job)
        }),
        ("unknown state", {
            let mut job = frozen_job("opj_0000000000e7", "retry_read");
            job.state = "reconciling".to_string();
            job.state_history[0].state = "reconciling".to_string();
            frozen_bytes(&job)
        }),
        ("empty history", {
            let mut job = frozen_job("opj_0000000000e8", "retry_read");
            job.state_history.clear();
            frozen_bytes(&job)
        }),
        ("history disagrees with state", {
            let mut job = frozen_job("opj_0000000000e9", "retry_read");
            job.state_history[0].state = "running".to_string();
            frozen_bytes(&job)
        }),
    ];
    for (label, body) in cases {
        let (status, response) = post_job(&app, Some("Bearer jobs-token"), &body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{label}");
        assert_eq!(error_code(&response), "bad_request", "{label}");
    }
    // Nothing durable was written for any refused body.
    let files = std::fs::read_dir(&dir.path)
        .expect("dir")
        .filter_map(|entry| entry.ok())
        .count();
    assert_eq!(files, 0, "refused bodies write nothing");

    let (status, _) = post_job(&app, Some("Bearer jobs-token"), &valid).await;
    assert_eq!(status, StatusCode::OK);
}

/// The split-host governance channel: the control plane's pause and policy
/// mutations must land on the console host, where the runner and the console
/// read them.
#[tokio::test]
async fn governance_receiver_writes_the_pause_register_and_the_effective_policy() {
    let dir = TempDir::new("governance");
    let interventions = dir.path.join("interventions.jsonl");
    let policy_path = dir.path.join("effective-policy.json");
    let _env = with_jobs_env(None, Some(&dir.path));
    std::env::set_var(INTERVENTIONS_ENV, &interventions);
    std::env::set_var(POLICY_ENV, &policy_path);
    let app = app();

    let body = serde_json::json!({
        "kind": "intervention",
        "record": {
            "interventionId": "itv_000000000001",
            "kind": "pause_scheduling",
            "platform": "applovin_max",
            "customerPopulation": "all",
            "reason": "integration pause",
            "expectedEffect": "no new writes",
            "actor": "ops",
            "atMs": 1_792_400_000_000_i64,
            "correlationId": "corr-1"
        }
    });
    let bytes = serde_json::to_vec(&body).unwrap();
    assert_eq!(
        send(
            &app,
            Request::builder()
                .method("POST")
                .uri("/v1/operator/governance")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(bytes.clone()))
                .unwrap()
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE,
        "an unset token closes the channel"
    );

    std::env::set_var(TOKEN_ENV, "gov-token");
    assert_eq!(
        send(
            &app,
            Request::builder()
                .method("POST")
                .uri("/v1/operator/governance")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(bytes.clone()))
                .unwrap()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );

    let (status, response) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/v1/operator/governance")
            .header(header::AUTHORIZATION, "Bearer gov-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(bytes))
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&response)
    );
    let line = std::fs::read_to_string(&interventions).expect("intervention register");
    assert!(line.contains("pause_scheduling"), "{line}");
    let pause = apex_autopilot::interventions::platform_pause(Some(&interventions), "applovin_max")
        .expect("register readable")
        .expect("platform paused");
    assert_eq!(pause.reason, "integration pause");

    // A kind the runner ignores is refused before it reaches the register.
    let bad = serde_json::json!({
        "kind": "intervention",
        "record": {"kind": "drop_tables", "atMs": 1}
    });
    let (status, _) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/v1/operator/governance")
            .header(header::AUTHORIZATION, "Bearer gov-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&bad).unwrap()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        std::fs::read_to_string(&interventions)
            .unwrap()
            .lines()
            .count(),
        1
    );

    // A valid policy lands atomically and parses.
    let policy = apex_autopilot::model::AutopilotPolicy::conservative();
    let policy_body = serde_json::json!({"kind": "policy", "policy": policy});
    let (status, response) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/v1/operator/governance")
            .header(header::AUTHORIZATION, "Bearer gov-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&policy_body).unwrap()))
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&response)
    );
    let stored: apex_autopilot::model::AutopilotPolicy =
        serde_json::from_slice(&std::fs::read(&policy_path).expect("effective policy")).unwrap();
    assert_eq!(stored.window_days, policy.window_days);

    // An out-of-bounds policy is refused and the stored file is unchanged.
    let mut outside = serde_json::to_value(&policy).unwrap();
    outside["maxStepBp"] = serde_json::json!(999_999);
    let before = std::fs::read(&policy_path).unwrap();
    let (status, _) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/v1/operator/governance")
            .header(header::AUTHORIZATION, "Bearer gov-token")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({"kind": "policy", "policy": outside}))
                    .unwrap(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(std::fs::read(&policy_path).unwrap(), before);
    std::env::remove_var(INTERVENTIONS_ENV);
    std::env::remove_var(POLICY_ENV);
}
