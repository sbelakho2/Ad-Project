//! The in-memory reader: the same read models computed from a
//! [`MemoryStore`], used by unit tests and by fixtures.

use async_trait::async_trait;

use crate::projections::compute_operator_queues;
use crate::store::{MemoryStore, ProjectionStore, StoreError};

use super::dto::{
    ActivationFunnel, BreakdownDimension, DataQuality, ExperimentEffectiveness,
    ExperimentLandscapeRow, FeeDetail, FeeRow, GrowthCohortRow, IngestionHealthRow, LiftRow,
    ObjectTimelineEntry, OperatorQueues, PageRequestsDay, QueryScope, RevenueBreakdownRow,
    RevenueDailyRow,
};
use super::model;
use super::AnalyticsReader;

/// A reader over a [`MemoryStore`].
#[derive(Debug, Clone)]
pub struct MemoryReader {
    store: MemoryStore,
}

impl MemoryReader {
    /// Read the given store.
    pub fn new(store: MemoryStore) -> Self {
        Self { store }
    }

    fn queues_from_rows(
        as_of: &crate::timestamp::UtcMillis,
        rows: Vec<crate::projections::OperatorQueueRow>,
    ) -> OperatorQueues {
        let mut queues = OperatorQueues {
            as_of: as_of.to_rfc3339_millis(),
            incidents: Vec::new(),
            due_decisions: Vec::new(),
            stale_accounts: Vec::new(),
            overdue_jobs: Vec::new(),
            held_fees: Vec::new(),
        };
        for row in rows {
            match row.queue.as_str() {
                "incidents" => queues.incidents.push(row),
                "due_decisions" => queues.due_decisions.push(row),
                "stale_accounts" => queues.stale_accounts.push(row),
                "overdue_jobs" => queues.overdue_jobs.push(row),
                "held_fees" => queues.held_fees.push(row),
                _ => {}
            }
        }
        queues
    }
}

#[async_trait]
impl AnalyticsReader for MemoryReader {
    async fn revenue_daily(&self, scope: &QueryScope) -> Result<Vec<RevenueDailyRow>, StoreError> {
        let days = self.store.load_report_days().await?;
        Ok(model::revenue_daily(scope, &days))
    }

    async fn revenue_breakdown(
        &self,
        scope: &QueryScope,
        dimension: BreakdownDimension,
    ) -> Result<Vec<RevenueBreakdownRow>, StoreError> {
        let days = self.store.load_report_days().await?;
        Ok(model::revenue_breakdown(scope, &days, dimension))
    }

    async fn lift_rows(&self, scope: &QueryScope) -> Result<Vec<LiftRow>, StoreError> {
        let events = self.store.load_events().await?;
        Ok(model::lift_rows(scope, &events))
    }

    async fn fee_rows(&self, scope: &QueryScope) -> Result<Vec<FeeRow>, StoreError> {
        let events = self.store.load_events().await?;
        let days = self.store.load_report_days().await?;
        Ok(model::fee_rows(scope, &events, &days))
    }

    async fn fee_detail(
        &self,
        scope: &QueryScope,
        change_id: &str,
    ) -> Result<Option<FeeDetail>, StoreError> {
        let events = self.store.load_events().await?;
        Ok(model::fee_detail(scope, &events, change_id))
    }

    async fn data_quality(&self, scope: &QueryScope) -> Result<DataQuality, StoreError> {
        let days = self.store.load_report_days().await?;
        let (_, duplicates) = self.store.insert_stats();
        Ok(model::data_quality(scope, &days, Some(duplicates)))
    }

    async fn object_timeline(
        &self,
        scope: &QueryScope,
        object_type: &str,
        object_id: &str,
    ) -> Result<Vec<ObjectTimelineEntry>, StoreError> {
        let events = self.store.load_events().await?;
        Ok(model::object_timeline(
            scope,
            &events,
            object_type,
            object_id,
        ))
    }

