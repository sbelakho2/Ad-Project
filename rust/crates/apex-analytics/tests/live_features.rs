//! Live verification of the enumerated analytics features against a real
//! ClickHouse server, the real `apex-ingest` binary, and real Caddy access
//! logs.
//!
//! Gated on `APEX_CLICKHOUSE_HTTP`; each test skips with a printed reason
//! when the variable is unset. The access-log scenario additionally needs
//! `APEX_LIVE_CADDY_LOG` (a JSON access log written by a real Caddy run), and
//! the performance scenario needs `APEX_LIVE_PERF=1`.
//!
//! Every test uses its own unique database and drops it at the end. No
//! fixture is mocked: rows are built by the real event builder, inserted
//! through the real HTTP store, and read through the real readers.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_arguments
)]

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use apex_analytics::envelope::{DomainEvent, EventPayload, EventSource, EventType};
use apex_analytics::projections::{
    compare_report, compute_platform_daily, enforce_switch, publish, rebuild_batch,
    rebuild_if_changed, ComparisonStatus, ProjectionName, ProjectionRow,
};
use apex_analytics::queries::{
    AnalyticsReader, BreakdownDimension, ClickHouseReader, MemoryReader, QueryScope,
};
use apex_analytics::store::{
    ClickHouseConfig, ClickHouseStore, EventStore, MemoryStore, ProjectionStore, ReportDayRow,
    RequestLogRow, StoreError,
};
use apex_analytics::timestamp::UtcMillis;

const HORIZON: &str = "2026-10-15T09:10:00.000Z";

