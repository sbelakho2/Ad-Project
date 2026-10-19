//! The daily projection refresh: rebuild every analytics projection from the
//! raw facts, compare the rebuilt totals against what the rebuild staged and
//! only then publish the new as-of version for the console and control plane
//! to read (section 05: "Rebuild projections from source facts and compare
//! the rebuilt totals before switching readers").
//!
//! The refresh follows the frozen order `rebuild_batch` (staged, unpublished)
//! -> `compare_report` -> `enforce_switch` -> `publish`: readers keep the
//! previously published version until the comparison confirms the staged
//! one, and a mismatch fails loudly without calling `publish`, so the
//! mismatch stays visible and the readers keep their old, confirmed numbers.
//!
//! The refresh runs inside the daily pass when `APEX_CLICKHOUSE_HTTP` is
//! configured (the same environment contract as the event emitter in
//! [`crate::events`]).
//!
//! `apex_projection_lag_seconds` is written from the published version rows:
//! the newest source event time minus now (falling back to the rebuild time
//! when no source events exist yet).

use std::sync::Arc;

use apex_analytics::projections;
use apex_analytics::store::{
    ClickHouseConfig, ClickHouseStore, EventStore, ProjectionStore, ProjectionVersionRow,
};
use apex_analytics::UtcMillis;

use crate::events::{CLICKHOUSE_ENV, ENVIRONMENT_ENV};
use crate::telemetry::RunnerTelemetry;

/// One refresh report: what was rebuilt, whether the switch was clean, the
/// published version and the recorded lag per projection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectionRefreshReport {
    /// The batch id the rebuild wrote under.
    pub batch_id: String,
    /// The version the readers now see (the published as-of version).
    pub published_version: u64,
    /// Projections rebuilt.
    pub projections: usize,
    /// Rows written across all projections.
    pub rows: u64,
    /// Whether the comparison confirmed the rebuilt totals.
    pub switched: bool,
    /// Per-projection lag in seconds (name, seconds), sorted by name.
    pub lag_seconds: Vec<(String, u64)>,
}

impl ProjectionRefreshReport {
    /// A one-line summary for the CLI.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "Projections: rebuilt {} ({} rows) and switched",
            self.projections, self.rows
        )
    }
}

/// The runner's projection refresh handle. Production builds it from the
/// environment; tests build it over a `MemoryStore`.
#[derive(Debug, Clone)]
pub struct RunnerProjections {
    store: Arc<dyn ProjectionStore>,
    schema: Option<Arc<ClickHouseStore>>,
    environment: String,
}

impl RunnerProjections {
    /// Build over any projection store (tests use `MemoryStore`; the schema
    /// check then has nothing to do).
    pub fn new(store: Arc<dyn ProjectionStore>, environment: impl Into<String>) -> Self {
        Self {
            store,
            schema: None,
            environment: environment.into(),
        }
    }