    async fn operator_queues(&self, scope: &QueryScope) -> Result<OperatorQueues, StoreError> {
        // The queues are operator-scoped rows; like the ClickHouse projection
        // they honor the publisher filter but not the platform filter.
        let publisher = scope.publisher_id.clone();
        let events: Vec<crate::envelope::DomainEvent> = self
            .store
            .load_events()
            .await?
            .into_iter()
            .filter(|event| {
                publisher.is_empty() || event.publisher_id.as_deref() == Some(publisher.as_str())
            })
            .collect();
        let accounts = self
            .store
            .load_accounts()
            .await?
            .into_iter()
            .filter(|account| publisher.is_empty() || account.publisher_id == publisher)
            .collect::<Vec<_>>();
        let rows = compute_operator_queues(&events, &accounts, scope.as_of);
        Ok(Self::queues_from_rows(&scope.as_of, rows))
    }

    async fn ingestion_health(
        &self,
        scope: &QueryScope,
    ) -> Result<Vec<IngestionHealthRow>, StoreError> {
        let days = self.store.load_report_days().await?;
        Ok(model::ingestion_health(scope, &days))
    }

    async fn growth_cohorts(&self, scope: &QueryScope) -> Result<Vec<GrowthCohortRow>, StoreError> {
        let events = self.store.load_events().await?;
        Ok(model::growth_cohorts(scope, &events))
    }

    async fn activation_funnel(
        &self,
        scope: &QueryScope,
        cohort_month: &str,
    ) -> Result<ActivationFunnel, StoreError> {
        let events = self.store.load_events().await?;
        Ok(model::activation_funnel(scope, &events, cohort_month))
    }

    async fn experiment_landscape(
        &self,
        scope: &QueryScope,
    ) -> Result<Vec<ExperimentLandscapeRow>, StoreError> {
        let events = self.store.load_events().await?;
        Ok(model::experiment_landscape(scope, &events))
    }

    async fn experiment_effectiveness(
        &self,
        scope: &QueryScope,
        cohort_month: &str,
    ) -> Result<ExperimentEffectiveness, StoreError> {
        let events = self.store.load_events().await?;
        Ok(model::experiment_effectiveness(
            scope,
            &events,
            cohort_month,
        ))
    }

