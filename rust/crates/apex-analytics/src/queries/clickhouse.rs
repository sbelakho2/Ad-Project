//! The ClickHouse reader: bound-parameter fetches plus the shared pure
//! computations.
//!
//! Every statement uses `{name:Type}` bound parameters delivered over the
//! HTTP `param_*` keys; no scope value is ever interpolated into SQL. An
//! empty string parameter means "no filter", which keeps one statement shape
//! per read model. The money read models (`revenue_daily`,
//! `operator_queues`) read the projection tables the spec names for exactly
//! this purpose, pinned to the currently PUBLISHED as-of version (staged
//! rebuilds stay invisible until `projections::publish`); the remaining
//! models are event-sourced and must stay behind the projection compare
//! before a switch (see [`crate::projections::enforce_switch`]).
//!
//! Event-scoped models fetch the scope's history: bounded by the scope's
//! day range when one is set, and by the 90-day raw-event TTL otherwise.
//! The duplicate scan honors the scope's day range too, and `fee_detail`
//! narrows to the one change (its `change_id` or `object_id`) so a detail
//! page never scans the publisher's whole history.

use async_trait::async_trait;

use crate::envelope::DomainEvent;
use crate::projections::OperatorQueueRow;
use crate::store::{domain_event_from_row, ClickHouseStore, ReportDayRow, StoreError};

use super::dto::{
    ActivationFunnel, BreakdownDimension, DataQuality, ExperimentEffectiveness,
    ExperimentLandscapeRow, FeeDetail, FeeRow, GrowthCohortRow, IngestionHealthRow, LiftRow, Money,
    ObjectTimelineEntry, OperatorQueues, QueryScope, RevenueBreakdownRow, RevenueDailyRow,
};
use super::model;
use super::AnalyticsReader;

/// A reader over a [`ClickHouseStore`].
#[derive(Debug, Clone)]
pub struct ClickHouseReader {
    store: ClickHouseStore,
}

impl ClickHouseReader {
    /// Read the given store.
    pub fn new(store: ClickHouseStore) -> Self {
        Self { store }
    }

    /// The underlying store (for rebuilds and health checks).
    pub fn store(&self) -> &ClickHouseStore {
        &self.store
    }

    /// The scope filters every statement shares. An empty parameter means
    /// "no filter"; the values are bound, never interpolated.
    fn parameters(scope: &QueryScope) -> Vec<(&str, &str)> {
        vec![
            ("publisher", scope.publisher_id.as_str()),
            ("platform", scope.platform.as_deref().unwrap_or("")),
            ("currency", scope.currency.as_deref().unwrap_or("")),
            ("from", scope.from_day.as_deref().unwrap_or("")),
            ("to", scope.to_day.as_deref().unwrap_or("")),
        ]
    }

    async fn scoped_events(&self, scope: &QueryScope) -> Result<Vec<DomainEvent>, StoreError> {
        let database = self.store.database();
        let select = change_scoped_select(database, None);
        let rows = self
            .store
            .fetch_rows(&select, &Self::parameters(scope))
            .await?;
        rows.iter().map(domain_event_from_row).collect()
    }

    /// The events of one change only: `fee_detail` needs the change's own
    /// rows (its `change_id` or `object_id`), not the publisher's history.
    async fn change_events(
        &self,
        scope: &QueryScope,
        change_id: &str,
    ) -> Result<Vec<DomainEvent>, StoreError> {
        let database = self.store.database();
        let select = change_scoped_select(database, Some("{change:String}"));
        let mut parameters = Self::parameters(scope);
        parameters.push(("change", change_id));
        let rows = self.store.fetch_rows(&select, &parameters).await?;
        rows.iter().map(domain_event_from_row).collect()
    }

