//! The ClickHouse reader: bound-parameter fetches plus the shared pure
//! computations.
//!
//! Every statement uses `{name:Type}` bound parameters delivered over the
//! HTTP `param_*` keys; no scope value is ever interpolated into SQL. An
//! empty string parameter means "no filter", which keeps one statement shape
//! per read model. `revenue_daily` reads the `customer_daily` projection the
//! spec names for exactly this purpose, pinned to the currently PUBLISHED
//! as-of version (staged rebuilds stay invisible until
//! `projections::publish`). `operator_queues` is likewise pinned to the
//! published snapshot, but recomputes membership and ages from the covered
//! facts against the horizon that snapshot recorded (`source_event_max`), so
//! the queues are as of the covered data rather than the wall clock, exactly
//! like the memory reader. The remaining models are event-sourced and must
//! stay behind the projection compare before a switch (see
//! [`crate::projections::enforce_switch`]).
//!
//! Event-scoped models fetch the scope's history: bounded by the scope's
//! day range when one is set, and by the 90-day raw-event TTL otherwise.
//! The duplicate scan honors the scope's day range too, and `fee_detail`
//! narrows to the one change (its `change_id` or `object_id`) so a detail
//! page never scans the publisher's whole history.

use async_trait::async_trait;

use crate::envelope::{DomainEvent, EventType};
use crate::projections::compute_operator_queues;
use crate::store::{
    domain_event_from_row, AccountRow, ClickHouseStore, ProjectionStore, ReportDayRow, StoreError,
};
use crate::timestamp::UtcMillis;

use super::dto::{
    ActivationFunnel, BreakdownDimension, DataQuality, ExperimentEffectiveness,
    ExperimentLandscapeRow, FeeDetail, FeeRow, GrowthCohortRow, IngestionHealthRow, LiftRow, Money,
    ObjectTimelineEntry, OperatorQueues, PageRequestsDay, QueryScope, RevenueBreakdownRow,
    RevenueDailyRow,
};
use super::model;
use super::AnalyticsReader;

/// The raw events one operator-queue read may fold. The event types the
/// queues need are filtered server-side, and the bound keeps an
/// operator-wide read from silently materializing an unbounded history: past
/// it the read fails with the honest reason instead of truncating.
pub const OPERATOR_QUEUE_EVENT_LIMIT: usize = 250_000;

/// The event types `compute_operator_queues` reads. Everything else is
/// filtered server-side, so the queue read no longer ships unrelated rows.
const OPERATOR_QUEUE_EVENT_TYPES: &str = "'incident_changed', \
     'experiment_registered', 'experiment_started', 'experiment_decided', \
     'job_run', 'fee_held', 'fee_invoice_issued', 'fee_credited'";

/// The scoped operator-queue event select. The `LIMIT` carries one extra row
/// so an over-bound read is detectable, never silently cut.
fn operator_queue_select(database: &str) -> String {
    format!(
        "SELECT * FROM {database}.domain_events FINAL\n\
         WHERE ({{publisher:String}} = '' OR publisher_id = {{publisher:String}})\n\
         \x20 AND event_type IN ({OPERATOR_QUEUE_EVENT_TYPES})\n\
         ORDER BY occurred_at_utc, source, event_id\n\
         LIMIT {}",
        OPERATOR_QUEUE_EVENT_LIMIT + 1
    )
}

