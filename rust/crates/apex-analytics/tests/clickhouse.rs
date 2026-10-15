//! Real ClickHouse integration: schema, insert, dedup replay, queries, the
//! staged projection flow (rebuild -> compare -> publish) with its reader
//! visibility guarantees, the acceptance known values (US$ 51,500 / 3
//! percent / US$ 300), the 30-day `request_logs` table, and the
//! `rebuild_sql` fast-path parity with the portable path (including the
//! incident latest-state rule).
//!
//! Gated on `APEX_CLICKHOUSE_HTTP`: when it is set the tests run against the
//! real server (start it with `make clickhouse-dev` from the repository
//! root) using a unique test database that is dropped at the end; when it is
//! unset each test prints a clear SKIP and passes.
//!
//! The same fixture feeds a [`MemoryStore`] and the real server, and the two
//! readers must return identical read models; the server-side
//! `INSERT ... SELECT` rebuild path must agree with the portable totals.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use apex_analytics::envelope::{DomainEvent, EventPayload, EventSource, EventType};
use apex_analytics::projections::{
    compare, compare_report, compute_customer_daily, compute_operator_queues, enforce_switch,
    publish, rebuild_batch, rebuild_sql, totals_sql, ComparisonStatus, ProjectionName,
    ProjectionRow, ProjectionTotals,
};
use apex_analytics::queries::{
    BreakdownDimension, ClickHouseReader, MemoryReader, QueryScope, RevenueDailyRow,
};
use apex_analytics::store::{
    ClickHouseConfig, ClickHouseStore, EventStore, MemoryStore, ProjectionStore, ReportDayRow,
    RequestLogRow,
};
use apex_analytics::timestamp::UtcMillis;
use apex_analytics::AnalyticsReader;

const CH_ENV: &str = "APEX_CLICKHOUSE_HTTP";
const HORIZON: &str = "2026-10-15T09:10:00.000Z";

fn time(text: &str) -> UtcMillis {
    UtcMillis::parse_rfc3339(text).expect("fixture time")
}

fn unique_database() -> String {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!(
        "apex_analytics_test_{}_{}_{}",
        std::process::id(),
        nanos,
        sequence
    )
}

fn builder(event_type: EventType, source: EventSource) -> apex_analytics::DomainEventBuilder {
    DomainEvent::builder(event_type, source)
        .with_environment("production")
        .with_service_version("apex-0.1.0")
        .with_occurred_at(time(HORIZON))
        .with_received_at(time(HORIZON))
}

fn scoped(mut builder: apex_analytics::DomainEventBuilder) -> apex_analytics::DomainEventBuilder {
    builder = builder.with_publisher_id("northstar");
    builder.with_platform("applovin_max")
}

/// The acceptance known-value report rows: US$ 51,500.00 over ten final
/// days of October 2026 (5,150.00 each), DE six days and GB four.
fn report_days() -> Vec<ReportDayRow> {
    (1..=10)
        .map(|day| ReportDayRow {
            publisher_id: "northstar".into(),
            platform: "applovin_max".into(),
            platform_account_id: "max_acct_1".into(),
            day: format!("2026-10-{day:02}"),
            finality: "final".into(),
            revenue_micros: Some(5_150_000_000),
            impressions: Some(2_500_000),
            currency: Some("USD".into()),
            country: Some(if day <= 6 { "DE" } else { "GB" }.into()),
            ad_unit_id: Some("rw_de_01".into()),
            format: Some("rewarded".into()),
            source_digest: Some(format!("{:064x}", day)),
        })
        .collect()
}

/// The acceptance known-value fee: 51,500.00 at a 3 percent bound and the
/// agreed 20 percent rate gives 1,500.00 attributed and 300.00 fee.
fn fee_computed_known_value() -> DomainEvent {
    let payload = EventPayload {
        change_id: Some("ch_0246".into()),
        period: Some("2026-10".into()),
        currency: Some("USD".into()),
        source_amount_micros: Some(51_500_000_000),
        bound_bp: Some(300),
        factor_bp: Some(10_300),
        rate_bp: Some(2_000),
        active_days: Some(31),
        proof_hashes: Some(vec![format!("{:064x}", 246)]),
        ..EventPayload::default()
    };
    scoped(builder(EventType::FeeComputed, EventSource::Finance))
        .with_event_id("evt_fee_computed")
        .with_payload(payload)
        .with_object("change", "ch_0246")
        .build()
        .expect("fee_computed")
}

/// One incident transition, `minutes_ago` before the fixture horizon.
fn incident_event(
    event_id: &str,
    incident_id: &str,
    minutes_ago: i64,
    outcome: &str,
    state: &str,
    owner: &str,
) -> DomainEvent {
    let occurred =
        UtcMillis::from_unix_millis(time(HORIZON).unix_millis() - minutes_ago * 60 * 1_000);
    scoped(builder(EventType::IncidentChanged, EventSource::Operator))
        .with_event_id(event_id)
        .with_occurred_at(occurred)
        .with_received_at(occurred)
        .with_outcome(outcome)
        .with_payload(EventPayload {
            incident_id: Some(incident_id.into()),
            owner: Some(owner.into()),
            impact: Some("queue parity fixture".into()),
            state: Some(state.into()),
            ..EventPayload::default()
        })
        .build()
        .expect("incident event")
}