    async fn page_requests_daily(
        &self,
        scope: &QueryScope,
        from_day: &str,
        to_day: &str,
    ) -> Result<Vec<PageRequestsDay>, StoreError> {
        // The page-request family is site-wide: no tenant filter applies.
        let events = self.store.load_events().await?;
        let from = model::effective_day_bound(from_day, scope.from_day.as_deref());
        let to = model::effective_day_bound(to_day, scope.to_day.as_deref());
        Ok(model::page_requests_daily(&events, &from, &to))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::envelope::{DomainEvent, EventPayload, EventSource, EventType};
    use crate::queries::dto::{
        BreakdownDimension, FeeState, FreshnessStatus, QueryScope, RevenueDailyRow,
    };
    use crate::store::{EventStore, MemoryStore, ReportDayRow};
    use crate::test_support::{
        known_value_fee_event, known_value_report_days, occurred, payload_for, valid_event_builder,
    };
    use crate::timestamp::{UtcMillis, MILLIS_PER_DAY};

    async fn fixture_store() -> MemoryStore {
        let store = MemoryStore::new();
        store.ensure_schema().await.expect("schema");
        store
            .insert_report_days(&known_value_report_days())
            .await
            .expect("days");
        store.insert(&[known_value_fee_event()]).await.expect("fee");
        store
    }

    fn days_ago(days: i64) -> UtcMillis {
        UtcMillis::from_unix_millis(occurred().unix_millis() - days * MILLIS_PER_DAY)
    }

    /// A richer event set covering fees in every state, an experiment
    /// family, a request cohort and an incident.
    async fn populated_store() -> MemoryStore {
        let store = fixture_store().await;
        let mut events: Vec<DomainEvent> = vec![valid_event_builder(EventType::FeeHeld)
            .with_event_id("fee_held_1")
            .with_payload(EventPayload {
                change_id: Some("ch_0247".into()),
                period: Some("2026-10".into()),
                currency: Some("USD".into()),
                ..EventPayload::default()
            })
            .build()
            .expect("held")];
        events.push(
            valid_event_builder(EventType::FeeInvoiceIssued)
                .with_event_id("fee_issued_1")
                .with_payload(EventPayload {
                    change_id: Some("ch_0248".into()),
                    period: Some("2026-10".into()),
                    currency: Some("USD".into()),
                    invoice_id: Some("INV-2026-104".into()),
                    amount_micros: Some(225_000_000),
                    ..EventPayload::default()
                })
                .build()
                .expect("issued"),
        );
        events.push(
            valid_event_builder(EventType::PaymentReconciled)
                .with_event_id("payment_1")
                .with_payload(EventPayload {
                    invoice_id: Some("INV-2026-104".into()),
                    currency: Some("USD".into()),
                    amount_micros: Some(225_000_000),
                    ..EventPayload::default()
                })
                .build()
                .expect("payment"),
        );
        events.push(
            valid_event_builder(EventType::FeeCredited)
                .with_event_id("fee_credited_1")
                .with_payload(EventPayload {
                    change_id: Some("ch_0249".into()),
                    period: Some("2026-10".into()),
                    currency: Some("USD".into()),
                    invoice_id: Some("INV-2026-105".into()),
                    amount_micros: Some(10_000_000),
                    ..EventPayload::default()
                })
                .build()
                .expect("credited"),
        );
        // Held yesterday, invoiced today: not held any more.
        events.push(
            valid_event_builder(EventType::FeeHeld)
                .with_event_id("fee_held_2")
                .with_occurred_at(days_ago(1))
                .with_received_at(days_ago(1))
                .with_payload(EventPayload {
                    change_id: Some("ch_0250".into()),
                    period: Some("2026-10".into()),
                    currency: Some("USD".into()),
                    ..EventPayload::default()
                })
                .build()
                .expect("held 2"),
        );
        events.push(
            valid_event_builder(EventType::FeeInvoiceIssued)
                .with_event_id("fee_issued_2")
                .with_payload(EventPayload {
                    change_id: Some("ch_0250".into()),
                    period: Some("2026-10".into()),
                    currency: Some("USD".into()),
                    invoice_id: Some("INV-2026-106".into()),
                    amount_micros: Some(50_000_000),
                    ..EventPayload::default()
                })
                .build()
                .expect("issued 2"),
        );

        events.push(
            valid_event_builder(EventType::ExperimentRegistered)
                .with_event_id("exp_reg_1")
                .with_payload(EventPayload {
                    experiment_id: Some("exp_keep".into()),
                    source_digest: Some("a".repeat(64)),
                    registered_metric: Some("revenue_per_user".into()),
                    window_days: Some(14),
                    ..EventPayload::default()
                })
                .build()
                .expect("registered"),
        );
        events.push(
            valid_event_builder(EventType::ExperimentStarted)
                .with_event_id("exp_start_1")
                .with_occurred_at(days_ago(20))
                .with_received_at(days_ago(20))
                .with_payload(EventPayload {
                    experiment_id: Some("exp_keep".into()),
                    state: Some("running".into()),
                    ..EventPayload::default()
                })
                .build()
                .expect("started keep"),
        );
        events.push(
            valid_event_builder(EventType::ExperimentDecided)
                .with_event_id("exp_decided_1")
                .with_payload(EventPayload {
                    experiment_id: Some("exp_keep".into()),
                    decision: Some("keep".into()),
                    point_estimate_bp: Some(540),
                    decision_bound_bp: Some(300),
                    ..EventPayload::default()
                })
                .build()
                .expect("decided keep"),
        );
        events.push(
            valid_event_builder(EventType::ExperimentDecided)
                .with_event_id("exp_decided_void")
                .with_payload(EventPayload {
                    experiment_id: Some("exp_void".into()),
                    decision: Some("void".into()),
                    ..EventPayload::default()
                })
                .build()
                .expect("decided void"),
        );

        // A request cohort in August 2026 (mature at the as-of) whose one
        // account only qualified.
        let august = UtcMillis::parse_rfc3339("2026-08-05T09:00:00.000Z").expect("august");
        events.push(
            DomainEvent::builder(EventType::AuditRequestAccepted, EventSource::Axum)
                .with_event_id("aug_request")
                .with_publisher_id("northstar")
                .with_environment("production")
                .with_service_version("apex-0.1.0")
                .with_occurred_at(august)
                .with_received_at(august)
                .with_payload(EventPayload {
                    lead_id: Some("lead_aug".into()),
                    account_id: Some("acct_aug".into()),
                    platforms: Some(vec!["applovin_max".into()]),
                    range_start: Some("2026-07-01".into()),
                    range_end: Some("2026-07-31".into()),
                    source_code: Some("seo".into()),
                    ..EventPayload::default()
                })
                .build()
                .expect("august request"),
        );
        events.push(
            DomainEvent::builder(EventType::AccountStageChanged, EventSource::Axum)
                .with_event_id("aug_stage")
                .with_publisher_id("northstar")
                .with_environment("production")
                .with_service_version("apex-0.1.0")
                .with_occurred_at(august)
                .with_received_at(august)
                .with_reason_code("qualified")
                .with_object("account", "acct_aug")
                .with_payload(EventPayload {
                    previous_stage: Some("requested".into()),
                    next_stage: Some("qualified".into()),
                    owner: Some("maya".into()),
                    effective_at: Some("2026-08-05T09:00:00Z".into()),
                    ..EventPayload::default()
                })
                .build()
                .expect("august stage"),
        );

        events.push(
            valid_event_builder(EventType::IncidentChanged)
                .with_event_id("incident_1")
                .with_outcome("critical")
                .with_payload(EventPayload {
                    incident_id: Some("inc_1".into()),
                    owner: Some("maya".into()),
                    impact: Some("state unresolved".into()),
                    state: Some("open".into()),
                    ..EventPayload::default()
                })
                .build()
                .expect("incident"),
        );
        store.insert(&events).await.expect("events");
        store
    }

    fn scope() -> QueryScope {
        QueryScope::new("northstar", occurred())
    }

    #[tokio::test]
    async fn revenue_models_use_the_known_values() {
        let reader = MemoryReader::new(populated_store().await);
        let daily = reader.revenue_daily(&scope()).await.expect("daily");
        assert_eq!(daily.len(), 10);
        let total: i64 = daily.iter().map(|row| row.revenue.micros).sum();
        assert_eq!(total, 51_500_000_000);
        assert!(daily.iter().all(|row| row.revenue.currency == "USD"));
        assert!(daily.iter().all(|row| row.impressions == Some(2_500_000)));

        let by_country = reader
            .revenue_breakdown(&scope(), BreakdownDimension::Country)
            .await
            .expect("countries");
        assert_eq!(by_country.len(), 2);
        let de = by_country
            .iter()
            .find(|row| row.dimension_value == "DE")
            .expect("DE");
        assert_eq!(de.revenue.micros, 30_900_000_000);
        assert_eq!(de.share_bp, Some(6_000));
        let total_shares: u32 = by_country.iter().map(|row| row.share_bp.unwrap_or(0)).sum();
        assert_eq!(total_shares, 10_000);

        // Filters narrow the scope; another tenant sees nothing.
        let scoped_platform = reader
            .revenue_daily(&scope().with_platform("levelplay"))
            .await
            .expect("platform filter");
        assert!(scoped_platform.is_empty());
        let ranged = reader
            .revenue_daily(&scope().with_day_range("2026-10-02", "2026-10-03"))
            .await
            .expect("range");
        assert_eq!(ranged.len(), 2);
        let other = reader
            .revenue_daily(&QueryScope::new("orbit", occurred()))
            .await
            .expect("other");
        assert!(other.is_empty());
    }

    #[tokio::test]
    async fn lift_rows_carry_outcomes_and_measurement_sources() {
        let reader = MemoryReader::new(populated_store().await);
        let rows = reader.lift_rows(&scope()).await.expect("lift");
        assert_eq!(rows.len(), 2);
        let keep = rows
            .iter()
            .find(|row| row.experiment_id == "exp_keep")
            .expect("keep");
        assert_eq!(keep.outcome, "keep");
        assert_eq!(keep.observed_lift_bp, Some(540));
        assert_eq!(keep.billable_bound_bp, Some(300));
        assert_eq!(keep.measurement_source.as_deref(), Some("revenue_per_user"));
        let void = rows
            .iter()
            .find(|row| row.experiment_id == "exp_void")
            .expect("void");
        assert_eq!(void.outcome, "void");
        assert_eq!(void.billable_bound_bp, None, "void has no billable bound");
        assert_eq!(
            void.measurement_source, None,
            "no registration means no measurement-source claim"
        );
    }

    #[tokio::test]
    async fn fee_models_cover_every_state() {
        let reader = MemoryReader::new(populated_store().await);
        let rows = reader.fee_rows(&scope()).await.expect("fees");
        let state_of = |change: &str| {
            rows.iter()
                .find(|row| row.change_id == change)
                .map(|row| row.state)
                .unwrap_or_else(|| panic!("missing fee row for {change}"))
        };
        assert_eq!(state_of("ch_0246"), FeeState::Estimated);
        assert_eq!(state_of("ch_0247"), FeeState::Held);
        assert_eq!(state_of("ch_0248"), FeeState::Paid);
        assert_eq!(state_of("ch_0249"), FeeState::Credited);
        assert_eq!(state_of("ch_0250"), FeeState::Invoiced);
        let estimated = rows
            .iter()
            .find(|row| row.change_id == "ch_0246")
            .expect("estimated");
        assert_eq!(
            estimated.amount.as_ref().map(|money| money.micros),
            Some(300_000_000)
        );
        assert_eq!(estimated.proof_refs.len(), 1);
        let invoiced = rows
            .iter()
            .find(|row| row.change_id == "ch_0250")
            .expect("invoiced");
        assert_eq!(invoiced.invoice_ref.as_deref(), Some("INV-2026-106"));
    }

    #[tokio::test]
    async fn a_provisional_period_makes_a_computed_fee_pending() {
        let store = fixture_store().await;
        store
            .insert_report_days(&[ReportDayRow {
                publisher_id: "northstar".into(),
                platform: "applovin_max".into(),
                platform_account_id: "max_acct_1".into(),
                day: "2026-10-11".into(),
                finality: "provisional".into(),
                revenue_micros: Some(100_000_000),
                impressions: None,
                currency: Some("USD".into()),
                country: None,
                ad_unit_id: None,
                format: None,
                source_digest: None,
            }])
            .await
            .expect("provisional day");
        let reader = MemoryReader::new(store);
        let rows = reader.fee_rows(&scope()).await.expect("fees");
        let fee = rows
            .iter()
            .find(|row| row.change_id == "ch_0246")
            .expect("fee");
        assert_eq!(fee.state, FeeState::Pending);
    }

    #[tokio::test]
    async fn fee_detail_reproduces_the_known_value_and_names_gaps() {
        let reader = MemoryReader::new(populated_store().await);
        let detail = reader
            .fee_detail(&scope(), "ch_0246")
            .await
            .expect("read")
            .expect("known-value detail");
        assert_eq!(detail.eligible_revenue.micros, 51_500_000_000);
        assert_eq!(detail.eligible_revenue.currency, "USD");
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

        // No computed event named: missing, not zero.
        assert!(reader
            .fee_detail(&scope(), "ch_missing")
            .await
            .expect("read")
            .is_none());

        // A rate outside the contract: the fee is not invented.
        let store = fixture_store().await;
        store
            .insert(&[valid_event_builder(EventType::FeeComputed)
                .with_event_id("fee_bad_rate")
                .with_payload(EventPayload {
                    change_id: Some("ch_0251".into()),
                    period: Some("2026-10".into()),
                    currency: Some("USD".into()),
                    source_amount_micros: Some(10_000_000_000),
                    bound_bp: Some(300),
                    rate_bp: Some(5_000),
                    active_days: Some(31),
                    proof_hashes: Some(vec!["e".repeat(64)]),
                    ..EventPayload::default()
                })
                .build()
                .expect("bad rate event")])
            .await
            .expect("insert");
        let reader = MemoryReader::new(store);
        let detail = reader
            .fee_detail(&scope(), "ch_0251")
            .await
            .expect("read")
            .expect("detail");
        assert_eq!(detail.fee_before_tax, None);
        assert!(detail.attributed_increment.is_some());
        assert!(detail
            .unavailable
            .iter()
            .any(|reason| reason.contains("rate outside the agreed contract")));
    }

    #[tokio::test]
    async fn data_quality_reports_gaps_stale_sources_and_reasons() {
        let store = populated_store().await;
        // An amount-less row is a gap in the money facts.
        store
            .insert_report_days(&[ReportDayRow {
                publisher_id: "northstar".into(),
                platform: "applovin_max".into(),
                platform_account_id: "max_acct_1".into(),
                day: "2026-10-11".into(),
                finality: "final".into(),
                revenue_micros: None,
                impressions: None,
                currency: Some("USD".into()),
                country: None,
                ad_unit_id: None,
                format: None,
                source_digest: None,
            }])
            .await
            .expect("gap row");
        let reader = MemoryReader::new(store);
        let quality = reader
            .data_quality(&scope().with_day_range("2026-10-01", "2026-10-12"))
            .await
            .expect("quality");
        assert_eq!(quality.matched_source_days, 11);
        assert_eq!(quality.missing_days, Some(1), "12 October has no row");
        assert_eq!(quality.unmapped_rows, 1, "the amount-less row");
        assert_eq!(quality.dedup_rejects, Some(0));
        assert_eq!(quality.stale_sources.len(), 1);
        assert_eq!(
            quality.stale_sources[0].last_final_day.as_deref(),
            Some("2026-10-11"),
            "the amount-less row is still a finalized day"
        );
        assert_eq!(quality.stale_sources[0].freshness_days, Some(4));

        // Without a range there is nothing to be missing from.
        let quality = reader.data_quality(&scope()).await.expect("quality");
        assert_eq!(quality.missing_days, None);
        assert!(quality
            .unavailable
            .iter()
            .any(|reason| reason.contains("no day range")));
    }

    #[tokio::test]
    async fn timelines_merge_by_occurrence_and_keep_received() {
        let store = populated_store().await;
        let late = UtcMillis::parse_rfc3339("2026-10-15T08:00:00.000Z").expect("late import");
        store
            .insert(&[
                DomainEvent::builder(EventType::ReportPullCompleted, EventSource::Worker)
                    .with_event_id("late_event")
                    .with_publisher_id("northstar")
                    .with_platform("applovin_max")
                    .with_environment("production")
                    .with_service_version("apex-0.1.0")
                    .with_object("change", "ch_0246")
                    .with_occurred_at(days_ago(1))
                    .with_received_at(late)
                    .with_payload(payload_for(EventType::ReportPullCompleted))
                    .build()
                    .expect("late event"),
            ])
            .await
            .expect("insert");
        let reader = MemoryReader::new(store);
        let timeline = reader
            .object_timeline(&scope(), "change", "ch_0246")
            .await
            .expect("timeline");
        assert_eq!(timeline.len(), 2);
        assert!(
            timeline[0].occurred_at_utc < timeline[1].occurred_at_utc,
            "merged by occurrence, not receipt"
        );
        assert_eq!(timeline[0].event_type, EventType::ReportPullCompleted);
        assert_eq!(timeline[1].event_type, EventType::FeeComputed);
        assert!(timeline[0].received_at_utc >= timeline[0].occurred_at_utc);
    }

    #[tokio::test]
    async fn operator_queues_cover_the_five_queues() {
        let store = populated_store().await;
        let reader = MemoryReader::new(store);
        let queues = reader
            .operator_queues(&QueryScope::operator_wide(occurred()))
            .await
            .expect("queues");
        assert_eq!(queues.incidents.len(), 1);
        assert_eq!(queues.incidents[0].object_id, "inc_1");
        assert_eq!(queues.incidents[0].severity, "critical");
        assert!(
            queues.due_decisions.is_empty(),
            "exp_keep was decided; exp_void never started"
        );
        assert_eq!(queues.held_fees.len(), 1);
        assert_eq!(queues.held_fees[0].object_id, "ch_0247:2026-10");
        assert!(queues.overdue_jobs.is_empty());
        assert!(queues.stale_accounts.is_empty());
    }

    #[tokio::test]
    async fn ingestion_health_measures_freshness() {
        let store = populated_store().await;
        // A second platform account with no finalized day at all.
        store
            .insert_report_days(&[ReportDayRow {
                publisher_id: "northstar".into(),
                platform: "levelplay".into(),
                platform_account_id: "lp_acct_1".into(),
                day: "2026-09-01".into(),
                finality: "partial".into(),
                revenue_micros: Some(1_000_000),
                impressions: None,
                currency: Some("USD".into()),
                country: None,
                ad_unit_id: None,
                format: None,
                source_digest: None,
            }])
            .await
            .expect("partial-only source row");
        let reader = MemoryReader::new(store);
        let rows = reader.ingestion_health(&scope()).await.expect("health");
        assert_eq!(rows.len(), 2);
        let applovin = rows
            .iter()
            .find(|row| row.platform == "applovin_max")
            .expect("applovin");
        assert_eq!(applovin.last_final_day.as_deref(), Some("2026-10-10"));
        assert_eq!(applovin.freshness_days, Some(5));
        assert_eq!(applovin.status, FreshnessStatus::Stale);
        let levelplay = rows
            .iter()
            .find(|row| row.platform == "levelplay")
            .expect("levelplay");
        assert_eq!(levelplay.last_final_day, None);
        assert_eq!(levelplay.status, FreshnessStatus::MissingData);
    }

    #[tokio::test]
    async fn growth_and_funnel_gate_on_cohort_maturity() {
        let reader = MemoryReader::new(populated_store().await);
        let cohorts = reader.growth_cohorts(&scope()).await.expect("cohorts");
        assert_eq!(cohorts.len(), 1, "only August has an accepted request");
        let august = &cohorts[0];
        assert_eq!(august.cohort_month, "2026-08");
        assert_eq!(august.accepted, 1);
        assert_eq!(august.accepted_accounts, 1);
        assert_eq!(august.qualified, 1);
        assert_eq!(august.connected, 0);
        assert_eq!(august.qualification_rate_bp, Some(10_000));
        assert!(august.mature, "August cleared the 30-day horizon");

        let funnel = reader
            .activation_funnel(&scope(), "2026-08")
            .await
            .expect("funnel");
        assert!(funnel.mature);
        let stages: Vec<(&str, u64)> = funnel
            .stages
            .iter()
            .map(|stage| (stage.stage.as_str(), stage.accounts))
            .collect();
        assert_eq!(
            stages,
            vec![
                ("requested", 1),
                ("qualified", 1),
                ("connected", 0),
                ("audit_delivered", 0),
                ("paid", 0),
            ]
        );
    }

    #[tokio::test]
    async fn landscape_and_effectiveness_keep_unknowns_out_of_denominators() {
        let reader = MemoryReader::new(populated_store().await);
        let landscape = reader
            .experiment_landscape(&scope())
            .await
            .expect("landscape");
        let states: Vec<(&str, u64)> = landscape
            .iter()
            .map(|row| (row.state.as_str(), row.count))
            .collect();
        assert_eq!(states, vec![("keep", 1), ("void", 1)]);

        // A month with no started experiments: rates are None, never zero.
        let empty = reader
            .experiment_effectiveness(&scope(), "2026-07")
            .await
            .expect("empty");
        assert_eq!(empty.started, 0);
        assert_eq!(empty.keep_rate_valid_bp, None);
        assert_eq!(empty.keep_rate_started_bp, None);
        assert_eq!(empty.completion_p50_days, None);

        // The keep experiment started 25 September (20 days before the
        // as-of) and was decided on 15 October, so its completion time is 20
        // days; p50 and p95 agree on one settled test.
        let effectiveness = reader
            .experiment_effectiveness(&scope(), "2026-09")
            .await
            .expect("effectiveness");
        assert_eq!(effectiveness.started, 1);
        assert_eq!(effectiveness.kept, 1);
        assert_eq!(effectiveness.valid_settled, 1);
        assert_eq!(effectiveness.pending, 0);
        assert_eq!(effectiveness.unknown, 0);
        assert_eq!(effectiveness.in_flight, 0);
        assert_eq!(effectiveness.keep_rate_valid_bp, Some(10_000));
        assert_eq!(effectiveness.keep_rate_started_bp, Some(10_000));
        assert_eq!(effectiveness.completion_p50_days, Some(20));
        assert_eq!(effectiveness.completion_p95_days, Some(20));
        assert_eq!(
            effectiveness.excluded_from_denominators,
            vec![
                "pending".to_string(),
                "void".to_string(),
                "unknown".to_string(),
                "in_flight".to_string()
            ]
        );

        // October's start cohort is empty: nothing settled there.
        let october = reader
            .experiment_effectiveness(&scope(), "2026-10")
            .await
            .expect("october");
        assert_eq!(october.started, 0);
        assert_eq!(october.keep_rate_valid_bp, None);
    }

    #[tokio::test]
    async fn readers_refuse_before_the_schema_exists() {
        let reader = MemoryReader::new(MemoryStore::new());
        assert!(matches!(
            reader.revenue_daily(&scope()).await,
            Err(StoreError::SchemaNotReady)
        ));
        assert!(matches!(
            reader.fee_rows(&scope()).await,
            Err(StoreError::SchemaNotReady)
        ));
        assert!(matches!(
            reader.page_requests_daily(&scope(), "", "").await,
            Err(StoreError::SchemaNotReady)
        ));
    }

    fn page_event(id: &str, occurred_at: &str, status: u16, bot_class: &str) -> DomainEvent {
        let at = UtcMillis::parse_rfc3339(occurred_at).expect("time");
        DomainEvent::builder(EventType::PublicPageRequested, EventSource::AccessLog)
            .with_event_id(id)
            .with_occurred_at(at)
            .with_received_at(at)
            .with_environment("production")
            .with_service_version("apex-0.1.0")
            .with_object("document", "/pricing")
            .with_outcome(if status == 404 { "4xx" } else { "2xx" })
            .with_payload(EventPayload {
                route_template: Some("/pricing".into()),
                status: Some(status),
                bot_class: Some(bot_class.into()),
                ..EventPayload::default()
            })
            .build()
            .expect("page event")
    }

    #[tokio::test]
    async fn page_requests_are_site_wide_daily_counts() {
        let store = MemoryStore::new();
        store.ensure_schema().await.expect("schema");
        store
            .insert(&[
                page_event("pr_1", "2026-10-14T08:00:00.000Z", 200, "unclassified"),
                page_event("pr_2", "2026-10-14T09:30:00.000Z", 200, "search_engine_bot"),
                page_event("pr_3", "2026-10-14T23:59:59.999Z", 404, "unclassified"),
                page_event("pr_4", "2026-10-15T00:00:00.000Z", 404, "automation_client"),
            ])
            .await
            .expect("events");
        let reader = MemoryReader::new(store);
        let wide = QueryScope::operator_wide(occurred());
        let days = reader
            .page_requests_daily(&wide, "2026-10-14", "2026-10-15")
            .await
            .expect("days");
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].day, "2026-10-14");
        assert_eq!(days[0].requests, 3);
        assert_eq!(days[0].not_found, 1);
        assert_eq!(
            days[0].bot_class_mix,
            vec![
                ("search_engine_bot".to_string(), 1),
                ("unclassified".to_string(), 2),
            ]
        );
        assert_eq!(days[1].day, "2026-10-15");
        assert_eq!(days[1].requests, 1);
        assert_eq!(days[1].not_found, 1);
        assert_eq!(
            days[1].bot_class_mix,
            vec![("automation_client".to_string(), 1)]
        );