/// Refuse an operator-queue read that reached the bound. The caller either
/// narrows the scope (a publisher or day range) or relies on the published
/// projections; it never receives a silently truncated queue.
fn enforce_queue_event_bound(count: usize, limit: usize) -> Result<(), StoreError> {
    if count > limit {
        return Err(StoreError::Query(format!(
            "the operator queue read covered more than {limit} events; narrow the scope or read the published queues"
        )));
    }
    Ok(())
}

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
        rows.into_iter()
            .map(|row| {
                serde_json::from_value(row).map_err(|error| StoreError::Json(error.to_string()))
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

/// The site-wide daily page-request aggregation.
///
/// The `public_page_requested` family carries no tenant identity, so the
/// statement binds only the event type and the day bounds: no publisher,
/// platform or currency filter may appear, or every request would silently
/// disappear. It groups into (day, bot class) counts server-side and never
/// returns a request row.
fn page_requests_select(database: &str) -> String {
    format!(
        "SELECT toString(toDate(occurred_at_utc)) AS day, \
         toString(count()) AS requests, \
         toString(countIf(JSONExtractUInt(payload_json, 'status') = 404)) AS not_found, \
         JSONExtractString(payload_json, 'bot_class') AS bot_class, \
         toString(count()) AS bot_requests \
         FROM {database}.domain_events FINAL\n\
         WHERE event_type = {{event_type:String}}\n\
         \x20 AND ({{from:String}} = '' OR toDate(occurred_at_utc) >= toDateOrZero({{from:String}}))\n\
         \x20 AND ({{to:String}} = '' OR toDate(occurred_at_utc) <= toDateOrZero({{to:String}}))\n\
         GROUP BY day, bot_class\n\
         ORDER BY day, bot_class"
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
             ORDER BY day, platform, currency, publisher_id"
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
             GROUP BY dimension_value, currency ORDER BY revenue_micros DESC, dimension_value, currency"
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
        // The queues are a rebuilt snapshot and describe the covered data:
        // ages and membership are computed against the horizon the published
        // snapshot recorded (`source_event_max`), never the wall clock. A
        // read before the first publish returns empty queues, exactly the
        // state of the empty published projection.
        let version_rows = self
            .store
            .fetch_rows(
                &format!(
                    "SELECT toString(source_event_max) AS source_event_max \
                     FROM {database}.projection_versions \
                     WHERE projection_name = 'operator_queues' AND published = 1 \
                     ORDER BY as_of_version DESC LIMIT 1"
                ),
                &[],
            )
            .await?;
        let Some(version_row) = version_rows.first() else {
            return Ok(model::group_operator_queues(scope.as_of, Vec::new()));
        };
        let anchor = version_row
            .get("source_event_max")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            // ClickHouse renders DateTime64 as `YYYY-MM-DD hh:mm:ss.fff`,
            // which `parse_rfc3339` reads as UTC.
            .and_then(|value| UtcMillis::parse_rfc3339(value).ok())
            .unwrap_or(scope.as_of);
        // Recompute membership and ages from the stored facts against the
        // covered-data anchor: the identical pure computation the memory
        // reader runs over its facts, so both readers answer identically.
        // Only the queue event types are fetched, and the read is bounded.
        let select = operator_queue_select(database);
        let rows = self
            .store
            .fetch_rows(&select, &[("publisher", scope.publisher_id.as_str())])
            .await?;
        enforce_queue_event_bound(rows.len(), OPERATOR_QUEUE_EVENT_LIMIT)?;
        let events: Vec<DomainEvent> = rows
            .iter()
            .map(domain_event_from_row)
            .collect::<Result<_, _>>()?;
        // Only the covered data counts: facts past the snapshot's horizon
        // are not part of the published queues.
        let events: Vec<DomainEvent> = events
            .into_iter()
            .filter(|event| event.occurred_at_utc <= anchor)
            .collect();
        let accounts: Vec<AccountRow> = self
            .store
            .load_accounts()
            .await?
            .into_iter()
            .filter(|account| {
                scope.publisher_id.is_empty() || account.publisher_id == scope.publisher_id
            })
            .collect();
        let rows = compute_operator_queues(&events, &accounts, anchor);
        Ok(model::group_operator_queues(scope.as_of, rows))
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

    async fn page_requests_daily(
        &self,
        scope: &QueryScope,
        from_day: &str,
        to_day: &str,
    ) -> Result<Vec<PageRequestsDay>, StoreError> {
        // Site-wide by design: the statement binds only the day bounds,
        // never publisher/platform/currency, and aggregates server-side.
        let from = model::effective_day_bound(from_day, scope.from_day.as_deref());
        let to = model::effective_day_bound(to_day, scope.to_day.as_deref());
        let select = page_requests_select(self.store.database());
        let rows = self
            .store
            .fetch_rows(
                &select,
                &[
                    ("event_type", EventType::PublicPageRequested.as_str()),
                    ("from", from.as_str()),
                    ("to", to.as_str()),
                ],
            )
            .await?;
        let mut days: Vec<PageRequestsDay> = Vec::new();
        for row in &rows {
            let text = |key: &str| {
                row.get(key)
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| StoreError::Json(format!("missing column '{key}'")))
            };
            let day = text("day")?.to_string();
            let requests = crate::store::json_u64(row.get("requests"))
                .ok_or_else(|| StoreError::Json("missing column 'requests'".to_string()))?;
            let not_found = crate::store::json_u64(row.get("not_found"))
                .ok_or_else(|| StoreError::Json("missing column 'not_found'".to_string()))?;
            let bot_requests = crate::store::json_u64(row.get("bot_requests"))
                .ok_or_else(|| StoreError::Json("missing column 'bot_requests'".to_string()))?;
            // A row without a stored class is reported as `unknown`, exactly
            // like the portable aggregation, never guessed at.
            let bot_class = match text("bot_class")? {
                "" => "unknown".to_string(),
                class => class.to_string(),
            };
            match days.last_mut() {
                Some(entry) if entry.day == day => {
                    entry.requests += requests;
                    entry.not_found += not_found;
                    entry.bot_class_mix.push((bot_class, bot_requests));
                }
                _ => days.push(PageRequestsDay {
                    day,
                    requests,
                    not_found,
                    bot_class_mix: vec![(bot_class, bot_requests)],
                }),
            }
        }
        Ok(days)
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
            ..ClickHouseConfig::default()
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
        assert!(reader
            .page_requests_daily(&scope, "2026-10-01", "2026-10-14")
            .await
            .is_err());
    }

    #[test]
    fn the_page_request_aggregation_is_site_wide_and_count_only() {
        let sql = page_requests_select("apex");
        assert!(sql.contains("FROM apex.domain_events FINAL"), "{sql}");
        assert!(sql.contains("event_type = {event_type:String}"), "{sql}");
        assert!(
            sql.contains(
                "toString(countIf(JSONExtractUInt(payload_json, 'status') = 404)) AS not_found"
            ),
            "{sql}"
        );
        assert!(sql.contains("GROUP BY day, bot_class"), "{sql}");
        assert!(sql.contains("ORDER BY day, bot_class"), "{sql}");
        // No tenant filter may appear: the family is site-wide, and a
        // publisher/platform/currency predicate would silently hide every
        // request (no event carries those fields).
        for bound in [
            "{publisher:String}",
            "{platform:String}",
            "{currency:String}",
            "publisher_id",
            "platform =",
        ] {
            assert!(!sql.contains(bound), "{bound} must not be bound: {sql}");
        }
        assert!(
            !sql.contains("SELECT *") && !sql.contains("payload_json FROM"),
            "counts only, never rows: {sql}"
        );
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
    fn the_operator_queue_read_filters_types_and_carries_the_bound() {
        let sql = operator_queue_select("apex");
        assert!(sql.contains("event_type IN ('incident_changed'"), "{sql}");
        assert!(sql.contains("LIMIT 250001"), "{sql}");
        assert!(sql.contains("publisher_id = {publisher:String}"), "{sql}");
        // One extra row makes an over-bound read detectable.
        assert!(enforce_queue_event_bound(250_000, 250_000).is_ok());
        let error = enforce_queue_event_bound(250_001, 250_000).expect_err("over bound");
        assert!(
            error.to_string().contains("more than 250000 events"),
            "{error}"
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