/// The fixture event set used for both stores.
fn events() -> Vec<DomainEvent> {
    let mut all = Vec::new();
    all.push(fee_computed_known_value());

    // A second change: held.
    all.push(
        scoped(builder(EventType::FeeHeld, EventSource::Finance))
            .with_event_id("evt_fee_held")
            .with_reason_code("proof_hold")
            .with_payload(EventPayload {
                change_id: Some("ch_0247".into()),
                period: Some("2026-10".into()),
                currency: Some("USD".into()),
                ..EventPayload::default()
            })
            .build()
            .expect("fee_held"),
    );

    // A third change: invoiced and paid.
    all.push(
        scoped(builder(EventType::FeeInvoiceIssued, EventSource::Finance))
            .with_event_id("evt_fee_invoiced")
            .with_payload(EventPayload {
                change_id: Some("ch_0248".into()),
                period: Some("2026-10".into()),
                currency: Some("USD".into()),
                invoice_id: Some("INV-2026-104".into()),
                amount_micros: Some(225_000_000),
                ..EventPayload::default()
            })
            .build()
            .expect("fee_invoice"),
    );
    all.push(
        scoped(builder(EventType::PaymentReconciled, EventSource::Finance))
            .with_event_id("evt_payment_reconciled")
            .with_payload(EventPayload {
                invoice_id: Some("INV-2026-104".into()),
                currency: Some("USD".into()),
                amount_micros: Some(225_000_000),
                ..EventPayload::default()
            })
            .build()
            .expect("payment"),
    );

    // An experiment: registered, started, kept with the 3 percent bound.
    all.push(
        scoped(builder(
            EventType::ExperimentRegistered,
            EventSource::Worker,
        ))
        .with_event_id("evt_experiment_registered")
        .with_payload(EventPayload {
            experiment_id: Some("exp_0246".into()),
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
            .with_event_id("evt_experiment_started")
            .with_occurred_at(time("2026-10-01T08:10:00.000Z"))
            .with_received_at(time("2026-10-01T08:11:00.000Z"))
            .with_payload(EventPayload {
                experiment_id: Some("exp_0246".into()),
                state: Some("running".into()),
                ..EventPayload::default()
            })
            .build()
            .expect("started"),
    );
    all.push(
        scoped(builder(EventType::ExperimentDecided, EventSource::Worker))
            .with_event_id("evt_experiment_decided")
            .with_occurred_at(time("2026-10-15T10:00:00.000Z"))
            .with_received_at(time("2026-10-15T10:01:00.000Z"))
            .with_payload(EventPayload {
                experiment_id: Some("exp_0246".into()),
                decision: Some("keep".into()),
                point_estimate_bp: Some(540),
                decision_bound_bp: Some(300),
                ..EventPayload::default()
            })
            .with_object("change", "ch_0246")
            .build()
            .expect("decided"),
    );

    // An open critical incident.
    all.push(
        scoped(builder(EventType::IncidentChanged, EventSource::Operator))
            .with_event_id("evt_incident")
            .with_outcome("critical")
            .with_payload(EventPayload {
                incident_id: Some("inc_018".into()),
                owner: Some("maya".into()),
                impact: Some("1 test state unresolved".into()),
                state: Some("open".into()),
                ..EventPayload::default()
            })
            .build()
            .expect("incident"),
    );

    // A request cohort of October 2026 that reached qualification.
    all.push(
        builder(EventType::AuditRequestAccepted, EventSource::Axum)
            .with_event_id("evt_request_accepted")
            .with_publisher_id("northstar")
            .with_payload(EventPayload {
                lead_id: Some("lead_01".into()),
                account_id: Some("acct_1".into()),
                platforms: Some(vec!["applovin_max".into()]),
                range_start: Some("2026-09-15".into()),
                range_end: Some("2026-10-14".into()),
                source_code: Some("seo".into()),
                ..EventPayload::default()
            })
            .build()
            .expect("accepted"),
    );
    all.push(
        builder(EventType::AccountStageChanged, EventSource::Axum)
            .with_event_id("evt_stage_qualified")
            .with_publisher_id("northstar")
            .with_reason_code("qualified")
            .with_object("account", "acct_1")
            .with_payload(EventPayload {
                previous_stage: Some("requested".into()),
                next_stage: Some("qualified".into()),
                owner: Some("maya".into()),
                effective_at: Some("2026-10-10T00:00:00Z".into()),
                ..EventPayload::default()
            })
            .build()
            .expect("stage"),
    );
    all
}

/// Two sanitized request rows for the 30-day `request_logs` table.
fn request_log_rows() -> Vec<RequestLogRow> {
    vec![
        RequestLogRow {
            event_id: "req_row_1".into(),
            occurred_at_utc: time("2026-10-15T09:00:00.000Z"),
            received_at_utc: time("2026-10-15T09:01:00.000Z"),
            route_template: "/pricing".into(),
            status: 200,
            status_class: "2xx".into(),
            bot_class: "unclassified".into(),
            referrer_host: Some("news.example.com".into()),
            campaign_code: Some("fall".into()),
            query_keys: vec!["utm_source".into()],
            environment: "production".into(),
        },
        RequestLogRow {
            event_id: "req_row_2".into(),
            occurred_at_utc: time("2026-10-15T09:02:00.000Z"),
            received_at_utc: time("2026-10-15T09:03:00.000Z"),
            route_template: "/unmatched".into(),
            status: 404,
            status_class: "4xx".into(),
            bot_class: "search_engine_bot".into(),
            referrer_host: None,
            campaign_code: None,
            query_keys: Vec::new(),
            environment: "production".into(),
        },
    ]
}

fn scope() -> QueryScope {
    QueryScope::new("northstar", time(HORIZON))
}

async fn memory_fixture() -> MemoryStore {
    let store = MemoryStore::new();
    store.ensure_schema().await.expect("memory schema");
    store.insert(&events()).await.expect("memory events");
    store
        .insert_report_days(&report_days())
        .await
        .expect("memory days");
    store
}

fn assert_revenue_known_value(rows: &[RevenueDailyRow], source: &str) {
    let total: i64 = rows.iter().map(|row| row.revenue.micros).sum();
    assert_eq!(
        total, 51_500_000_000,
        "{source}: US$ 51,500.00 exactly across the ten final days"
    );
    assert_eq!(rows.len(), 10, "{source}: one row per day");
    assert!(rows.iter().all(|row| row.revenue.currency == "USD"));
    assert!(rows.iter().all(|row| row.final_days == 1));
}

async fn run(store: &ClickHouseStore) -> Result<(), String> {
    let map = |error: apex_analytics::store::StoreError| error.to_string();

    store.ensure_schema().await.map_err(map)?;
    // Idempotent: a second ensure_schema must pass too.
    store.ensure_schema().await.map_err(map)?;
    let health = store.health().await.map_err(map)?;
    assert!(health.server_version.is_some(), "server version reported");

    let fixture = events();
    let inserted = store.insert(&fixture).await.map_err(map)?;
    assert_eq!(inserted.inserted, fixture.len() as u64);
    store
        .insert_report_days(&report_days())
        .await
        .map_err(map)?;

    // Dedup replay: the same batch again. FINAL reads collapse the
    // (source, event_id) key to one row per event.
    store.insert(&fixture).await.map_err(map)?;
    let replay_store = MemoryStore::new();
    replay_store.ensure_schema().await.map_err(map)?;
    replay_store.insert(&fixture).await.map_err(map)?;
    replay_store.insert(&fixture).await.map_err(map)?;
    let (inserts, duplicates) = replay_store.insert_stats();
    assert_eq!(inserts, 2 * fixture.len() as u64);
    assert_eq!(duplicates, fixture.len() as u64, "memory replay dedups");

    let loaded = store.load_events().await.map_err(map)?;
    assert_eq!(loaded.len(), fixture.len(), "FINAL collapses the replay");
    // Force the physical merge and re-check: one row per key on disk too.
    store
        .execute_statement(&format!(
            "OPTIMIZE TABLE {}.domain_events FINAL",
            store.database()
        ))
        .await
        .map_err(map)?;
    let count = store
        .fetch_rows(
            &format!(
                "SELECT count() AS n FROM {}.domain_events",
                store.database()
            ),
            &[],
        )
        .await
        .map_err(map)?;
    assert_eq!(
        count[0]["n"].as_u64(),
        Some(fixture.len() as u64),
        "after OPTIMIZE one row per (source, event_id)"
    );

    // Sanitized request logs land in the 30-day TTL table.
    store
        .insert_request_logs(&request_log_rows())
        .await
        .map_err(map)?;
    let request_rows = store
        .fetch_rows(
            &format!(
                "SELECT count() AS n, uniqExact(event_id) AS ids, \
                 max(route_template) AS route FROM {}.request_logs",
                store.database()
            ),
            &[],
        )
        .await
        .map_err(map)?;
    assert_eq!(request_rows[0]["n"].as_u64(), Some(2));
    assert_eq!(request_rows[0]["ids"].as_u64(), Some(2));
    assert_eq!(request_rows[0]["route"].as_str(), Some("/unmatched"));
    let ddl = store
        .fetch_rows(
            "SELECT create_table_query FROM system.tables \
             WHERE database = {db:String} AND name = 'request_logs'",
            &[("db", store.database())],
        )
        .await
        .map_err(map)?;
    let create = ddl[0]["create_table_query"]
        .as_str()
        .expect("create_table_query");
    assert!(
        create.contains("toIntervalDay(30)"),
        "30-day retention on the live table: {create}"
    );

    // Projections: nothing is reader-visible before the first publish.
    let report = compare(store).await.map_err(|error| error.to_string())?;
    assert!(!report.is_clean(), "no projection has run yet");
    let first = rebuild_batch(store, "integration-1")
        .await
        .map_err(|error| error.to_string())?;
    assert_eq!(first.rebuilt.len(), 4);
    let customer = first
        .rebuilt
        .iter()
        .find(|entry| entry.projection == ProjectionName::CustomerDaily)
        .expect("customer_daily rebuilt");
    assert_eq!(customer.totals.revenue_micros, Some(51_500_000_000));
    assert_eq!(customer.rows, 10);
    assert_eq!(
        store
            .projection_version(ProjectionName::CustomerDaily)
            .await
            .map_err(map)?,
        None,
        "a staged version is not reader-visible"
    );
    assert!(store
        .load_projection(ProjectionName::CustomerDaily)
        .await
        .map_err(map)?
        .is_empty());
    assert!(
        store.projection_versions().await.map_err(map)?.is_empty(),
        "the lag gauge read stays empty until publish"
    );
    let reader = ClickHouseReader::new(store.clone());
    assert!(
        reader
            .revenue_daily(&scope())
            .await
            .map_err(map)?
            .is_empty(),
        "the reader sees nothing unpublished"
    );

    // A clean compare_report (no recompute) then publish switches readers.
    let comparison = compare_report(store, &first)
        .await
        .map_err(|error| error.to_string())?;
    assert!(comparison.is_clean(), "{comparison:?}");
    assert_eq!(
        comparison,
        compare(store).await.map_err(|error| error.to_string())?,
        "compare_report agrees with compare on the same batch"
    );
    enforce_switch(&comparison).map_err(|error| error.to_string())?;
    let published_version = publish(store, "integration-1")
        .await
        .map_err(|error| error.to_string())?;
    assert_eq!(published_version, first.as_of_version);
    assert_eq!(
        store
            .projection_version(ProjectionName::CustomerDaily)
            .await
            .map_err(map)?,
        Some(published_version)
    );
    assert_eq!(
        store
            .load_projection(ProjectionName::CustomerDaily)
            .await
            .map_err(map)?
            .len(),
        10
    );

    // The version rows the daily runner reads back for the lag gauge.
    let versions = store.projection_versions().await.map_err(map)?;
    assert_eq!(versions.len(), 4, "{versions:?}");
    let customer_version = versions
        .iter()
        .find(|version| version.projection_name == "customer_daily")
        .expect("customer_daily version");
    assert_eq!(customer_version.batch_id, "integration-1");
    assert!(customer_version.source_event_max_unix_ms.is_some());
    assert!(customer_version.rebuilt_at_unix_ms > 0);
    let queued = versions
        .iter()
        .find(|version| version.projection_name == "operator_queues")
        .expect("operator_queues version");
    assert_eq!(queued.batch_id, "integration-1");

    // Readers: the ClickHouse reader and the memory reader must agree.
    let memory = memory_fixture().await;
    let memory_reader = MemoryReader::new(memory.clone());
    let scope = scope();

    let ch_daily = reader.revenue_daily(&scope).await.map_err(map)?;
    let mem_daily = memory_reader.revenue_daily(&scope).await.map_err(map)?;
    assert_revenue_known_value(&ch_daily, "clickhouse revenue_daily");
    assert_revenue_known_value(&mem_daily, "memory revenue_daily");
    assert_eq!(ch_daily, mem_daily, "revenue_daily parity");

    for dimension in [
        BreakdownDimension::Platform,
        BreakdownDimension::Format,
        BreakdownDimension::Country,
        BreakdownDimension::AdUnit,
    ] {
        let ch = reader
            .revenue_breakdown(&scope, dimension)
            .await
            .map_err(map)?;
        let mem = memory_reader
            .revenue_breakdown(&scope, dimension)
            .await
            .map_err(map)?;
        assert_eq!(ch, mem, "breakdown parity for {dimension}");
    }
    let countries = reader
        .revenue_breakdown(&scope, BreakdownDimension::Country)
        .await
        .map_err(map)?;
    assert_eq!(countries.len(), 2);
    let de = countries
        .iter()
        .find(|row| row.dimension_value == "DE")
        .expect("DE row");
    assert_eq!(de.revenue.micros, 30_900_000_000);
    assert_eq!(de.share_bp, Some(6_000));

    let ch_lift = reader.lift_rows(&scope).await.map_err(map)?;
    let mem_lift = memory_reader.lift_rows(&scope).await.map_err(map)?;
    assert_eq!(ch_lift, mem_lift, "lift parity");
    assert_eq!(ch_lift.len(), 1);
    assert_eq!(ch_lift[0].experiment_id, "exp_0246");
    assert_eq!(ch_lift[0].outcome, "keep");
    assert_eq!(ch_lift[0].observed_lift_bp, Some(540));
    assert_eq!(ch_lift[0].billable_bound_bp, Some(300));
    assert_eq!(
        ch_lift[0].measurement_source.as_deref(),
        Some("revenue_per_user")
    );

    let ch_fees = reader.fee_rows(&scope).await.map_err(map)?;
    let mem_fees = memory_reader.fee_rows(&scope).await.map_err(map)?;
    assert_eq!(ch_fees, mem_fees, "fee parity");
    assert_eq!(ch_fees.len(), 3);
    let estimated = ch_fees
        .iter()
        .find(|row| row.change_id == "ch_0246")
        .expect("estimated fee");
    assert_eq!(estimated.state.as_str(), "estimated");
    assert_eq!(
        estimated.amount.as_ref().map(|money| money.micros),
        Some(300_000_000),
        "the known-value fee is US$ 300.00"
    );
    let held = ch_fees
        .iter()
        .find(|row| row.change_id == "ch_0247")
        .expect("held fee");
    assert_eq!(held.state.as_str(), "held");
    let paid = ch_fees
        .iter()
        .find(|row| row.change_id == "ch_0248")
        .expect("paid fee");
    assert_eq!(paid.state.as_str(), "paid");
    assert_eq!(paid.invoice_ref.as_deref(), Some("INV-2026-104"));

    let detail = reader
        .fee_detail(&scope, "ch_0246")
        .await
        .map_err(map)?
        .expect("known-value fee detail");
    let mem_detail = memory_reader
        .fee_detail(&scope, "ch_0246")
        .await
        .map_err(map)?
        .expect("memory detail");
    assert_eq!(detail, mem_detail, "fee detail parity");
    assert_eq!(detail.eligible_revenue.micros, 51_500_000_000);
    assert_eq!(detail.conservative_bound_bp, 300);
    assert_eq!(detail.agreed_rate_bp, 2_000);
    assert_eq!(
        detail
            .attributed_increment
            .as_ref()
            .map(|money| money.micros),
        Some(1_500_000_000)
    );
    assert_eq!(
        detail.fee_before_tax.as_ref().map(|money| money.micros),
        Some(300_000_000)
    );
    assert!(detail.unavailable.is_empty());

    let ch_timeline = reader
        .object_timeline(&scope, "change", "ch_0246")
        .await
        .map_err(map)?;
    let mem_timeline = memory_reader
        .object_timeline(&scope, "change", "ch_0246")
        .await
        .map_err(map)?;
    assert_eq!(ch_timeline, mem_timeline, "timeline parity");
    assert_eq!(ch_timeline.len(), 2);

    let quality = reader.data_quality(&scope).await.map_err(map)?;
    let mem_quality = memory_reader.data_quality(&scope).await.map_err(map)?;
    assert_eq!(quality.matched_source_days, 10);
    assert_eq!(quality.dedup_rejects, Some(0), "merged duplicates counted");
    assert_eq!(quality.unmapped_rows, 0);
    assert_eq!(quality.stale_sources.len(), 1);
    assert_eq!(mem_quality.matched_source_days, quality.matched_source_days);
    assert_eq!(mem_quality.dedup_rejects, Some(0));

    let health_rows = reader.ingestion_health(&scope).await.map_err(map)?;
    let mem_health_rows = memory_reader.ingestion_health(&scope).await.map_err(map)?;
    assert_eq!(health_rows, mem_health_rows, "ingestion health parity");
    assert_eq!(health_rows.len(), 1);
    assert_eq!(health_rows[0].last_final_day.as_deref(), Some("2026-10-10"));
    assert_eq!(health_rows[0].freshness_days, Some(5));
    assert_eq!(health_rows[0].status.as_str(), "stale");

    let cohorts = reader.growth_cohorts(&scope).await.map_err(map)?;
    let mem_cohorts = memory_reader.growth_cohorts(&scope).await.map_err(map)?;
    assert_eq!(cohorts, mem_cohorts, "growth parity");
    assert_eq!(cohorts.len(), 1);
    assert_eq!(cohorts[0].cohort_month, "2026-10");
    assert_eq!(cohorts[0].accepted, 1);
    assert_eq!(cohorts[0].qualified, 1);
    assert_eq!(cohorts[0].qualification_rate_bp, Some(10_000));
    assert!(!cohorts[0].mature, "October 2026 is not mature on 15 Oct");

    let funnel = reader
        .activation_funnel(&scope, "2026-10")
        .await
        .map_err(map)?;
    let mem_funnel = memory_reader
        .activation_funnel(&scope, "2026-10")
        .await
        .map_err(map)?;
    assert_eq!(funnel, mem_funnel, "funnel parity");
    let stages: Vec<(String, u64)> = funnel
        .stages
        .iter()
        .map(|stage| (stage.stage.clone(), stage.accounts))
        .collect();
    assert_eq!(
        stages,
        vec![
            ("requested".to_string(), 1),
            ("qualified".to_string(), 1),
            ("connected".to_string(), 0),
            ("audit_delivered".to_string(), 0),
            ("paid".to_string(), 0),
        ]
    );

    let landscape = reader.experiment_landscape(&scope).await.map_err(map)?;
    let mem_landscape = memory_reader
        .experiment_landscape(&scope)
        .await
        .map_err(map)?;
    assert_eq!(landscape, mem_landscape, "landscape parity");
    assert_eq!(landscape.len(), 1);
    assert_eq!(landscape[0].state, "keep");
    assert_eq!(landscape[0].count, 1);

    let effectiveness = reader
        .experiment_effectiveness(&scope, "2026-10")
        .await
        .map_err(map)?;
    let mem_effectiveness = memory_reader
        .experiment_effectiveness(&scope, "2026-10")
        .await
        .map_err(map)?;
    assert_eq!(effectiveness, mem_effectiveness, "effectiveness parity");
    assert_eq!(effectiveness.started, 1);
    assert_eq!(effectiveness.kept, 1);
    assert_eq!(effectiveness.valid_settled, 1);
    assert_eq!(effectiveness.in_flight, 0);
    assert_eq!(effectiveness.keep_rate_valid_bp, Some(10_000));
    assert_eq!(effectiveness.completion_p50_days, Some(14));

    // Operator queues: the memory reader derives them from events, the
    // ClickHouse reader reads the rebuilt projection.
    let mem_queues = memory_reader
        .operator_queues(&QueryScope::operator_wide(time(HORIZON)))
        .await
        .map_err(map)?;
    assert_eq!(mem_queues.incidents.len(), 1);
    assert_eq!(mem_queues.incidents[0].object_id, "inc_018");
    assert_eq!(mem_queues.incidents[0].severity, "critical");
    assert_eq!(mem_queues.held_fees.len(), 1);
    assert_eq!(mem_queues.held_fees[0].object_id, "ch_0247:2026-10");
    let ch_queues = reader
        .operator_queues(&QueryScope::operator_wide(time(HORIZON)))
        .await
        .map_err(map)?;
    assert_eq!(
        ch_queues.incidents.len(),
        mem_queues.incidents.len(),
        "incident queue parity (projection vs events)"
    );
    assert_eq!(
        ch_queues.incidents.first().map(|row| row.object_id.clone()),
        Some("inc_018".to_string())
    );
    assert_eq!(ch_queues.held_fees.len(), mem_queues.held_fees.len());

    // Tenant isolation: another publisher sees nothing.
    let other = QueryScope::new("orbit", time(HORIZON));
    assert!(reader.revenue_daily(&other).await.map_err(map)?.is_empty());
    assert!(memory_reader
        .revenue_daily(&other)
        .await
        .map_err(map)?
        .is_empty());

    // A tampered staged version: the compare refuses it, the publish never
    // runs, and readers stay on the published v1.
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
        .write_projection(ProjectionName::CustomerDaily, 9_000, "tamper", &[tampered])
        .await
        .map_err(map)?;
    let report = compare(store).await.map_err(|error| error.to_string())?;
    assert!(!report.is_clean());
    assert!(matches!(
        report.comparisons[0].status,
        ComparisonStatus::Mismatch { .. }
    ));
    assert!(
        enforce_switch(&report).is_err(),
        "the mismatch refuses the reader switch"
    );
    // The first batch's rows were superseded by the tamper: comparing that
    // report now fails with the batch named instead of publishing anything.
    match compare_report(store, &first).await {
        Err(apex_analytics::projections::ProjectionError::StagedUnreadable {
            batch_id,
            reason,
        }) => {
            assert_eq!(batch_id, "integration-1");
            assert!(reason.contains("tamper"), "{reason}");
        }
        other => panic!("expected StagedUnreadable, got {other:?}"),
    }
    let rows = reader.revenue_daily(&scope).await.map_err(map)?;
    assert_revenue_known_value(&rows, "readers stay on v1 after tamper");

    // Force the physical merges the staged marker must survive: the staged
    // tamper version must not collapse the published v1 rows away.
    store
        .execute_statement(&format!(
            "OPTIMIZE TABLE {}.customer_daily FINAL",
            store.database()
        ))
        .await
        .map_err(map)?;
    let rows = reader.revenue_daily(&scope).await.map_err(map)?;
    assert_revenue_known_value(&rows, "readers stay on v1 after merges");

    // A fresh rebuild recovers and publishes.
    let recovery = rebuild_batch(store, "integration-2")
        .await
        .map_err(|error| error.to_string())?;
    let comparison = compare_report(store, &recovery)
        .await
        .map_err(|error| error.to_string())?;
    assert!(comparison.is_clean(), "{comparison:?}");
    let recovered_version = publish(store, "integration-2")
        .await
        .map_err(|error| error.to_string())?;
    assert_eq!(recovered_version, recovery.as_of_version);
    let rows = reader.revenue_daily(&scope).await.map_err(map)?;
    assert_revenue_known_value(&rows, "readers after recovery");
    // Superseded versions are pruned at publish: v1 and the tampered v9000.
    let superseded = store
        .fetch_rows(
            &format!(
                "SELECT count() AS n FROM {}.customer_daily WHERE as_of_version < {}",
                store.database(),
                recovered_version
            ),
            &[],
        )
        .await
        .map_err(map)?;
    assert_eq!(
        superseded[0]["n"].as_u64(),
        Some(0),
        "publish prunes superseded versions"
    );
    assert_eq!(
        store
            .load_projection(ProjectionName::CustomerDaily)
            .await
            .map_err(map)?
            .len(),
        10
    );

    // A v2 with a DIFFERENT value, unpublished: readers still see v1; after
    // publish they see v2.
    let mut extra_day = report_days()[0].clone();
    extra_day.day = "2026-10-11".into();
    extra_day.revenue_micros = Some(1_000_000);
    extra_day.impressions = Some(1);
    store.insert_report_days(&[extra_day]).await.map_err(map)?;
    let staged = rebuild_batch(store, "integration-3")
        .await
        .map_err(|error| error.to_string())?;
    // Merge the staged v2 into the table before reading: the published v1
    // rows must survive (the as-of version is part of the sorting key).
    store
        .execute_statement(&format!(
            "OPTIMIZE TABLE {}.customer_daily FINAL",
            store.database()
        ))
        .await
        .map_err(map)?;
    let rows = reader.revenue_daily(&scope).await.map_err(map)?;
    assert_revenue_known_value(&rows, "unpublished v2 stays invisible after merges");
    let comparison = compare_report(store, &staged)
        .await
        .map_err(|error| error.to_string())?;
    assert!(comparison.is_clean(), "{comparison:?}");
    publish(store, "integration-3")
        .await
        .map_err(|error| error.to_string())?;
    let rows = reader.revenue_daily(&scope).await.map_err(map)?;
    assert_eq!(rows.len(), 11, "v2 adds the eleventh day");
    let total: i64 = rows.iter().map(|row| row.revenue.micros).sum();
    assert_eq!(total, 51_501_000_000, "v2 carries the changed value");
    let remaining = store
        .fetch_rows(
            &format!(
                "SELECT count() AS n, min(as_of_version) AS oldest FROM {}.customer_daily",
                store.database()
            ),
            &[],
        )
        .await
        .map_err(map)?;
    assert_eq!(remaining[0]["n"].as_u64(), Some(11));
    assert_eq!(
        remaining[0]["oldest"].as_u64(),
        Some(staged.as_of_version),
        "only the published version remains"
    );

    // The sink path over the real store.
    let client = apex_analytics::AnalyticsClient::new(
        Arc::new(store.clone()),
        "production",
        "apex-ingest-test",
    );
    let spooled_event = client
        .builder(EventType::JobRun, EventSource::Worker)
        .with_event_id("evt_job_run")
        .with_occurred_at(time(HORIZON))
        .with_received_at(time(HORIZON))
        .with_outcome("completed")
        .with_payload(EventPayload {
            job_id: Some("job_daily".into()),
            duration_ms: Some(1_500),
            ..EventPayload::default()
        })
        .build()
        .map_err(|error| error.to_string())?;
    let outcome = client
        .emit(&spooled_event)
        .await
        .map_err(|error| error.to_string())?;
    assert_eq!(outcome.delivered, 1);
    assert_eq!(outcome.spooled, 0);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn real_clickhouse_round_trip() {
    let http_url = match std::env::var(CH_ENV) {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            println!("SKIP: {CH_ENV} is not set; start the dev server with `make clickhouse-dev` and run APEX_CLICKHOUSE_HTTP=http://127.0.0.1:8123 cargo test -p apex-analytics -- --nocapture");
            return;
        }
    };
    let database = unique_database();
    let config = ClickHouseConfig {
        http_url,
        database: database.clone(),
        request_timeout_ms: 60_000,
    };
    let store = match ClickHouseStore::new(config) {
        Ok(store) => store,
        Err(error) => panic!("store configuration: {error}"),
    };
    println!("integration database: {database}");
    let result = run(&store).await;
    let dropped = store.drop_database().await;
    match (result, dropped) {
        (Ok(()), Ok(())) => println!("integration PASS; test database {database} dropped"),
        (Err(error), _) => panic!("integration failed: {error}"),
        (Ok(()), Err(error)) => panic!("test database cleanup failed: {error}"),
    }
}

/// The pre-staging DDL, written out as a legacy deployment created it:
/// `projection_versions` ordered by `projection_name` and the projection
/// tables ordered by their business keys only (no `as_of_version`), so a
/// staged rebuild and the published version collapse into one row per key at
/// merge time exactly as the adversarial probe reproduced.
fn legacy_ddl(database: &str) -> Vec<String> {
    vec![
        format!(
            "CREATE TABLE {database}.projection_versions (\n\
             \x20   projection_name LowCardinality(String),\n\
             \x20   as_of_version UInt64,\n\
             \x20   batch_id String,\n\
             \x20   source_event_max Nullable(DateTime64(3, 'UTC')),\n\
             \x20   rebuilt_at DateTime64(3, 'UTC')\n\
             ) ENGINE = ReplacingMergeTree(rebuilt_at)\n\
             ORDER BY projection_name"
        ),
        format!(
            "CREATE TABLE {database}.customer_daily (\n\
             \x20   publisher_id LowCardinality(String),\n\
             \x20   day Date,\n\
             \x20   platform LowCardinality(String),\n\
             \x20   currency LowCardinality(String),\n\
             \x20   revenue_micros Int64,\n\
             \x20   impressions Nullable(UInt64),\n\
             \x20   source_days UInt32,\n\
             \x20   final_days UInt32,\n\
             \x20   provisional_days UInt32,\n\
             \x20   as_of_version UInt64\n\
             ) ENGINE = ReplacingMergeTree(as_of_version)\n\
             ORDER BY (publisher_id, day, platform, currency)\n\
             TTL day + INTERVAL 24 MONTH"
        ),
        format!(
            "CREATE TABLE {database}.platform_daily (\n\
             \x20   platform LowCardinality(String),\n\
             \x20   day Date,\n\
             \x20   currency LowCardinality(String),\n\
             \x20   revenue_micros Int64,\n\
             \x20   impressions Nullable(UInt64),\n\
             \x20   final_accounts UInt32,\n\
             \x20   provisional_accounts UInt32,\n\
             \x20   as_of_version UInt64\n\
             ) ENGINE = ReplacingMergeTree(as_of_version)\n\
             ORDER BY (platform, day, currency)\n\
             TTL day + INTERVAL 24 MONTH"
        ),
        format!(
            "CREATE TABLE {database}.lifecycle_cohorts (\n\
             \x20   cohort_month Date,\n\
             \x20   publisher_id LowCardinality(String),\n\
             \x20   stage LowCardinality(String),\n\
             \x20   accounts UInt32,\n\
             \x20   as_of_version UInt64\n\
             ) ENGINE = ReplacingMergeTree(as_of_version)\n\
             ORDER BY (cohort_month, publisher_id, stage)\n\
             TTL cohort_month + INTERVAL 24 MONTH"
        ),
        format!(
            "CREATE TABLE {database}.operator_queues (\n\
             \x20   queue LowCardinality(String),\n\
             \x20   object_id String,\n\
             \x20   publisher_id LowCardinality(String),\n\
             \x20   severity LowCardinality(String),\n\
             \x20   age_seconds UInt64,\n\
             \x20   owner Nullable(String),\n\
             \x20   next_action String,\n\
             \x20   as_of_version UInt64\n\
             ) ENGINE = ReplacingMergeTree(as_of_version)\n\
             ORDER BY (queue, object_id)"
        ),
    ]
}

/// Insert raw JSONEachRow rows through the admin path (used for the legacy
/// shapes and other statements the typed API deliberately does not support).
async fn insert_rows(
    store: &ClickHouseStore,
    table: &str,
    rows: &[serde_json::Value],
) -> Result<(), String> {
    let mut body = format!(
        "INSERT INTO {}.{table} FORMAT JSONEachRow\n",
        store.database()
    );
    for row in rows {
        body.push_str(&serde_json::to_string(row).map_err(|error| error.to_string())?);
        body.push('\n');
    }
    store
        .execute_statement(&body)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn u64_field(row: &serde_json::Value, key: &str) -> u64 {
    row.get(key)
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        })
        .unwrap_or(0)
}

async fn sorting_keys(store: &ClickHouseStore) -> Result<Vec<(String, String)>, String> {
    let rows = store
        .fetch_rows(
            "SELECT name, sorting_key FROM system.tables WHERE database = {db:String}",
            &[("db", store.database())],
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            Some((
                row.get("name")?.as_str()?.to_string(),
                row.get("sorting_key")?
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            ))
        })
        .collect())
}