        // Inclusive bounds; an empty bound is unbounded on that side.
        let single = reader
            .page_requests_daily(&wide, "2026-10-14", "2026-10-14")
            .await
            .expect("single day");
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].day, "2026-10-14");
        let all = reader
            .page_requests_daily(&wide, "", "")
            .await
            .expect("unbounded");
        assert_eq!(all, days);

        // The family is site-wide: tenant, platform, currency and a scope
        // day range are NOT filters, so a tampered scope changes nothing.
        let tampered = QueryScope::new("northstar", occurred())
            .with_platform("applovin_max")
            .with_currency("EUR")
            .with_day_range("2020-01-01", "2020-01-02");
        let same = reader
            .page_requests_daily(&tampered, "2026-10-14", "2026-10-15")
            .await
            .expect("wide");
        assert_eq!(same, days, "a tampered scope changes nothing");
        // The scope range is only a fallback for an empty argument.
        let fallback = reader
            .page_requests_daily(&tampered, "", "")
            .await
            .expect("fallback");
        assert!(fallback.is_empty(), "{fallback:?}");
    }

    #[tokio::test]
    async fn revenue_days_round_trip_through_json() {
        let reader = MemoryReader::new(populated_store().await);
        let daily: Vec<RevenueDailyRow> = reader.revenue_daily(&scope()).await.expect("daily");
        let json = serde_json::to_string(&daily).expect("json");
        assert!(json.contains("5150000000"), "{json}");
        let back: Vec<RevenueDailyRow> = serde_json::from_str(&json).expect("back");
        assert_eq!(back, daily);
    }

    #[tokio::test]
    async fn replays_are_one_fact_for_the_reader() {
        let store = MemoryStore::new();
        store.ensure_schema().await.expect("schema");
        let event = known_value_fee_event();
        store
            .insert(std::slice::from_ref(&event))
            .await
            .expect("first");
        store
            .insert(std::slice::from_ref(&event))
            .await
            .expect("replay");
        let reader = MemoryReader::new(store);
        let rows = reader.fee_rows(&scope()).await.expect("fees");
        assert_eq!(rows.len(), 1, "a replay is one fact");
    }
}