    /// Build from the environment: `Ok(None)` when `APEX_CLICKHOUSE_HTTP` is
    /// unset (refresh disabled; the caller logs the clear line).
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let Some(http) = lookup(CLICKHOUSE_ENV).filter(|value| !value.trim().is_empty()) else {
            return Ok(None);
        };
        let environment = lookup(ENVIRONMENT_ENV)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "production".to_string());
        let mut config = ClickHouseConfig::from_lookup(&lookup)
            .map_err(|error| format!("analytics configuration: {error}"))?;
        config.http_url = http;
        let store =
            ClickHouseStore::new(config).map_err(|error| format!("analytics store: {error}"))?;
        let store = Arc::new(store);
        let projection_store: Arc<dyn ProjectionStore> = store.clone();
        Ok(Some(Self {
            store: projection_store,
            schema: Some(store),
            environment,
        }))
    }

    /// Create the database and tables when absent (idempotent). Boot calls
    /// this once so a fresh deployment writes instead of spooling.
    pub async fn ensure_schema(&self) -> Result<(), String> {
        let Some(schema) = &self.schema else {
            return Ok(());
        };
        EventStore::ensure_schema(&**schema)
            .await
            .map_err(|error| format!("analytics schema setup: {error}"))
    }

    /// Refresh every projection under `batch_id` and record the lag gauge.
    /// The rebuild stages the new version (readers keep the published one),
    /// the comparison is run against the staged rows, and only a confirmed
    /// comparison publishes: a mismatch returns the refusal and `publish` is
    /// never called, so the readers keep their old, confirmed data.
    pub async fn refresh(
        &self,
        batch_id: &str,
        telemetry: Option<&RunnerTelemetry>,
        now_unix_ms: i64,
    ) -> Result<ProjectionRefreshReport, String> {
        // Self-healing: the refresh creates a missing schema before reading.
        self.ensure_schema().await?;
        // Nothing new since the published version: skip the compare, the
        // switch and the publish entirely. The lag gauges are still recorded
        // from the published versions so monitoring stays honest.
        let Some(report) = projections::rebuild_if_changed(&*self.store, batch_id)
            .await
            .map_err(|error| format!("projection rebuild: {error}"))?
        else {
            let versions = self
                .store
                .projection_versions()
                .await
                .map_err(|error| format!("projection versions: {error}"))?;
            let mut lag_seconds = lag_by_projection(&versions, now_unix_ms);
            lag_seconds.sort();
            if let Some(telemetry) = telemetry {
                for (name, seconds) in &lag_seconds {
                    if let Some(projection) = apex_metrics::vocab::ProjectionName::parse(name) {
                        telemetry.projection_lag(projection, &self.environment, *seconds as f64);
                    }
                }
            }
            let published_version = versions
                .iter()
                .map(|version| version.as_of_version)
                .max()
                .unwrap_or(0);
            return Ok(ProjectionRefreshReport {
                batch_id: batch_id.to_string(),
                published_version,
                projections: 0,
                rows: 0,
                switched: false,
                lag_seconds,
            });
        };
        let comparison = projections::compare_report(&*self.store, &report)
            .await
            .map_err(|error| format!("projection comparison: {error}"))?;
        projections::enforce_switch(&comparison)
            .map_err(|error| format!("projection switch refused: {error}"))?;
        // Confirmed: the staged versions become the reader-visible ones.
        let published_version = projections::publish(&*self.store, batch_id)
            .await
            .map_err(|error| format!("projection publish: {error}"))?;
        let versions = self
            .store
            .projection_versions()
            .await
            .map_err(|error| format!("projection versions: {error}"))?;
        let rows: u64 = report.rebuilt.iter().map(|rebuild| rebuild.rows).sum();
        let mut lag_seconds = lag_by_projection(&versions, now_unix_ms);
        lag_seconds.sort();
        if let Some(telemetry) = telemetry {
            for (name, seconds) in &lag_seconds {
                if let Some(projection) = apex_metrics::vocab::ProjectionName::parse(name) {
                    telemetry.projection_lag(projection, &self.environment, *seconds as f64);
                }
            }
        }
        Ok(ProjectionRefreshReport {
            batch_id: batch_id.to_string(),
            published_version,
            projections: report.rebuilt.len(),
            rows,
            switched: comparison.is_clean(),
            lag_seconds,
        })
    }
}

/// Lag per projection: how far the newest covered source event is behind
/// `now`, falling back to the rebuild time when there are no source events.
fn lag_by_projection(versions: &[ProjectionVersionRow], now_unix_ms: i64) -> Vec<(String, u64)> {
    versions
        .iter()
        .map(|version| {
            let anchor = version
                .source_event_max_unix_ms
                .unwrap_or(version.rebuilt_at_unix_ms);
            let seconds = (now_unix_ms.saturating_sub(anchor) / 1_000).max(0);
            (version.projection_name.clone(), seconds as u64)
        })
        .collect()
}

