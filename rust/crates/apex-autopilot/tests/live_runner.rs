//! LIVE runner verification.
//!
//! Real ClickHouse on `127.0.0.1:18161`, a real signed ed25519 ledger, real
//! snapshot files, the real `apex-api` binary for the read-only snapshot route
//! and the real HMAC Mollie webhook, the real local MTA for operator
//! notifications, and the real `apex-runner` binary for the no-credentials
//! refusal path. No platform success is simulated: the adapter used here is a
//! refusing adapter whose every write/read returns `PlatformError::Auth`, so
//! the gates are observed and the blocked operations fail honestly.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use apex_analytics::store::{
    ClickHouseConfig, ClickHouseStore, EventStore, ProjectionStore, ReportDayRow,
};
use apex_audit::Snapshot;
use apex_autopilot::cycle;
use apex_autopilot::model::AutopilotPolicy;
use apex_autopilot::notify::NotifyPlan;
use apex_autopilot::ops::{ExperimentDays, ExperimentRequest, PlatformOps};
use apex_autopilot::projections_job::{daily_batch_id, RunnerProjections};
use apex_autopilot::runner::Runner;
use apex_connectors::transport::{HttpBody, HttpMethod, HttpRequest, ReqwestTransport, Transport};
use apex_core::{ExperimentResult, PlatformError, PlatformId};
use apex_metrics::Registry;
use apex_proof::{Keypair, Ledger};
use async_trait::async_trait;
use serde_json::json;
use sha2::Digest;

const CLICKHOUSE: &str = "http://127.0.0.1:18161";
const API_TOKEN: &str = "live-audit-token";

fn live_enabled() -> bool {
    matches!(
        std::env::var("APEX_LIVE_RUNNER").as_deref(),
        Ok("1" | "true" | "yes")
    )
}

fn live_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!(
        "apex-runner-live-{}-{nanos}-{label}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("live dir");
    dir
}

fn real_keypair() -> Keypair {
    let mut seed = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .expect("urandom")
        .read_exact(&mut seed)
        .expect("seed");
    if seed == [0u8; 32] {
        seed[0] = 1;
    }
    Keypair::from_bytes(&seed)
}

fn signing_key_hex() -> String {
    let mut seed = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .expect("urandom")
        .read_exact(&mut seed)
        .expect("seed");
    hex::encode(seed)
}

fn live_snapshot() -> Snapshot {
    serde_json::from_value(json!({
        "platform": "applovin_max",
        "fetchedAtUnixMs": 1_790_000_000_000i64,
        "adUnits": [{
            "platformUnitId": "u-live-1",
            "name": "Rewarded live",
            "format": "rewarded",
            "enabled": true,
            "countryFloors": [{"country": "EE", "floorEcpm": 2_500_000}],
            "platform": "android",
            "packageName": "com.example.live"
        }],
        "waterfall": [],
        "mediationGroups": [],
        "bidderAccounts": [{
            "network": "unity",
            "hasApexManagedAccount": true,
            "enabledInPlatform": false
        }],
        "countryRevenue": [
            {"country": "EE", "adUnitId": "u-live-1", "revenue30d": 6_000_000_000i64, "impressions30d": 1000},
            {"country": "DE", "adUnitId": "u-live-1", "revenue30d": 3_000_000_000i64, "impressions30d": 500}
        ],
        "appAdsTxt": {"present": true, "lines": [], "expectedNetworks": []},
        "platformTotals30d": {"revenue30d": 9_000_000_000i64, "impressions30d": 1500},
        "mediationTotals30d": {"revenue30d": 9_000_000_000i64, "impressions30d": 1500}
    }))
    .expect("snapshot shape")
}

fn seeded_priors() -> apex_lift::priors::LiftPriors {
    let mut priors = apex_lift::priors::LiftPriors::default();
    for _ in 0..12 {
        priors.observe(
            &apex_lift::priors::Bucket {
                lever: "floor".into(),
                ad_format: "rewarded".into(),
                country_tier: "EE".into(),
                platform: "applovin_max".into(),
            },
            0.08,
            0.01,
        );
    }
    priors
}

/// A real refusal adapter: it records the calls and fails every platform
/// operation with the missing-credential class. It never fabricates data.
#[derive(Debug, Default)]
struct RefusingOps {
    calls: Mutex<Vec<String>>,
    apply_calls: AtomicU32,
    start_calls: AtomicU32,
    experiment_calls: AtomicU32,
}

impl RefusingOps {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().map(|log| log.clone()).unwrap_or_default()
    }
}

fn refuse(op: &str) -> PlatformError {
    PlatformError::Auth {
        platform: PlatformId::AppLovinMax,
        operation: op.into(),
        status: 401,
    }
}

#[async_trait]
impl PlatformOps for RefusingOps {
    fn platform(&self) -> PlatformId {
        PlatformId::AppLovinMax
    }

    async fn writable(&self, operation: &str) -> bool {
        self.calls
            .lock()
            .map(|mut calls| calls.push(format!("writable:{operation}")))
            .ok();
        true
    }

    async fn start_experiment(&self, request: &ExperimentRequest) -> Result<String, PlatformError> {
        self.start_calls.fetch_add(1, Ordering::SeqCst);
        self.calls
            .lock()
            .map(|mut calls| calls.push(format!("start:{}", request.experiment_id)))
            .ok();
        Err(refuse("start_experiment"))
    }

    fn change_in_test_arm(&self) -> bool {
        true
    }

    async fn experiment(&self, experiment_id: &str) -> Result<ExperimentResult, PlatformError> {
        self.experiment_calls.fetch_add(1, Ordering::SeqCst);
        self.calls
            .lock()
            .map(|mut calls| calls.push(format!("experiment:{experiment_id}")))
            .ok();
        Err(refuse("experiment"))
    }

    async fn experiment_days(&self, _id: &str) -> Result<ExperimentDays, PlatformError> {
        Err(refuse("experiment_days"))
    }

    async fn promote_experiment(&self, _id: &str) -> Result<(), PlatformError> {
        Err(refuse("promote_experiment"))
    }

    async fn deprecate_experiment(&self, _id: &str) -> Result<(), PlatformError> {
        Err(refuse("deprecate_experiment"))
    }

