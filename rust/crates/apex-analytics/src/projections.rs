//! Projections: pure SQL builders, rebuild from source facts under a new
//! as-of version, and the staged publish rule.
//!
//! Section 05 (page 86, Reconciliation): "Rebuild projections from source
//! facts and compare the rebuilt totals before switching readers." And
//! (page 86, Deduplication and order): "A backfill has a batch ID and
//! changes the as-of version; it does not masquerade as new customer
//! activity."
//!
//! The switch is STAGED, not implied by the write: [`rebuild_batch`] writes
//! every projection under a new as-of version marked unpublished, readers
//! resolve only published versions, [`compare`] or [`compare_report`]
//! verifies the staged rows against computed totals, and [`publish`] marks
//! the batch's versions reader-visible. A failed compare therefore leaves
//! readers on the previous version: `rebuild -> compare -> publish`.
//!
//! The portable rebuild path computes every projection in Rust from the
//! stored source facts (report days, accounts, events) and writes the rows
//! through [`crate::store::ProjectionStore`], so [`MemoryStore`] fixtures and
//! the real ClickHouse store take the identical code path. The
//! server-side `INSERT ... SELECT` builders ([`rebuild_sql`],
//! [`totals_sql`]) are the deployment fast path; the integration test runs
//! them against the real server and asserts they agree with the Rust
//! totals.
//!
//! [`compare`] recomputes the totals from source facts and compares them
//! with the newest stored (staged) rows; [`compare_report`] does the same
//! against a rebuild report's already-computed totals without recomputing.
//! [`ComparisonReport::is_clean`] gates [`publish`] and [`enforce_switch`]
//! refuses the switch on any mismatch.

use std::collections::BTreeMap;

use crate::envelope::{DomainEvent, EventType};
use crate::schema::{validate_identifier, SchemaError};
use crate::store::{AccountRow, ProjectionStore, ReportDayRow, StoreError};
use crate::timestamp::{UtcMillis, MILLIS_PER_DAY};

/// Accounts with no stage movement within this many days are queued as
/// stale. The spec shows a stale-account queue without fixing a day count;
/// two weeks is the review default and a deployment tunable.
pub const STALE_ACCOUNT_DAYS: i64 = 14;

/// The four read projections named by the spec (page 86).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectionName {
    /// Tenant daily revenue and reporting coverage.
    CustomerDaily,
    /// Platform daily revenue and coverage.
    PlatformDaily,
    /// Lifecycle cohort counts by stage.
    LifecycleCohorts,
    /// Operator intervention queues.
    OperatorQueues,
}

impl ProjectionName {
    /// Every projection, in dependency order.
    pub const ALL: [ProjectionName; 4] = [
        ProjectionName::CustomerDaily,
        ProjectionName::PlatformDaily,
        ProjectionName::LifecycleCohorts,
        ProjectionName::OperatorQueues,
    ];

    /// Spec table name.
    pub fn as_str(self) -> &'static str {
        match self {
            ProjectionName::CustomerDaily => "customer_daily",
            ProjectionName::PlatformDaily => "platform_daily",
            ProjectionName::LifecycleCohorts => "lifecycle_cohorts",
            ProjectionName::OperatorQueues => "operator_queues",
        }
    }

    /// The table name (identical to the spec's projection name).
    pub fn table_name(self) -> &'static str {
        self.as_str()
    }

    /// Whether this projection carries money totals.
    pub fn carries_money(self) -> bool {
        matches!(
            self,
            ProjectionName::CustomerDaily | ProjectionName::PlatformDaily
        )
    }
}

impl std::fmt::Display for ProjectionName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl serde::Serialize for ProjectionName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for ProjectionName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = ProjectionName;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a projection name")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<ProjectionName, E> {
                ProjectionName::ALL
                    .into_iter()
                    .find(|name| name.as_str() == value)
                    .ok_or_else(|| E::custom(format!("unknown projection '{value}'")))
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

/// One tenant daily aggregate row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CustomerDailyRow {
    /// Tenant scope.
    pub publisher_id: String,
    /// Reporting day `YYYY-MM-DD`.
    pub day: String,
    /// Platform the revenue came from.
    pub platform: String,
    /// ISO-4217 currency of the amount.
    pub currency: String,
    /// Total revenue in integer micros.
    pub revenue_micros: i64,
    /// Impressions when every contributing source row supplied them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impressions: Option<u64>,
    /// Contributing source rows.
    pub source_days: u32,
    /// Contributing rows with finality `final`.
    pub final_days: u32,
    /// Contributing rows with finality `provisional`.
    pub provisional_days: u32,
}

/// One platform daily aggregate row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlatformDailyRow {
    /// Platform.
    pub platform: String,
    /// Reporting day `YYYY-MM-DD`.
    pub day: String,
    /// ISO-4217 currency of the amount.
    pub currency: String,
    /// Total revenue in integer micros.
    pub revenue_micros: i64,
    /// Impressions when every contributing source row supplied them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impressions: Option<u64>,
    /// Distinct accounts with a final row that day.
    pub final_accounts: u32,
    /// Distinct accounts with only provisional rows that day.
    pub provisional_accounts: u32,
}

/// One lifecycle cohort row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LifecycleCohortRow {
    /// Cohort month `YYYY-MM-01`.
    pub cohort_month: String,
    /// Tenant scope.
    pub publisher_id: String,
    /// Lifecycle stage.
    pub stage: String,
    /// Accounts in the cohort at this stage.
    pub accounts: u32,
}

/// One operator queue entry.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OperatorQueueRow {
    /// Bounded queue name (`incidents`, `due_decisions`, `stale_accounts`,
    /// `overdue_jobs`, `held_fees`).
    pub queue: String,
    /// Object identity inside the queue.
    pub object_id: String,
    /// Tenant scope.
    pub publisher_id: String,
    /// Bounded severity (`critical`, `warning`, `unknown`).
    pub severity: String,
    /// Seconds since the queued condition was last observed.
    pub age_seconds: u64,
    /// Owner when one is assigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Bounded action class the operator should take next.
    pub next_action: String,
}

/// A typed projection row.
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectionRow {
    /// A customer daily row.
    CustomerDaily(CustomerDailyRow),
    /// A platform daily row.
    PlatformDaily(PlatformDailyRow),
    /// A lifecycle cohort row.
    LifecycleCohorts(LifecycleCohortRow),
    /// An operator queue row.
    OperatorQueues(OperatorQueueRow),
}

impl ProjectionRow {
    /// Serialize to the table's JSONEachRow shape, with the as-of version.
    pub fn to_table_json(
        &self,
        name: ProjectionName,
        as_of_version: u64,
    ) -> Result<serde_json::Value, StoreError> {
        let mut value = match self {
            ProjectionRow::CustomerDaily(row) if name == ProjectionName::CustomerDaily => {
                serde_json::to_value(row)
            }
            ProjectionRow::PlatformDaily(row) if name == ProjectionName::PlatformDaily => {
                serde_json::to_value(row)
            }
            ProjectionRow::LifecycleCohorts(row) if name == ProjectionName::LifecycleCohorts => {
                serde_json::to_value(row)
            }
            ProjectionRow::OperatorQueues(row) if name == ProjectionName::OperatorQueues => {
                serde_json::to_value(row)
            }
            other => {
                return Err(StoreError::Schema(format!(
                    "row {other:?} does not belong to projection {name}"
                )))
            }
        }
        .map_err(|error| StoreError::Json(error.to_string()))?;
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "as_of_version".to_string(),
                serde_json::Value::from(as_of_version),
            );
        }
        Ok(value)
    }

    /// Deserialize a row read from the projection table.
    pub fn from_table_json(
        name: ProjectionName,
        value: &serde_json::Value,
    ) -> Result<ProjectionRow, StoreError> {
        Self::from_table_json_owned(name, value.clone())
    }

    /// Owned counterpart of [`Self::from_table_json`]: the read path already
    /// owns the `Value` returned by the fetch, so consuming it avoids a deep
    /// clone (all string and array fields) per row.
    pub fn from_table_json_owned(
        name: ProjectionName,
        value: serde_json::Value,
    ) -> Result<ProjectionRow, StoreError> {
        let parse = |error: serde_json::Error| StoreError::Json(error.to_string());
        Ok(match name {
            ProjectionName::CustomerDaily => {
                ProjectionRow::CustomerDaily(serde_json::from_value(value).map_err(parse)?)
            }
            ProjectionName::PlatformDaily => {
                ProjectionRow::PlatformDaily(serde_json::from_value(value).map_err(parse)?)
            }
            ProjectionName::LifecycleCohorts => {
                ProjectionRow::LifecycleCohorts(serde_json::from_value(value).map_err(parse)?)
            }
            ProjectionName::OperatorQueues => {
                ProjectionRow::OperatorQueues(serde_json::from_value(value).map_err(parse)?)
            }
        })
    }
}

/// Row counts and money totals of one projection.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProjectionTotals {
    /// Number of rows.
    pub rows: u64,
    /// Total revenue micros for money projections; `None` where the
    /// projection carries no money (never a fake zero).
    pub revenue_micros: Option<i64>,
}