async fn table_counts(store: &ClickHouseStore) -> Result<Vec<(String, u64)>, String> {
    let mut counts = Vec::new();
    for table in apex_analytics::schema::STAGED_TABLES {
        let rows = store
            .fetch_rows(
                &format!(
                    "SELECT count() AS n FROM {}.{}",
                    store.database(),
                    table.name
                ),
                &[],
            )
            .await
            .map_err(|error| error.to_string())?;
        counts.push((
            table.name.to_string(),
            rows.first().map(|row| u64_field(row, "n")).unwrap_or(0),
        ));
    }
    Ok(counts)
}

/// The legacy migration scenario: a database created with the old DDL, one
/// published v1 per projection, then `ensure_schema` migrates every shape.
/// A staged rebuild plus forced merges must leave readers on v1 (the
/// adversarial probe observed an empty read), publish must switch to v2,
/// and a second `ensure_schema` must be a no-op.
async fn run_legacy_migration(store: &ClickHouseStore) -> Result<(), String> {
    let map = |error: apex_analytics::store::StoreError| error.to_string();
    let reader = ClickHouseReader::new(store.clone());
    let scope = QueryScope::new("northstar", time(HORIZON));

    store
        .execute_statement(&format!(
            "CREATE DATABASE IF NOT EXISTS {}",
            store.database()
        ))
        .await
        .map_err(map)?;
    for statement in legacy_ddl(store.database()) {
        store.execute_statement(&statement).await.map_err(map)?;
    }
    // One reader-visible v1 version row per projection and one v1 data row
    // for customer_daily whose value differs from the recomputed v2.
    let version_rows: Vec<serde_json::Value> = [
        "customer_daily",
        "platform_daily",
        "lifecycle_cohorts",
        "operator_queues",
    ]
    .iter()
    .map(|name| {
        serde_json::json!({
            "projection_name": name,
            "as_of_version": 1,
            "batch_id": "legacy-batch",
            "source_event_max": "2026-10-01 00:00:00.000",
            "rebuilt_at": "2026-10-02 00:00:00.000",
        })
    })
    .collect();
    insert_rows(store, "projection_versions", &version_rows).await?;
    insert_rows(
        store,
        "customer_daily",
        &[serde_json::json!({
            "publisher_id": "northstar",
            "day": "2026-10-01",
            "platform": "applovin_max",
            "currency": "USD",
            "revenue_micros": 1_000_000,
            "impressions": 10,
            "source_days": 1,
            "final_days": 1,
            "provisional_days": 0,
            "as_of_version": 1,
        })],
    )
    .await?;
    insert_rows(
        store,
        "platform_daily",
        &[serde_json::json!({
            "platform": "applovin_max",
            "day": "2026-10-01",
            "currency": "USD",
            "revenue_micros": 1_000_000,
            "impressions": 10,
            "final_accounts": 1,
            "provisional_accounts": 0,
            "as_of_version": 1,
        })],
    )
    .await?;
    insert_rows(
        store,
        "lifecycle_cohorts",
        &[serde_json::json!({
            "cohort_month": "2026-08-01",
            "publisher_id": "northstar",
            "stage": "qualified",
            "accounts": 1,
            "as_of_version": 1,
        })],
    )
    .await?;
    insert_rows(
        store,
        "operator_queues",
        &[serde_json::json!({
            "queue": "incidents",
            "object_id": "inc_legacy",
            "publisher_id": "northstar",
            "severity": "critical",
            "age_seconds": 120,
            "owner": "maya",
            "next_action": "review_incident",
            "as_of_version": 1,
        })],
    )
    .await?;

    let legacy_keys = sorting_keys(store).await?;
    println!("legacy sorting keys: {legacy_keys:?}");

    // The boot migration under test.
    store.ensure_schema().await.map_err(map)?;

    let migrated_keys = sorting_keys(store).await?;
    println!("migrated sorting keys: {migrated_keys:?}");
    for table in apex_analytics::schema::STAGED_TABLES {
        let key = migrated_keys
            .iter()
            .find(|(name, _)| name == table.name)
            .map(|(_, key)| key.clone())
            .expect("every staged table exists after ensure_schema");
        assert_eq!(
            key.replace(' ', ""),
            table.sorting_key,
            "{} must carry the staged sorting key",
            table.name
        );
    }
    let leftovers = store
        .fetch_rows(
            "SELECT count() AS n FROM system.tables \
             WHERE database = {db:String} AND name LIKE '%__migration'",
            &[("db", store.database())],
        )
        .await
        .map_err(map)?;
    assert_eq!(
        u64_field(&leftovers[0], "n"),
        0,
        "a migrated boot leaves no migration siblings"
    );

    // Readers see the migrated published v1, and its bookkeeping is exact.
    assert_eq!(
        store
            .projection_version(ProjectionName::CustomerDaily)
            .await
            .map_err(map)?,
        Some(1)
    );
    let rows = reader.revenue_daily(&scope).await.map_err(map)?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].revenue.micros, 1_000_000);
    let preserved = store
        .fetch_rows(
            &format!(
                "SELECT batch_id, toString(as_of_version) AS v, toString(published) AS p \
                 FROM {}.projection_versions WHERE projection_name = 'customer_daily'",
                store.database()
            ),
            &[],
        )
        .await
        .map_err(map)?;
    assert_eq!(preserved[0]["batch_id"].as_str(), Some("legacy-batch"));
    assert_eq!(preserved[0]["v"].as_str(), Some("1"));
    assert_eq!(preserved[0]["p"].as_str(), Some("1"));

    // Facts for the staged v2: two days at 5,150.00 each.
    let facts: Vec<ReportDayRow> = ["2026-10-01", "2026-10-02"]
        .iter()
        .map(|day| ReportDayRow {
            publisher_id: "northstar".into(),
            platform: "applovin_max".into(),
            platform_account_id: "max_acct_1".into(),
            day: (*day).into(),
            finality: "final".into(),
            revenue_micros: Some(5_150_000_000),
            impressions: Some(2_500_000),
            currency: Some("USD".into()),
            country: None,
            ad_unit_id: None,
            format: None,
            source_digest: None,
        })
        .collect();
    store.insert_report_days(&facts).await.map_err(map)?;
    let staged = rebuild_batch(store, "legacy-1")
        .await
        .map_err(|error| error.to_string())?;
    let customer = staged
        .rebuilt
        .iter()
        .find(|entry| entry.projection == ProjectionName::CustomerDaily)
        .expect("customer_daily rebuilt");
    assert_eq!(customer.as_of_version, 2, "the rebuild stages v2");

    // Force the merges that collapsed the published v1 away on the legacy
    // shape.
    for table in ["projection_versions", "customer_daily"] {
        store
            .execute_statement(&format!(
                "OPTIMIZE TABLE {}.{table} FINAL",
                store.database()
            ))
            .await
            .map_err(map)?;
    }
    let rows = reader.revenue_daily(&scope).await.map_err(map)?;
    assert_eq!(
        rows.len(),
        1,
        "readers must not fall off the published version after merges"
    );
    assert_eq!(
        rows[0].revenue.micros, 1_000_000,
        "readers still see the published v1 value"
    );
    assert_eq!(
        store
            .projection_version(ProjectionName::CustomerDaily)
            .await
            .map_err(map)?,
        Some(1)
    );
    println!("after staged v2 + OPTIMIZE FINAL, readers still see v1 (no empty read)");

    // Publishing switches readers to v2.
    let published = publish(store, "legacy-1")
        .await
        .map_err(|error| error.to_string())?;
    assert_eq!(published, 2);
    let rows = reader.revenue_daily(&scope).await.map_err(map)?;
    assert_eq!(rows.len(), 2);
    let total: i64 = rows.iter().map(|row| row.revenue.micros).sum();
    assert_eq!(total, 10_300_000_000, "v2 carries the recomputed value");
    println!("publish switched readers to v2 (total micros {total})");

    // A second ensure_schema is a no-op: same shapes, same rows, same
    // readers, still no migration siblings.
    let before_counts = table_counts(store).await?;
    let before_keys = migrated_keys.clone();
    store.ensure_schema().await.map_err(map)?;
    let after_counts = table_counts(store).await?;
    let after_keys = sorting_keys(store).await?;
    assert_eq!(before_counts, after_counts, "a second boot changes no rows");
    let staged_after: Vec<(String, String)> = after_keys
        .into_iter()
        .filter(|(name, _)| {
            apex_analytics::schema::STAGED_TABLES
                .iter()
                .any(|table| table.name == name)
        })
        .collect();
    let staged_before: Vec<(String, String)> = before_keys
        .into_iter()
        .filter(|(name, _)| {
            apex_analytics::schema::STAGED_TABLES
                .iter()
                .any(|table| table.name == name)
        })
        .collect();
    assert_eq!(
        staged_before, staged_after,
        "a second boot changes no shape"
    );
    let leftovers = store
        .fetch_rows(
            "SELECT count() AS n FROM system.tables \
             WHERE database = {db:String} AND name LIKE '%__migration'",
            &[("db", store.database())],
        )
        .await
        .map_err(map)?;
    assert_eq!(u64_field(&leftovers[0], "n"), 0);
    let rows = reader.revenue_daily(&scope).await.map_err(map)?;
    assert_eq!(rows.len(), 2, "readers stay on v2 after the second boot");

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn real_clickhouse_legacy_staged_tables_migrate() {
    let http_url = match std::env::var(CH_ENV) {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            println!("SKIP: {CH_ENV} is not set; start the dev server and run APEX_CLICKHOUSE_HTTP=http://127.0.0.1:8123 cargo test -p apex-analytics -- --nocapture");
            return;
        }
    };
    let database = unique_database();
    let config = ClickHouseConfig {
        http_url,
        database: database.clone(),
        request_timeout_ms: 60_000,
    };
    let store = match ClickHouseStore::new(config) {
        Ok(store) => store,
        Err(error) => panic!("store configuration: {error}"),
    };
    println!("legacy-migration database: {database}");
    let result = run_legacy_migration(&store).await;
    let dropped = store.drop_database().await;
    match (result, dropped) {
        (Ok(()), Ok(())) => println!("legacy-migration PASS; test database {database} dropped"),
        (Err(error), _) => panic!("legacy-migration failed: {error}"),
        (Ok(()), Err(error)) => panic!("test database cleanup failed: {error}"),
    }
}