    async fn scoped_report_days(
        &self,
        scope: &QueryScope,
    ) -> Result<Vec<ReportDayRow>, StoreError> {
        let database = self.store.database();
        let select = format!(
            "SELECT publisher_id, platform, platform_account_id, toString(day) AS day, \
             finality, revenue_micros, impressions, currency, country, ad_unit_id, format, \
             source_digest FROM {database}.report_days FINAL\n\
             WHERE ({{publisher:String}} = '' OR publisher_id = {{publisher:String}})\n\
             \x20 AND ({{platform:String}} = '' OR platform = {{platform:String}})\n\
             \x20 AND ({{currency:String}} = '' OR currency = {{currency:String}})\n\
             \x20 AND ({{from:String}} = '' OR report_days.day >= toDateOrZero({{from:String}}))\n\
             \x20 AND ({{to:String}} = '' OR report_days.day <= toDateOrZero({{to:String}}))\n\
             ORDER BY publisher_id, platform, platform_account_id, day"
        );
        let rows = self
            .store
            .fetch_rows(&select, &Self::parameters(scope))
            .await?;
        rows.iter()
            .map(|row| {
                serde_json::from_value(row.clone())
                    .map_err(|error| StoreError::Json(error.to_string()))
            })
            .collect()
    }
}

/// The scope-filtered `domain_events` select. With `change` as
/// `Some("{change:String}")` the statement also narrows to one change's
/// events (`change_id` or `object_id`), which keeps a fee detail page from
/// scanning the publisher's whole history; the `{change:String}` parameter
/// is bound by the caller and never interpolated.
fn change_scoped_select(database: &str, change: Option<&str>) -> String {
    let change_filter = match change {
        Some(parameter) => format!(
            "({parameter} = '' OR change_id = {parameter} OR object_id = {parameter})\n\
             \x20 AND "
        ),
        None => String::new(),
    };
    format!(
        "SELECT * FROM {database}.domain_events FINAL\n\
         WHERE {change_filter}({{publisher:String}} = '' OR publisher_id = {{publisher:String}})\n\
         \x20 AND ({{platform:String}} = '' OR platform = {{platform:String}})\n\
         \x20 AND ({{currency:String}} = '' OR currency = {{currency:String}})\n\
         \x20 AND ({{from:String}} = '' OR toDate(occurred_at_utc) >= toDateOrZero({{from:String}}))\n\
         \x20 AND ({{to:String}} = '' OR toDate(occurred_at_utc) <= toDateOrZero({{to:String}}))\n\
         ORDER BY occurred_at_utc, source, event_id"
    )
}

/// The physical-duplicate scan over `domain_events`: bounded by the scope's
/// day range when one is set, unbounded (the whole 90-day raw-event window)
/// otherwise.
fn duplicate_scan_select(database: &str) -> String {
    format!(
        "SELECT count() - uniqExact((source, event_id)) AS duplicate_rows \
         FROM {database}.domain_events\n\
         WHERE ({{publisher:String}} = '' OR publisher_id = {{publisher:String}})\n\
         \x20 AND ({{from:String}} = '' OR toDate(occurred_at_utc) >= toDateOrZero({{from:String}}))\n\
         \x20 AND ({{to:String}} = '' OR toDate(occurred_at_utc) <= toDateOrZero({{to:String}}))"
    )
}

/// Attach share basis points to a raw breakdown, per currency.
fn finish_breakdown(rows: Vec<(String, Money, Option<u64>, u32)>) -> Vec<RevenueBreakdownRow> {
    let mut totals: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    for (_, money, _, _) in &rows {
        *totals.entry(money.currency.clone()).or_insert(0) += money.micros;
    }
    rows.into_iter()
        .map(|(dimension_value, revenue, impressions, count)| {
            let total = totals.get(&revenue.currency).copied().unwrap_or(0);
            let share_bp = if revenue.micros >= 0 {
                model::rate_basis_points(revenue.micros as u64, total.max(0) as u64)
            } else {
                None
            };
            RevenueBreakdownRow {
                dimension_value,
                revenue,
                impressions,
                rows: count,
                share_bp,
            }
        })
        .collect()
}