impl ProjectionTotals {
    /// Totals of a set of rows.
    pub fn of(name: ProjectionName, rows: &[ProjectionRow]) -> Self {
        let revenue_micros = if name.carries_money() {
            Some(
                rows.iter()
                    .map(|row| match row {
                        ProjectionRow::CustomerDaily(row) => row.revenue_micros,
                        ProjectionRow::PlatformDaily(row) => row.revenue_micros,
                        _ => 0,
                    })
                    .sum(),
            )
        } else {
            None
        };
        Self {
            rows: rows.len() as u64,
            revenue_micros,
        }
    }
}

/// One projection's rebuild result.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProjectionRebuild {
    /// Which projection was rebuilt.
    pub projection: ProjectionName,
    /// New as-of version.
    pub as_of_version: u64,
    /// Row count written.
    pub rows: u64,
    /// Totals of the written rows.
    pub totals: ProjectionTotals,
}

/// The result of a rebuild run.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProjectionReport {
    /// Batch identity of this rebuild ("a backfill has a batch ID").
    pub batch_id: String,
    /// The highest as-of version written.
    pub as_of_version: u64,
    /// Per-projection results.
    pub rebuilt: Vec<ProjectionRebuild>,
}

/// How one projection's rebuilt totals compare with the stored ones.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ComparisonStatus {
    /// Rebuilt totals equal the stored totals.
    Match {
        /// The agreeing totals.
        totals: ProjectionTotals,
    },
    /// Rebuilt totals differ from the stored totals.
    Mismatch {
        /// Totals recomputed from source facts.
        rebuilt: ProjectionTotals,
        /// Totals the current readers would see.
        current: ProjectionTotals,
    },
    /// The projection has never been rebuilt.
    Absent {
        /// Totals recomputed from source facts.
        rebuilt: ProjectionTotals,
    },
}

/// One projection in a comparison report.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProjectionComparison {
    /// Which projection.
    pub projection: ProjectionName,
    /// As-of version of the compared (newest recorded, staged) rows.
    pub current_as_of_version: Option<u64>,
    /// Comparison outcome.
    pub status: ComparisonStatus,
}

/// The result of a compare run.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ComparisonReport {
    /// Per-projection comparisons, in [`ProjectionName::ALL`] order.
    pub comparisons: Vec<ProjectionComparison>,
}

impl ComparisonReport {
    /// Whether every projection matches; only then may readers switch.
    pub fn is_clean(&self) -> bool {
        self.comparisons
            .iter()
            .all(|comparison| matches!(comparison.status, ComparisonStatus::Match { .. }))
    }

    /// The projections that refused the switch, with their totals.
    pub fn mismatches(&self) -> Vec<&ProjectionComparison> {
        self.comparisons
            .iter()
            .filter(|comparison| !matches!(comparison.status, ComparisonStatus::Match { .. }))
            .collect()
    }
}

/// Projection failures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectionError {
    /// A store operation failed.
    #[error("store failed during projection work: {0}")]
    Store(String),
    /// A schema identifier was refused.
    #[error("schema error: {0}")]
    Schema(String),
    /// The comparison found mismatches; the reader switch is refused.
    #[error("refusing reader switch: {} projection(s) disagree with rebuilt totals", .0.len())]
    RefusedSwitch(Vec<String>),
    /// The batch named has no current stored projection versions to publish
    /// (it was never written or a newer batch superseded it).
    #[error("no current stored projection versions for batch '{0}'")]
    UnknownBatch(String),
    /// The stored rows a comparison needs are not readable for the batch.
    #[error("stored projection rows for batch '{batch_id}' are not readable: {reason}")]
    StagedUnreadable {
        /// The batch whose rows could not be read.
        batch_id: String,
        /// Why the rows are unusable.
        reason: String,
    },
}

impl From<StoreError> for ProjectionError {
    fn from(error: StoreError) -> Self {
        ProjectionError::Store(error.to_string())
    }
}

impl From<SchemaError> for ProjectionError {
    fn from(error: SchemaError) -> Self {
        ProjectionError::Schema(error.to_string())
    }
}

/// The two report-day projections, computed together.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportDayProjections {
    /// The customer daily rows.
    pub customer_daily: Vec<CustomerDailyRow>,
    /// The platform daily rows.
    pub platform_daily: Vec<PlatformDailyRow>,
}

/// Fold both report-day projections in ONE pass over the source rows.
///
/// A rebuild needs both and loads the report days once; before this fold the
/// same slice was walked twice. The per-row rules are unchanged:
///
/// - Only report days with a known revenue **and** a known currency become
///   money rows: a missing amount is never folded into a zero. Days whose rows
///   lack an amount stay visible as gaps in the data-quality read model.
/// - Impressions are summed only when every contributing row supplied them;
///   otherwise the aggregate is `None` (unknown), never zero.
/// - Platform distinct-account counts fold through sets: a linear `contains`
///   over the accumulated account list would make one day cell quadratic in
///   its account count.
///
/// The separate [`compute_customer_daily`] and [`compute_platform_daily`]
/// helpers delegate here, so every caller shares the exact same fold.
pub fn compute_report_day_projections(report_days: &[ReportDayRow]) -> ReportDayProjections {
    struct CustomerAccumulator {
        revenue_micros: i64,
        impressions: Option<u64>,
        source_days: u32,
        final_days: u32,
        provisional_days: u32,
    }
    struct PlatformAccumulator {
        revenue_micros: i64,
        impressions: Option<u64>,
        // Only the distinct counts are read, so hashing suffices; the
        // previous `Vec::contains` was quadratic in the cell's account count.
        final_accounts: std::collections::HashSet<String>,
        provisional_accounts: std::collections::HashSet<String>,
    }
    let mut customers: BTreeMap<(String, String, String, String), CustomerAccumulator> =
        BTreeMap::new();
    let mut platforms: BTreeMap<(String, String, String), PlatformAccumulator> = BTreeMap::new();
    for row in report_days {
        let (Some(revenue_micros), Some(currency)) = (row.revenue_micros, row.currency.as_deref())
        else {
            continue;
        };
        let currency = currency.to_string();
        let customer = customers
            .entry((
                row.publisher_id.clone(),
                row.day.clone(),
                row.platform.clone(),
                currency.clone(),
            ))
            .or_insert_with(|| CustomerAccumulator {
                revenue_micros: 0,
                impressions: Some(0),
                source_days: 0,
                final_days: 0,
                provisional_days: 0,
            });
        customer.revenue_micros = customer.revenue_micros.saturating_add(revenue_micros);
        customer.impressions = match (customer.impressions, row.impressions) {
            (Some(sum), Some(value)) => sum.checked_add(value),
            _ => None,
        };
        customer.source_days += 1;
        match row.finality.as_str() {
            "final" => customer.final_days += 1,
            "provisional" => customer.provisional_days += 1,
            _ => {}
        }

        let platform = platforms
            .entry((row.platform.clone(), row.day.clone(), currency))
            .or_insert_with(|| PlatformAccumulator {
                revenue_micros: 0,
                impressions: Some(0),
                final_accounts: std::collections::HashSet::new(),
                provisional_accounts: std::collections::HashSet::new(),
            });
        platform.revenue_micros = platform.revenue_micros.saturating_add(revenue_micros);
        platform.impressions = match (platform.impressions, row.impressions) {
            (Some(sum), Some(value)) => sum.checked_add(value),
            _ => None,
        };
        let accounts = if row.finality == "final" {
            &mut platform.final_accounts
        } else if row.finality == "provisional" {
            &mut platform.provisional_accounts
        } else {
            continue;
        };
        accounts.insert(row.platform_account_id.clone());
    }
    ReportDayProjections {
        customer_daily: customers
            .into_iter()
            .map(
                |((publisher_id, day, platform, currency), accumulator)| CustomerDailyRow {
                    publisher_id,
                    day,
                    platform,
                    currency,
                    revenue_micros: accumulator.revenue_micros,
                    impressions: accumulator.impressions,
                    source_days: accumulator.source_days,
                    final_days: accumulator.final_days,
                    provisional_days: accumulator.provisional_days,
                },
            )
            .collect(),
        platform_daily: platforms
            .into_iter()
            .map(
                |((platform, day, currency), accumulator)| PlatformDailyRow {
                    platform,
                    day,
                    currency,
                    revenue_micros: accumulator.revenue_micros,
                    impressions: accumulator.impressions,
                    final_accounts: accumulator.final_accounts.len() as u32,
                    provisional_accounts: accumulator.provisional_accounts.len() as u32,
                },
            )
            .collect(),
    }
}

/// Compute the customer daily projection.
pub fn compute_customer_daily(report_days: &[ReportDayRow]) -> Vec<CustomerDailyRow> {
    compute_report_day_projections(report_days).customer_daily
}

/// Compute the platform daily projection.
pub fn compute_platform_daily(report_days: &[ReportDayRow]) -> Vec<PlatformDailyRow> {
    compute_report_day_projections(report_days).platform_daily
}

/// Compute the lifecycle cohort projection: accounts grouped by stage and by
/// the month they first appeared.
pub fn compute_lifecycle_cohorts(accounts: &[AccountRow]) -> Vec<LifecycleCohortRow> {
    let mut map: BTreeMap<(String, String, String), u32> = BTreeMap::new();
    for row in accounts {
        let key = (
            format!("{}-01", row.first_seen.get(..7).unwrap_or(&row.first_seen)),
            row.publisher_id.clone(),
            row.stage.clone(),
        );
        *map.entry(key).or_insert(0) += 1;
    }
    map.into_iter()
        .map(
            |((cohort_month, publisher_id, stage), accounts)| LifecycleCohortRow {
                cohort_month,
                publisher_id,
                stage,
                accounts,
            },
        )
        .collect()
}