fn live_url() -> Option<String> {
    std::env::var("APEX_CLICKHOUSE_HTTP")
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn skip(test: &str) -> bool {
    if live_url().is_none() {
        println!("SKIP {test}: APEX_CLICKHOUSE_HTTP is unset");
        true
    } else {
        false
    }
}

fn unique_database() -> String {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("apex_live_{}_{}_{}", std::process::id(), nanos, sequence)
}

fn time(text: &str) -> UtcMillis {
    UtcMillis::parse_rfc3339(text).expect("fixture time")
}

fn store(database: &str) -> ClickHouseStore {
    ClickHouseStore::new(ClickHouseConfig {
        http_url: live_url().expect("live url"),
        database: database.to_string(),
        request_timeout_ms: 60_000,
        user: None,
        password: None,
    })
    .expect("store")
}

fn builder(event_type: EventType, source: EventSource) -> apex_analytics::DomainEventBuilder {
    DomainEvent::builder(event_type, source)
        .with_environment("production")
        .with_service_version("apex-analytics-live")
        .with_occurred_at(time(HORIZON))
        .with_received_at(time(HORIZON))
}

fn scoped(mut builder: apex_analytics::DomainEventBuilder) -> apex_analytics::DomainEventBuilder {
    builder = builder.with_publisher_id("northstar");
    builder.with_platform("applovin_max")
}

fn report_day(
    publisher: &str,
    platform: &str,
    account: &str,
    day: &str,
    finality: &str,
    revenue: Option<i64>,
    impressions: Option<u64>,
    currency: Option<&str>,
    country: Option<&str>,
    ad_unit: Option<&str>,
    format: Option<&str>,
) -> ReportDayRow {
    ReportDayRow {
        publisher_id: publisher.into(),
        platform: platform.into(),
        platform_account_id: account.into(),
        day: day.into(),
        finality: finality.into(),
        revenue_micros: revenue,
        impressions,
        currency: currency.map(str::to_string),
        country: country.map(str::to_string),
        ad_unit_id: ad_unit.map(str::to_string),
        format: format.map(str::to_string),
        source_digest: Some(format!("{:064x}", revenue.unwrap_or(0).unsigned_abs())),
    }
}

fn known_report_days() -> Vec<ReportDayRow> {
    let mut days = Vec::new();
    for day in 1..=10 {
        days.push(report_day(
            "northstar",
            "applovin_max",
            "max_acct_1",
            &format!("2026-10-{day:02}"),
            "final",
            Some(5_150_000_000),
            Some(2_500_000),
            Some("USD"),
            Some(if day <= 6 { "DE" } else { "GB" }),
            Some("rw_de_01"),
            Some("rewarded"),
        ));
    }
    // A second publisher and a second platform so operator-wide and scoped
    // totals differ, and an account that reports final and provisional rows
    // for the same day (the provisional-only rule).
    days.push(report_day(
        "orbit",
        "levelplay",
        "lp_acct_1",
        "2026-10-01",
        "final",
        Some(1_000_000_000),
        Some(100),
        Some("USD"),
        Some("US"),
        Some("lp_us_01"),
        Some("interstitial"),
    ));
    days.push(report_day(
        "orbit",
        "levelplay",
        "lp_acct_1",
        "2026-10-01",
        "provisional",
        Some(250_000_000),
        Some(25),
        Some("USD"),
        // A distinct breakdown dimension keeps the provisional re-pull
        // alongside the final row for the same account/day; the business
        // key is the full breakdown, not finality.
        Some("CA"),
        Some("lp_us_01"),
        Some("interstitial"),
    ));
    days.push(report_day(
        "orbit",
        "levelplay",
        "lp_acct_2",
        "2026-10-01",
        "provisional",
        Some(500_000_000),
        Some(50),
        Some("USD"),
        Some("CA"),
        Some("lp_ca_01"),
        Some("banner"),
    ));
    // A gap row: a known day with no amount or currency cannot enter money
    // totals and must be counted as unmapped.
    days.push(report_day(
        "northstar",
        "applovin_max",
        "max_acct_1",
        "2026-10-11",
        "provisional",
        None,
        None,
        Some("USD"),
        Some("DE"),
        Some("rw_de_01"),
        Some("rewarded"),
    ));
    days
}

fn accounts() -> Vec<apex_analytics::store::AccountRow> {
    vec![
        apex_analytics::store::AccountRow {
            publisher_id: "northstar".into(),
            account_id: "acct_north".into(),
            stage: "paying".into(),
            owner: Some("maya".into()),
            platforms: vec!["applovin_max".into()],
            first_seen: "2026-08-31".into(),
            stage_updated: "2026-10-14".into(),
        },
        apex_analytics::store::AccountRow {
            publisher_id: "orbit".into(),
            account_id: "acct_orbit".into(),
            stage: "audit_requested".into(),
            owner: Some("lina".into()),
            platforms: vec!["levelplay".into()],
            first_seen: "2026-09-02".into(),
            stage_updated: "2026-09-20".into(),
        },
    ]
}

fn events() -> Vec<DomainEvent> {
    let mut all = Vec::new();
    // Audit request accepted (August cohort).
    all.push(
        scoped(builder(EventType::AuditRequestAccepted, EventSource::Axum))
            .with_event_id("live_audit_accepted")
            .with_occurred_at(time("2026-08-31T20:00:00.000Z"))
            .with_received_at(time("2026-08-31T20:00:00.000Z"))
            .with_object("account", "acct_north")
            .with_payload(EventPayload {
                lead_id: Some("lead_live_1".into()),
                account_id: Some("acct_north".into()),
                platforms: Some(vec!["applovin_max".into()]),
                range_start: Some("2026-08-01".into()),
                range_end: Some("2026-08-31".into()),
                source_code: Some("inbound".into()),
                ..EventPayload::default()
            })
            .build()
            .expect("audit accepted"),
    );
    // Account lifecycle: accepted in August, qualified in September. The
    // cohort row for August must see this (regression for the cross-month
    // fold).
    all.push(
        scoped(builder(EventType::AccountStageChanged, EventSource::Worker))
            .with_event_id("live_stage_qualified")
            .with_occurred_at(time("2026-09-02T09:00:00.000Z"))
            .with_received_at(time("2026-09-02T09:00:00.000Z"))
            .with_reason_code("stage_move")
            .with_object("account", "acct_north")
            .with_payload(EventPayload {
                account_id: Some("acct_north".into()),
                previous_stage: Some("audit_requested".into()),
                next_stage: Some("qualified".into()),
                owner: Some("maya".into()),
                effective_at: Some("2026-09-02".into()),
                ..EventPayload::default()
            })
            .build()
            .expect("qualified"),
    );
    // Experiment lifecycle: registered, started, decided.
    all.push(
        scoped(builder(
            EventType::ExperimentRegistered,
            EventSource::Worker,
        ))
        .with_event_id("live_exp_registered")
        .with_occurred_at(time("2026-10-01T08:10:00.000Z"))
        .with_received_at(time("2026-10-01T08:10:00.000Z"))
        .with_object("experiment", "exp_live")
        .with_payload(EventPayload {
            experiment_id: Some("exp_live".into()),
            source_digest: Some(format!("{:064x}", 7)),
            registered_metric: Some("revenue_per_user".into()),
            window_days: Some(14),
            ..EventPayload::default()
        })
        .build()
        .expect("registered"),
    );
    all.push(
        scoped(builder(EventType::ExperimentStarted, EventSource::Worker))
            .with_event_id("live_exp_started")
            .with_occurred_at(time("2026-10-01T08:10:00.000Z"))
            .with_received_at(time("2026-10-01T08:11:00.000Z"))
            .with_object("experiment", "exp_live")
            .with_payload(EventPayload {
                experiment_id: Some("exp_live".into()),
                state: Some("running".into()),
                ..EventPayload::default()
            })
            .build()
            .expect("started"),
    );
    all.push(
        scoped(builder(EventType::ExperimentDecided, EventSource::Worker))
            .with_event_id("live_exp_decided")
            .with_occurred_at(time("2026-10-15T10:00:00.000Z"))
            .with_received_at(time("2026-10-15T10:00:00.000Z"))
            .with_object("experiment", "exp_live")
            .with_payload(EventPayload {
                experiment_id: Some("exp_live".into()),
                decision: Some("keep".into()),
                point_estimate_bp: Some(540),
                decision_bound_bp: Some(300),
                change_id: Some("ch_live".into()),
                ..EventPayload::default()
            })
            .build()
            .expect("decided"),
    );
    // Fee computation with the acceptance arithmetic.
    all.push(
        scoped(builder(EventType::FeeComputed, EventSource::Finance))
            .with_event_id("live_fee_computed")
            .with_object("change", "ch_live")
            .with_payload(EventPayload {
                change_id: Some("ch_live".into()),
                period: Some("2026-10".into()),
                currency: Some("USD".into()),
                source_amount_micros: Some(51_500_000_000),
                bound_bp: Some(300),
                factor_bp: Some(10_300),
                rate_bp: Some(2_000),
                active_days: Some(31),
                proof_hashes: Some(vec![format!("{:064x}", 246)]),
                ..EventPayload::default()
            })
            .build()
            .expect("fee"),
    );
    // Report pull completion (the platform report fact; the platform
    // credentials themselves are not needed to exercise the reader path).
    all.push(
        scoped(builder(
            EventType::ReportPullCompleted,
            EventSource::Connector,
        ))
        .with_event_id("live_report_pull")
        .with_occurred_at(time("2026-10-15T08:00:00.000Z"))
        .with_received_at(time("2026-10-15T08:01:00.000Z"))
        .with_object("platform_account", "max_acct_1")
        .with_payload(EventPayload {
            job_id: Some("job_report_live".into()),
            platform_operation: Some("report_pull".into()),
            range_start: Some("2026-10-01".into()),
            range_end: Some("2026-10-14".into()),
            rows: Some(14),
            finality: Some("final".into()),
            duration_ms: Some(1_200),
            ..EventPayload::default()
        })
        .build()
        .expect("report pull"),
    );
    // A public page request event (real Caddy rows produce the same shape).
    all.push(
        builder(EventType::PublicPageRequested, EventSource::AccessLog)
            .with_event_id("live_page_request")
            .with_occurred_at(time("2026-10-14T09:00:00.000Z"))
            .with_received_at(time("2026-10-14T09:00:01.000Z"))
            .with_object("document", "/pricing")
            .with_outcome("2xx")
            .with_payload(EventPayload {
                route_template: Some("/pricing".into()),
                status: Some(404),
                bot_class: Some("search_engine_bot".into()),
                query_keys: Some(vec!["utm_source".into()]),
                ..EventPayload::default()
            })
            .build()
            .expect("page request"),
    );
    // Ledger verification (analytics carries the fact and its object
    // timeline; it is not a ledger reader).
    all.push(
        scoped(builder(EventType::LedgerVerified, EventSource::Finance))
            .with_event_id("live_ledger_verified")
            .with_object("ledger", "northstar")
            .with_payload(EventPayload {
                highest_sequence: Some(42),
                verification_version: Some("v1".into()),
                duration_ms: Some(12),
                ..EventPayload::default()
            })
            .build()
            .expect("ledger verified"),
    );
    all
}

fn scope() -> QueryScope {
    QueryScope::new("northstar", time(HORIZON)).with_day_range("2026-08-01", "2026-10-15")
}

fn wide_scope() -> QueryScope {
    QueryScope::operator_wide(time(HORIZON)).with_day_range("2026-08-01", "2026-10-15")
}

async fn seed(store: &ClickHouseStore) -> Result<(), StoreError> {
    store.ensure_schema().await?;
    store.insert(&events()).await?;
    store.insert_report_days(&known_report_days()).await?;
    store.insert_accounts(&accounts()).await?;
    Ok(())
}

async fn count(store: &ClickHouseStore, query: &str) -> u64 {
    let rows = store.fetch_rows(query, &[]).await.expect("count query");
    rows.first()
        .and_then(|row| row.get("n"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

async fn drop_db(store: &ClickHouseStore) {
    store.drop_database().await.expect("drop database");
}

// ---------------------------------------------------------------------------
// 1. Real ClickHouse bootstrap, idempotency and the documented migration.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_schema_bootstrap_is_idempotent_and_the_request_logs_migration_collapses_replays() {
    if skip("schema") {
        return;
    }
    let database = unique_database();
    let store = store(&database);
    store.drop_database().await.expect("clean start");
    // A database created before the replacing engine: plain MergeTree with
    // two physical replays of one stable event id.
    store
        .execute_statement(&format!("CREATE DATABASE {database}"))
        .await
        .expect("create db");
    store
        .execute_statement(&format!(
            "CREATE TABLE {database}.request_logs (\n\
             event_id String, occurred_at_utc DateTime64(3, 'UTC'), \
             received_at_utc DateTime64(3, 'UTC'), route_template LowCardinality(String), \
             status UInt16, status_class LowCardinality(String), bot_class LowCardinality(String), \
             referrer_host Nullable(String), campaign_code Nullable(String), \
             query_keys Array(String), environment LowCardinality(String)) \
             ENGINE = MergeTree PARTITION BY toYYYYMM(occurred_at_utc) \
             ORDER BY (route_template, occurred_at_utc, event_id) \
             TTL occurred_at_utc + INTERVAL 30 DAY"
        ))
        .await
        .expect("legacy table");
    store
        .execute_statement(&format!(
            "INSERT INTO {database}.request_logs VALUES \
             ('dup1','2026-10-01 10:00:00.000','2026-10-01 10:00:01.000','/pricing',200,'2xx','unclassified',NULL,NULL,['utm_source'],'production'),\
             ('dup1','2026-10-01 10:00:00.000','2026-10-01 10:00:02.000','/pricing',200,'2xx','unclassified',NULL,NULL,['utm_source'],'production'),\
             ('dup2','2026-10-01 10:00:03.000','2026-10-01 10:00:04.000','/proof',200,'2xx','unclassified',NULL,NULL,[],'production')"
        ))
        .await
        .expect("legacy rows");
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.request_logs")
        )
        .await,
        3
    );

    // The real CLI migration (which also bootstraps the full schema).
    let output = Command::new(env!("CARGO_BIN_EXE_apex-ingest"))
        .arg("--migrate-request-logs")
        .env("APEX_CLICKHOUSE_HTTP", live_url().unwrap())
        .env("APEX_CLICKHOUSE_DB", &database)
        .output()
        .expect("run migration");
    assert!(
        output.status.success(),
        "migration failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let engine = store
        .fetch_rows(
            "SELECT engine FROM system.tables WHERE database = {db:String} AND name = 'request_logs'",
            &[("db", database.as_str())],
        )
        .await
        .expect("engine");
    assert_eq!(engine[0]["engine"], "ReplacingMergeTree");
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.request_logs FINAL")
        )
        .await,
        2,
        "the migration keeps one row per event_id"
    );
    assert_eq!(
        count(
            &store,
            &format!(
                "SELECT count() AS n FROM system.tables WHERE database = '{database}' AND name = 'request_logs__replacing'"
            )
        )
        .await,
        0
    );

    // Rerun the migration: idempotent, still one row per id.
    let output = Command::new(env!("CARGO_BIN_EXE_apex-ingest"))
        .arg("--migrate-request-logs")
        .env("APEX_CLICKHOUSE_HTTP", live_url().unwrap())
        .env("APEX_CLICKHOUSE_DB", &database)
        .output()
        .expect("rerun migration");
    assert!(output.status.success());
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.request_logs FINAL")
        )
        .await,
        2
    );

    // The full bootstrap creates every documented table and is idempotent.
    store.ensure_schema().await.expect("ensure 1");
    store.ensure_schema().await.expect("ensure 2");
    let tables = store
        .fetch_rows(
            "SELECT name, engine, sorting_key FROM system.tables \
             WHERE database = {db:String} ORDER BY name",
            &[("db", database.as_str())],
        )
        .await
        .expect("tables");
    let names: Vec<&str> = tables
        .iter()
        .filter_map(|row| row.get("name").and_then(serde_json::Value::as_str))
        .collect();
    for expected in [
        "accounts",
        "connections",
        "customer_daily",
        "decisions",
        "domain_events",
        "experiments",
        "fee_lines",
        "findings",
        "incidents",
        "invoices",
        "jobs",
        "lifecycle_cohorts",
        "operator_queues",
        "payment_events",
        "platform_daily",
        "projection_versions",
        "proposals",
        "report_days",
        "request_logs",
        "source_snapshot_index",
    ] {
        assert!(
            names.contains(&expected),
            "missing table {expected}: {names:?}"
        );
    }
    assert_eq!(names.len(), 21, "no duplicate or missing table: {names:?}");
    for row in &tables {
        assert_eq!(
            row["engine"], "ReplacingMergeTree",
            "{} must deduplicate: {row}",
            row["name"]
        );
    }
    let sorting_key = |table: &str| {
        tables
            .iter()
            .find(|row| row["name"] == table)
            .and_then(|row| row["sorting_key"].as_str())
            .expect("sorting key")
    };
    assert_eq!(sorting_key("domain_events"), "source, event_id");
    assert_eq!(
        sorting_key("request_logs"),
        "route_template, occurred_at_utc, event_id"
    );
    assert_eq!(
        sorting_key("report_days"),
        "publisher_id, platform, platform_account_id, day, country, ad_unit_id, format"
    );
    assert_eq!(
        sorting_key("customer_daily"),
        "publisher_id, day, platform, currency, as_of_version"
    );
    // Retention numbers are on the live DDL.
    let ddl = store
        .fetch_rows(
            "SELECT name, create_table_query FROM system.tables WHERE database = {db:String}",
            &[("db", database.as_str())],
        )
        .await
        .expect("ddl");
    let query_for = |table: &str| {
        ddl.iter()
            .find(|row| row["name"] == table)
            .and_then(|row| row["create_table_query"].as_str())
            .expect("table ddl")
            .to_string()
    };
    assert!(query_for("domain_events").contains("toIntervalDay(90)"));
    assert!(query_for("request_logs").contains("toIntervalDay(30)"));
    assert!(query_for("customer_daily").contains("toIntervalMonth(24)"));
    assert!(query_for("platform_daily").contains("toIntervalMonth(24)"));
    assert!(query_for("lifecycle_cohorts").contains("toIntervalMonth(24)"));
    assert!(!query_for("report_days").contains("TTL"));
    assert!(!query_for("fee_lines").contains("TTL"));
    drop_db(&store).await;
}