/// The batch id for one daily pass: stable per day so a retry publishes the
/// same batch identity instead of inventing a new one (the retry stages a
/// fresh version under the same batch and publishes it after the compare).
#[must_use]
pub fn daily_batch_id(now_unix_ms: i64) -> String {
    let day = UtcMillis::from_unix_millis(now_unix_ms).to_day_string();
    format!("daily-{day}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use apex_analytics::store::{EventStore, ReportDayRow};

    /// A minimal persisted event: advancing `source_event_max` is what makes
    /// a later refresh rebuild instead of skipping.
    fn tick_event(occurred_ms: i64) -> apex_analytics::DomainEvent {
        let at = apex_analytics::UtcMillis::from_unix_millis(occurred_ms);
        let mut payload = apex_analytics::EventPayload::new();
        payload.job_id = Some("projections".to_string());
        apex_analytics::DomainEvent::builder(
            apex_analytics::EventType::JobRun,
            apex_analytics::EventSource::Worker,
        )
        .with_event_id(format!("job_run:test:{occurred_ms}"))
        .with_occurred_at(at)
        .with_received_at(at)
        .with_environment("test")
        .with_service_version("apex-runner-test")
        .with_object("job", "projections")
        .with_outcome("completed")
        .with_payload(payload)
        .build()
        .expect("a valid test event")
    }

    fn report_day(publisher: &str, day: &str, revenue_micros: i64) -> ReportDayRow {
        ReportDayRow {
            publisher_id: publisher.to_string(),
            platform: "applovin_max".to_string(),
            platform_account_id: "acct-1".to_string(),
            day: day.to_string(),
            finality: "final".to_string(),
            revenue_micros: Some(revenue_micros),
            impressions: Some(100),
            currency: Some("USD".to_string()),
            country: Some("DE".to_string()),
            ad_unit_id: Some("rw_de_01".to_string()),
            format: Some("rewarded".to_string()),
            source_digest: Some("digest-1".to_string()),
        }
    }

    #[tokio::test]
    async fn a_refresh_rebuilds_compares_and_records_the_lag() {
        let store = Arc::new(apex_analytics::MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        store
            .insert_report_days(&[
                report_day("northstar", "2026-10-13", 5_150_000_000),
                report_day("northstar", "2026-10-14", 5_150_000_000),
            ])
            .await
            .expect("report days");
        let projections = RunnerProjections::new(store.clone(), "staging");
        let now = 1_792_056_000_000_i64;
        let report = projections
            .refresh(&daily_batch_id(now), None, now)
            .await
            .expect("refresh");
        assert_eq!(report.batch_id, "daily-2026-10-15");
        assert_eq!(report.published_version, 1, "the staged version published");
        assert_eq!(report.projections, 4);
        assert!(report.switched);
        assert!(!report.lag_seconds.is_empty());
        // The projections are now visible to readers.
        let reader = apex_analytics::queries::MemoryReader::new((*store).clone());
        let scope = apex_analytics::queries::QueryScope::new(
            "northstar",
            apex_analytics::UtcMillis::from_unix_millis(now),
        );
        let rows = apex_analytics::queries::AnalyticsReader::revenue_daily(&reader, &scope)
            .await
            .expect("reader");
        assert!(!rows.is_empty());

        // A later batch with new source facts stages a new version and
        // publishes it after its own clean compare: readers follow the newest
        // published version. (An unchanged source skips entirely, tested
        // separately.)
        store
            .insert(&[tick_event(now + 86_400_000)])
            .await
            .expect("new source event");
        assert_eq!(
            store.source_event_max().await.expect("source max"),
            Some(apex_analytics::UtcMillis::from_unix_millis(
                now + 86_400_000
            )),
            "the new event is the source maximum"
        );
        let report = projections
            .refresh("daily-2026-10-16", None, now + 86_400_000)
            .await
            .expect("second refresh");
        assert_eq!(report.published_version, 2);
        let version = store
            .projection_version(apex_analytics::projections::ProjectionName::CustomerDaily)
            .await
            .expect("version");
        assert_eq!(version, Some(2));
    }

    #[tokio::test]
    async fn the_lag_gauge_lands_in_the_registry() {
        let store = Arc::new(apex_analytics::MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        store
            .insert_report_days(&[report_day("northstar", "2026-10-14", 1_000_000_000)])
            .await
            .expect("report days");
        let registry = Arc::new(apex_metrics::Registry::new());
        let telemetry = RunnerTelemetry::new(registry.clone());
        let projections = RunnerProjections::new(store, "staging");
        let now = 1_792_056_000_000_i64;
        projections
            .refresh(&daily_batch_id(now), Some(&telemetry), now)
            .await
            .expect("refresh");
        let family = registry
            .family_snapshot("apex_projection_lag_seconds")
            .expect("lag family");
        assert_eq!(family.series.len(), 4);
        assert!(family.series.iter().all(|series| series
            .labels
            .iter()
            .any(|(name, value)| *name == "environment" && value.as_str() == "staging")));
    }

    #[test]
    fn from_lookup_is_none_without_the_environment() {
        let none = RunnerProjections::from_lookup(|_| None).expect("lookup");
        assert!(none.is_none());
    }

    /// A store wrapper that corrupts the newest staged `customer_daily` rows
    /// once armed, standing in for storage corruption between the rebuild and
    /// the compare.
    #[derive(Debug)]
    struct TamperingStore {
        inner: Arc<apex_analytics::MemoryStore>,
        armed: std::sync::atomic::AtomicBool,
    }

    #[async_trait::async_trait]
    impl apex_analytics::store::ProjectionStore for TamperingStore {
        async fn load_events(
            &self,
        ) -> Result<Vec<apex_analytics::DomainEvent>, apex_analytics::store::StoreError> {
            self.inner.load_events().await
        }

        async fn load_report_days(
            &self,
        ) -> Result<Vec<apex_analytics::store::ReportDayRow>, apex_analytics::store::StoreError>
        {
            self.inner.load_report_days().await
        }

        async fn load_accounts(
            &self,
        ) -> Result<Vec<apex_analytics::store::AccountRow>, apex_analytics::store::StoreError>
        {
            self.inner.load_accounts().await
        }

        async fn write_projection(
            &self,
            name: apex_analytics::projections::ProjectionName,
            as_of_version: u64,
            batch_id: &str,
            rows: &[apex_analytics::projections::ProjectionRow],
        ) -> Result<(), apex_analytics::store::StoreError> {
            self.inner
                .write_projection(name, as_of_version, batch_id, rows)
                .await
        }

        async fn load_projection(
            &self,
            name: apex_analytics::projections::ProjectionName,
        ) -> Result<
            Vec<apex_analytics::projections::ProjectionRow>,
            apex_analytics::store::StoreError,
        > {
            self.inner.load_projection(name).await
        }

        async fn projection_version(
            &self,
            name: apex_analytics::projections::ProjectionName,
        ) -> Result<Option<u64>, apex_analytics::store::StoreError> {
            self.inner.projection_version(name).await
        }

        async fn projection_versions(
            &self,
        ) -> Result<
            Vec<apex_analytics::store::ProjectionVersionRow>,
            apex_analytics::store::StoreError,
        > {
            self.inner.projection_versions().await
        }

        async fn latest_projection_version(
            &self,
            name: apex_analytics::projections::ProjectionName,
        ) -> Result<Option<u64>, apex_analytics::store::StoreError> {
            self.inner.latest_projection_version(name).await
        }

        async fn latest_projection(
            &self,
            name: apex_analytics::projections::ProjectionName,
        ) -> Result<
            Option<apex_analytics::store::StagedProjection>,
            apex_analytics::store::StoreError,
        > {
            let mut staged = self.inner.latest_projection(name).await?;
            if self.armed.load(std::sync::atomic::Ordering::SeqCst) {
                if let Some(staged) = staged.as_mut() {
                    for row in &mut staged.rows {
                        if let apex_analytics::projections::ProjectionRow::CustomerDaily(row) = row
                        {
                            row.revenue_micros = row.revenue_micros.saturating_add(1_000_000);
                        }
                    }
                }
            }
            Ok(staged)
        }

        async fn publish_batch(
            &self,
            batch_id: &str,
        ) -> Result<Option<u64>, apex_analytics::store::StoreError> {
            self.inner.publish_batch(batch_id).await
        }
    }

    #[tokio::test]
    async fn a_mismatching_rebuild_refuses_the_switch_and_keeps_readers_on_the_published_version() {
        let store = Arc::new(apex_analytics::MemoryStore::new());
        store.ensure_schema().await.expect("schema");
        store
            .insert_report_days(&[report_day("northstar", "2026-10-14", 5_150_000_000)])
            .await
            .expect("report days");
        let tampering = Arc::new(TamperingStore {
            inner: store.clone(),
            armed: std::sync::atomic::AtomicBool::new(false),
        });
        let projections: RunnerProjections = RunnerProjections::new(tampering.clone(), "staging");
        let now = 1_792_056_000_000_i64;

        // The first refresh publishes v1; readers see the confirmed rows.
        let first = projections
            .refresh("daily-2026-10-15", None, now)
            .await
            .expect("first refresh");
        assert_eq!(first.published_version, 1);
        let published_rows = store
            .load_projection(apex_analytics::projections::ProjectionName::CustomerDaily)
            .await
            .expect("published rows");
        assert_eq!(published_rows.len(), 1);

        // New source facts are required for a second rebuild: an unchanged
        // source is skipped before the compare can see a tampered stage.
        store
            .insert(&[tick_event(now + 86_400_000)])
            .await
            .expect("new source event");
        // Tamper the staged rows after the rebuild: the compare must refuse,
        // publish must not run and the readers keep v1's data.
        tampering
            .armed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let error = projections
            .refresh("daily-2026-10-16", None, now + 86_400_000)
            .await
            .expect_err("the tampered rebuild is refused");
        assert!(error.contains("refused"), "{error}");
        assert_eq!(
            store
                .projection_version(apex_analytics::projections::ProjectionName::CustomerDaily)
                .await
                .expect("version"),
            Some(1),
            "the readers were not switched to the unconfirmed version"
        );
        assert_eq!(
            store
                .load_projection(apex_analytics::projections::ProjectionName::CustomerDaily)
                .await
                .expect("published rows"),
            published_rows,
            "the readers keep the old, confirmed data"
        );
        // The tampered v2 is staged, never published.
        assert_eq!(
            store
                .latest_projection_version(
                    apex_analytics::projections::ProjectionName::CustomerDaily
                )
                .await
                .expect("latest"),
            Some(2)
        );
    }
}