/// Compute the operator queues: incidents, due decisions, stale accounts,
/// overdue jobs and held fees.
///
/// The queues are derived from typed facts only: incident state from
/// `incident_changed` events, due decisions from a completed registered
/// window without a decision, staleness from the account stage timestamps,
/// overdue jobs from failed or missed `job_run` outcomes, and held fees from
/// `fee_held` events without a later release for the same change and period.
pub fn compute_operator_queues(
    events: &[DomainEvent],
    accounts: &[AccountRow],
    as_of: UtcMillis,
) -> Vec<OperatorQueueRow> {
    let mut rows: Vec<OperatorQueueRow> = Vec::new();
    let age_seconds = |at: UtcMillis| -> u64 {
        (as_of.unix_millis().saturating_sub(at.unix_millis()) / 1_000).max(0) as u64
    };
    let publisher_of = |event: &DomainEvent| event.publisher_id.clone().unwrap_or_default();

    // One pass over the events collects every fact the five queues need.
    // Before this fold each queue re-scanned the whole slice, so a rebuild
    // walked the same 100k-event history five times.
    let fee_key = |event: &DomainEvent| -> Option<(String, String)> {
        Some((
            event.payload.change_id.clone()?,
            event.payload.period.clone()?,
        ))
    };
    // Incidents: latest state per incident identity.
    let mut incidents: BTreeMap<String, &DomainEvent> = BTreeMap::new();
    // Due decisions: window registration, start and decision facts.
    let mut window_days: BTreeMap<String, u32> = BTreeMap::new();
    let mut started: BTreeMap<String, &DomainEvent> = BTreeMap::new();
    let mut decided: BTreeMap<String, UtcMillis> = BTreeMap::new();
    // Overdue jobs and held fees keep their source event until emission, so
    // the final row order is exactly the pre-fold order.
    let mut job_events: Vec<&DomainEvent> = Vec::new();
    let mut fee_held_events: Vec<&DomainEvent> = Vec::new();
    let mut released: BTreeMap<(String, String), UtcMillis> = BTreeMap::new();
    for event in events {
        match event.event_type {
            EventType::IncidentChanged => {
                let Some(incident_id) = event.payload.incident_id.clone() else {
                    continue;
                };
                match incidents.get(&incident_id) {
                    Some(existing) if existing.occurred_at_utc >= event.occurred_at_utc => {}
                    _ => {
                        incidents.insert(incident_id, event);
                    }
                }
            }
            EventType::ExperimentRegistered => {
                if let (Some(experiment_id), Some(days)) = (
                    event.payload.experiment_id.clone(),
                    event.payload.window_days,
                ) {
                    window_days.insert(experiment_id, days);
                }
            }
            EventType::ExperimentStarted => {
                if let Some(experiment_id) = event.payload.experiment_id.clone() {
                    started.insert(experiment_id, event);
                }
            }
            EventType::ExperimentDecided => {
                if let Some(experiment_id) = event.payload.experiment_id.clone() {
                    let decided_at = event.occurred_at_utc;
                    decided
                        .entry(experiment_id)
                        .and_modify(|existing| {
                            if decided_at > *existing {
                                *existing = decided_at;
                            }
                        })
                        .or_insert(decided_at);
                }
            }
            EventType::JobRun => {
                let outcome = event.outcome.as_deref().unwrap_or("");
                if matches!(outcome, "failed" | "missed") {
                    job_events.push(event);
                }
            }
            EventType::FeeInvoiceIssued | EventType::FeeCredited => {
                if let Some(key) = fee_key(event) {
                    released
                        .entry(key)
                        .and_modify(|existing| {
                            if event.occurred_at_utc > *existing {
                                *existing = event.occurred_at_utc;
                            }
                        })
                        .or_insert(event.occurred_at_utc);
                }
            }
            EventType::FeeHeld => fee_held_events.push(event),
            _ => {}
        }
    }
    for (incident_id, event) in &incidents {
        if event.payload.state.as_deref() == Some("resolved") {
            continue;
        }
        rows.push(OperatorQueueRow {
            queue: "incidents".into(),
            object_id: incident_id.clone(),
            publisher_id: publisher_of(event),
            severity: event.outcome.clone().unwrap_or_else(|| "unknown".into()),
            age_seconds: age_seconds(event.occurred_at_utc),
            owner: event.payload.owner.clone(),
            next_action: "review_incident".into(),
        });
    }

    // Due decisions: a started experiment whose registered window completed
    // with no decision recorded.
    for (experiment_id, start) in &started {
        if decided.contains_key(experiment_id) {
            continue;
        }
        let Some(days) = window_days.get(experiment_id) else {
            continue;
        };
        let due_at = UtcMillis::from_unix_millis(
            start
                .occurred_at_utc
                .unix_millis()
                .saturating_add(i64::from(*days) * MILLIS_PER_DAY),
        );
        if due_at > as_of {
            continue;
        }
        rows.push(OperatorQueueRow {
            queue: "due_decisions".into(),
            object_id: experiment_id.clone(),
            publisher_id: publisher_of(start),
            severity: "warning".into(),
            age_seconds: age_seconds(due_at),
            owner: None,
            next_action: "record_decision".into(),
        });
    }

    // Stale accounts.
    for account in accounts {
        let Ok(updated_day) =
            UtcMillis::parse_rfc3339(&format!("{}T00:00:00Z", account.stage_updated))
        else {
            continue;
        };
        let stale_after = UtcMillis::from_unix_millis(
            updated_day
                .unix_millis()
                .saturating_add(STALE_ACCOUNT_DAYS * MILLIS_PER_DAY),
        );
        if stale_after > as_of {
            continue;
        }
        rows.push(OperatorQueueRow {
            queue: "stale_accounts".into(),
            object_id: account.account_id.clone(),
            publisher_id: account.publisher_id.clone(),
            severity: "warning".into(),
            age_seconds: age_seconds(updated_day),
            owner: account.owner.clone(),
            next_action: "check_connection".into(),
        });
    }

    // Overdue jobs: a job run whose durable outcome is failed or missed.
    for event in job_events {
        rows.push(OperatorQueueRow {
            queue: "overdue_jobs".into(),
            object_id: event.payload.job_id.clone().unwrap_or_default(),
            publisher_id: publisher_of(event),
            severity: "warning".into(),
            age_seconds: age_seconds(event.occurred_at_utc),
            owner: None,
            next_action: "inspect_job".into(),
        });
    }

    // Held fees: a hold without a later invoice or credit for the same
    // change and period.
    for event in fee_held_events {
        let Some(key) = fee_key(event) else {
            continue;
        };
        let resolved = released
            .get(&key)
            .is_some_and(|at| *at >= event.occurred_at_utc);
        if resolved {
            continue;
        }
        rows.push(OperatorQueueRow {
            queue: "held_fees".into(),
            object_id: format!("{}:{}", key.0, key.1),
            publisher_id: publisher_of(event),
            severity: "warning".into(),
            age_seconds: age_seconds(event.occurred_at_utc),
            owner: None,
            next_action: "verify_proof".into(),
        });
    }

    rows.sort_by(|left, right| {
        left.queue
            .cmp(&right.queue)
            .then(right.age_seconds.cmp(&left.age_seconds))
            .then(left.object_id.cmp(&right.object_id))
    });
    rows
}