// ---------------------------------------------------------------------------
// 2. Real Caddy access-log ingestion through the real binary.
// ---------------------------------------------------------------------------

fn run_ingest(
    database: &str,
    log: &std::path::Path,
    cursor: &std::path::Path,
    spool: &std::path::Path,
    extra: &[&str],
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_apex-ingest"));
    command
        .arg("--json")
        .args(extra)
        .arg(log)
        .env("APEX_CLICKHOUSE_HTTP", live_url().unwrap())
        .env("APEX_CLICKHOUSE_DB", database)
        .env("APEX_INGEST_CURSOR", cursor)
        .env("APEX_INGEST_SPOOL", spool)
        .env("APEX_ENVIRONMENT", "production");
    command.output().expect("run apex-ingest")
}

#[tokio::test]
async fn live_caddy_access_log_is_ingested_incrementally_and_reruns_do_not_duplicate_events() {
    if skip("caddy-ingest") {
        return;
    }
    let Some(log_path) = std::env::var("APEX_LIVE_CADDY_LOG")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(std::path::PathBuf::from)
    else {
        println!("SKIP caddy-ingest: APEX_LIVE_CADDY_LOG is unset (run Caddy first)");
        return;
    };
    let database = unique_database();
    let store = store(&database);
    store.ensure_schema().await.expect("schema");
    let dir = std::env::temp_dir().join(format!("apex-live-ingest-{database}"));
    std::fs::create_dir_all(&dir).expect("dir");
    let cursor = dir.join("access.cursor");
    let spool = dir.join("spool.jsonl");
    let log = dir.join("access.json");

    let original = std::fs::read_to_string(&log_path).expect("caddy log");
    let lines: Vec<&str> = original
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert!(lines.len() >= 2, "a real access log needs request lines");
    // Keep a partial final line out of the first run to exercise the
    // hold-back rule, then complete it.
    let partial = lines.last().expect("last line");
    let complete: String = lines[..lines.len() - 1]
        .iter()
        .map(|line| format!("{line}\n"))
        .collect();
    std::fs::write(&log, format!("{complete}{partial}")).expect("write partial log");

    let first = run_ingest(&database, &log, &cursor, &spool, &[]);
    let first_json: serde_json::Value =
        serde_json::from_slice(&first.stdout).expect("first JSON summary");
    assert_eq!(first.status.code(), Some(0), "{first_json}");
    assert_eq!(
        first_json["lines"].as_u64(),
        Some((lines.len() - 1) as u64),
        "the unterminated final line is held back"
    );
    let first_rows = count(
        &store,
        &format!("SELECT count() AS n FROM {database}.request_logs FINAL"),
    )
    .await;
    let first_events = count(
        &store,
        &format!("SELECT count() AS n FROM {database}.domain_events FINAL"),
    )
    .await;
    assert_eq!(first_rows, (lines.len() - 1) as u64);
    assert_eq!(first_events, (lines.len() - 1) as u64);
    let cursor_after_first = std::fs::read_to_string(&cursor).expect("cursor");
    assert!(!cursor_after_first.trim().is_empty());

    // Complete the final line: only it is new.
    std::fs::write(&log, &original).expect("complete log");
    let second = run_ingest(&database, &log, &cursor, &spool, &[]);
    let second_json: serde_json::Value =
        serde_json::from_slice(&second.stdout).expect("second JSON summary");
    assert_eq!(second.status.code(), Some(0), "{second_json}");
    assert_eq!(second_json["lines"].as_u64(), Some(1));
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.request_logs FINAL")
        )
        .await,
        lines.len() as u64
    );
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.domain_events FINAL")
        )
        .await,
        lines.len() as u64
    );

    // A third run reads nothing new.
    let third = run_ingest(&database, &log, &cursor, &spool, &[]);
    let third_json: serde_json::Value =
        serde_json::from_slice(&third.stdout).expect("third JSON summary");
    assert_eq!(third.status.code(), Some(0));
    assert_eq!(third_json["lines"].as_u64(), Some(0));
    assert_eq!(third_json["request_logs"].as_u64(), Some(0));

    // A lost/rotated cursor replays the file: request_logs stays collapsed
    // (`FINAL`) because the stable event id is the row identity, and the
    // event dedup key absorbs the replay.
    let bypass = dir.join("bypass.cursor");
    let replay = run_ingest(&database, &log, &bypass, &spool, &[]);
    assert_eq!(replay.status.code(), Some(0));
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.request_logs FINAL")
        )
        .await,
        lines.len() as u64,
        "a replay collapses to the same sanitized row identity"
    );
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.domain_events FINAL")
        )
        .await,
        lines.len() as u64,
        "a replay collapses to the same (source, event_id)"
    );

    // In-place rotation: a new first line and a shorter file restart the
    // cursor instead of skipping the new prefix.
    let rotated = format!(
        "{}\n{}\n",
        caddy_line("/pricing/", 200, 1_792_056_000.5, "Mozilla/5.0"),
        caddy_line("/proof/", 200, 1_792_056_001.5, "curl/8.4.0"),
    );
    std::fs::write(&log, &rotated).expect("rotated log");
    let after_rotation = run_ingest(&database, &log, &cursor, &spool, &[]);
    assert_eq!(after_rotation.status.code(), Some(0));
    let rotated_json: serde_json::Value =
        serde_json::from_slice(&after_rotation.stdout).expect("rotation json");
    assert_eq!(rotated_json["cursor_restarted"], true);
    assert_eq!(rotated_json["lines"], 2);
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.request_logs FINAL")
        )
        .await,
        (lines.len() + 2) as u64,
        "the rotated file's lines are ingested"
    );
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.domain_events FINAL")
        )
        .await,
        (lines.len() + 2) as u64
    );

    // Redaction: no IP, raw UA or query value from the real Caddy log is in
    // the stored rows/events.
    let sample = store
        .fetch_rows(
            &format!(
                "SELECT route_template, status_class, bot_class, campaign_code, query_keys \
                 FROM {database}.request_logs FINAL LIMIT 5"
            ),
            &[],
        )
        .await
        .expect("sample");
    assert!(!sample.is_empty());
    for row in &sample {
        let text = row.to_string();
        assert!(!text.contains("127.0.0.1"), "{text}");
        assert!(!text.contains("User-Agent"), "{text}");
    }
    drop_db(&store).await;
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// 3. Real events: envelope/builder, dedup, payload stability and queries.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_events_round_trip_with_stable_payloads_and_tenant_safe_identities() {
    if skip("events") {
        return;
    }
    let database = unique_database();
    let store = store(&database);
    let fixture = events();
    store.ensure_schema().await.expect("schema");
    let first = store.insert(&fixture).await.expect("insert");
    assert_eq!(first.inserted, fixture.len() as u64);
    assert_eq!(first.duplicates_observed, None, "ClickHouse merges async");
    // Replay the exact same batch.
    store.insert(&fixture).await.expect("replay");
    let loaded = store.load_events().await.expect("load");
    assert_eq!(loaded.len(), fixture.len(), "FINAL collapses the replay");

    // Payload bytes are stable across the build, the row and the read-back.
    for event in &fixture {
        let row = event.to_row();
        let stored = loaded
            .iter()
            .find(|candidate| candidate.event_id == event.event_id)
            .expect("stored event");
        assert_eq!(stored.to_row().payload_json, row.payload_json);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&row.payload_json).expect("payload json"),
            serde_json::to_value(&event.payload).expect("payload value")
        );
    }

    // A failed and a completed job run with the same occurrence, object and
    // payload are two facts, not one (the derived identity regression).
    let completed = builder(EventType::JobRun, EventSource::Worker)
        .with_publisher_id("northstar")
        .with_occurred_at(time(HORIZON))
        .with_received_at(time(HORIZON))
        .with_outcome("completed")
        .with_payload(EventPayload {
            job_id: Some("job_collision".into()),
            duration_ms: Some(10),
            ..EventPayload::default()
        })
        .build()
        .expect("completed job");
    let failed = builder(EventType::JobRun, EventSource::Worker)
        .with_publisher_id("northstar")
        .with_occurred_at(time(HORIZON))
        .with_received_at(time(HORIZON))
        .with_outcome("failed")
        .with_payload(EventPayload {
            job_id: Some("job_collision".into()),
            duration_ms: Some(10),
            ..EventPayload::default()
        })
        .build()
        .expect("failed job");
    assert_ne!(completed.event_id, failed.event_id);
    store
        .insert(&[completed, failed])
        .await
        .expect("insert jobs");
    assert_eq!(
        store.load_events().await.expect("load jobs").len(),
        fixture.len() + 2
    );

    // The readers see the emitted facts.
    store
        .insert_report_days(&known_report_days())
        .await
        .expect("report days");
    store.insert_accounts(&accounts()).await.expect("accounts");
    let batch = rebuild_batch(&store, "events-1").await.expect("rebuild");
    let comparison = compare_report(&store, &batch).await.expect("compare");
    assert!(comparison.is_clean(), "{comparison:?}");
    publish(&store, "events-1").await.expect("publish");
    let reader = ClickHouseReader::new(store.clone());
    let scope = wide_scope();
    let lift = reader.lift_rows(&scope).await.expect("lift");
    assert_eq!(lift.len(), 1);
    assert_eq!(lift[0].experiment_id, "exp_live");
    assert_eq!(lift[0].outcome, "keep");
    assert_eq!(lift[0].observed_lift_bp, Some(540));
    let detail = reader
        .fee_detail(&scope, "ch_live")
        .await
        .expect("fee detail")
        .expect("fee present");
    assert_eq!(
        detail.attributed_increment.as_ref().map(|m| m.micros),
        Some(1_500_000_000)
    );
    assert_eq!(
        detail.fee_before_tax.as_ref().map(|m| m.micros),
        Some(300_000_000)
    );
    let timeline = reader
        .object_timeline(&scope, "ledger", "northstar")
        .await
        .expect("ledger timeline");
    assert_eq!(timeline.len(), 1);
    assert_eq!(timeline[0].event_type, EventType::LedgerVerified);
    let pages = reader
        .page_requests_daily(&scope, "2026-10-01", "2026-10-31")
        .await
        .expect("page requests");
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].requests, 1);
    assert_eq!(pages[0].not_found, 1);
    drop_db(&store).await;
}