#[async_trait]
impl AnalyticsReader for ClickHouseReader {
    async fn revenue_daily(&self, scope: &QueryScope) -> Result<Vec<RevenueDailyRow>, StoreError> {
        let database = self.store.database();
        // Pinned to the currently PUBLISHED as-of version: a staged rebuild
        // stays invisible until `projections::publish`. No FINAL: FINAL
        // would collapse rows across versions and hide the published rows
        // behind a newer staged one.
        let select = format!(
            "SELECT toString(day) AS day, platform, currency, revenue_micros, impressions, \
             source_days, final_days, provisional_days FROM {database}.customer_daily\n\
             WHERE ({{publisher:String}} = '' OR publisher_id = {{publisher:String}})\n\
             \x20 AND ({{platform:String}} = '' OR platform = {{platform:String}})\n\
             \x20 AND ({{currency:String}} = '' OR currency = {{currency:String}})\n\
             \x20 AND ({{from:String}} = '' OR customer_daily.day >= toDateOrZero({{from:String}}))\n\
             \x20 AND ({{to:String}} = '' OR customer_daily.day <= toDateOrZero({{to:String}}))\n\
             \x20 AND as_of_version = (SELECT max(as_of_version) FROM {database}.projection_versions \
             WHERE projection_name = 'customer_daily' AND published = 1)\n\
             ORDER BY day, platform, currency"
        );
        let rows = self
            .store
            .fetch_rows(&select, &Self::parameters(scope))
            .await?;
        rows.iter()
            .map(|row| {
                let text = |key: &str| {
                    row.get(key)
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| StoreError::Json(format!("missing column '{key}'")))
                };
                let revenue_micros = row
                    .get("revenue_micros")
                    .and_then(serde_json::Value::as_i64)
                    .ok_or_else(|| {
                        StoreError::Json("missing column 'revenue_micros'".to_string())
                    })?;
                Ok(RevenueDailyRow {
                    day: text("day")?.to_string(),
                    platform: text("platform")?.to_string(),
                    revenue: Money::new(revenue_micros, text("currency")?),
                    impressions: row.get("impressions").and_then(serde_json::Value::as_u64),
                    source_days: row
                        .get("source_days")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok())
                        .unwrap_or(0),
                    final_days: row
                        .get("final_days")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok())
                        .unwrap_or(0),
                    provisional_days: row
                        .get("provisional_days")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok())
                        .unwrap_or(0),
                })
            })
            .collect()
    }

    async fn revenue_breakdown(
        &self,
        scope: &QueryScope,
        dimension: BreakdownDimension,
    ) -> Result<Vec<RevenueBreakdownRow>, StoreError> {
        let database = self.store.database();
        let dimension_column = match dimension {
            BreakdownDimension::Platform => "platform",
            BreakdownDimension::Format => "format",
            BreakdownDimension::Country => "country",
            BreakdownDimension::AdUnit => "ad_unit_id",
        };
        let select = format!(
            "SELECT {dimension_column} AS dimension_value, currency, \
             toInt64(sum(revenue_micros)) AS revenue_micros, \
             if(countIf(impressions IS NULL) = 0, toNullable(sum(impressions)), NULL) AS impressions, \
             toUInt32(count()) AS rows FROM {database}.report_days AS report FINAL\n\
             WHERE report.revenue_micros IS NOT NULL AND report.currency IS NOT NULL AND report.{dimension_column} IS NOT NULL\n\
             \x20 AND ({{publisher:String}} = '' OR publisher_id = {{publisher:String}})\n\
             \x20 AND ({{platform:String}} = '' OR platform = {{platform:String}})\n\
             \x20 AND ({{currency:String}} = '' OR currency = {{currency:String}})\n\
             \x20 AND ({{from:String}} = '' OR report.day >= toDateOrZero({{from:String}}))\n\
             \x20 AND ({{to:String}} = '' OR report.day <= toDateOrZero({{to:String}}))\n\
             GROUP BY dimension_value, currency ORDER BY revenue_micros DESC, dimension_value"
        );
        let rows = self
            .store
            .fetch_rows(&select, &Self::parameters(scope))
            .await?;
        let mut raw = Vec::with_capacity(rows.len());
        for row in &rows {
            let dimension_value = row
                .get("dimension_value")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| StoreError::Json("missing dimension_value".to_string()))?
                .to_string();
            let currency = row
                .get("currency")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| StoreError::Json("missing currency".to_string()))?;
            let revenue_micros = row
                .get("revenue_micros")
                .and_then(serde_json::Value::as_i64)
                .ok_or_else(|| StoreError::Json("missing revenue_micros".to_string()))?;
            let impressions = row.get("impressions").and_then(serde_json::Value::as_u64);
            let count = row
                .get("rows")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(0);
            raw.push((
                dimension_value,
                Money::new(revenue_micros, currency),
                impressions,
                count,
            ));
        }
        Ok(finish_breakdown(raw))
    }

    async fn lift_rows(&self, scope: &QueryScope) -> Result<Vec<LiftRow>, StoreError> {
        let events = self.scoped_events(scope).await?;
        Ok(model::lift_rows(scope, &events))
    }

    async fn fee_rows(&self, scope: &QueryScope) -> Result<Vec<FeeRow>, StoreError> {
        let events = self.scoped_events(scope).await?;
        let days = self.scoped_report_days(scope).await?;
        Ok(model::fee_rows(scope, &events, &days))
    }

    async fn fee_detail(
        &self,
        scope: &QueryScope,
        change_id: &str,
    ) -> Result<Option<FeeDetail>, StoreError> {
        let events = self.change_events(scope, change_id).await?;
        Ok(model::fee_detail(scope, &events, change_id))
    }

    async fn data_quality(&self, scope: &QueryScope) -> Result<DataQuality, StoreError> {
        let days = self.scoped_report_days(scope).await?;
        // Physical duplicate rows: before merges collapse them, a replay is
        // visible as extra rows sharing the (source, event_id) key. The scan
        // honors the scope's day range when one exists.
        let select = duplicate_scan_select(self.store.database());
        let rows = self
            .store
            .fetch_rows(&select, &Self::parameters(scope))
            .await?;
        let dedup_rejects = rows
            .first()
            .and_then(|row| row.get("duplicate_rows"))
            .and_then(serde_json::Value::as_u64);
        Ok(model::data_quality(scope, &days, dedup_rejects))
    }

    async fn object_timeline(
        &self,
        scope: &QueryScope,
        object_type: &str,
        object_id: &str,
    ) -> Result<Vec<ObjectTimelineEntry>, StoreError> {
        let database = self.store.database();
        let select = format!(
            "SELECT * FROM {database}.domain_events FINAL\n\
             WHERE object_type = {{object_type:String}} AND object_id = {{object_id:String}}\n\
             \x20 AND ({{publisher:String}} = '' OR publisher_id = {{publisher:String}})\n\
             \x20 AND ({{platform:String}} = '' OR platform = {{platform:String}})\n\
             ORDER BY occurred_at_utc, source, event_id"
        );
        let mut parameters = Self::parameters(scope);
        parameters.push(("object_type", object_type));
        parameters.push(("object_id", object_id));
        let rows = self.store.fetch_rows(&select, &parameters).await?;
        let events: Vec<DomainEvent> = rows
            .iter()
            .map(domain_event_from_row)
            .collect::<Result<_, _>>()?;
        Ok(model::object_timeline(
            scope,
            &events,
            object_type,
            object_id,
        ))
    }

    async fn operator_queues(&self, scope: &QueryScope) -> Result<OperatorQueues, StoreError> {
        let database = self.store.database();
        // The queues are a rebuilt snapshot; read the latest PUBLISHED
        // as-of version (a staged rebuild stays invisible until publish).
        let select = format!(
            "SELECT queue, object_id, publisher_id, severity, age_seconds, owner, next_action \
             FROM {database}.operator_queues\n\
             WHERE as_of_version = (SELECT max(as_of_version) FROM {database}.projection_versions \
             WHERE projection_name = 'operator_queues' AND published = 1)\n\
             \x20 AND ({{publisher:String}} = '' OR publisher_id = {{publisher:String}})\n\
             ORDER BY queue, age_seconds DESC, object_id"
        );
        let rows = self
            .store
            .fetch_rows(&select, &Self::parameters(scope))
            .await?;
        let mut queues = OperatorQueues {
            as_of: scope.as_of.to_rfc3339_millis(),
            incidents: Vec::new(),
            due_decisions: Vec::new(),
            stale_accounts: Vec::new(),
            overdue_jobs: Vec::new(),
            held_fees: Vec::new(),
        };
        for row in &rows {
            let text = |key: &str| {
                row.get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            };
            let queue_row = OperatorQueueRow {
                queue: text("queue").unwrap_or_default(),
                object_id: text("object_id").unwrap_or_default(),
                publisher_id: text("publisher_id").unwrap_or_default(),
                severity: text("severity").unwrap_or_default(),
                age_seconds: row
                    .get("age_seconds")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                owner: text("owner"),
                next_action: text("next_action").unwrap_or_default(),
            };
            match queue_row.queue.as_str() {
                "incidents" => queues.incidents.push(queue_row),
                "due_decisions" => queues.due_decisions.push(queue_row),
                "stale_accounts" => queues.stale_accounts.push(queue_row),
                "overdue_jobs" => queues.overdue_jobs.push(queue_row),
                "held_fees" => queues.held_fees.push(queue_row),
                _ => {}
            }
        }
        Ok(queues)
    }

    async fn ingestion_health(
        &self,
        scope: &QueryScope,
    ) -> Result<Vec<IngestionHealthRow>, StoreError> {
        let days = self.scoped_report_days(scope).await?;
        Ok(model::ingestion_health(scope, &days))
    }

    async fn growth_cohorts(&self, scope: &QueryScope) -> Result<Vec<GrowthCohortRow>, StoreError> {
        let events = self.scoped_events(scope).await?;
        Ok(model::growth_cohorts(scope, &events))
    }

    async fn activation_funnel(
        &self,
        scope: &QueryScope,
        cohort_month: &str,
    ) -> Result<ActivationFunnel, StoreError> {
        let events = self.scoped_events(scope).await?;
        Ok(model::activation_funnel(scope, &events, cohort_month))
    }

    async fn experiment_landscape(
        &self,
        scope: &QueryScope,
    ) -> Result<Vec<ExperimentLandscapeRow>, StoreError> {
        let events = self.scoped_events(scope).await?;
        Ok(model::experiment_landscape(scope, &events))
    }

    async fn experiment_effectiveness(
        &self,
        scope: &QueryScope,
        cohort_month: &str,
    ) -> Result<ExperimentEffectiveness, StoreError> {
        let events = self.scoped_events(scope).await?;
        Ok(model::experiment_effectiveness(
            scope,
            &events,
            cohort_month,
        ))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::queries::model::rate_basis_points;
    use crate::store::ClickHouseConfig;

    /// A reader whose store points at a closed port: every method must
    /// build its scoped, bound-parameter statement and surface a typed
    /// transport error instead of panicking.
    fn reader_without_server() -> ClickHouseReader {
        let store = ClickHouseStore::new(ClickHouseConfig {
            http_url: "http://127.0.0.1:1".into(),
            database: "apex_probe".into(),
            request_timeout_ms: 1_500,
        })
        .expect("store");
        ClickHouseReader::new(store)
    }

    fn scope() -> QueryScope {
        QueryScope::new(
            "northstar",
            crate::timestamp::UtcMillis::parse_rfc3339("2026-10-15T09:10:00.000Z").expect("time"),
        )
        .with_platform("applovin_max")
        .with_day_range("2026-10-01", "2026-10-14")
    }

    #[tokio::test]
    async fn every_read_model_surfaces_transport_errors() {
        let reader = reader_without_server();
        assert_eq!(reader.store().database(), "apex_probe");
        let scope = scope();
        assert!(reader.revenue_daily(&scope).await.is_err());
        assert!(reader
            .revenue_breakdown(&scope, BreakdownDimension::Country)
            .await
            .is_err());
        assert!(reader.lift_rows(&scope).await.is_err());
        assert!(reader.fee_rows(&scope).await.is_err());
        assert!(reader.fee_detail(&scope, "ch_0246").await.is_err());
        assert!(reader.data_quality(&scope).await.is_err());
        assert!(reader
            .object_timeline(&scope, "change", "ch_0246")
            .await
            .is_err());
        assert!(reader.operator_queues(&scope).await.is_err());
        assert!(reader.ingestion_health(&scope).await.is_err());
        assert!(reader.growth_cohorts(&scope).await.is_err());
        assert!(reader.activation_funnel(&scope, "2026-10").await.is_err());
        assert!(reader.experiment_landscape(&scope).await.is_err());
        assert!(reader
            .experiment_effectiveness(&scope, "2026-10")
            .await
            .is_err());
    }

    #[test]
    fn scope_parameters_bind_every_filter() {
        let scope = scope();
        let parameters = ClickHouseReader::parameters(&scope);
        let names: Vec<&str> = parameters.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            vec!["publisher", "platform", "currency", "from", "to"]
        );
        let values: Vec<&str> = parameters.iter().map(|(_, value)| *value).collect();
        assert_eq!(
            values,
            vec!["northstar", "applovin_max", "", "2026-10-01", "2026-10-14"]
        );
        // An operator-wide scope binds empty strings, never interpolated SQL.
        let wide = QueryScope::operator_wide(scope.as_of);
        let parameters = ClickHouseReader::parameters(&wide);
        let values: Vec<&str> = parameters.iter().map(|(_, value)| *value).collect();
        assert_eq!(values, vec!["", "", "", "", ""]);
    }

    #[test]
    fn fee_detail_narrows_the_event_scan_to_one_change() {
        // The scope-wide read carries no change filter.
        let scoped = change_scoped_select("apex", None);
        assert!(!scoped.contains("change_id ="), "{scoped}");
        assert!(
            scoped.contains("publisher_id = {publisher:String}"),
            "{scoped}"
        );
        // The detail read binds the change by promoted column or object.
        let detail = change_scoped_select("apex", Some("{change:String}"));
        assert!(
            detail.contains("change_id = {change:String} OR object_id = {change:String}"),
            "{detail}"
        );
        assert!(detail.contains("FINAL"), "{detail}");
    }

    #[test]
    fn the_duplicate_scan_honors_the_scope_day_range() {
        let sql = duplicate_scan_select("apex");
        assert!(sql.contains("uniqExact((source, event_id))"), "{sql}");
        assert!(
            sql.contains(
                "{from:String} = '' OR toDate(occurred_at_utc) >= toDateOrZero({from:String})"
            ),
            "{sql}"
        );
        assert!(
            sql.contains(
                "{to:String} = '' OR toDate(occurred_at_utc) <= toDateOrZero({to:String})"
            ),
            "{sql}"
        );
    }

    #[test]
    fn breakdown_shares_come_from_currency_totals() {
        let rows = finish_breakdown(vec![
            (
                "DE".to_string(),
                Money::new(30_900_000_000, "USD"),
                Some(1_500_000),
                6,
            ),
            ("GB".to_string(), Money::new(20_600_000_000, "USD"), None, 4),
        ]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].share_bp, Some(6_000));
        assert_eq!(rows[1].share_bp, Some(4_000));
        assert_eq!(rows[0].rows, 6);
        assert_eq!(rows[1].impressions, None);
        // A zero total has no share to give.
        let rows = finish_breakdown(vec![("DE".to_string(), Money::new(0, "USD"), None, 1)]);
        assert_eq!(rows[0].share_bp, None);
        assert_eq!(rate_basis_points(0, 0), None);
    }
}