/// Server-side `INSERT ... SELECT` rebuild for a projection (deployment fast
/// path; the portable path recomputes in Rust and writes rows). Both paths
/// follow the same rules; the integration test runs every statement against
/// the real server and asserts the counts and money totals agree.
pub fn rebuild_sql(
    database: &str,
    name: ProjectionName,
    as_of_version: u64,
) -> Result<String, SchemaError> {
    let db = validate_identifier(database)?;
    let sql = match name {
        ProjectionName::CustomerDaily => format!(
            "INSERT INTO {db}.customer_daily\n\
             SELECT publisher_id, day, platform, currency,\n\
             \x20   toInt64(sum(revenue_micros)) AS revenue_micros,\n\
             \x20   if(countIf(impressions IS NULL) = 0, toNullable(sum(impressions)), NULL) AS impressions,\n\
             \x20   toUInt32(count()) AS source_days,\n\
             \x20   toUInt32(countIf(finality = 'final')) AS final_days,\n\
             \x20   toUInt32(countIf(finality = 'provisional')) AS provisional_days,\n\
             \x20   {as_of_version} AS as_of_version\n\
             FROM {db}.report_days AS report FINAL\n\
             WHERE report.revenue_micros IS NOT NULL AND report.currency IS NOT NULL\n\
             GROUP BY publisher_id, day, platform, currency"
        ),
        ProjectionName::PlatformDaily => format!(
            "INSERT INTO {db}.platform_daily\n\
             SELECT platform, day, currency,\n\
             \x20   toInt64(sum(revenue_micros)) AS revenue_micros,\n\
             \x20   if(countIf(impressions IS NULL) = 0, toNullable(sum(impressions)), NULL) AS impressions,\n\
             \x20   toUInt32(uniqExactIf(platform_account_id, finality = 'final')) AS final_accounts,\n\
             \x20   toUInt32(uniqExactIf(platform_account_id, finality = 'provisional')) AS provisional_accounts,\n\
             \x20   {as_of_version} AS as_of_version\n\
             FROM {db}.report_days AS report FINAL\n\
             WHERE report.revenue_micros IS NOT NULL AND report.currency IS NOT NULL\n\
             GROUP BY platform, day, currency"
        ),
        ProjectionName::LifecycleCohorts => format!(
            "INSERT INTO {db}.lifecycle_cohorts\n\
             SELECT toDate(concat(substring(toString(first_seen), 1, 7), '-01')) AS cohort_month,\n\
             \x20   publisher_id, stage, toUInt32(count()) AS accounts,\n\
             \x20   {as_of_version} AS as_of_version\n\
             FROM {db}.accounts FINAL\n\
             GROUP BY cohort_month, publisher_id, stage"
        ),
        ProjectionName::OperatorQueues => {
            let stale_days = STALE_ACCOUNT_DAYS;
            format!(
                "INSERT INTO {db}.operator_queues\n\
             SELECT 'incidents' AS queue,\n\
             \x20   incident_id AS object_id,\n\
             \x20   publisher_id,\n\
             \x20   severity,\n\
             \x20   toUInt64(greatest(0, dateDiff('second', last_occurred, now64(3)))) AS age_seconds,\n\
             \x20   owner, 'review_incident' AS next_action, {as_of_version} AS as_of_version\n\
             FROM (\n\
             \x20   SELECT JSONExtractString(payload_json, 'incident_id') AS incident_id,\n\
             \x20       argMax(coalesce(publisher_id, ''), occurred_at_utc) AS publisher_id,\n\
             \x20       argMax(coalesce(outcome, 'unknown'), occurred_at_utc) AS severity,\n\
             \x20       argMax(JSONExtractString(payload_json, 'state'), occurred_at_utc) AS state,\n\
             \x20       nullIf(argMax(JSONExtractString(payload_json, 'owner'), occurred_at_utc), '') AS owner,\n\
             \x20       max(occurred_at_utc) AS last_occurred\n\
             \x20   FROM {db}.domain_events FINAL\n\
             \x20   WHERE event_type = 'incident_changed'\n\
             \x20   GROUP BY incident_id\n\
             \x20   HAVING incident_id != ''\n\
             ) AS incident\n\
             WHERE state != 'resolved'\n\
             UNION ALL\n\
             SELECT 'due_decisions' AS queue, e.experiment_id AS object_id,\n\
             \x20   e.publisher_id, 'warning' AS severity,\n\
             \x20   toUInt64(greatest(0, dateDiff('second', addDays(e.started_at, r.window_days), now64(3)))) AS age_seconds,\n\
             \x20   NULL AS owner, 'record_decision' AS next_action, {as_of_version} AS as_of_version\n\
             FROM (\n\
             \x20   SELECT JSONExtractString(payload_json, 'experiment_id') AS experiment_id,\n\
             \x20       argMax(coalesce(publisher_id, ''), occurred_at_utc) AS publisher_id,\n\
             \x20       max(occurred_at_utc) AS started_at\n\
             \x20   FROM {db}.domain_events FINAL\n\
             \x20   WHERE event_type = 'experiment_started'\n\
             \x20   GROUP BY experiment_id\n\
             ) AS e\n\
             INNER JOIN (\n\
             \x20   SELECT JSONExtractString(payload_json, 'experiment_id') AS experiment_id,\n\
             \x20       max(toUInt32OrZero(JSONExtractString(payload_json, 'window_days'))) AS window_days\n\
             \x20   FROM {db}.domain_events FINAL\n\
             \x20   WHERE event_type = 'experiment_registered'\n\
             \x20   GROUP BY experiment_id\n\
             ) AS r USING (experiment_id)\n\
             WHERE e.experiment_id NOT IN (\n\
             \x20   SELECT JSONExtractString(payload_json, 'experiment_id')\n\
             \x20   FROM {db}.domain_events FINAL WHERE event_type = 'experiment_decided'\n\
             )\n\
             \x20 AND addDays(e.started_at, r.window_days) <= now64(3)\n\
             UNION ALL\n\
             SELECT 'stale_accounts' AS queue, account_id AS object_id, publisher_id,\n\
             \x20   'warning' AS severity,\n\
             \x20   toUInt64(greatest(0, dateDiff('second', stage_updated, now64(3)))) AS age_seconds,\n\
             \x20   owner, 'check_connection' AS next_action, {as_of_version} AS as_of_version\n\
             FROM {db}.accounts FINAL\n\
             WHERE addDays(stage_updated, {stale_days}) <= now64(3)\n\
             UNION ALL\n\
             SELECT 'overdue_jobs' AS queue,\n\
             \x20   JSONExtractString(payload_json, 'job_id') AS object_id,\n\
             \x20   coalesce(publisher_id, '') AS publisher_id, 'warning' AS severity,\n\
             \x20   toUInt64(greatest(0, dateDiff('second', occurred_at_utc, now64(3)))) AS age_seconds,\n\
             \x20   NULL AS owner, 'inspect_job' AS next_action, {as_of_version} AS as_of_version\n\
             FROM {db}.domain_events FINAL\n\
             WHERE event_type = 'job_run' AND outcome IN ('failed', 'missed')\n\
             UNION ALL\n\
             SELECT 'held_fees' AS queue, concat(change_id, ':', period) AS object_id,\n\
             \x20   coalesce(publisher_id, '') AS publisher_id, 'warning' AS severity,\n\
             \x20   toUInt64(greatest(0, dateDiff('second', occurred_at_utc, now64(3)))) AS age_seconds,\n\
             \x20   NULL AS owner, 'verify_proof' AS next_action, {as_of_version} AS as_of_version\n\
             FROM {db}.domain_events AS e FINAL\n\
             WHERE event_type = 'fee_held' AND change_id IS NOT NULL AND period IS NOT NULL\n\
             \x20 AND NOT EXISTS (\n\
             \x20   SELECT 1 FROM {db}.domain_events AS release FINAL\n\
             \x20   WHERE release.event_type IN ('fee_invoice_issued', 'fee_credited')\n\
             \x20     AND release.change_id = e.change_id AND release.period = e.period\n\
             \x20     AND release.occurred_at_utc >= e.occurred_at_utc\n\
             \x20 )",
            )
        }
    };
    Ok(sql)
}

/// `SELECT count(), sum(revenue_micros)` totals for one explicit as-of
/// version of a projection table, used by the compare step on the server
/// side of the `rebuild_sql` fast path. The version is an internal `u64`,
/// never user input, and pinning it keeps the totals free of rows from
/// other (staged or older) versions.
pub fn totals_sql(
    database: &str,
    name: ProjectionName,
    as_of_version: u64,
) -> Result<String, SchemaError> {
    let db = validate_identifier(database)?;
    let table = name.table_name();
    Ok(match name {
        ProjectionName::CustomerDaily | ProjectionName::PlatformDaily => format!(
            "SELECT count() AS rows, toString(sum(revenue_micros)) AS revenue_micros \
             FROM {db}.{table} WHERE as_of_version = {as_of_version}"
        ),
        _ => format!(
            "SELECT count() AS rows FROM {db}.{table} WHERE as_of_version = {as_of_version}"
        ),
    })
}

/// The column list a projection read returns, in table order.
fn projection_columns(name: ProjectionName) -> &'static str {
    match name {
        ProjectionName::CustomerDaily => {
            "publisher_id, toString(day) AS day, platform, currency, revenue_micros, \
             impressions, source_days, final_days, provisional_days"
        }
        ProjectionName::PlatformDaily => {
            "platform, toString(day) AS day, currency, revenue_micros, impressions, \
             final_accounts, provisional_accounts"
        }
        ProjectionName::LifecycleCohorts => {
            "toString(cohort_month) AS cohort_month, publisher_id, stage, accounts"
        }
        ProjectionName::OperatorQueues => {
            "queue, object_id, publisher_id, severity, age_seconds, owner, next_action"
        }
    }
}

/// The row-order clause of a projection read.
fn projection_order(name: ProjectionName) -> &'static str {
    match name {
        ProjectionName::CustomerDaily => "publisher_id, day, platform, currency",
        ProjectionName::PlatformDaily => "platform, day, currency",
        ProjectionName::LifecycleCohorts => "cohort_month, publisher_id, stage",
        ProjectionName::OperatorQueues => "queue, object_id",
    }
}

/// The `SELECT` a store uses to read one projection back into typed rows:
/// only the rows of the currently PUBLISHED as-of version. The projection
/// name travels as the bound `{projection:String}` parameter. The database
/// is a validated identifier and named explicitly, matching the store's
/// no-connection-default posture.
///
/// No `FINAL` is used: `FINAL` collapses rows across as-of versions and
/// would hide a published version behind a newer staged one; equality on
/// the published version is exact because each version is written once.
pub fn projection_select_sql(database: &str, name: ProjectionName) -> Option<String> {
    let columns = projection_columns(name);
    let order = projection_order(name);
    let table = name.table_name();
    Some(format!(
        "SELECT {columns}\n\
         FROM {database}.{table}\n\
         WHERE as_of_version = (SELECT max(as_of_version) FROM {database}.projection_versions \
         WHERE projection_name = {{projection:String}} AND published = 1)\n\
         ORDER BY {order}"
    ))
}