// ---------------------------------------------------------------------------
// 4. Analytics models: independent totals and type-scoped parity.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_models_match_independently_computed_totals_and_scoped_reads_match_the_reference() {
    if skip("models") {
        return;
    }
    let database = unique_database();
    let store = store(&database);
    seed(&store).await.expect("seed");
    let batch = rebuild_batch(&store, "models-1").await.expect("rebuild");
    let comparison = compare_report(&store, &batch).await.expect("compare");
    assert!(comparison.is_clean(), "{comparison:?}");
    publish(&store, "models-1").await.expect("publish");
    let reader = ClickHouseReader::new(store.clone());
    let scope = scope();
    let all_report_days = known_report_days();
    let all_events = store.load_events().await.expect("events");
    // The reference reader folds the same source facts in memory through
    // the shared model, so a ClickHouse/scoped result that differs from it
    // is a defect in the SQL statement or its type predicate.
    let memory_store = MemoryStore::new();
    memory_store.ensure_schema().await.expect("memory schema");
    memory_store
        .insert(&all_events)
        .await
        .expect("memory events");
    memory_store
        .insert_report_days(&all_report_days)
        .await
        .expect("memory days");
    memory_store
        .insert_accounts(&accounts())
        .await
        .expect("memory accounts");
    let reference = MemoryReader::new(memory_store);

    // Revenue daily: independently sum the rows inside the scope.
    let daily = reader.revenue_daily(&scope).await.expect("daily");
    let expected_daily = reference
        .revenue_daily(&scope)
        .await
        .expect("reference daily");
    assert_eq!(daily, expected_daily, "type-scoped daily read == reference");
    let expected_northstar_total: i64 = expected_daily
        .iter()
        .filter(|row| row.platform == "applovin_max")
        .map(|row| row.revenue.micros)
        .sum();
    assert_eq!(expected_northstar_total, 51_500_000_000);
    assert_eq!(
        daily
            .iter()
            .filter(|row| row.platform == "applovin_max")
            .map(|row| row.revenue.micros)
            .sum::<i64>(),
        51_500_000_000
    );

    // Breakdowns: each dimension's totals must sum to the same money.
    for (dimension, expected_total) in [
        (BreakdownDimension::Platform, 51_500_000_000),
        (BreakdownDimension::Format, 51_500_000_000),
        (BreakdownDimension::Country, 51_500_000_000),
        (BreakdownDimension::AdUnit, 51_500_000_000),
    ] {
        let rows = reader
            .revenue_breakdown(&scope, dimension)
            .await
            .expect("breakdown");
        assert_eq!(
            rows,
            reference
                .revenue_breakdown(&scope, dimension)
                .await
                .expect("reference breakdown")
        );
        assert_eq!(
            rows.iter().map(|row| row.revenue.micros).sum::<i64>(),
            expected_total
        );
        assert!(rows.iter().all(|row| row.share_bp.is_some()));
    }
    let countries = reader
        .revenue_breakdown(&scope, BreakdownDimension::Country)
        .await
        .expect("countries");
    let de = countries
        .iter()
        .find(|row| row.dimension_value == "DE")
        .expect("DE");
    assert_eq!(de.revenue.micros, 30_900_000_000);
    assert_eq!(de.share_bp, Some(6_000));

    // Platform daily: the account with both a final and a provisional row is
    // final, not provisional-only; the second account is provisional-only.
    let platform_daily = store
        .load_projection(ProjectionName::PlatformDaily)
        .await
        .expect("platform daily");
    let orbit_oct_1 = platform_daily
        .iter()
        .find_map(|row| match row {
            ProjectionRow::PlatformDaily(row)
                if row.platform == "levelplay" && row.day == "2026-10-01" =>
            {
                Some(row)
            }
            _ => None,
        })
        .expect("orbit platform row");
    assert_eq!(orbit_oct_1.final_accounts, 1);
    assert_eq!(
        orbit_oct_1.provisional_accounts, 1,
        "lp_acct_1 has a final row, so only lp_acct_2 is provisional-only"
    );
    assert_eq!(orbit_oct_1.revenue_micros, 1_750_000_000);

    // The Rust and SQL fast paths agree on the provisional-only rule.
    let rust_platform = compute_platform_daily(&all_report_days);
    let rust_orbit = rust_platform
        .iter()
        .find(|row| row.platform == "levelplay" && row.day == "2026-10-01")
        .expect("rust platform row");
    assert_eq!(rust_orbit.final_accounts, 1);
    assert_eq!(rust_orbit.provisional_accounts, 1);

    // Fee detail arithmetic.
    let detail = reader
        .fee_detail(&scope, "ch_live")
        .await
        .expect("detail")
        .expect("fee");
    assert_eq!(detail.eligible_revenue.micros, 51_500_000_000);
    assert_eq!(detail.conservative_bound_bp, 300);
    assert_eq!(detail.agreed_rate_bp, 2_000);
    assert_eq!(
        detail.attributed_increment.as_ref().map(|m| m.micros),
        Some(1_500_000_000)
    );
    assert_eq!(
        detail.fee_before_tax.as_ref().map(|m| m.micros),
        Some(300_000_000)
    );
    assert!(detail.unavailable.is_empty());

    // Lift, landscape, effectiveness and their scoped/reference parity.
    let lift = reader.lift_rows(&scope).await.expect("lift");
    assert_eq!(
        lift,
        reference.lift_rows(&scope).await.expect("reference lift")
    );
    let landscape = reader
        .experiment_landscape(&scope)
        .await
        .expect("landscape");
    assert_eq!(
        landscape,
        reference
            .experiment_landscape(&scope)
            .await
            .expect("reference landscape")
    );
    assert!(landscape
        .iter()
        .any(|row| row.state == "keep" && row.count == 1));
    let effectiveness = reader
        .experiment_effectiveness(&scope, "2026-10")
        .await
        .expect("effectiveness");
    assert_eq!(
        effectiveness,
        reference
            .experiment_effectiveness(&scope, "2026-10")
            .await
            .expect("reference effectiveness")
    );
    assert_eq!(effectiveness.started, 1);
    assert_eq!(effectiveness.kept, 1);
    assert_eq!(effectiveness.valid_settled, 1);
    assert_eq!(effectiveness.keep_rate_valid_bp, Some(10_000));
    assert_eq!(effectiveness.completion_p50_days, Some(14));

    // Growth: the August cohort's September qualification is visible.
    let cohorts = reader.growth_cohorts(&scope).await.expect("cohorts");
    assert_eq!(
        cohorts,
        reference
            .growth_cohorts(&scope)
            .await
            .expect("reference cohorts")
    );
    let august = cohorts
        .iter()
        .find(|row| row.cohort_month == "2026-08")
        .expect("August cohort");
    assert_eq!(august.accepted, 1);
    assert_eq!(august.accepted_accounts, 1);
    assert_eq!(august.qualified, 1, "September stage movement counts");
    let funnel = reader
        .activation_funnel(&scope, "2026-08")
        .await
        .expect("funnel");
    assert_eq!(
        funnel,
        reference
            .activation_funnel(&scope, "2026-08")
            .await
            .expect("reference funnel")
    );
    assert_eq!(funnel.stages[1].accounts, 1, "qualified");

    // Data quality and freshness against independently counted rows.
    let quality = reader.data_quality(&scope).await.expect("quality");
    assert_eq!(
        quality.matched_source_days, 11,
        "11 distinct northstar days"
    );
    assert_eq!(quality.provisional_days, 1, "the October 11 re-pull");
    assert_eq!(quality.unmapped_rows, 1, "the amount-less October 11 row");
    assert_eq!(
        quality.missing_days,
        Some(65),
        "76-day range minus 11 covered days"
    );
    assert_eq!(quality.dedup_rejects, Some(0));
    assert_eq!(quality.stale_sources.len(), 1);
    let health = reader.ingestion_health(&scope).await.expect("health");
    let north = health
        .iter()
        .find(|row| row.publisher_id == "northstar")
        .expect("northstar health");
    assert_eq!(north.last_final_day.as_deref(), Some("2026-10-10"));
    assert_eq!(north.freshness_days, Some(5));
    assert_eq!(north.status.as_str(), "stale");

    // Object timeline: the ClickHouse statement now honors the same day
    // range and currency filter as the reference fold.
    let narrow =
        QueryScope::new("northstar", time(HORIZON)).with_day_range("2026-09-01", "2026-09-30");
    let memory = MemoryStore::new();
    memory.ensure_schema().await.expect("memory schema");
    memory.insert(&all_events).await.expect("memory events");
    let memory_reader = MemoryReader::new(memory);
    let ch_timeline = reader
        .object_timeline(&narrow, "experiment", "exp_live")
        .await
        .expect("ch timeline");
    let mem_timeline = memory_reader
        .object_timeline(&narrow, "experiment", "exp_live")
        .await
        .expect("mem timeline");
    assert_eq!(ch_timeline, mem_timeline, "timeline scope parity");
    assert!(
        ch_timeline.is_empty(),
        "the experiment facts are outside the September scope"
    );
    let ch_all = reader
        .object_timeline(&scope, "experiment", "exp_live")
        .await
        .expect("ch timeline all");
    assert_eq!(ch_all.len(), 3);

    drop_db(&store).await;
}