/// Page-request events spread over two days: three on 14 October (one 404)
/// and one 404 on 15 October.
fn page_request_events() -> Vec<DomainEvent> {
    let page = |event_id: &str, occurred_at: &str, route: &str, status: u16, bot_class: &str| {
        builder(EventType::PublicPageRequested, EventSource::AccessLog)
            .with_event_id(event_id)
            .with_occurred_at(time(occurred_at))
            .with_received_at(time(occurred_at))
            .with_object("document", route)
            .with_outcome(apex_analytics::accesslog::status_class(status))
            .with_payload(EventPayload {
                route_template: Some(route.into()),
                status: Some(status),
                bot_class: Some(bot_class.into()),
                ..EventPayload::default()
            })
            .build()
            .expect("page event")
    };
    vec![
        page(
            "evt_page_1",
            "2026-10-14T08:00:00.000Z",
            "/pricing",
            200,
            "unclassified",
        ),
        page(
            "evt_page_2",
            "2026-10-14T09:00:00.000Z",
            "/",
            200,
            "search_engine_bot",
        ),
        page(
            "evt_page_3",
            "2026-10-14T23:59:59.999Z",
            "/unmatched",
            404,
            "unclassified",
        ),
        page(
            "evt_page_4",
            "2026-10-15T00:00:00.000Z",
            "/unmatched",
            404,
            "automation_client",
        ),
    ]
}