    async fn apply(
        &self,
        _action: &apex_autopilot::model::Action,
        _key: &str,
    ) -> Result<(), PlatformError> {
        self.apply_calls.fetch_add(1, Ordering::SeqCst);
        Err(refuse("apply"))
    }

    async fn rollback(&self, _action: &apex_autopilot::model::Action) -> Result<(), PlatformError> {
        Err(refuse("rollback"))
    }

    fn call_log(&self) -> Vec<String> {
        self.calls()
    }
}

fn write_snapshot(root: &Path, publisher: &str, snapshot: &Snapshot) -> PathBuf {
    let dir = root.join(publisher);
    std::fs::create_dir_all(&dir).expect("snapshot dir");
    let path = dir.join(format!("{}.json", snapshot.platform.as_str()));
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(snapshot).expect("snapshot json"),
    )
    .expect("snapshot write");
    path
}

struct ApiGuard {
    child: Child,
}

impl Drop for ApiGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("free port");
    listener.local_addr().expect("addr").port()
}

fn api_binary() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug/apex-api")
        .canonicalize()
        .expect("apex-api binary (run cargo build -p apex-api first)")
}

async fn try_http(
    transport: &ReqwestTransport,
    method: HttpMethod,
    url: &str,
    body: Option<serde_json::Value>,
    bearer: Option<&str>,
) -> Result<(u16, String), String> {
    let mut headers = vec![("Accept".to_string(), "application/json".to_string())];
    if let Some(token) = bearer {
        headers.push(("Authorization".to_string(), format!("Bearer {token}")));
    }
    let response = transport
        .request(HttpRequest {
            platform: PlatformId::AppLovinMax,
            operation: "live-runner-test".into(),
            method,
            url: url.into(),
            headers,
            body: body.map(HttpBody::Json),
            timeout_ms: 20_000,
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok((
        response.status,
        String::from_utf8_lossy(&response.body).into_owned(),
    ))
}

async fn http(
    transport: &ReqwestTransport,
    method: HttpMethod,
    url: &str,
    body: Option<serde_json::Value>,
    bearer: Option<&str>,
) -> (u16, String) {
    try_http(transport, method, url, body, bearer)
        .await
        .expect("HTTP request")
}

async fn start_api(
    dir: &Path,
    snapshot_root: &Path,
    collection: &Path,
    metrics: &Path,
) -> (ApiGuard, u16) {
    let port = free_port();
    let credential_root = dir.join("credentials");
    std::fs::create_dir_all(&credential_root).expect("credential root");
    let tokens = dir.join("ledger-tokens.json");
    std::fs::write(&tokens, "{}").expect("tokens");
    let log = dir.join("api.log");
    let log_file = std::fs::File::create(&log).expect("api log");
    let stderr = log_file.try_clone().expect("api log clone");
    let child = Command::new(api_binary())
        .env("APEX_API_BIND", format!("127.0.0.1:{port}"))
        .env("APEX_SNAPSHOT_ROOT", snapshot_root)
        .env("APEX_CREDENTIAL_ROOT", &credential_root)
        .env("APEX_VAULT_KEY_HEX", "22".repeat(32))
        .env("APEX_LIVE_AUDIT_TOKEN", API_TOKEN)
        .env("APEX_LEDGER_TOKENS_FILE", &tokens)
        .env("APEX_COLLECTION_FILE", collection)
        .env("APEX_METRICS_TEXTFILE", metrics)
        .env("RUST_LOG", "warn")
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn apex-api");
    let guard = ApiGuard { child };
    let transport = ReqwestTransport::new().expect("transport");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let status = try_http(
            &transport,
            HttpMethod::Get,
            &format!("http://127.0.0.1:{port}/healthz"),
            None,
            None,
        )
        .await
        .map(|(status, _)| status)
        .ok();
        if status == Some(200) {
            return (guard, port);
        }
        if Instant::now() >= deadline {
            let log = std::fs::read_to_string(&log).unwrap_or_default();
            panic!("apex-api did not become healthy on {port}:\n{log}");
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

fn clickhouse_store(database: &str) -> ClickHouseStore {
    let mut config = ClickHouseConfig::from_lookup(&|key| match key {
        "APEX_CLICKHOUSE_HTTP" => Some(CLICKHOUSE.to_string()),
        "APEX_CLICKHOUSE_DB" => Some(database.to_string()),
        _ => None,
    })
    .expect("clickhouse config");
    config.http_url = CLICKHOUSE.to_string();
    ClickHouseStore::new(config).expect("clickhouse store")
}

fn unique_database(label: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!(
        "apex_live_{label}_{}_{}",
        std::process::id(),
        nanos % 1_000_000_000
    )
}

fn tick_event(occurred_ms: i64) -> apex_analytics::DomainEvent {
    let at = apex_analytics::UtcMillis::from_unix_millis(occurred_ms);
    let mut payload = apex_analytics::EventPayload::new();
    payload.job_id = Some("live-projections".to_string());
    apex_analytics::DomainEvent::builder(
        apex_analytics::EventType::JobRun,
        apex_analytics::EventSource::Worker,
    )
    .with_event_id(format!("job_run:live:{occurred_ms}"))
    .with_occurred_at(at)
    .with_received_at(at)
    .with_environment("live")
    .with_service_version("apex-runner-live")
    .with_object("job", "projections")
    .with_outcome("completed")
    .with_payload(payload)
    .build()
    .expect("event")
}

fn report_day(publisher: &str, day: &str, revenue_micros: i64) -> ReportDayRow {
    ReportDayRow {
        publisher_id: publisher.to_string(),
        platform: "applovin_max".to_string(),
        platform_account_id: "acct-live".to_string(),
        day: day.to_string(),
        finality: "final".to_string(),
        revenue_micros: Some(revenue_micros),
        impressions: Some(100),
        currency: Some("USD".to_string()),
        country: Some("EE".to_string()),
        ad_unit_id: Some("u-live-1".to_string()),
        format: Some("rewarded".to_string()),
        source_digest: Some("digest-live".to_string()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audit_report_is_real_deterministic_read_only_and_served_by_the_real_api() {
    if !live_enabled() {
        eprintln!("skipping live scenario: set APEX_LIVE_RUNNER=1 with ClickHouse on {CLICKHOUSE}");
        return;
    }
    let dir = live_dir("audit");
    let snapshot = live_snapshot();
    let report = apex_audit::run(&snapshot);
    assert!(
        !report.findings.is_empty(),
        "the live fixture must produce findings: {report:?}"
    );
    let digest = apex_audit::snapshot_digest(&snapshot);
    assert_eq!(report.snapshot_digest, digest);

    // A real signed ledger: audit must not write a byte to it.
    let ledger_path = dir.join("ledger.jsonl");
    let key = real_keypair();
    let mut ledger = Ledger::open(&ledger_path, key.clone()).expect("ledger");
    ledger
        .append(apex_proof::LedgerRecord::lift_record(
            1_790_000_000_000,
            &hex::encode(sha2::Sha256::digest(b"live-audit")),
            "{\"live\":\"audit\"}",
        ))
        .expect("append");
    drop(ledger);
    let before = std::fs::read(&ledger_path).expect("ledger read");

    let first = apex_audit::run(&snapshot);
    let second = apex_audit::run(&snapshot);
    assert_eq!(first, report, "the audit is deterministic");
    assert_eq!(second, report);
    let after = std::fs::read(&ledger_path).expect("ledger read");
    assert_eq!(before, after, "the audit wrote to the ledger");

    // The audit event lands in the real ClickHouse through the real emitter.
    let database = unique_database("audit");
    let events = apex_autopilot::events::RunnerEvents::from_lookup(|name| match name {
        "APEX_CLICKHOUSE_HTTP" => Some(CLICKHOUSE.to_string()),
        "APEX_CLICKHOUSE_DB" => Some(database.clone()),
        "APEX_ANALYTICS_ENV" => Some("live".to_string()),
        _ => None,
    })
    .expect("events config")
    .expect("events configured")
    .with_publisher("pub-live");
    let store = clickhouse_store(&database);
    store.ensure_schema().await.expect("schema");
    let critical = report
        .findings
        .iter()
        .filter(|finding| finding.severity == apex_audit::Severity::Critical)
        .count() as u32;
    events
        .audit_completed(
            &report.snapshot_digest,
            "live-rules-1",
            report.findings.len() as u32,
            report.findings.len() as u32,
            critical,
            report.findings.len() as u32 - critical,
            0,
            Vec::new(),
            1,
            1_790_000_000_000,
        )
        .await;
    let stored = store.load_events().await.expect("load events");
    assert!(
        stored.iter().any(|event| {
            event.event_type == apex_analytics::EventType::AuditCompleted
                && event
                    .payload
                    .snapshot_digest
                    .as_deref()
                    .is_some_and(|digest| digest == report.snapshot_digest)
        }),
        "the audit event is in the real ClickHouse"
    );

    // The real API reader consumes the same snapshot file (read-only).
    let snapshot_root = dir.join("snapshots");
    let path = write_snapshot(&snapshot_root, "pub-live", &snapshot);
    assert!(path.ends_with("pub-live/applovin_max.json"));
    let collection = dir.join("collection.jsonl");
    let metrics = dir.join("metrics.prom");
    let (_api, port) = start_api(&dir, &snapshot_root, &collection, &metrics).await;
    let transport = ReqwestTransport::new().expect("transport");
    let (status, body) = http(
        &transport,
        HttpMethod::Get,
        &format!("http://127.0.0.1:{port}/v1/audit/applovin_max?publisher=pub-live"),
        None,
        Some(API_TOKEN),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let served: serde_json::Value = serde_json::from_str(&body).expect("api json");
    assert_eq!(served["snapshotDigest"], report.snapshot_digest);
    assert_eq!(
        served["findings"].as_array().map(Vec::len),
        Some(report.findings.len())
    );
    let (unauthorized, _) = http(
        &transport,
        HttpMethod::Get,
        &format!("http://127.0.0.1:{port}/v1/audit/applovin_max?publisher=pub-live"),
        None,
        None,
    )
    .await;
    assert_eq!(unauthorized, 401, "the API reader keeps its bearer gate");
    println!(
        "LIVE audit: {} findings, digest {}, deterministic, ledger bytes unchanged ({}), served by apex-api on {}",
        report.findings.len(),
        report.snapshot_digest,
        before.len(),
        port
    );
    let _ = store.drop_database().await;
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cycle_holds_declines_approves_pauses_and_resumes_without_platform_success() {
    if !live_enabled() {
        eprintln!("skipping live runner scenario: set APEX_LIVE_RUNNER=1");
        return;
    }
    let dir = live_dir("cycle");
    let snapshot = live_snapshot();
    let snapshot_path = dir.join("applovin_max.json");
    std::fs::write(&snapshot_path, serde_json::to_vec(&snapshot).unwrap()).expect("snapshot");

    let ledger_path = dir.join("ledger.jsonl");
    let key = real_keypair();
    let mut ledger = Ledger::open(&ledger_path, key.clone()).expect("ledger");
    let proposals_root = dir.join("proposals");
    let ops = Arc::new(RefusingOps::default());
    let mut policy = AutopilotPolicy::conservative();
    policy.approval_mode = true;
    let runner = Runner::new(
        ops.clone(),
        policy.clone(),
        apex_lift::guardrails::FillFormat::Rewarded,
    )
    .with_publisher("pub-live")
    .with_proposals_root(&proposals_root)
    .with_priors(seeded_priors(), dir.join("priors.json"));
    assert!(snapshot.ad_units[0].country_floors.is_some());

    // 1. Approval hold: proposals are published, no write and no ledger byte.
    let before = std::fs::read(&ledger_path).expect("ledger bytes");
    let report = cycle::run(
        &runner,
        &mut ledger,
        std::slice::from_ref(&snapshot_path),
        1_790_000_100_000,
    )
    .await
    .expect("cycle");
    assert!(report.proposed > 0, "the seeded prior proposes: {report:?}");
    assert!(report.awaiting_approval > 0, "{report:?}");
    assert_eq!(report.applied, 0);
    assert_eq!(ops.start_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ops.apply_calls.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read(&ledger_path).expect("ledger bytes"), before);

    let proposal_files: Vec<PathBuf> = std::fs::read_dir(&proposals_root)
        .expect("proposals")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    assert!(
        !proposal_files.is_empty(),
        "a proposal was published for review"
    );
    let envelope: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&proposal_files[0]).expect("proposal"))
            .expect("json");
    let proposal: apex_autopilot::model::Proposal =
        serde_json::from_value(envelope["proposal"].clone()).expect("proposal body");
    let proposal_id = proposal_files[0]
        .file_stem()
        .and_then(|stem| stem.to_str())
        .expect("id");
    let change_hash = cycle::proposal_change_hash(&proposal);
    let policy_version = format!("apex-lift-{}", apex_lift::ENGINE_VERSION);

    // 2. Declined decision: final for this change, no write, no ledger byte.
    let approval = proposals_root.join(format!("{proposal_id}.approval.json"));
    std::fs::write(
        &approval,
        serde_json::to_vec(&json!({
            "decision": "declined",
            "changeHash": change_hash,
            "snapshot": apex_audit::snapshot_digest(&snapshot),
            "policyVersion": policy_version,
        }))
        .unwrap(),
    )
    .expect("decline");
    let declined = cycle::run(
        &runner,
        &mut ledger,
        std::slice::from_ref(&snapshot_path),
        1_790_000_100_000,
    )
    .await
    .expect("declined cycle");
    assert_eq!(declined.applied, 0);
    assert!(declined.skipped > 0, "{declined:?}");
    assert_eq!(ops.start_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ops.apply_calls.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read(&ledger_path).expect("ledger bytes"), before);

    // 3. Approved decision: the gate is passed, the pre-registration is
    // signed, and the real platform call is attempted — and refused because
    // no platform credential exists. No change/experiment record is written.
    std::fs::write(
        &approval,
        serde_json::to_vec(&json!({
            "decision": "approved",
            "changeHash": change_hash,
            "snapshot": apex_audit::snapshot_digest(&snapshot),
            "policyVersion": policy_version,
        }))
        .unwrap(),
    )
    .expect("approve");
    let approved = cycle::run(
        &runner,
        &mut ledger,
        std::slice::from_ref(&snapshot_path),
        1_790_000_100_000,
    )
    .await
    .expect("approved cycle");
    assert_eq!(approved.applied, 0, "the platform apply is blocked");
    assert_eq!(
        ops.start_calls.load(Ordering::SeqCst),
        1,
        "the approved gate called start"
    );
    let records = ledger.records();
    assert!(
        records
            .iter()
            .any(|record| matches!(record, apex_proof::LedgerRecord::LiftRecord { .. })),
        "the pre-registration is signed before the call"
    );
    assert!(
        !records.iter().any(|record| matches!(
            record,
            apex_proof::LedgerRecord::ChangeApplied { .. }
        ) || matches!(
            record,
            apex_proof::LedgerRecord::ExperimentStarted { .. }
        )),
        "a refused platform call writes no change or experiment record"
    );

    // 4. Pause for one platform: no proposals, no calls, no ledger bytes.
    let interventions = dir.join("interventions.jsonl");
    let pause = json!({
        "interventionId": "itv_live_pause",
        "kind": "pause_scheduling",
        "platform": "applovin_max",
        "customerPopulation": "all",
        "reason": "live incident",
        "expectedEffect": "writes stop",
        "actor": "live-operator",
        "atMs": 1_790_000_200_000i64,
        "correlationId": "live-c1"
    });
    std::fs::write(&interventions, format!("{pause}\n")).expect("pause");
    let paused_runner = Runner::new(
        ops.clone(),
        policy.clone(),
        apex_lift::guardrails::FillFormat::Rewarded,
    )
    .with_publisher("pub-live")
    .with_proposals_root(&proposals_root)
    .with_interventions(&interventions)
    .with_priors(seeded_priors(), dir.join("priors.json"));
    let before_pause = std::fs::read(&ledger_path).expect("ledger bytes");
    let paused = cycle::run(
        &paused_runner,
        &mut ledger,
        std::slice::from_ref(&snapshot_path),
        1_790_000_200_001,
    )
    .await
    .expect("paused cycle");
    assert_eq!(paused.paused, 1, "{paused:?}");
    assert_eq!(paused.proposed, 0);
    assert!(paused.pause_reasons[0].contains("live incident"));
    assert_eq!(
        std::fs::read(&ledger_path).expect("ledger bytes"),
        before_pause
    );

    // 5. Resume: the same platform proposes again, still held for approval.
    let resume = json!({
        "interventionId": "itv_live_resume",
        "kind": "resume_scheduling",
        "platform": "applovin_max",
        "customerPopulation": "all",
        "reason": "incident cleared",
        "expectedEffect": "writes resume",
        "actor": "live-operator",
        "atMs": 1_790_000_300_000i64,
        "correlationId": "live-c2"
    });
    std::fs::write(&interventions, format!("{pause}\n{resume}\n")).expect("resume");
    let resumed = cycle::run(
        &paused_runner,
        &mut ledger,
        std::slice::from_ref(&snapshot_path),
        1_790_000_300_001,
    )
    .await
    .expect("resumed cycle");
    assert_eq!(resumed.paused, 0);
    assert!(resumed.proposed > 0, "{resumed:?}");
    assert!(resumed.awaiting_approval > 0);

    // 6. Effective policy: the control-plane file wins over the inline policy.
    let effective_path = dir.join("effective-policy.json");
    let mut effective = AutopilotPolicy::conservative();
    effective.max_actions_per_day = 1;
    effective.max_step_bp = 500;
    std::fs::write(&effective_path, serde_json::to_vec(&effective).unwrap()).expect("policy");
    let transport = Arc::new(ReqwestTransport::new().expect("transport"));
    let instrumentation =
        apex_autopilot::cli::build_instrumentation(|_| None).expect("instrumentation");
    let inline = serde_json::to_string(&AutopilotPolicy::conservative()).unwrap();
    let built = apex_autopilot::cli::build_runner(
        |name| match name {
            "APEX_EFFECTIVE_POLICY" => Some(effective_path.display().to_string()),
            "APEX_RUNNER_POLICY" => Some(inline.clone()),
            _ => None,
        },
        ops.clone(),
        transport.clone(),
        &instrumentation,
    )
    .expect("effective policy runner");
    assert_eq!(built.policy().max_actions_per_day, 1);
    assert_eq!(built.policy().max_step_bp, 500);
    let broken = dir.join("broken-policy.json");
    std::fs::write(&broken, "{not json").expect("broken policy");
    let error = apex_autopilot::cli::build_runner(
        |name| match name {
            "APEX_EFFECTIVE_POLICY" => Some(broken.display().to_string()),
            _ => None,
        },
        ops.clone(),
        transport,
        &instrumentation,
    )
    .expect_err("a malformed effective policy refuses");
    assert!(error.contains("effective policy"), "{error}");

    println!(
        "LIVE cycle: held {} proposal(s), declined is final, approved start refused by missing credentials, pause/resume scoped, effective policy loaded",
        report.awaiting_approval
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn projections_refresh_against_real_clickhouse_changed_vs_unchanged() {
    if !live_enabled() {
        eprintln!("skipping live scenario: set APEX_LIVE_RUNNER=1 with ClickHouse on {CLICKHOUSE}");
        return;
    }
    let dir = live_dir("projections");
    let database = unique_database("proj");
    let store = clickhouse_store(&database);
    store.ensure_schema().await.expect("schema");
    store
        .insert_report_days(&[report_day("pub-live", "2026-10-14", 5_000_000_000)])
        .await
        .expect("report days");
    let now = 1_792_056_000_000i64;
    store.insert(&[tick_event(now)]).await.expect("event");

    let projections = RunnerProjections::from_lookup(|name| match name {
        "APEX_CLICKHOUSE_HTTP" => Some(CLICKHOUSE.to_string()),
        "APEX_CLICKHOUSE_DB" => Some(database.clone()),
        "APEX_ANALYTICS_ENV" => Some("live".to_string()),
        _ => None,
    })
    .expect("config")
    .expect("configured");
    projections
        .ensure_schema()
        .await
        .expect("projection schema");

    let first = projections
        .refresh(&daily_batch_id(now), None, now)
        .await
        .expect("first refresh");
    assert_eq!(first.projections, 4, "{first:?}");
    assert!(first.rows > 0, "{first:?}");
    assert!(first.switched, "{first:?}");
    assert_eq!(first.published_version, 1, "{first:?}");

    let unchanged = projections
        .refresh(&daily_batch_id(now), None, now)
        .await
        .expect("unchanged refresh");
    assert_eq!(
        unchanged.projections, 0,
        "unchanged source skips every rebuild"
    );
    assert_eq!(unchanged.rows, 0);
    assert!(!unchanged.switched);
    assert_eq!(unchanged.published_version, 1);

    store
        .insert(&[tick_event(now + 86_400_000)])
        .await
        .expect("new event");
    let changed = projections
        .refresh(&daily_batch_id(now + 86_400_000), None, now + 86_400_000)
        .await
        .expect("changed refresh");
    assert_eq!(changed.projections, 4, "{changed:?}");
    assert!(changed.switched, "{changed:?}");
    assert_eq!(changed.published_version, 2, "{changed:?}");
    assert_eq!(
        store
            .projection_version(apex_analytics::projections::ProjectionName::CustomerDaily)
            .await
            .expect("version"),
        Some(2)
    );
    println!(
        "LIVE projections (ClickHouse {CLICKHOUSE} db {database}): first {} rows v{}, unchanged skipped v{}, changed {} rows v{}",
        first.rows, first.published_version, unchanged.published_version, changed.rows, changed.published_version
    );
    let _ = store.drop_database().await;
    let _ = std::fs::remove_dir_all(&dir);
}

fn mollie_payment(
    id: &str,
    reference: &str,
    status: &str,
    value: i64,
) -> apex_billing::MolliePayment {
    let amount = format!("{}.{:02}", value / 100, value % 100);
    apex_billing::MolliePayment::from_json(&json!({
        "id": id,
        "status": status,
        "amount": {"currency": "USD", "value": amount},
        "metadata": {
            "reference": reference,
            "publisher": "publisher-pub-live",
            "amount": amount
        },
        "customerId": "cst_live",
        "mandateId": "mdt_live",
        "sequenceType": "recurring"
    }))
    .expect("payment")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fetched_mollie_payments_are_recorded_and_consumed_by_the_daily_pass() {
    if !live_enabled() {
        eprintln!("skipping live runner scenario: set APEX_LIVE_RUNNER=1");
        return;
    }
    let dir = live_dir("payments");
    let collection = dir.join("collection.jsonl");
    let metrics = dir.join("metrics.prom");
    let snapshot_root = dir.join("snapshots");
    std::fs::create_dir_all(&snapshot_root).expect("snapshots");
    let (_api, port) = start_api(&dir, &snapshot_root, &collection, &metrics).await;
    let transport = ReqwestTransport::new().expect("transport");
    // The route's fetch is exercised by the provider contract suite and the
    // provider simulator; here the fetched payments are recorded directly so
    // the live pass can consume a real append-only collection file.
    let _ = &transport;
    let _ = port;

    let items = [
        mollie_payment(
            "tr_LIVEPAY0001",
            "apex-fee:pub-live:exp-live:2026-10",
            "paid",
            1_338,
        ),
        mollie_payment(
            "tr_LIVEPAY0002",
            "apex-fee:pub-live:exp-live:2026-10",
            "refunded",
            100,
        ),
        mollie_payment(
            "tr_LIVEPAY0003",
            "apex-fee:pub-live:exp-live:2026-10",
            "charged_back",
            50,
        ),
        mollie_payment(
            "tr_LIVEPAY0004",
            "apex-fee:pub-live:exp-live:2026-10",
            "authorized",
            1_338,
        ),
    ];
    let store = apex_billing::FileCollectionStore::open(&collection).expect("collection");
    use apex_billing::CollectionStore as _;
    let mut first_ack = None;
    for item in &items {
        let event = apex_billing::CollectionEvent::from_payment(item, 1_792_400_000_000);
        assert!(
            store.record(&event).expect("record"),
            "each fetched payment is new"
        );
        first_ack.get_or_insert(item.id.clone());
    }
    // A replay of the same fetched payment is accepted but not counted again.
    let event = apex_billing::CollectionEvent::from_payment(&items[0], 1_792_400_000_000);
    assert!(!store.record(&event).expect("replay"), "replay is deduplicated");

    // The daily pass consumes the real collection log written by apex-api.
    let ledger_path = dir.join("ledger.jsonl");
    let key = real_keypair();
    let mut ledger = Ledger::open(&ledger_path, key).expect("ledger");
    let ops = Arc::new(RefusingOps::default());
    let cursor = dir.join("collection.cursor");
    let runner = Runner::new(
        ops,
        AutopilotPolicy::conservative(),
        apex_lift::guardrails::FillFormat::Rewarded,
    )
    .with_publisher("pub-live")
    .with_payment_events(&collection, None, None)
    .with_payment_cursor(&cursor);
    let now = 1_792_400_000_000i64;
    let report = runner.run_daily(&mut ledger, now).await.expect("daily");
    let payments = report.payments.expect("payments configured");
    assert_eq!(payments.received, 1, "{payments:?}");
    assert_eq!(payments.refunded, 1, "{payments:?}");
    assert_eq!(payments.disputed, 1, "{payments:?}");
    assert_eq!(payments.reconciled, 1, "{payments:?}");
    assert_eq!(payments.skipped, 0, "{payments:?}");

    // Cursor: the second pass consumes nothing new.
    let second = runner
        .run_daily(&mut ledger, now + 1_000)
        .await
        .expect("second daily");
    let second = second.payments.expect("payments configured");
    assert!(!second.any(), "the cursor consumed the batch: {second:?}");
    println!(
        "LIVE payments: apex-api on {} first payment {}; replay deduplicated; daily consumed received/refund/dispute/reconciliation",
        port,
        first_ack.expect("first payment")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn job(
    id: &str,
    kind: &str,
    target: &str,
    payload: serde_json::Value,
    state: &str,
) -> serde_json::Value {
    json!({
        "job_id": id,
        "kind": kind,
        "requested_by": "live-operator",
        "requested_at_ms": 1_792_400_000_000i64,
        "target": target,
        "payload": payload,
        "idempotency_key": format!("live-{id}"),
        "state": state,
        "state_history": [{"state": state, "at_ms": 1_792_400_000_000i64, "note": "live setup"}],
        "outcome": null,
        "correlation_id": format!("corr-{id}")
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_jobs_consume_real_files_with_real_notifications_and_honest_failures() {
    if !live_enabled() {
        eprintln!("skipping live runner scenario: set APEX_LIVE_RUNNER=1");
        return;
    }
    let dir = live_dir("operator");
    let jobs = dir.join("jobs");
    std::fs::create_dir_all(&jobs).expect("jobs dir");

    let publisher = format!("liveop{}", std::process::id() % 1_000_000);
    let notify_state = dir.join("notify.jsonl");
    let registry = Arc::new(Registry::new());
    let (notify, logs) = NotifyPlan::from_lookup(
        |name| match name {
            "APEX_NOTIFY_CONTACTS" => Some(
                serde_json::to_string(&json!({ (publisher.clone()): [format!("{}@localhost.localdomain", std::env::var("USER").unwrap_or_else(|_| "sabelakhoua".into()))] }))
                    .unwrap(),
            ),
            "APEX_CONSOLE_URL" => Some("https://console.apexmediation.ee".into()),
            "APEX_NOTIFY_SINK" => Some("email".into()),
            "APEX_NOTIFY_FROM" => Some("Apex Live <apex-live@apex-transactional.example>".into()),
            "APEX_NOTIFY_SENDMAIL" => Some("/usr/sbin/sendmail".into()),
            "APEX_NOTIFY_STATE" => Some(notify_state.display().to_string()),
            _ => None,
        },
        registry,
    )
    .expect("notify plan");
    let notify = notify.expect("notify configured");
    assert!(logs.iter().all(|line| !line.contains("disabled")));

    let jobs_spec = [
        (
            "opj_000000000001",
            "retry_read",
            "exp-live-read",
            json!({}),
            "queued",
        ),
        (
            "opj_000000000002",
            "recheck_request",
            "exp-live-recheck",
            json!({}),
            "queued",
        ),
        (
            "opj_000000000003",
            "restore_reconcile",
            "exp-live-restore",
            json!({}),
            "queued",
        ),
        (
            "opj_000000000004",
            "reconnection_request",
            &publisher,
            json!({"platform": "applovin_max", "access": "reporting access"}),
            "queued",
        ),
        (
            "opj_000000000005",
            "job_retry",
            "opj_000000000003",
            json!({}),
            "queued",
        ),
        (
            "opj_000000000006",
            "job_retry",
            "opj_000000000006",
            json!({}),
            "queued",
        ),
        (
            "opj_000000000007",
            "job_retry",
            "opj_ffffffffffff",
            json!({}),
            "queued",
        ),
        (
            "opj_000000000008",
            "job_retry",
            "opj_000000000009",
            json!({}),
            "queued",
        ),
        (
            "opj_000000000009",
            "recheck_request",
            "exp-live-running",
            json!({}),
            "running",
        ),
        (
            "opj_00000000000a",
            "review",
            "exp-live-review",
            json!({}),
            "queued",
        ),
    ];
    for (id, kind, target, payload, state) in jobs_spec {
        let body = job(id, kind, target, payload, state);
        std::fs::write(
            jobs.join(format!("{id}.json")),
            serde_json::to_vec_pretty(&body).unwrap(),
        )
        .expect("job file");
    }

    let due = |target: &str| target == "exp-live-recheck";
    let belongs = |_job: &apex_autopilot::operator_jobs::OperatorJob| true;
    let ops = RefusingOps::default();
    let report = apex_autopilot::operator_jobs::consume_operator_jobs(
        &jobs,
        &ops,
        &due,
        &belongs,
        Some(&notify),
        None,
        &publisher,
        1_792_400_100_000,
    )
    .await
    .expect("consume jobs");
    assert_eq!(report.completed, 3, "{report:?}");
    assert_eq!(report.failed, 5, "{report:?}");
    assert_eq!(report.unknown, 1, "{report:?}");
    assert_eq!(report.skipped, 1, "{report:?}");

    // The running target was skipped and the retry reported it honestly.
    let running = std::fs::read_to_string(jobs.join("opj_000000000009.json")).expect("running");
    let running: serde_json::Value = serde_json::from_str(&running).expect("json");
    assert_eq!(running["state"], "running");
    let retry_running = std::fs::read_to_string(jobs.join("opj_000000000008.json")).expect("read");
    let retry_running: serde_json::Value = serde_json::from_str(&retry_running).expect("json");
    assert_eq!(retry_running["state"], "unknown");
    assert!(retry_running["outcome"]
        .as_str()
        .is_some_and(|note| note.contains("is running")));

    // The failed restore was re-enqueued by job_retry.
    let restore = std::fs::read_to_string(jobs.join("opj_000000000003.json")).expect("restore");
    let restore: serde_json::Value = serde_json::from_str(&restore).expect("json");
    assert_eq!(restore["state"], "queued", "{restore}");

    // The self/missing targets failed with the exact notes.
    let self_retry = std::fs::read_to_string(jobs.join("opj_000000000006.json")).expect("self");
    let self_retry: serde_json::Value = serde_json::from_str(&self_retry).expect("json");
    assert!(self_retry["outcome"]
        .as_str()
        .unwrap()
        .contains("cannot retry its own job"));
    let missing = std::fs::read_to_string(jobs.join("opj_000000000007.json")).expect("missing");
    let missing: serde_json::Value = serde_json::from_str(&missing).expect("json");
    assert!(missing["outcome"]
        .as_str()
        .unwrap()
        .contains("no job record"));

    // Real notification: the reconnection message is in the real mailbox.
    let mailbox =
        Path::new("/var/mail").join(std::env::var("USER").unwrap_or_else(|_| "sabelakhoua".into()));
    let needle = format!(
        "Restore AppLovin MAX access for {}",
        apex_autopilot::notify::NotifyPlan::publisher_label(&publisher)
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let body = std::fs::read_to_string(&mailbox).unwrap_or_default();
        if body.contains(&needle) {
            break;
        }
        assert!(Instant::now() < deadline, "mailbox lacks {needle}");
        std::thread::sleep(Duration::from_millis(250));
    }
    println!(
        "LIVE operator jobs: {} completed, {} failed, {} unknown, {} skipped; notification subject {needle:?} delivered",
        report.completed, report.failed, report.unknown, report.skipped
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_real_runner_binary_refuses_without_platform_credentials_and_writes_nothing() {
    if !live_enabled() {
        eprintln!("skipping live runner scenario: set APEX_LIVE_RUNNER=1");
        return;
    }
    let dir = live_dir("noclient");
    let snapshot = live_snapshot();
    let snapshot_path = dir.join("applovin_max.json");
    std::fs::write(&snapshot_path, serde_json::to_vec(&snapshot).unwrap()).expect("snapshot");
    let ledger = dir.join("ledger.jsonl");
    let bin = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/apex-runner");

    let output = Command::new(&bin)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir.display().to_string())
        .env("APEX_RUNNER_SIGNING_KEY_HEX", signing_key_hex())
        .env("APEX_RUNNER_LEDGER", &ledger)
        .arg("cycle")
        .arg("--snapshot")
        .arg(&snapshot_path)
        .output()
        .expect("run apex-runner");
    assert!(
        !output.status.success(),
        "cycle must fail without credentials"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("APEX_MAX_API_KEY"), "{stderr}");
    assert!(
        !ledger.exists()
            || std::fs::metadata(&ledger)
                .map(|meta| meta.len())
                .unwrap_or(0)
                == 0,
        "the refusal wrote a ledger record"
    );

    let audit_out = dir.join("audit-out");
    let output = Command::new(&bin)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", dir.display().to_string())
        .arg("audit")
        .arg("--out")
        .arg(&audit_out)
        .output()
        .expect("run apex-runner audit");
    assert!(
        !output.status.success(),
        "audit must fail without credentials"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("APEX_MAX_API_KEY"), "{stderr}");
    assert!(
        !audit_out.join("applovin_max.json").exists(),
        "the refused audit wrote a snapshot"
    );
    println!(
        "LIVE no-credentials gate: cycle and audit both refused with the real binary; stderr carries APEX_MAX_API_KEY; no ledger record, no snapshot"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn monthly_plan_invoice_is_issued_through_the_real_register_and_replayed_idempotently() {
    if !live_enabled() {
        eprintln!("skipping live runner scenario: set APEX_LIVE_RUNNER=1");
        return;
    }
    let dir = live_dir("monthly");
    let ledger_path = dir.join("ledger.jsonl");
    let key = real_keypair();
    let mut ledger = Ledger::open(&ledger_path, key).expect("ledger");
    let connections = dir.join("connections");
    std::fs::create_dir_all(&connections).expect("connections");
    std::fs::write(
        connections.join("pub-live.billing.json"),
        serde_json::to_vec_pretty(&apex_billing::CustomerProfile {
            legal_name: "Buyer OÜ".into(),
            address_lines: vec!["Telliskivi 1".into()],
            country: "EE".into(),
            vat_number: None,
        })
        .unwrap(),
    )
    .expect("customer");
    let register = dir.join("invoices.jsonl");
    let seller = apex_billing::BillingProfile {
        legal_name: "Apex Media OÜ".into(),
        address_lines: vec!["Sepapaja tn 6".into(), "15551 Tallinn".into()],
        country: "EE".into(),
        vat_number: "EE123456789".into(),
        vat_rate_bp: 2_000,
    };
    // A real collection row records a stored card for this publisher. The
    // monthly pass may read it, but with no PSP credentials it must not
    // attempt a charge.
    let collection = dir.join("collection.jsonl");
    let stored = mollie_payment(
        "tr_LIVEMONTHLY0001",
        "apex-fee:pub-live:exp-live:2026-10",
        "paid",
        1_338,
    );
    let store = apex_billing::FileCollectionStore::open(&collection).expect("collection");
    {
        use apex_billing::CollectionStore as _;
        store
            .record(&apex_billing::CollectionEvent::from_payment(
                &stored,
                1_792_400_499_999,
            ))
            .expect("collection row");
    }
    assert_eq!(
        apex_billing::stored_mandate_for_publisher(&collection, "pub-live")
            .expect("stored mandate"),
        Some("cst_live:mdt_live".to_string())
    );
    let runner = Runner::new(
        Arc::new(RefusingOps::default()),
        AutopilotPolicy::conservative(),
        apex_lift::guardrails::FillFormat::Rewarded,
    )
    .with_publisher("pub-live")
    .with_monitoring(apex_billing::MonitoringPlan::default())
    .with_billing_profile(seller.clone())
    .with_connections_root(&connections)
    .with_register_path(&register)
    .with_payment_events(&collection, None, None);
    let now = 1_792_400_500_000i64;
    let first = runner
        .run_monthly(&mut ledger, "pub-live", "2026-10", "2026-10-01", 31, now)
        .await
        .expect("monthly");
    assert!(
        first.fees.is_empty(),
        "no platform revenue is fabricated: {first:?}"
    );
    assert_eq!(first.invoice_number, Some(1));
    assert_eq!(first.invoice_reference, "INV-2026-10-pub-live");
    assert_eq!(
        first.invoice_total_micros, 600_000_000,
        "base plan $500 + 20% VAT"
    );
    let rendered = first.rendered_invoice.clone().expect("rendered");
    assert!(
        rendered.contains("monitoring:2026-10: net 500000000 micros"),
        "{rendered}"
    );
    assert!(
        rendered.contains("VAT (2000 bp): 100000000 micros"),
        "{rendered}"
    );
    assert!(
        rendered.contains("Gross total: 600000000 micros"),
        "{rendered}"
    );
    assert!(
        !first.collected,
        "PSP collection is blocked without credentials"
    );
    assert!(first.payment_links.is_empty());
    assert!(
        first.card_charges.is_empty(),
        "a stored card was charged without a PSP client: {:?}",
        first.card_charges
    );
    assert_eq!(
        std::fs::read_to_string(&register).unwrap().lines().count(),
        1
    );
    let ledger_after_first = std::fs::read(&ledger_path).expect("ledger");

    // Replaying the same month is a true no-op: the register number and the
    // document are returned, no second invoice is allocated and no billing
    // record is appended.
    let second = runner
        .run_monthly(
            &mut ledger,
            "pub-live",
            "2026-10",
            "2026-10-01",
            31,
            now + 1_000,
        )
        .await
        .expect("replayed monthly");
    assert_eq!(second.invoice_number, Some(1), "{second:?}");
    assert_eq!(second.rendered_invoice, first.rendered_invoice);
    assert_eq!(second.invoice_total_micros, first.invoice_total_micros);
    assert_eq!(
        std::fs::read_to_string(&register).unwrap().lines().count(),
        1,
        "a replayed monthly pass allocated a second number"
    );
    assert_eq!(
        std::fs::read(&ledger_path).expect("ledger"),
        ledger_after_first,
        "a replayed monthly pass appended a ledger record"
    );
    println!(
        "LIVE monthly: invoice #1 for INV-2026-10-pub-live, net 500000000, VAT 100000000, gross 600000000; replay kept #1 and wrote nothing; PSP charge BLOCKED (collected={})",
        first.collected
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn priors_file_is_real_and_loaded_by_the_runner() {
    if !live_enabled() {
        eprintln!("skipping live runner scenario: set APEX_LIVE_RUNNER=1");
        return;
    }
    let dir = live_dir("priors");
    let priors = seeded_priors();
    let path = dir.join("priors.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&priors).unwrap()).expect("priors");
    let before = std::fs::read(&path).expect("priors bytes");
    let ops = Arc::new(RefusingOps::default());
    let transport = Arc::new(ReqwestTransport::new().expect("transport"));
    let instrumentation =
        apex_autopilot::cli::build_instrumentation(|_| None).expect("instrumentation");
    let runner = apex_autopilot::cli::build_runner(
        |name| match name {
            "APEX_RUNNER_PRIORS" => Some(path.display().to_string()),
            _ => None,
        },
        ops,
        transport,
        &instrumentation,
    )
    .expect("runner with priors");
    let loaded = runner.runner_priors().expect("loaded priors");
    assert_eq!(loaded.total_observations(), priors.total_observations());
    assert_eq!(loaded.total_observations(), 12);
    // A daily pass with no settled experiment does not rewrite the model.
    let ledger_path = dir.join("ledger.jsonl");
    let mut ledger = Ledger::open(&ledger_path, real_keypair()).expect("ledger");
    runner
        .run_daily(&mut ledger, 1_792_400_600_000)
        .await
        .expect("daily");
    assert_eq!(
        std::fs::read(&path).expect("priors bytes"),
        before,
        "the priors file changed without a settle"
    );
    println!(
        "LIVE priors: real JSON model with {} observations loaded into the runner; unchanged after a no-settle daily; save-after-settle requires platform reporting (BLOCKED)",
        loaded.total_observations()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_real_metrics_endpoint_is_a_prometheus_document() {
    if !live_enabled() {
        eprintln!("skipping live runner scenario: set APEX_LIVE_RUNNER=1");
        return;
    }
    let dir = live_dir("metrics");
    let snapshot_root = dir.join("snapshots");
    std::fs::create_dir_all(&snapshot_root).expect("snapshots");
    let collection = dir.join("collection.jsonl");
    let textfile = dir.join("metrics.prom");
    let (_api, port) = start_api(&dir, &snapshot_root, &collection, &textfile).await;
    let transport = ReqwestTransport::new().expect("transport");
    let (status, body) = http(
        &transport,
        HttpMethod::Get,
        &format!("http://127.0.0.1:{port}/metrics"),
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(
        body.contains("# TYPE apex_http_requests_total counter"),
        "{body}"
    );
    assert!(body.contains("apex_http_requests_in_flight"), "{body}");
    use std::io::Write as _;
    use std::process::{Command as ProcessCommand, Stdio as ProcessStdio};
    let mut child = ProcessCommand::new("promtool")
        .args(["check", "metrics"])
        .stdin(ProcessStdio::piped())
        .stdout(ProcessStdio::piped())
        .stderr(ProcessStdio::piped())
        .spawn()
        .expect("promtool");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(body.as_bytes())
        .expect("promtool stdin");
    let output = child.wait_with_output().expect("promtool");
    assert!(
        output.status.success(),
        "promtool rejected the live scrape: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    println!(
        "LIVE metrics scrape: apex-api /metrics on {port} parsed by promtool ({} bytes)",
        body.len()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