// ---------------------------------------------------------------------------
// 5. Projections: staged rebuild, compare, publish, change/skip, tamper,
//    pruning.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_projection_rebuild_compare_publish_skip_tamper_and_prune() {
    if skip("projections") {
        return;
    }
    let database = unique_database();
    let store = store(&database);
    seed(&store).await.expect("seed");
    let reader = ClickHouseReader::new(store.clone());
    let scope = scope();

    // v1: staged, compared, published.
    let v1 = rebuild_batch(&store, "proj-1").await.expect("rebuild v1");
    let comparison = compare_report(&store, &v1).await.expect("compare v1");
    assert!(comparison.is_clean(), "{comparison:?}");
    enforce_switch(&comparison).expect("switch v1");
    publish(&store, "proj-1").await.expect("publish v1");
    let visible = reader.revenue_daily(&scope).await.expect("v1 rows");
    assert_eq!(
        visible
            .iter()
            .filter(|row| row.platform == "applovin_max")
            .map(|row| row.revenue.micros)
            .sum::<i64>(),
        51_500_000_000
    );

    // A changed source rebuilds. Insert a new report day and a new event so
    // the source horizon advances (the documented event-sourced gate).
    store
        .insert_report_days(&[report_day(
            "northstar",
            "applovin_max",
            "max_acct_1",
            "2026-10-12",
            "final",
            Some(1_000_000),
            Some(1),
            Some("USD"),
            Some("DE"),
            Some("rw_de_01"),
            Some("rewarded"),
        )])
        .await
        .expect("new day");
    store
        .insert(&[builder(EventType::JobRun, EventSource::Worker)
            .with_publisher_id("northstar")
            .with_event_id("proj_advance")
            .with_occurred_at(time("2026-10-15T11:00:00.000Z"))
            .with_received_at(time("2026-10-15T11:00:00.000Z"))
            .with_outcome("completed")
            .with_payload(EventPayload {
                job_id: Some("job_advance".into()),
                duration_ms: Some(1),
                ..EventPayload::default()
            })
            .build()
            .expect("advance")])
        .await
        .expect("advance event");
    let v2 = rebuild_if_changed(&store, "proj-2")
        .await
        .expect("rebuild if changed")
        .expect("a changed source rebuilds");
    let comparison = compare_report(&store, &v2).await.expect("compare v2");
    assert!(comparison.is_clean(), "{comparison:?}");
    publish(&store, "proj-2").await.expect("publish v2");
    let visible = reader.revenue_daily(&scope).await.expect("v2 rows");
    assert_eq!(visible.len(), 11);
    assert_eq!(
        visible
            .iter()
            .filter(|row| row.platform == "applovin_max")
            .map(|row| row.revenue.micros)
            .sum::<i64>(),
        51_501_000_000
    );

    // An unchanged source skips: only the source-max and published-version
    // reads run, no projection table is read or written and no version is
    // allocated.
    store
        .execute_statement("SYSTEM FLUSH LOGS")
        .await
        .expect("flush before skip");
    let before_versions = count(
        &store,
        &format!("SELECT count() AS n FROM {database}.projection_versions"),
    )
    .await;
    let marker = store
        .fetch_rows(
            "SELECT toUInt64(toUnixTimestamp64Micro(now64(6))) AS marker",
            &[],
        )
        .await
        .expect("marker")[0]["marker"]
        .as_u64()
        .expect("marker millis");
    let skipped = rebuild_if_changed(&store, "proj-3").await.expect("skip");
    assert!(skipped.is_none(), "an unchanged source skips");
    store
        .execute_statement("SYSTEM FLUSH LOGS")
        .await
        .expect("flush after skip");
    let pattern = format!("%{database}%");
    let marker_text = marker.to_string();
    let skip_queries = store
        .fetch_rows(
            "SELECT query FROM system.query_log \
             WHERE event_time_microseconds >= fromUnixTimestamp64Micro({marker:UInt64}) AND type = 'QueryFinish' \
             AND query LIKE {pattern:String}",
            &[("pattern", pattern.as_str()), ("marker", marker_text.as_str())],
        )
        .await
        .expect("query log");
    let skip_text = skip_queries
        .iter()
        .filter_map(|row| row.get("query").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    for table in [
        "customer_daily",
        "platform_daily",
        "lifecycle_cohorts",
        "operator_queues",
    ] {
        assert!(
            !skip_text.contains(&format!("FROM {database}.{table}"))
                && !skip_text.contains(&format!("INSERT INTO {database}.{table}")),
            "the skip must not read or write {table}: {skip_text}"
        );
    }
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.projection_versions")
        )
        .await,
        before_versions,
        "a skip allocates no version row"
    );

    // A tampered staging is refused and readers stay on the published v2.
    // Stage a full batch first so every projection's newest rows belong to
    // the same batch id, then overwrite one customer_daily row: the compare
    // sees the mismatch instead of a batch-identity refusal.
    let tamper_batch = rebuild_batch(&store, "proj-tamper")
        .await
        .expect("tamper rebuild");
    let tampered = ProjectionRow::CustomerDaily(apex_analytics::projections::CustomerDailyRow {
        publisher_id: "northstar".into(),
        day: "2026-10-01".into(),
        platform: "applovin_max".into(),
        currency: "USD".into(),
        revenue_micros: 1,
        impressions: None,
        source_days: 1,
        final_days: 1,
        provisional_days: 0,
    });
    store
        .write_projection(
            ProjectionName::CustomerDaily,
            tamper_batch.as_of_version,
            "proj-tamper",
            &[tampered],
        )
        .await
        .expect("tamper");
    let report = compare_report(&store, &tamper_batch)
        .await
        .expect("the tampered batch is comparable by its own batch id");
    assert!(!report.is_clean(), "{report:?}");
    assert!(matches!(
        report.comparisons[0].status,
        ComparisonStatus::Mismatch { .. }
    ));
    assert!(enforce_switch(&report).is_err());
    let visible = reader.revenue_daily(&scope).await.expect("readers stay");
    assert_eq!(
        visible
            .iter()
            .filter(|row| row.platform == "applovin_max")
            .map(|row| row.revenue.micros)
            .sum::<i64>(),
        51_501_000_000,
        "readers never see the tampered staged version"
    );

    // Recover with a fresh rebuild/publish; superseded rows and version rows
    // are pruned.
    let v3 = rebuild_batch(&store, "proj-3").await.expect("rebuild v3");
    let comparison = compare_report(&store, &v3).await.expect("compare v3");
    assert!(comparison.is_clean(), "{comparison:?}");
    publish(&store, "proj-3").await.expect("publish v3");
    let current = v3.as_of_version;
    assert_eq!(
        count(
            &store,
            &format!(
                "SELECT count() AS n FROM {database}.customer_daily WHERE as_of_version < {current}"
            )
        )
        .await,
        0,
        "superseded projection rows are pruned"
    );
    assert_eq!(
        count(
            &store,
            &format!(
                "SELECT count() AS n FROM {database}.projection_versions \
                 WHERE projection_name = 'customer_daily' AND as_of_version < {current}"
            )
        )
        .await,
        0,
        "superseded version bookkeeping rows are pruned"
    );
    drop_db(&store).await;
}