/// Memory vs ClickHouse parity for `page_requests_daily`, exact sums, and
/// the site-wide contract (tenant parameters are not filters).
async fn run_page_requests_parity(store: &ClickHouseStore) -> Result<(), String> {
    let map = |error: apex_analytics::store::StoreError| error.to_string();
    store.ensure_schema().await.map_err(map)?;
    let events = page_request_events();
    store.insert(&events).await.map_err(map)?;
    let memory = MemoryStore::new();
    memory.ensure_schema().await.map_err(map)?;
    memory.insert(&events).await.map_err(map)?;
    let reader = ClickHouseReader::new(store.clone());
    let memory_reader = MemoryReader::new(memory);
    let wide = QueryScope::operator_wide(time(HORIZON));

    let ch = reader
        .page_requests_daily(&wide, "2026-10-14", "2026-10-15")
        .await
        .map_err(map)?;
    let mem = memory_reader
        .page_requests_daily(&wide, "2026-10-14", "2026-10-15")
        .await
        .map_err(map)?;
    assert_eq!(ch, mem, "memory vs clickhouse parity");
    assert_eq!(ch.len(), 2);
    assert_eq!(ch[0].day, "2026-10-14");
    assert_eq!(ch[0].requests, 3);
    assert_eq!(ch[0].not_found, 1);
    assert_eq!(
        ch[0].bot_class_mix,
        vec![
            ("search_engine_bot".to_string(), 1),
            ("unclassified".to_string(), 2),
        ]
    );
    assert_eq!(ch[1].day, "2026-10-15");
    assert_eq!(ch[1].requests, 1);
    assert_eq!(ch[1].not_found, 1);
    assert_eq!(
        ch[1].bot_class_mix,
        vec![("automation_client".to_string(), 1)]
    );
    println!("page-request parity: {ch:?}");

    // The family is site-wide: publisher/platform/currency and a scope day
    // range are not filters, so a tampered scope returns the same counts.
    let tampered = QueryScope::new("orbit", time(HORIZON))
        .with_platform("levelplay")
        .with_currency("EUR")
        .with_day_range("2020-01-01", "2020-12-31");
    let same = reader
        .page_requests_daily(&tampered, "2026-10-14", "2026-10-15")
        .await
        .map_err(map)?;
    assert_eq!(same, ch, "a tampered scope changes nothing");
    let memory_same = memory_reader
        .page_requests_daily(&tampered, "2026-10-14", "2026-10-15")
        .await
        .map_err(map)?;
    assert_eq!(memory_same, mem, "memory agrees on the site-wide contract");

    // Inclusive day bounds; only 15 October in the one-day window.
    let single = reader
        .page_requests_daily(&wide, "2026-10-15", "2026-10-15")
        .await
        .map_err(map)?;
    assert_eq!(single.len(), 1);
    assert_eq!(single[0].requests, 1);
    assert_eq!(single[0].not_found, 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn real_clickhouse_page_requests_daily_parity() {
    let http_url = match std::env::var(CH_ENV) {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            println!("SKIP: {CH_ENV} is not set");
            return;
        }
    };
    let database = unique_database();
    let config = ClickHouseConfig {
        http_url,
        database: database.clone(),
        request_timeout_ms: 60_000,
    };
    let store = match ClickHouseStore::new(config) {
        Ok(store) => store,
        Err(error) => panic!("store configuration: {error}"),
    };
    println!("page-requests database: {database}");
    let result = run_page_requests_parity(&store).await;
    let dropped = store.drop_database().await;
    match (result, dropped) {
        (Ok(()), Ok(())) => println!("page-requests PASS; test database {database} dropped"),
        (Err(error), _) => panic!("page-requests failed: {error}"),
        (Ok(()), Err(error)) => panic!("test database cleanup failed: {error}"),
    }
}

/// The whole binary against the real server: the cursor makes a rerun a
/// no-op, an appended line is delivered alone, and a rotated (smaller) file
/// reprocesses from byte 0 where the stable event id deduplicates while the
/// plain log table may repeat the row.
#[tokio::test(flavor = "multi_thread")]
async fn real_clickhouse_ingest_cursor_is_incremental() {
    let http_url = match std::env::var(CH_ENV) {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            println!("SKIP: {CH_ENV} is not set");
            return;
        }
    };
    let database = unique_database();
    let directory = std::env::temp_dir().join(format!(
        "apex-ingest-live-{}-{}",
        std::process::id(),
        database
    ));
    tokio::fs::create_dir_all(&directory).await.expect("dir");
    let log = directory.join("access.json");
    let cursor = directory.join("ingest.cursor");
    let spool = directory.join("ingest-spool.jsonl");
    // Timestamps near the fixture horizon: the 30/90-day TTLs must not drop
    // the freshly ingested rows inside the test.
    let line_a = serde_json::json!({
        "ts": 1_792_055_400.0_f64,
        "status": 200,
        "request": { "method": "GET", "host": "apexmediation.ee", "uri": "/pricing",
            "remote_ip": "203.0.113.9",
            "headers": { "User-Agent": ["Mozilla/5.0 Safari/605.1.15"] } }
    })
    .to_string();
    let line_b = serde_json::json!({
        "ts": 1_792_055_401.0_f64,
        "status": 404,
        "request": { "method": "GET", "host": "apexmediation.ee", "uri": "/gone",
            "remote_ip": "203.0.113.9",
            "headers": { "User-Agent": ["curl/8.4.0"] } }
    })
    .to_string();
    tokio::fs::write(&log, format!("{line_a}\n{line_b}\n"))
        .await
        .expect("write log");

    let run = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_apex-ingest"))
            .arg("--json")
            .arg(&log)
            .env("APEX_CLICKHOUSE_HTTP", &http_url)
            .env("APEX_CLICKHOUSE_DB", &database)
            .env("APEX_INGEST_CURSOR", &cursor)
            .env("APEX_INGEST_SPOOL", &spool)
            .env("APEX_SERVICE_VERSION", "apex-ingest-live-test")
            .output()
            .expect("run apex-ingest")
    };
    let summary = |output: &std::process::Output| -> serde_json::Value {
        assert!(
            output.status.success(),
            "apex-ingest failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("json summary")
    };

    let first = summary(&run());
    assert_eq!(first["lines"], 2);
    assert_eq!(first["request_logs"], 2);
    assert_eq!(first["events"], 2);
    assert_eq!(first["delivered"], 2);
    assert_eq!(first["events_delivered"], 2);
    assert_eq!(first["spooled"], 0);

    let store = ClickHouseStore::new(ClickHouseConfig {
        http_url: http_url.clone(),
        database: database.clone(),
        request_timeout_ms: 60_000,
    })
    .expect("store");
    async fn ingest_counts(store: &ClickHouseStore, database: &str) -> (u64, u64) {
        let rows = store
            .fetch_rows(
                &format!(
                    "SELECT (SELECT count() FROM {database}.request_logs) AS logs, \
                     (SELECT count() FROM {database}.domain_events FINAL \
                      WHERE event_type = 'public_page_requested') AS events"
                ),
                &[],
            )
            .await
            .expect("counts");
        (
            rows.first().map(|row| u64_field(row, "logs")).unwrap_or(0),
            rows.first()
                .map(|row| u64_field(row, "events"))
                .unwrap_or(0),
        )
    }
    assert_eq!(
        ingest_counts(&store, &database).await,
        (2, 2),
        "one row and one event per line"
    );

    // The rerun sees nothing new: same counts, summary reports 0 new.
    let second = summary(&run());
    assert_eq!(second["lines"], 0);
    assert_eq!(second["request_logs"], 0);
    assert_eq!(second["events"], 0);
    assert_eq!(second["delivered"], 0);
    assert_eq!(
        ingest_counts(&store, &database).await,
        (2, 2),
        "a rerun inserts nothing"
    );
    println!("rerun summary: lines=0 request_logs=0 events=0; counts still (2, 2)");

    // An appended line is delivered alone.
    let mut content = tokio::fs::read_to_string(&log).await.expect("read log");
    content.push_str(&format!(
        "{}\n",
        serde_json::json!({
            "ts": 1_792_055_402.0_f64,
            "status": 200,
            "request": { "method": "GET", "host": "apexmediation.ee", "uri": "/proof",
                "remote_ip": "203.0.113.9",
                "headers": { "User-Agent": ["Mozilla/5.0 Safari/605.1.15"] } }
        })
    ));
    tokio::fs::write(&log, &content).await.expect("append");
    let third = summary(&run());
    assert_eq!(third["lines"], 1, "only the appended line is new");
    assert_eq!(third["events"], 1);
    assert_eq!(ingest_counts(&store, &database).await, (3, 3));
    println!("appended line: lines=1; counts (3, 3)");

    // A rotated smaller file (same leading line) restarts at byte 0 and
    // reprocesses; the event id deduplicates, the log row repeats.
    tokio::fs::write(&log, format!("{line_a}\n"))
        .await
        .expect("rotate");
    let fourth = summary(&run());
    assert_eq!(fourth["cursor_restarted"], true);
    assert_eq!(fourth["lines"], 1);
    assert_eq!(
        ingest_counts(&store, &database).await,
        (4, 3),
        "log row repeats, event dedups"
    );
    println!("rotation: cursor_restarted=true; counts (4, 3) (event dedup by stable id)");

    store.drop_database().await.expect("drop database");
    let _ = tokio::fs::remove_dir_all(&directory).await;
}