/// The `SELECT` for the rows of one explicit as-of version, published or
/// not: the staged compare read. The version is an internal `u64`, never
/// user input.
pub fn projection_version_select_sql(
    database: &str,
    name: ProjectionName,
    as_of_version: u64,
) -> Option<String> {
    let columns = projection_columns(name);
    let order = projection_order(name);
    let table = name.table_name();
    Some(format!(
        "SELECT {columns}\n\
         FROM {database}.{table}\n\
         WHERE as_of_version = {as_of_version}\n\
         ORDER BY {order}"
    ))
}

/// The as-of a rebuild uses when the caller does not inject one: the newest
/// received event time (the store's data horizon), or the system clock when
/// the store is empty.
fn data_horizon(events: &[DomainEvent]) -> Result<UtcMillis, ProjectionError> {
    match events.iter().map(|event| event.received_at_utc).max() {
        Some(horizon) => Ok(horizon),
        None => UtcMillis::system_now().map_err(|error| ProjectionError::Store(error.to_string())),
    }
}

async fn compute_all<S: ProjectionStore + ?Sized>(
    store: &S,
) -> Result<Vec<(ProjectionName, Vec<ProjectionRow>)>, ProjectionError> {
    let events = store.load_events().await?;
    let report_days = store.load_report_days().await?;
    let accounts = store.load_accounts().await?;
    let horizon = data_horizon(&events)?;
    // Both report-day projections share one fold over the source rows.
    let ReportDayProjections {
        customer_daily,
        platform_daily,
    } = compute_report_day_projections(&report_days);
    let rows = vec![
        (
            ProjectionName::CustomerDaily,
            customer_daily
                .into_iter()
                .map(ProjectionRow::CustomerDaily)
                .collect(),
        ),
        (
            ProjectionName::PlatformDaily,
            platform_daily
                .into_iter()
                .map(ProjectionRow::PlatformDaily)
                .collect(),
        ),
        (
            ProjectionName::LifecycleCohorts,
            compute_lifecycle_cohorts(&accounts)
                .into_iter()
                .map(ProjectionRow::LifecycleCohorts)
                .collect(),
        ),
        (
            ProjectionName::OperatorQueues,
            compute_operator_queues(&events, &accounts, horizon)
                .into_iter()
                .map(ProjectionRow::OperatorQueues)
                .collect(),
        ),
    ];
    Ok(rows)
}

/// Rebuild every projection from source facts under a new as-of version.
///
/// A batch ID names the run ("a backfill has a batch ID and changes the
/// as-of version"); the default batch is `rebuild-<unix-millis>`. The new
/// versions are STAGED (unpublished): readers keep the previous version
/// until [`publish`] runs after a clean compare.
pub async fn rebuild<S: ProjectionStore + ?Sized>(
    store: &S,
) -> Result<ProjectionReport, ProjectionError> {
    let batch_id = format!(
        "rebuild-{}",
        UtcMillis::system_now()
            .map_err(|error| ProjectionError::Store(error.to_string()))?
            .unix_millis()
    );
    rebuild_batch(store, &batch_id).await
}

/// Rebuild under an explicit batch identity (used by backfills and tests).
///
/// Each projection advances to `newest recorded version + 1`, so a staged
/// version is never overwritten by a later rebuild, and the written version
/// stays invisible to readers until [`publish`].
pub async fn rebuild_batch<S: ProjectionStore + ?Sized>(
    store: &S,
    batch_id: &str,
) -> Result<ProjectionReport, ProjectionError> {
    let computed = compute_all(store).await?;
    let mut rebuilt = Vec::new();
    let mut highest = 0u64;
    for (name, rows) in computed {
        let next_version = store
            .latest_projection_version(name)
            .await?
            .unwrap_or(0)
            .saturating_add(1);
        store
            .write_projection(name, next_version, batch_id, &rows)
            .await?;
        highest = highest.max(next_version);
        rebuilt.push(ProjectionRebuild {
            projection: name,
            as_of_version: next_version,
            rows: rows.len() as u64,
            totals: ProjectionTotals::of(name, &rows),
        });
    }
    Ok(ProjectionReport {
        batch_id: batch_id.to_string(),
        as_of_version: highest,
        rebuilt,
    })
}

/// Compare one projection's stored (staged) rows with expected totals.
fn compare_stored(
    name: ProjectionName,
    staged: crate::store::StagedProjection,
    expected: ProjectionTotals,
) -> ProjectionComparison {
    let current = ProjectionTotals::of(name, &staged.rows);
    let status = if current == expected {
        ComparisonStatus::Match { totals: expected }
    } else {
        ComparisonStatus::Mismatch {
            rebuilt: expected,
            current,
        }
    };
    ProjectionComparison {
        projection: name,
        current_as_of_version: Some(staged.as_of_version),
        status,
    }
}

/// Recompute totals from source facts and compare them with the newest
/// STORED rows of every projection (published or not). The switch is staged:
/// only a clean [`ComparisonReport::is_clean`] may be followed by
/// [`publish`].
pub async fn compare<S: ProjectionStore + ?Sized>(
    store: &S,
) -> Result<ComparisonReport, ProjectionError> {
    let computed = compute_all(store).await?;
    let mut comparisons = Vec::new();
    for (name, rows) in computed {
        let rebuilt = ProjectionTotals::of(name, &rows);
        match store.latest_projection(name).await? {
            Some(staged) => comparisons.push(compare_stored(name, staged, rebuilt)),
            None => comparisons.push(ProjectionComparison {
                projection: name,
                current_as_of_version: None,
                status: ComparisonStatus::Absent { rebuilt },
            }),
        }
    }
    Ok(ComparisonReport { comparisons })
}

/// Compare a rebuild report's computed totals with the currently STORED
/// (new, unpublished) rows of the same batch, without recomputing anything.
///
/// This is the runner's compare: [`rebuild_batch`] already computed the
/// totals, so recomputing them again in [`compare`] would double the work.
/// Every projection of the report must still have its staged rows readable
/// and attributable to the report's batch; otherwise the error names the
/// batch.
pub async fn compare_report<S: ProjectionStore + ?Sized>(
    store: &S,
    report: &ProjectionReport,
) -> Result<ComparisonReport, ProjectionError> {
    let mut comparisons = Vec::new();
    for name in ProjectionName::ALL {
        let Some(rebuild) = report.rebuilt.iter().find(|entry| entry.projection == name) else {
            return Err(ProjectionError::StagedUnreadable {
                batch_id: report.batch_id.clone(),
                reason: format!("the report has no rebuild entry for {name}"),
            });
        };
        let staged = match store.latest_projection(name).await? {
            Some(staged) => staged,
            None => {
                return Err(ProjectionError::StagedUnreadable {
                    batch_id: report.batch_id.clone(),
                    reason: format!("no stored rows for {name}"),
                })
            }
        };
        if staged.batch_id != report.batch_id {
            return Err(ProjectionError::StagedUnreadable {
                batch_id: report.batch_id.clone(),
                reason: format!(
                    "the newest stored {name} rows belong to batch '{}'",
                    staged.batch_id
                ),
            });
        }
        comparisons.push(compare_stored(name, staged, rebuild.totals.clone()));
    }
    Ok(ComparisonReport { comparisons })
}

/// Make one batch's staged versions reader-visible and return the published
/// version. Only call this after a clean compare: `rebuild -> compare ->
/// publish`. Returns [`ProjectionError::UnknownBatch`] when the batch has no
/// recorded version rows at all. Readers always resolve the highest
/// published version, so publishing a superseded batch can never roll them
/// back below a newer published one.
pub async fn publish(
    store: &(impl ProjectionStore + ?Sized),
    batch_id: &str,
) -> Result<u64, ProjectionError> {
    match store.publish_batch(batch_id).await? {
        Some(version) => Ok(version),
        None => Err(ProjectionError::UnknownBatch(batch_id.to_string())),
    }
}