// ---------------------------------------------------------------------------
// 6. Retention/TTL and the ledger/snapshot fact path.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn live_retention_ttl_removes_expired_raw_rows_and_keeps_financial_facts() {
    if skip("retention") {
        return;
    }
    let database = unique_database();
    let store = store(&database);
    store.ensure_schema().await.expect("schema");

    // Raw events and request logs older than their TTL are removed by a
    // merge; financial source facts have no TTL and survive.
    let old_event = builder(EventType::JobRun, EventSource::Worker)
        .with_publisher_id("northstar")
        .with_event_id("old_event")
        .with_occurred_at(UtcMillis::parse_rfc3339("2026-06-01T00:00:00Z").expect("old"))
        .with_received_at(UtcMillis::parse_rfc3339("2026-06-01T00:00:01Z").expect("old recv"))
        .with_outcome("completed")
        .with_payload(EventPayload {
            job_id: Some("job_old".into()),
            duration_ms: Some(1),
            ..EventPayload::default()
        })
        .build()
        .expect("old event");
    store.insert(&[old_event]).await.expect("old event insert");
    let old_request = RequestLogRow {
        event_id: "old_request".into(),
        occurred_at_utc: UtcMillis::parse_rfc3339("2026-06-01T00:00:00Z").expect("old"),
        received_at_utc: UtcMillis::parse_rfc3339("2026-06-01T00:00:01Z").expect("old recv"),
        route_template: "/pricing".into(),
        status: 200,
        status_class: "2xx".into(),
        bot_class: "unclassified".into(),
        referrer_host: None,
        campaign_code: None,
        query_keys: Vec::new(),
        environment: "production".into(),
    };
    store
        .insert_request_logs(&[old_request])
        .await
        .expect("old request");
    let old_day = report_day(
        "northstar",
        "applovin_max",
        "max_acct_1",
        "2025-01-01",
        "final",
        Some(1),
        Some(1),
        Some("USD"),
        None,
        None,
        None,
    );
    store.insert_report_days(&[old_day]).await.expect("old day");
    let old_fee = scoped(builder(EventType::FeeComputed, EventSource::Finance))
        .with_event_id("old_fee")
        .with_occurred_at(UtcMillis::parse_rfc3339("2025-01-01T00:00:00Z").expect("old fee"))
        .with_received_at(UtcMillis::parse_rfc3339("2025-01-01T00:00:01Z").expect("old fee recv"))
        .with_object("change", "ch_old")
        .with_payload(EventPayload {
            change_id: Some("ch_old".into()),
            period: Some("2025-01".into()),
            currency: Some("USD".into()),
            source_amount_micros: Some(1),
            bound_bp: Some(300),
            rate_bp: Some(2_000),
            active_days: Some(1),
            proof_hashes: Some(vec![format!("{:064x}", 1)]),
            ..EventPayload::default()
        })
        .build()
        .expect("old fee event");
    // The fee event itself is old too: it lives in the 90-day raw-event TTL,
    // so only its promoted fact insert path is exercised here. Financial
    // facts in their own tables (report_days, fee_lines) have no TTL.
    store.insert(&[old_fee]).await.expect("old fee event");

    store
        .execute_statement(&format!("OPTIMIZE TABLE {database}.domain_events FINAL"))
        .await
        .expect("optimize events");
    store
        .execute_statement(&format!("OPTIMIZE TABLE {database}.request_logs FINAL"))
        .await
        .expect("optimize logs");
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.domain_events")
        )
        .await,
        0,
        "the 90-day raw event TTL removes the old event"
    );
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.request_logs")
        )
        .await,
        0,
        "the 30-day request log TTL removes the old row"
    );
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.report_days")
        )
        .await,
        1,
        "financial report days carry no analytics TTL"
    );

    // The 24-month projection TTLs: an old projection row expires, a recent
    // one survives, and the cohort table follows its cohort_month.
    store
        .execute_statement(&format!(
            "INSERT INTO {database}.customer_daily VALUES              ('northstar','2024-01-01','applovin_max','USD',1,NULL,1,1,0,1),             ('northstar','2026-10-01','applovin_max','USD',1,NULL,1,1,0,1)"
        ))
        .await
        .expect("customer rows");
    store
        .execute_statement(&format!(
            "INSERT INTO {database}.platform_daily VALUES              ('applovin_max','2024-01-01','USD',1,NULL,1,0,1)"
        ))
        .await
        .expect("platform row");
    store
        .execute_statement(&format!(
            "INSERT INTO {database}.lifecycle_cohorts VALUES              ('2024-01-01','northstar','qualified',1,1)"
        ))
        .await
        .expect("cohort row");
    for table in ["customer_daily", "platform_daily", "lifecycle_cohorts"] {
        store
            .execute_statement(&format!("OPTIMIZE TABLE {database}.{table} FINAL"))
            .await
            .expect("optimize projection");
    }
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.customer_daily")
        )
        .await,
        1,
        "the 24-month customer TTL removes only the old row"
    );
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.customer_daily WHERE day = '2026-10-01'")
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.platform_daily")
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.lifecycle_cohorts")
        )
        .await,
        0
    );

    // The analytics crate carries ledger/snapshot facts (it is not a file
    // ledger reader): both objects round-trip through the timeline.
    let ledger = scoped(builder(EventType::LedgerVerified, EventSource::Finance))
        .with_event_id("ledger_live")
        .with_object("ledger", "northstar")
        .with_payload(EventPayload {
            highest_sequence: Some(9),
            verification_version: Some("v1".into()),
            duration_ms: Some(3),
            ..EventPayload::default()
        })
        .build()
        .expect("ledger");
    let snapshot = scoped(builder(EventType::AuditCompleted, EventSource::Worker))
        .with_event_id("snapshot_live")
        .with_object("snapshot", "digest_live")
        .with_payload(EventPayload {
            snapshot_digest: Some(format!("{:064x}", 9)),
            rule_version: Some("r1".into()),
            eligible_rule_groups: Some(1),
            expected_rule_groups: Some(1),
            findings_critical: Some(0),
            findings_warning: Some(0),
            findings_info: Some(0),
            missing_inputs: Some(Vec::new()),
            duration_ms: Some(5),
            ..EventPayload::default()
        })
        .build()
        .expect("snapshot");
    store.insert(&[ledger, snapshot]).await.expect("objects");
    let reader = ClickHouseReader::new(store.clone());
    let scope = wide_scope();
    assert_eq!(
        reader
            .object_timeline(&scope, "ledger", "northstar")
            .await
            .expect("ledger timeline")
            .len(),
        1
    );
    assert_eq!(
        reader
            .object_timeline(&scope, "snapshot", "digest_live")
            .await
            .expect("snapshot timeline")
            .len(),
        1
    );
    drop_db(&store).await;
}

// ---------------------------------------------------------------------------
// 7. Performance: type predicates and typed parsing on 100k events.
// ---------------------------------------------------------------------------

async fn raw_query(url: &str, sql: &str, params: &[(&str, &str)]) -> (u64, u64, u128, u64) {
    let mut request = reqwest::Client::new().post(url);
    for (name, value) in params {
        request = request.query(&[(format!("param_{name}"), value.to_string())]);
    }
    let started = Instant::now();
    let response = request
        .body(sql.to_string())
        .send()
        .await
        .expect("raw query");
    let elapsed = started.elapsed().as_micros();
    let summary: serde_json::Value = response
        .headers()
        .get("x-clickhouse-summary")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or(serde_json::Value::Null);
    let body = response.text().await.expect("raw body");
    assert!(!body.contains("DB::Exception"), "{body}");
    let number = |key: &str| {
        summary
            .get(key)
            .and_then(serde_json::Value::as_str)
            .and_then(|value| value.parse::<u64>().ok())
            .or_else(|| summary.get(key).and_then(serde_json::Value::as_u64))
            .unwrap_or(0)
    };
    let returned = body.lines().filter(|line| !line.trim().is_empty()).count() as u64;
    (number("read_rows"), number("read_bytes"), elapsed, returned)
}