/// The `rebuild_sql` fast path against the portable Rust path, on the same
/// facts: totals agree, and an incident sequence open -> acknowledged ->
/// resolved leaves no queue row for the resolved incident and exactly one
/// row (the acknowledged state) for an incident that stayed open.
async fn run_sql_parity(store: &ClickHouseStore) -> Result<(), String> {
    let map = |error: apex_analytics::store::StoreError| error.to_string();
    store.ensure_schema().await.map_err(map)?;
    store.insert(&events()).await.map_err(map)?;
    store
        .insert_report_days(&report_days())
        .await
        .map_err(map)?;

    // Incident sequence: A open -> acknowledged -> resolved; B open ->
    // acknowledged (still open).
    let incidents = vec![
        incident_event("inc_a_open", "inc_a", 30, "critical", "open", "maya"),
        incident_event("inc_a_ack", "inc_a", 20, "warning", "acknowledged", "sam"),
        incident_event("inc_a_resolved", "inc_a", 10, "resolved", "resolved", "sam"),
        incident_event("inc_b_open", "inc_b", 25, "critical", "open", "maya"),
        incident_event("inc_b_ack", "inc_b", 15, "warning", "acknowledged", "sam"),
    ];
    store.insert(&incidents).await.map_err(map)?;

    // The Rust path over the same facts.
    let mut all_events = events();
    all_events.extend(incidents);
    let rust_rows = compute_operator_queues(&all_events, &[], time(HORIZON));
    let rust_incidents: Vec<_> = rust_rows
        .iter()
        .filter(|row| row.queue == "incidents")
        .collect();
    assert_eq!(rust_incidents.len(), 2, "{rust_incidents:?}");
    assert!(
        rust_incidents.iter().all(|row| row.object_id != "inc_a"),
        "the resolved incident is not queued"
    );
    let inc_b = rust_incidents
        .iter()
        .find(|row| row.object_id == "inc_b")
        .expect("inc_b queued");
    assert_eq!(inc_b.severity, "warning");
    assert_eq!(inc_b.owner.as_deref(), Some("sam"));

    // The SQL fast path: one version for all four projections.
    for name in ProjectionName::ALL {
        let sql = rebuild_sql(store.database(), name, 11).map_err(|error| error.to_string())?;
        store
            .execute_statement(&sql)
            .await
            .map_err(|error| format!("rebuild_sql({name}) failed: {error}"))?;
    }
    let totals_sql = totals_sql(store.database(), ProjectionName::CustomerDaily, 11)
        .map_err(|error| error.to_string())?;
    let totals = store
        .fetch_rows(&totals_sql, &[])
        .await
        .map_err(|error| format!("totals_sql failed: {error}"))?;
    assert_eq!(totals[0]["rows"].as_u64(), Some(10));
    assert_eq!(
        totals[0]["revenue_micros"].as_str(),
        Some("51500000000"),
        "the SQL rebuild totals the known value exactly"
    );
    let expected: Vec<ProjectionRow> = compute_customer_daily(&report_days())
        .into_iter()
        .map(ProjectionRow::CustomerDaily)
        .collect();
    assert_eq!(
        ProjectionTotals::of(ProjectionName::CustomerDaily, &expected).revenue_micros,
        Some(51_500_000_000),
        "the portable path agrees"
    );

    // Incidents in the rebuilt queues: the resolved incident is absent and
    // the open -> acknowledged incident has exactly one row (no duplicate
    // for the historical open row).
    let queued = store
        .fetch_rows(
            &format!(
                "SELECT object_id, severity, owner, next_action FROM {}.operator_queues \
                 WHERE queue = 'incidents' AND as_of_version = 11 ORDER BY object_id",
                store.database()
            ),
            &[],
        )
        .await
        .map_err(map)?;
    assert!(
        queued
            .iter()
            .all(|row| row["object_id"].as_str() != Some("inc_a")),
        "the resolved incident must not be queued: {queued:?}"
    );
    let inc_b_rows: Vec<_> = queued
        .iter()
        .filter(|row| row["object_id"].as_str() == Some("inc_b"))
        .collect();
    assert_eq!(
        inc_b_rows.len(),
        1,
        "no duplicate rows for open+acknowledged: {queued:?}"
    );
    assert_eq!(inc_b_rows[0]["severity"].as_str(), Some("warning"));
    assert_eq!(inc_b_rows[0]["owner"].as_str(), Some("sam"));
    assert_eq!(
        inc_b_rows[0]["next_action"].as_str(),
        Some("review_incident")
    );
    assert!(
        queued
            .iter()
            .any(|row| row["object_id"].as_str() == Some("inc_018")),
        "the open fixture incident stays queued: {queued:?}"
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn real_clickhouse_sql_fast_path_agrees_with_the_portable_path() {
    let http_url = match std::env::var(CH_ENV) {
        Ok(url) if !url.trim().is_empty() => url,
        _ => {
            println!("SKIP: {CH_ENV} is not set; start the dev server with `make clickhouse-dev` and run APEX_CLICKHOUSE_HTTP=http://127.0.0.1:8123 cargo test -p apex-analytics -- --nocapture");
            return;
        }
    };
    let database = unique_database();
    let config = ClickHouseConfig {
        http_url,
        database: database.clone(),
        request_timeout_ms: 60_000,
    };
    let store = match ClickHouseStore::new(config) {
        Ok(store) => store,
        Err(error) => panic!("store configuration: {error}"),
    };
    println!("sql-parity database: {database}");
    let result = run_sql_parity(&store).await;
    let dropped = store.drop_database().await;
    match (result, dropped) {
        (Ok(()), Ok(())) => println!("sql-parity PASS; test database {database} dropped"),
        (Err(error), _) => panic!("sql-parity failed: {error}"),
        (Ok(()), Err(error)) => panic!("test database cleanup failed: {error}"),
    }
}