/// Refuse the reader switch unless every projection matches. Call it on the
/// comparison report before [`publish`]; a mismatch keeps readers on the
/// previously published version.
pub fn enforce_switch(report: &ComparisonReport) -> Result<(), ProjectionError> {
    if report.is_clean() {
        return Ok(());
    }
    let offenders = report
        .mismatches()
        .iter()
        .map(|comparison| comparison.projection.as_str().to_string())
        .collect();
    Err(ProjectionError::RefusedSwitch(offenders))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::store::{EventStore, MemoryStore};
    use crate::test_support::{
        known_value_fee_event, known_value_report_days, occurred, payload_for, valid_event,
        valid_event_builder,
    };

    async fn known_value_store() -> MemoryStore {
        let store = MemoryStore::new();
        store.ensure_schema().await.expect("schema");
        store
            .insert_report_days(&known_value_report_days())
            .await
            .expect("days");
        store.insert(&[known_value_fee_event()]).await.expect("fee");
        store
    }

    #[test]
    fn customer_daily_sums_the_known_value_fixture() {
        let rows = compute_customer_daily(&known_value_report_days());
        assert_eq!(rows.len(), 10);
        let total: i64 = rows.iter().map(|row| row.revenue_micros).sum();
        assert_eq!(total, 51_500_000_000, "US$ 51,500.00 exactly");
        let first = &rows[0];
        assert_eq!(first.day, "2026-10-01");
        assert_eq!(first.currency, "USD");
        assert_eq!(first.revenue_micros, 5_150_000_000);
        assert_eq!(first.impressions, Some(2_500_000));
        assert_eq!(first.final_days, 1);
        assert_eq!(first.provisional_days, 0);
    }

    #[test]
    fn missing_amounts_are_gaps_never_zero_rows() {
        let mut days = known_value_report_days();
        days.push(ReportDayRow {
            day: "2026-10-11".into(),
            revenue_micros: None,
            impressions: None,
            currency: Some("USD".into()),
            ..days[0].clone()
        });
        let rows = compute_customer_daily(&days);
        // The amount-less row is a gap: it creates no money row.
        assert_eq!(rows.len(), 10);
        assert!(!rows.iter().any(|row| row.day == "2026-10-11"));
    }

    #[test]
    fn impressions_are_unknown_when_any_contributor_lacks_them() {
        let mut days = known_value_report_days();
        days[3].impressions = None;
        let rows = compute_customer_daily(&days);
        let oct_04 = rows
            .iter()
            .find(|row| row.day == "2026-10-04")
            .expect("row");
        assert_eq!(oct_04.impressions, None);
        assert_eq!(oct_04.revenue_micros, 5_150_000_000);
    }

    #[test]
    fn platform_daily_counts_final_and_provisional_accounts() {
        let mut days = known_value_report_days();
        let mut provisional = days[0].clone();
        provisional.platform_account_id = "max_acct_2".into();
        provisional.finality = "provisional".into();
        days.push(provisional);
        let rows = compute_platform_daily(&days);
        let oct_01 = rows
            .iter()
            .find(|row| row.day == "2026-10-01")
            .expect("row");
        assert_eq!(oct_01.final_accounts, 1);
        assert_eq!(oct_01.provisional_accounts, 1);
        assert_eq!(oct_01.revenue_micros, 10_300_000_000);
    }

    #[test]
    fn lifecycle_cohorts_group_by_first_seen_month() {
        let accounts = vec![
            AccountRow {
                publisher_id: "northstar".into(),
                account_id: "a1".into(),
                stage: "paying".into(),
                owner: Some("maya".into()),
                platforms: vec!["applovin_max".into()],
                first_seen: "2026-09-03".into(),
                stage_updated: "2026-10-01".into(),
            },
            AccountRow {
                publisher_id: "northstar".into(),
                account_id: "a2".into(),
                stage: "paying".into(),
                owner: None,
                platforms: vec![],
                first_seen: "2026-09-20".into(),
                stage_updated: "2026-10-01".into(),
            },
        ];
        let rows = compute_lifecycle_cohorts(&accounts);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cohort_month, "2026-09-01");
        assert_eq!(rows[0].accounts, 2);
    }

    #[test]
    fn operator_queues_derive_from_typed_facts() {
        let mut incident = valid_event_builder(EventType::IncidentChanged)
            .with_event_id("incident_open")
            .with_occurred_at(UtcMillis::from_unix_millis(
                occurred().unix_millis() - 5 * 60 * 1_000,
            ))
            .with_outcome("critical")
            .build()
            .expect("incident");
        // A later resolved state clears the queue.
        let resolved = {
            let mut payload = payload_for(EventType::IncidentChanged);
            payload.state = Some("resolved".into());
            valid_event_builder(EventType::IncidentChanged)
                .with_event_id("incident_resolved")
                .with_occurred_at(occurred())
                .with_outcome("resolved")
                .with_payload(payload)
                .build()
                .expect("resolved")
        };
        let events = vec![incident.clone(), resolved];
        let accounts = vec![AccountRow {
            publisher_id: "northstar".into(),
            account_id: "acct_1".into(),
            stage: "audit_requested".into(),
            owner: Some("lina".into()),
            platforms: vec![],
            first_seen: "2026-09-01".into(),
            stage_updated: "2026-09-20".into(),
        }];
        let rows = compute_operator_queues(&events, &accounts, occurred());
        // The incident was resolved; the stale account (25 days) remains.
        assert!(rows.iter().all(|row| row.queue != "incidents"));
        let stale = rows
            .iter()
            .find(|row| row.queue == "stale_accounts")
            .expect("stale");
        assert_eq!(stale.object_id, "acct_1");
        assert_eq!(stale.owner.as_deref(), Some("lina"));
        assert!(stale.age_seconds >= 25 * 86_400);

        // Remove the resolve and the incident is queued at its severity.
        incident.payload.state = Some("open".into());
        let rows = compute_operator_queues(&[incident], &[], occurred());
        let queue = rows
            .iter()
            .find(|row| row.queue == "incidents")
            .expect("incident queued");
        assert_eq!(queue.severity, "critical");
        assert_eq!(queue.age_seconds, 300);
        assert_eq!(queue.next_action, "review_incident");
    }

    #[test]
    fn operator_queues_cover_due_decisions_overdue_jobs_and_held_fees() {
        let started = valid_event_builder(EventType::ExperimentStarted)
            .with_event_id("started_1")
            .with_occurred_at(UtcMillis::from_unix_millis(
                occurred().unix_millis() - 20 * MILLIS_PER_DAY,
            ))
            .build()
            .expect("started");
        let registered = valid_event_builder(EventType::ExperimentRegistered)
            .with_event_id("registered_1")
            .with_occurred_at(UtcMillis::from_unix_millis(
                occurred().unix_millis() - 20 * MILLIS_PER_DAY,
            ))
            .build()
            .expect("registered");
        let failed_job = {
            let mut payload = payload_for(EventType::JobRun);
            payload.job_id = Some("job_daily".into());
            valid_event_builder(EventType::JobRun)
                .with_event_id("job_failed")
                .with_outcome("failed")
                .with_payload(payload)
                .build()
                .expect("job")
        };
        let held = valid_event_builder(EventType::FeeHeld)
            .with_event_id("fee_held_1")
            .build()
            .expect("held");
        let events = vec![registered, started, failed_job, held.clone()];
        let rows = compute_operator_queues(&events, &[], occurred());
        assert!(rows.iter().any(|row| row.queue == "due_decisions"
            && row.object_id == "exp_0247"
            && row.next_action == "record_decision"));
        assert!(rows
            .iter()
            .any(|row| row.queue == "overdue_jobs" && row.object_id == "job_daily"));
        assert!(rows
            .iter()
            .any(|row| row.queue == "held_fees" && row.object_id == "ch_0246:2026-10"));

        // An invoiced fee releases the hold.
        let issued = valid_event_builder(EventType::FeeInvoiceIssued)
            .with_event_id("fee_issued_1")
            .build()
            .expect("issued");
        let events = vec![held, issued];
        let rows = compute_operator_queues(&events, &[], occurred());
        assert!(rows.iter().all(|row| row.queue != "held_fees"));
    }

    #[tokio::test]
    async fn rebuild_bumps_the_as_of_version_and_stages_it() {
        let store = known_value_store().await;
        // Nothing is reader-visible before the first publish.
        assert_eq!(
            store
                .projection_version(ProjectionName::CustomerDaily)
                .await
                .expect("version"),
            None
        );
        assert!(store
            .load_projection(ProjectionName::CustomerDaily)
            .await
            .expect("read")
            .is_empty());
        let report = rebuild_batch(&store, "backfill-1").await.expect("rebuild");
        assert_eq!(report.batch_id, "backfill-1");
        assert_eq!(report.as_of_version, 1);
        assert_eq!(report.rebuilt.len(), 4);
        let customer = report
            .rebuilt
            .iter()
            .find(|entry| entry.projection == ProjectionName::CustomerDaily)
            .expect("customer");
        assert_eq!(customer.rows, 10);
        assert_eq!(customer.totals.revenue_micros, Some(51_500_000_000));
        // The version is recorded, staged, and not yet reader-visible.
        assert_eq!(
            store
                .latest_projection_version(ProjectionName::CustomerDaily)
                .await
                .expect("latest"),
            Some(1)
        );
        assert_eq!(
            store
                .projection_version(ProjectionName::CustomerDaily)
                .await
                .expect("version"),
            None
        );

        // A second rebuild (a backfill) moves every projection forward.
        let report = rebuild_batch(&store, "backfill-2")
            .await
            .expect("rebuild 2");
        assert_eq!(report.as_of_version, 2);
        assert!(report.rebuilt.iter().all(|entry| entry.as_of_version == 2));

        // Readers still see nothing until the batch is published.
        assert_eq!(
            store
                .projection_version(ProjectionName::CustomerDaily)
                .await
                .expect("version"),
            None
        );
        assert_eq!(publish(&store, "backfill-2").await.expect("publish"), 2);
        assert_eq!(
            store
                .projection_version(ProjectionName::CustomerDaily)
                .await
                .expect("version"),
            Some(2)
        );

        // The totals survive the round trip through the stored rows.
        let stored = store
            .load_projection(ProjectionName::PlatformDaily)
            .await
            .expect("read");
        let totals = ProjectionTotals::of(ProjectionName::PlatformDaily, &stored);
        assert_eq!(totals.revenue_micros, Some(51_500_000_000));
        assert_eq!(totals.rows, 10);
    }

    #[tokio::test]
    async fn unpublished_versions_stay_invisible_and_publish_switches_readers() {
        let store = MemoryStore::new();
        store.ensure_schema().await.expect("schema");
        store
            .insert_report_days(&known_value_report_days())
            .await
            .expect("days");

        // Stage and publish v1 with the known value.
        let report = rebuild_batch(&store, "stage-1").await.expect("rebuild 1");
        assert_eq!(publish(&store, "stage-1").await.expect("publish 1"), 1);
        let visible = store
            .load_projection(ProjectionName::CustomerDaily)
            .await
            .expect("read");
        assert_eq!(
            ProjectionTotals::of(ProjectionName::CustomerDaily, &visible).revenue_micros,
            Some(51_500_000_000)
        );
        assert_eq!(report.as_of_version, 1);

        // Rebuild v2 from DIFFERENT source values, without publishing:
        // readers must still see v1's rows.
        let changed: Vec<ReportDayRow> = known_value_report_days()
            .into_iter()
            .map(|row| ReportDayRow {
                revenue_micros: Some(1),
                impressions: Some(1),
                ..row
            })
            .collect();
        store
            .insert_report_days(&changed)
            .await
            .expect("changed days");
        let report = rebuild_batch(&store, "stage-2").await.expect("rebuild 2");
        assert_eq!(report.as_of_version, 2);
        let visible = store
            .load_projection(ProjectionName::CustomerDaily)
            .await
            .expect("read");
        let totals = ProjectionTotals::of(ProjectionName::CustomerDaily, &visible);
        assert_eq!(
            totals.revenue_micros,
            Some(51_500_000_000),
            "readers stay on the published v1 until publish"
        );
        assert_eq!(
            store
                .projection_version(ProjectionName::CustomerDaily)
                .await
                .expect("version"),
            Some(1)
        );
        // The staged read (compare path) does see v2.
        let staged = store
            .latest_projection(ProjectionName::CustomerDaily)
            .await
            .expect("staged")
            .expect("v2 stored");
        assert_eq!(staged.as_of_version, 2);
        assert_eq!(staged.batch_id, "stage-2");
        assert_eq!(
            ProjectionTotals::of(ProjectionName::CustomerDaily, &staged.rows).revenue_micros,
            Some(10)
        );

        // A clean compare_report then publish switches readers to v2.
        let comparison = compare_report(&store, &report).await.expect("compare");
        assert!(comparison.is_clean(), "{comparison:?}");
        assert_eq!(
            comparison,
            compare(&store).await.expect("recomputed"),
            "compare_report agrees with compare on the same batch"
        );
        assert_eq!(publish(&store, "stage-2").await.expect("publish 2"), 2);
        let visible = store
            .load_projection(ProjectionName::CustomerDaily)
            .await
            .expect("read");
        assert_eq!(
            ProjectionTotals::of(ProjectionName::CustomerDaily, &visible).revenue_micros,
            Some(10)
        );
        assert_eq!(
            store
                .projection_version(ProjectionName::CustomerDaily)
                .await
                .expect("version"),
            Some(2)
        );
    }

    #[tokio::test]
    async fn a_tampered_staged_version_is_refused_and_readers_stay_on_v1() {
        let store = MemoryStore::new();
        store.ensure_schema().await.expect("schema");
        store
            .insert_report_days(&known_value_report_days())
            .await
            .expect("days");
        rebuild_batch(&store, "stage-1").await.expect("rebuild 1");
        publish(&store, "stage-1").await.expect("publish 1");

        // A v2 staged version with a tampered row: the stored totals no
        // longer agree with what the rebuild computed.
        let report = rebuild_batch(&store, "stage-2").await.expect("rebuild 2");
        let tampered: Vec<ProjectionRow> = vec![ProjectionRow::CustomerDaily(CustomerDailyRow {
            publisher_id: "northstar".into(),
            day: "2026-10-01".into(),
            platform: "applovin_max".into(),
            currency: "USD".into(),
            revenue_micros: 1,
            impressions: None,
            source_days: 1,
            final_days: 1,
            provisional_days: 0,
        })];
        store
            .write_projection(ProjectionName::CustomerDaily, 2, "stage-2", &tampered)
            .await
            .expect("tamper");
        let comparison = compare_report(&store, &report).await.expect("compare");
        assert!(!comparison.is_clean());
        assert!(matches!(
            enforce_switch(&comparison),
            Err(ProjectionError::RefusedSwitch(_))
        ));
        // Readers stay on the published v1; the tampered v2 is visible to
        // no one.
        let visible = store
            .load_projection(ProjectionName::CustomerDaily)
            .await
            .expect("read");
        assert_eq!(
            ProjectionTotals::of(ProjectionName::CustomerDaily, &visible).revenue_micros,
            Some(51_500_000_000)
        );
        assert_eq!(
            store
                .projection_version(ProjectionName::CustomerDaily)
                .await
                .expect("version"),
            Some(1)
        );
    }

    #[tokio::test]
    async fn publishing_names_the_failures_it_refuses() {
        let store = known_value_store().await;
        // A batch nobody wrote has nothing to publish.
        assert_eq!(
            publish(&store, "missing").await,
            Err(ProjectionError::UnknownBatch("missing".to_string()))
        );
        // A report whose staged rows belong to another batch is refused
        // with the batch named.
        let staged = rebuild_batch(&store, "first").await.expect("rebuild");
        let report = ProjectionReport {
            batch_id: "second".into(),
            as_of_version: staged.as_of_version,
            rebuilt: staged.rebuilt,
        };
        match compare_report(&store, &report).await {
            Err(ProjectionError::StagedUnreadable { batch_id, reason }) => {
                assert_eq!(batch_id, "second");
                assert!(reason.contains("batch 'first'"), "{reason}");
            }
            other => panic!("expected StagedUnreadable, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn compare_matches_after_rebuild_and_refuses_the_switch_on_tamper() {
        let store = known_value_store().await;
        // Before any rebuild the reader switch is refused.
        let report = compare(&store).await.expect("compare");
        assert!(!report.is_clean());
        assert!(matches!(
            enforce_switch(&report),
            Err(ProjectionError::RefusedSwitch(_))
        ));

        rebuild_batch(&store, "initial").await.expect("rebuild");
        let report = compare(&store).await.expect("compare");
        assert!(report.is_clean(), "{report:?}");
        enforce_switch(&report).expect("clean switch");

        // Tamper with a stored projection: totals no longer match the source
        // facts, so the switch is refused.
        let tampered: Vec<ProjectionRow> = vec![ProjectionRow::CustomerDaily(CustomerDailyRow {
            publisher_id: "northstar".into(),
            day: "2026-10-01".into(),
            platform: "applovin_max".into(),
            currency: "USD".into(),
            revenue_micros: 1,
            impressions: None,
            source_days: 1,
            final_days: 1,
            provisional_days: 0,
        })];
        store
            .write_projection(ProjectionName::CustomerDaily, 2, "tamper", &tampered)
            .await
            .expect("tamper");
        let report = compare(&store).await.expect("compare");
        assert!(!report.is_clean());
        assert_eq!(report.mismatches().len(), 1);
        assert_eq!(
            report.mismatches()[0].projection,
            ProjectionName::CustomerDaily
        );
        match enforce_switch(&report) {
            Err(ProjectionError::RefusedSwitch(offenders)) => {
                assert_eq!(offenders, vec!["customer_daily".to_string()]);
            }
            other => panic!("expected a refused switch, got {other:?}"),
        }
    }

    #[test]
    fn rebuild_sql_builds_from_source_facts_with_the_new_version() {
        let sql = rebuild_sql("db", ProjectionName::CustomerDaily, 7).expect("sql");
        assert!(sql.starts_with("INSERT INTO db.customer_daily"));
        assert!(sql.contains("FROM db.report_days AS report FINAL"));
        assert!(sql.contains("7 AS as_of_version"));
        assert!(
            sql.contains("WHERE report.revenue_micros IS NOT NULL AND report.currency IS NOT NULL")
        );
        let sql = rebuild_sql("db", ProjectionName::PlatformDaily, 7).expect("sql");
        assert!(sql.contains("uniqExactIf(platform_account_id, finality = 'final')"));
        let sql = rebuild_sql("db", ProjectionName::LifecycleCohorts, 7).expect("sql");
        assert!(sql.contains("FROM db.accounts FINAL"));
        let sql = rebuild_sql("db", ProjectionName::OperatorQueues, 7).expect("sql");
        assert!(sql.contains("event_type = 'incident_changed'"));
        assert!(rebuild_sql("bad name", ProjectionName::CustomerDaily, 1).is_err());
        assert!(totals_sql("bad name", ProjectionName::CustomerDaily, 1).is_err());
    }

    #[test]
    fn totals_sql_reads_rows_and_money_of_one_version() {
        let sql = totals_sql("db", ProjectionName::CustomerDaily, 4).expect("sql");
        assert!(sql.contains("count() AS rows"));
        assert!(sql.contains("sum(revenue_micros)"));
        assert!(sql.contains("FROM db.customer_daily WHERE as_of_version = 4"));
        let sql = totals_sql("db", ProjectionName::OperatorQueues, 4).expect("sql");
        assert!(!sql.contains("revenue_micros"));
        assert!(sql.contains("WHERE as_of_version = 4"));
    }

    #[test]
    fn projection_rows_round_trip_through_table_json() {
        let row = ProjectionRow::CustomerDaily(CustomerDailyRow {
            publisher_id: "northstar".into(),
            day: "2026-10-01".into(),
            platform: "applovin_max".into(),
            currency: "USD".into(),
            revenue_micros: 51_500_000_000,
            impressions: Some(2_500_000),
            source_days: 1,
            final_days: 1,
            provisional_days: 0,
        });
        let value = row
            .to_table_json(ProjectionName::CustomerDaily, 3)
            .expect("json");
        assert_eq!(value["as_of_version"], 3);
        let back =
            ProjectionRow::from_table_json(ProjectionName::CustomerDaily, &value).expect("back");
        assert_eq!(back, row);

        // A row cannot be written into a different projection's table.
        assert!(row
            .to_table_json(ProjectionName::OperatorQueues, 3)
            .is_err());

        // Absent impressions stay absent, never zero.
        let row = ProjectionRow::CustomerDaily(CustomerDailyRow {
            publisher_id: "northstar".into(),
            day: "2026-10-02".into(),
            platform: "applovin_max".into(),
            currency: "USD".into(),
            revenue_micros: 5_150_000_000,
            impressions: None,
            source_days: 1,
            final_days: 1,
            provisional_days: 0,
        });
        let value = row
            .to_table_json(ProjectionName::CustomerDaily, 3)
            .expect("json");
        assert!(value.get("impressions").is_none());
    }

    #[test]
    fn every_projection_row_round_trips_through_its_table_shape() {
        let rows = vec![
            ProjectionRow::CustomerDaily(CustomerDailyRow {
                publisher_id: "northstar".into(),
                day: "2026-10-01".into(),
                platform: "applovin_max".into(),
                currency: "USD".into(),
                revenue_micros: 5_150_000_000,
                impressions: Some(2_500_000),
                source_days: 1,
                final_days: 1,
                provisional_days: 0,
            }),
            ProjectionRow::PlatformDaily(PlatformDailyRow {
                platform: "applovin_max".into(),
                day: "2026-10-01".into(),
                currency: "USD".into(),
                revenue_micros: 5_150_000_000,
                impressions: Some(2_500_000),
                final_accounts: 1,
                provisional_accounts: 0,
            }),
            ProjectionRow::LifecycleCohorts(LifecycleCohortRow {
                cohort_month: "2026-09-01".into(),
                publisher_id: "northstar".into(),
                stage: "qualified".into(),
                accounts: 3,
            }),
            ProjectionRow::OperatorQueues(OperatorQueueRow {
                queue: "incidents".into(),
                object_id: "inc_018".into(),
                publisher_id: "northstar".into(),
                severity: "critical".into(),
                age_seconds: 300,
                owner: Some("maya".into()),
                next_action: "review_incident".into(),
            }),
        ];
        for row in &rows {
            let name = match row {
                ProjectionRow::CustomerDaily(_) => ProjectionName::CustomerDaily,
                ProjectionRow::PlatformDaily(_) => ProjectionName::PlatformDaily,
                ProjectionRow::LifecycleCohorts(_) => ProjectionName::LifecycleCohorts,
                ProjectionRow::OperatorQueues(_) => ProjectionName::OperatorQueues,
            };
            let value = row.to_table_json(name, 4).expect("json");
            assert_eq!(value["as_of_version"], 4);
            let back = ProjectionRow::from_table_json(name, &value).expect("back");
            assert_eq!(&back, row);
        }
    }

    #[tokio::test]
    async fn the_default_rebuild_names_its_batch() {
        let store = known_value_store().await;
        let report = rebuild(&store).await.expect("rebuild");
        assert!(
            report.batch_id.starts_with("rebuild-"),
            "{}",
            report.batch_id
        );
        assert_eq!(report.as_of_version, 1);
    }

    #[test]
    fn operator_queue_sql_aggregates_the_latest_incident_state() {
        let sql = rebuild_sql("db", ProjectionName::OperatorQueues, 1).expect("sql");
        assert!(sql.contains("JSONExtractString(payload_json, 'incident_id')"));
        // The incident rows are aggregated to the latest state per incident
        // (argMax by occurred_at_utc) before the resolved filter, matching
        // the Rust path instead of filtering every historical row.
        assert!(sql.contains("argMax(coalesce(outcome, 'unknown'), occurred_at_utc) AS severity"));
        assert!(sql.contains("max(occurred_at_utc) AS last_occurred"));
        assert!(sql.contains("GROUP BY incident_id"));
        assert!(sql.contains("HAVING incident_id != ''"));
        assert!(sql.contains("WHERE state != 'resolved'"));
    }

    #[test]
    fn projection_selects_name_their_tables_and_published_versions() {
        for name in ProjectionName::ALL {
            let sql = projection_select_sql("db", name).expect("select");
            assert!(sql.contains(&format!("db.{}", name.table_name())), "{sql}");
            assert!(sql.contains("published = 1"), "{sql}");
            assert!(sql.contains("{projection:String}"), "{sql}");
            // No FINAL: it would collapse across versions and hide the
            // published rows behind a newer staged version.
            assert!(!sql.contains("FINAL"), "{sql}");
            let staged = projection_version_select_sql("db", name, 7).expect("select");
            assert!(
                staged.contains(&format!("db.{}", name.table_name())),
                "{staged}"
            );
            assert!(staged.contains("as_of_version = 7"), "{staged}");
        }
    }

    #[test]
    fn data_horizon_prefers_the_newest_received_event() {
        let event = valid_event(EventType::JobRun);
        let horizon = data_horizon(std::slice::from_ref(&event)).expect("horizon");
        assert_eq!(horizon, event.received_at_utc);
        // An empty store falls back to the system clock.
        let now = data_horizon(&[]).expect("now");
        assert!(now.unix_millis() > 0);
    }

    #[test]
    fn projections_carry_money_only_where_money_exists() {
        assert!(ProjectionName::CustomerDaily.carries_money());
        assert!(ProjectionName::PlatformDaily.carries_money());
        assert!(!ProjectionName::LifecycleCohorts.carries_money());
        assert!(!ProjectionName::OperatorQueues.carries_money());
        let totals = ProjectionTotals::of(ProjectionName::OperatorQueues, &[]);
        assert_eq!(totals.revenue_micros, None);
    }
    /// The pre-optimization platform fold: a linear `contains` over the grown
    /// account list per row. Kept verbatim as the reference.
    fn platform_daily_naive(report_days: &[crate::store::ReportDayRow]) -> Vec<PlatformDailyRow> {
        struct Accumulator {
            revenue_micros: i64,
            impressions: Option<u64>,
            final_accounts: Vec<String>,
            provisional_accounts: Vec<String>,
        }
        let mut map: BTreeMap<(String, String, String), Accumulator> = BTreeMap::new();
        for row in report_days {
            let (Some(revenue_micros), Some(currency)) =
                (row.revenue_micros, row.currency.as_deref())
            else {
                continue;
            };
            let key = (row.platform.clone(), row.day.clone(), currency.to_string());
            let accumulator = map.entry(key).or_insert_with(|| Accumulator {
                revenue_micros: 0,
                impressions: Some(0),
                final_accounts: Vec::new(),
                provisional_accounts: Vec::new(),
            });
            accumulator.revenue_micros = accumulator.revenue_micros.saturating_add(revenue_micros);
            accumulator.impressions = match (accumulator.impressions, row.impressions) {
                (Some(sum), Some(value)) => sum.checked_add(value),
                _ => None,
            };
            let accounts = if row.finality == "final" {
                &mut accumulator.final_accounts
            } else if row.finality == "provisional" {
                &mut accumulator.provisional_accounts
            } else {
                continue;
            };
            if !accounts.contains(&row.platform_account_id) {
                accounts.push(row.platform_account_id.clone());
            }
        }
        map.into_iter()
            .map(
                |((platform, day, currency), accumulator)| PlatformDailyRow {
                    platform,
                    day,
                    currency,
                    revenue_micros: accumulator.revenue_micros,
                    impressions: accumulator.impressions,
                    final_accounts: accumulator.final_accounts.len() as u32,
                    provisional_accounts: accumulator.provisional_accounts.len() as u32,
                },
            )
            .collect()
    }

    #[test]
    fn platform_daily_counts_equal_the_naive_linear_scan() {
        // 10,000 rows in one (platform, day, currency) cell: the distinct
        // account count must not depend on the fold used.
        let mut rows = Vec::new();
        for i in 0..10_000 {
            rows.push(crate::store::ReportDayRow {
                publisher_id: format!("p{}", i % 3),
                platform: "applovin_max".to_string(),
                platform_account_id: format!("acc_{i:05}"),
                day: "2026-09-30".to_string(),
                finality: if i % 4 == 0 { "final" } else { "provisional" }.to_string(),
                revenue_micros: Some(1_000),
                impressions: Some(10),
                currency: Some("USD".to_string()),
                country: None,
                ad_unit_id: None,
                format: None,
                source_digest: None,
            });
        }
        // Duplicate accounts and an unknown finality exercise the dedup and
        // skip branches.
        let mut duplicate = rows[0].clone();
        duplicate.day = "2026-09-30".to_string();
        duplicate.finality = "partial".to_string();
        rows.push(duplicate);
        rows.push(rows[1].clone());
        let optimized = compute_platform_daily(&rows);
        let reference = platform_daily_naive(&rows);
        assert_eq!(optimized, reference);
        assert_eq!(optimized.len(), 1);
        assert_eq!(
            optimized[0].final_accounts + optimized[0].provisional_accounts,
            10_000
        );
    }

    #[test]
    fn the_combined_fold_answers_like_both_public_helpers() {
        let mut days = known_value_report_days();
        let mut provisional = days[0].clone();
        provisional.platform_account_id = "max_acct_2".into();
        provisional.finality = "provisional".into();
        days.push(provisional);
        let folded = compute_report_day_projections(&days);
        assert_eq!(folded.customer_daily, compute_customer_daily(&days));
        assert_eq!(folded.platform_daily, compute_platform_daily(&days));
    }
}