#[tokio::test]
async fn live_performance_type_predicates_and_typed_parsing_on_100k_events() {
    if skip("performance") {
        return;
    }
    if std::env::var("APEX_LIVE_PERF").ok().as_deref() != Some("1") {
        println!("SKIP performance: set APEX_LIVE_PERF=1");
        return;
    }
    let database = unique_database();
    let store = store(&database);
    store.ensure_schema().await.expect("schema");
    const TOTAL: usize = 100_000;
    let started = Instant::now();
    let mut batch = Vec::with_capacity(10_000);
    for index in 0..TOTAL {
        let event_type = match index % 5 {
            0 => EventType::JobRun,
            1 => EventType::FeeComputed,
            2 => EventType::ExperimentDecided,
            3 => EventType::AuditRequestAccepted,
            _ => EventType::PublicPageRequested,
        };
        let mut payload = EventPayload {
            job_id: Some(format!("job_{index:07}")),
            duration_ms: Some(1),
            ..EventPayload::default()
        };
        let mut event_builder = builder(event_type, EventSource::Worker)
            .with_event_id(format!("perf_{index:07}"))
            .with_publisher_id(if index % 7 == 0 { "orbit" } else { "northstar" })
            .with_platform("applovin_max")
            .with_occurred_at(UtcMillis::from_unix_millis(
                1_791_000_000_000 + index as i64,
            ))
            .with_received_at(UtcMillis::from_unix_millis(
                1_791_000_000_000 + index as i64,
            ));
        match event_type {
            EventType::JobRun => {
                event_builder = event_builder.with_outcome("completed");
            }
            EventType::FeeComputed => {
                payload.change_id = Some(format!("ch_{index:07}"));
                payload.period = Some("2026-10".into());
                payload.currency = Some("USD".into());
                payload.source_amount_micros = Some(1_000);
                payload.bound_bp = Some(300);
                payload.rate_bp = Some(2_000);
                payload.active_days = Some(1);
                payload.proof_hashes = Some(vec![format!("{:064x}", index)]);
            }
            EventType::ExperimentDecided => {
                payload.experiment_id = Some(format!("exp_{index:07}"));
                payload.decision = Some("keep".into());
                payload.point_estimate_bp = Some(100);
                payload.decision_bound_bp = Some(300);
            }
            EventType::AuditRequestAccepted => {
                payload.lead_id = Some(format!("lead_{index:07}"));
                payload.platforms = Some(vec!["applovin_max".into()]);
                payload.range_start = Some("2026-10-01".into());
                payload.range_end = Some("2026-10-14".into());
                payload.source_code = Some("live".into());
            }
            EventType::PublicPageRequested => {
                payload.route_template = Some("/pricing".into());
                payload.status = Some(200);
                payload.bot_class = Some("unclassified".into());
            }
            _ => {}
        }
        batch.push(
            event_builder
                .with_payload(payload)
                .build()
                .expect("perf event"),
        );
        if batch.len() == 10_000 {
            store.insert(&batch).await.expect("perf insert");
            batch.clear();
        }
    }
    let insert_elapsed = started.elapsed();
    println!("perf: inserted {TOTAL} events in {insert_elapsed:?}");

    // The type-scoped read the fee fold uses (FeeComputed only) versus the
    // same scope without the type predicate.
    let scoped_sql = format!(
        "SELECT event_id, event_type, schema_version, occurred_at_utc, received_at_utc, \
         environment, service_version, source, publisher_id, platform, object_type, object_id, \
         correlation_id, causation_id, outcome, reason_code, payload_json \
         FROM {database}.domain_events FINAL \
         WHERE ({{publisher:String}} = '' OR publisher_id = {{publisher:String}}) \
         AND ({{from:String}} = '' OR toDate(occurred_at_utc) >= toDateOrZero({{from:String}})) \
         AND ({{to:String}} = '' OR toDate(occurred_at_utc) <= toDateOrZero({{to:String}})) \
         AND event_type IN ({{type_0:String}}) FORMAT JSONEachRow"
    );
    let unscoped_sql = scoped_sql.replace(" AND event_type IN ({type_0:String})", "");
    let url = format!("{}/", live_url().unwrap());
    let (scoped_rows, scoped_bytes, scoped_us, scoped_returned) = raw_query(
        &url,
        &scoped_sql,
        &[
            ("publisher", ""),
            ("from", ""),
            ("to", ""),
            ("type_0", "fee_computed"),
        ],
    )
    .await;
    let (unscoped_rows, unscoped_bytes, unscoped_us, unscoped_returned) = raw_query(
        &url,
        &unscoped_sql,
        &[("publisher", ""), ("from", ""), ("to", "")],
    )
    .await;
    println!(
        "perf: fee_computed type predicate returned={scoped_returned} read_rows={scoped_rows} read_bytes={scoped_bytes} {scoped_us}us"
    );
    println!(
        "perf: unscoped reference returned={unscoped_returned} read_rows={unscoped_rows} read_bytes={unscoped_bytes} {unscoped_us}us"
    );
    assert_eq!(scoped_returned, (TOTAL / 5) as u64);
    assert_eq!(unscoped_returned, TOTAL as u64);
    assert!(
        scoped_returned < unscoped_returned,
        "the type predicate reduces the rows materialized and transferred"
    );

    // Typed parsing versus the dynamic Value path on the same answer.
    let select =
        format!("SELECT * FROM {database}.domain_events FINAL WHERE event_type = 'fee_computed'");
    let typed_started = Instant::now();
    let typed = store
        .fetch_typed::<apex_analytics::EventRow>(&select, &[])
        .await
        .expect("typed");
    let typed_elapsed = typed_started.elapsed();
    let dynamic_started = Instant::now();
    let dynamic = store.fetch_rows(&select, &[]).await.expect("dynamic");
    let dynamic_elapsed = dynamic_started.elapsed();
    assert_eq!(typed.len(), dynamic.len());
    println!(
        "perf: typed parse rows={} {typed_elapsed:?}; dynamic rows={} {dynamic_elapsed:?}",
        typed.len(),
        dynamic.len()
    );

    // Report-day reader timing for a realistic window (100 report days).
    let base_day = time("2026-07-08T00:00:00.000Z").unix_millis();
    let days: Vec<ReportDayRow> = (0..100)
        .map(|index| {
            report_day(
                "northstar",
                "applovin_max",
                "max_acct_1",
                &UtcMillis::from_unix_millis(base_day + index as i64 * 86_400_000).to_day_string(),
                "final",
                Some(1_000_000),
                Some(100),
                Some("USD"),
                Some("DE"),
                Some("rw_de_01"),
                Some("rewarded"),
            )
        })
        .collect();
    store.insert_report_days(&days).await.expect("perf days");
    let _batch = rebuild_batch(&store, "perf-1").await.expect("perf rebuild");
    publish(&store, "perf-1").await.expect("perf publish");
    let daily_started = Instant::now();
    let perf_scope =
        QueryScope::operator_wide(time(HORIZON)).with_day_range("2026-07-08", "2026-10-15");
    let daily = ClickHouseReader::new(store.clone())
        .revenue_daily(&perf_scope)
        .await
        .expect("daily");
    let daily_elapsed = daily_started.elapsed();
    println!(
        "perf: report days rows={} daily read rows={} {daily_elapsed:?}",
        days.len(),
        daily.len()
    );
    assert_eq!(daily.len(), 100);
    drop_db(&store).await;
}

// ---------------------------------------------------------------------------
// 8/9. CLI surface, error paths, counters and insert stats.
// ---------------------------------------------------------------------------

fn stdin_run(args: &[&str], input: &str, envs: &[(&str, &str)]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_apex-ingest"));
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = command.spawn().expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("wait")
}

fn caddy_line(uri: &str, status: u16, ts: f64, user_agent: &str) -> String {
    serde_json::json!({
        "ts": ts,
        "status": status,
        "request": {
            "method": "GET",
            "host": "apexmediation.ee",
            "uri": uri,
            "remote_ip": "127.0.0.1",
            "headers": { "User-Agent": [user_agent] }
        }
    })
    .to_string()
}

#[tokio::test]
async fn live_cli_flags_summaries_error_paths_spool_replay_and_insert_counters() {
    if skip("cli") {
        return;
    }
    // --check never contacts a store, even with a valid URL present.
    let log = format!(
        "{}\n{}\n{}\n",
        caddy_line(
            "/pricing?utm_source=news&utm_campaign=launch",
            200,
            1_792_055_400.1,
            "Mozilla/5.0"
        ),
        caddy_line("/guides/", 200, 1_792_055_401.2, "Googlebot/2.1"),
        serde_json::json!({"ts": 1.0, "msg": "log rotated"}),
    );
    let good = stdin_run(&["--check", "--json"], &log, &[]);
    assert_eq!(good.status.code(), Some(0));
    let summary: serde_json::Value = serde_json::from_slice(&good.stdout).expect("check json");
    assert_eq!(summary["lines"], 3);
    assert_eq!(summary["request_logs"], 2);
    assert_eq!(summary["events"], 2);
    assert_eq!(summary["skipped_non_request"], 1);
    assert_eq!(summary["rejected"], 0);
    assert_eq!(summary["check_only"], true);
    assert!(summary.get("cursor_path").is_none());

    // Malformed real lines are counted and exit 3.
    let bad_log = format!(
        "{}\nnot json at all\n{}\n",
        caddy_line("/", 200, 1_792_055_400.1, "curl/8.4.0"),
        serde_json::json!({"ts": 1.0, "status": 200, "request": {"uri": "no-slash"}}),
    );
    let bad = stdin_run(&["--check", "--json"], &bad_log, &[]);
    assert_eq!(bad.status.code(), Some(3));
    let bad_summary: serde_json::Value = serde_json::from_slice(&bad.stdout).expect("bad json");
    assert_eq!(bad_summary["rejected"], 2);
    assert_eq!(bad_summary["request_logs"], 1);
    assert!(bad_summary["errors"].as_array().expect("errors").len() >= 2);

    // Usage errors exit 2.
    for args in [
        vec!["--nonsense"],
        vec!["--spool"],
        vec!["--migrate-request-logs", "--json"],
        vec!["--migrate-request-logs", "/tmp/access.json"],
    ] {
        let output = stdin_run(&args, "", &[]);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
    // A migration with an invalid database name refuses before writing.
    let invalid = stdin_run(
        &["--migrate-request-logs"],
        "",
        &[
            ("APEX_CLICKHOUSE_HTTP", &live_url().unwrap()),
            ("APEX_CLICKHOUSE_DB", "bad name"),
        ],
    );
    assert_eq!(invalid.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("configuration"));

    // Delivery into the real server, then a spool replay whose progress
    // notice must not corrupt `--json` stdout.
    let database = unique_database();
    let store = store(&database);
    store.ensure_schema().await.expect("schema");
    let dir = std::env::temp_dir().join(format!("apex-live-cli-{database}"));
    std::fs::create_dir_all(&dir).expect("dir");
    let spool = dir.join("spool.jsonl");
    let log_path = dir.join("access.json");
    std::fs::write(&log_path, &log).expect("write log");
    let first = stdin_run(
        &["--json", log_path.to_str().expect("path")],
        "",
        &[
            ("APEX_CLICKHOUSE_HTTP", &live_url().unwrap()),
            ("APEX_CLICKHOUSE_DB", &database),
            ("APEX_INGEST_SPOOL", spool.to_str().expect("spool")),
            (
                "APEX_INGEST_CURSOR",
                dir.join("cursor").to_str().expect("cursor"),
            ),
        ],
    );
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let delivered: serde_json::Value =
        serde_json::from_slice(&first.stdout).expect("delivery json");
    assert_eq!(delivered["lines"], 3);
    assert_eq!(delivered["delivered"], 2);
    assert_eq!(delivered["events_delivered"], 2);
    assert_eq!(delivered["spooled"], 0);

    // Craft a spool row (the row shape the writer spools on a store
    // refusal) and run with --json; stdout must stay one JSON object.
    let spool_row = apex_analytics::store::RequestLogRow {
        event_id: "spooled_row".into(),
        occurred_at_utc: time("2026-10-14T09:00:00.000Z"),
        received_at_utc: time("2026-10-14T09:00:01.000Z"),
        route_template: "/proof".into(),
        status: 200,
        status_class: "2xx".into(),
        bot_class: "unclassified".into(),
        referrer_host: None,
        campaign_code: None,
        query_keys: Vec::new(),
        environment: "production".into(),
    };
    std::fs::write(
        &spool,
        format!(
            "{}\n",
            serde_json::to_string(&spool_row).expect("spool row")
        ),
    )
    .expect("spool");
    let second = stdin_run(
        &["--json", log_path.to_str().expect("path")],
        "",
        &[
            ("APEX_CLICKHOUSE_HTTP", &live_url().unwrap()),
            ("APEX_CLICKHOUSE_DB", &database),
            ("APEX_INGEST_SPOOL", spool.to_str().expect("spool")),
            (
                "APEX_INGEST_CURSOR",
                dir.join("cursor").to_str().expect("cursor"),
            ),
        ],
    );
    assert_eq!(second.status.code(), Some(0));
    let replayed: serde_json::Value =
        serde_json::from_slice(&second.stdout).expect("stdout must stay JSON");
    assert_eq!(replayed["lines"], 0);
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("spool replay"),
        "the replay notice is on stderr"
    );
    assert_eq!(
        count(
            &store,
            &format!("SELECT count() AS n FROM {database}.request_logs FINAL")
        )
        .await,
        3,
        "the replayed row landed"
    );

    // Insert counters: the summary's delivered/events_delivered match the
    // stored rows; the store reports ClickHouse's honest unknown duplicate
    // count.
    let stored_events = count(
        &store,
        &format!("SELECT count() AS n FROM {database}.domain_events FINAL"),
    )
    .await;
    assert_eq!(stored_events, 3, "two Caddy lines plus the spool row");
    let outcome = store.insert(&[]).await.expect("empty insert");
    assert_eq!(outcome.inserted, 0);
    let memory = MemoryStore::new();
    memory.ensure_schema().await.expect("memory schema");
    let one = events().remove(0);
    let outcome = memory
        .insert(std::slice::from_ref(&one))
        .await
        .expect("memory insert");
    assert_eq!(outcome.inserted, 1);
    assert_eq!(outcome.duplicates_observed, Some(0));

    // Server-side insert accounting is visible in query_log.
    store
        .execute_statement("SYSTEM FLUSH LOGS")
        .await
        .expect("flush");
    let insert_pattern = format!("%{database}%");
    let inserts = store
        .fetch_rows(
            "SELECT count() AS n, sum(written_rows) AS written FROM system.query_log \
             WHERE type = 'QueryFinish' AND query_kind = 'Insert' \
             AND query LIKE {pattern:String}",
            &[("pattern", insert_pattern.as_str())],
        )
        .await
        .expect("insert log");
    let written = inserts[0]["written"].as_u64().unwrap_or(0);
    assert!(written >= 3, "insert stats recorded: {inserts:?}");

    // A real store refusal spools the sanitized rows and advances the
    // cursor; replaying after the store is repaired delivers them exactly
    // once. The broken table keeps the replacing engine so the bootstrap
    // does not migrate it away.
    let broken_database = unique_database();
    let broken = self::store(&broken_database);
    broken.ensure_schema().await.expect("broken schema");
    broken
        .execute_statement(&format!("DROP TABLE {broken_database}.request_logs"))
        .await
        .expect("drop good logs");
    broken
        .execute_statement(&format!(
            "CREATE TABLE {broken_database}.request_logs (event_id UInt64)              ENGINE = ReplacingMergeTree ORDER BY event_id"
        ))
        .await
        .expect("broken logs");
    let broken_dir = std::env::temp_dir().join(format!("apex-live-spool-{broken_database}"));
    std::fs::create_dir_all(&broken_dir).expect("broken dir");
    let broken_log = broken_dir.join("access.json");
    std::fs::write(
        &broken_log,
        format!(
            "{}\n{}\n",
            caddy_line("/pricing/", 200, 1_792_055_500.1, "Mozilla/5.0"),
            caddy_line("/proof/", 200, 1_792_055_501.2, "curl/8.4.0"),
        ),
    )
    .expect("broken log");
    let broken_cursor = broken_dir.join("cursor");
    let broken_spool = broken_dir.join("spool.jsonl");
    let refused = run_ingest(
        &broken_database,
        &broken_log,
        &broken_cursor,
        &broken_spool,
        &[],
    );
    assert_eq!(refused.status.code(), Some(1), "a refused batch exits 1");
    let refused_json: serde_json::Value =
        serde_json::from_slice(&refused.stdout).expect("refused json");
    assert_eq!(refused_json["request_logs"], 2);
    assert_eq!(refused_json["delivered"], 0);
    assert_eq!(refused_json["spooled"], 2);
    assert!(
        std::fs::read_to_string(&broken_spool)
            .expect("spool")
            .lines()
            .count()
            == 2
    );
    broken
        .execute_statement(&format!("DROP TABLE {broken_database}.request_logs"))
        .await
        .expect("drop broken logs");
    let replayed_refused = run_ingest(
        &broken_database,
        &broken_log,
        &broken_cursor,
        &broken_spool,
        &[],
    );
    assert_eq!(replayed_refused.status.code(), Some(0));
    let replayed_json: serde_json::Value =
        serde_json::from_slice(&replayed_refused.stdout).expect("replayed json");
    assert_eq!(
        replayed_json["lines"], 0,
        "the cursor advanced past the spooled lines"
    );
    assert!(String::from_utf8_lossy(&replayed_refused.stderr).contains("spool replay: 2 delivered"));
    assert_eq!(
        count(
            &broken,
            &format!("SELECT count() AS n FROM {broken_database}.request_logs FINAL")
        )
        .await,
        2
    );
    assert_eq!(
        count(
            &broken,
            &format!("SELECT count() AS n FROM {broken_database}.domain_events FINAL")
        )
        .await,
        2
    );
    drop_db(&broken).await;
    let _ = std::fs::remove_dir_all(&broken_dir);

    drop_db(&store).await;
    let _ = std::fs::remove_dir_all(&dir);
}
