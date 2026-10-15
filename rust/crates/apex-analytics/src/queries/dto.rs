//! Typed read-model DTOs. Every type is `Debug + Clone + Serialize +
//! Deserialize`, tenant-scoped through [`QueryScope`], and carries money as
//! integer micros plus an explicit currency. Missing data is `Option` with a
//! reason recorded in the owning row, never zero.

use crate::envelope::{EventSource, EventType};
use crate::projections::OperatorQueueRow;
use crate::timestamp::UtcMillis;

/// An amount: integer micros plus its explicit ISO-4217 currency.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Money {
    /// Integer micros (1e-6 of one currency unit).
    pub micros: i64,
    /// ISO-4217 code; money is never unlabelled.
    pub currency: String,
}

impl Money {
    /// An amount in `currency`.
    pub fn new(micros: i64, currency: impl Into<String>) -> Self {
        Self {
            micros,
            currency: currency.into(),
        }
    }
}

/// Tenant scope plus the mandatory read context (spec page 84: metric name,
/// unit, population, period and as-of time accompany every read model).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QueryScope {
    /// Tenant the caller is authorized for. Empty means operator-wide (only
    /// operator read models accept that).
    pub publisher_id: String,
    /// Optional platform filter.
    pub platform: Option<String>,
    /// Optional currency filter.
    pub currency: Option<String>,
    /// Optional first day, `YYYY-MM-DD`.
    pub from_day: Option<String>,
    /// Optional last day, `YYYY-MM-DD`.
    pub to_day: Option<String>,
    /// The as-of time the read is evaluated at.
    pub as_of: UtcMillis,
}

impl QueryScope {
    /// Scope for one publisher at an as-of time.
    pub fn new(publisher_id: impl Into<String>, as_of: UtcMillis) -> Self {
        Self {
            publisher_id: publisher_id.into(),
            platform: None,
            currency: None,
            from_day: None,
            to_day: None,
            as_of,
        }
    }

    /// Scope across all publishers, for operator read models only.
    pub fn operator_wide(as_of: UtcMillis) -> Self {
        Self::new("", as_of)
    }

    /// Filter to one platform.
    pub fn with_platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = Some(platform.into());
        self
    }

    /// Filter to one currency.
    pub fn with_currency(mut self, currency: impl Into<String>) -> Self {
        self.currency = Some(currency.into());
        self
    }

    /// Filter to an inclusive day range.
    pub fn with_day_range(
        mut self,
        from_day: impl Into<String>,
        to_day: impl Into<String>,
    ) -> Self {
        self.from_day = Some(from_day.into());
        self.to_day = Some(to_day.into());
        self
    }
}

/// The revenue breakdown dimensions the spec names (page 51: app/ad unit,
/// country and format, "only where source fields exist").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BreakdownDimension {
    /// By platform.
    Platform,
    /// By ad format.
    Format,
    /// By country.
    Country,
    /// By ad unit.
    AdUnit,
}

impl BreakdownDimension {
    /// Stable name of the dimension.
    pub fn as_str(self) -> &'static str {
        match self {
            BreakdownDimension::Platform => "platform",
            BreakdownDimension::Format => "format",
            BreakdownDimension::Country => "country",
            BreakdownDimension::AdUnit => "ad_unit",
        }
    }
}

impl std::fmt::Display for BreakdownDimension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One day of platform revenue for one publisher.
///
/// The DTO deliberately carries no publisher identity, but a row is still
/// per publisher: under an operator-wide scope the same `(day, platform,
/// currency)` appears once per publisher, exactly as the `customer_daily`
/// projection stores it, and totals are never merged across publishers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RevenueDailyRow {
    /// Reporting day `YYYY-MM-DD`.
    pub day: String,
    /// Platform.
    pub platform: String,
    /// Revenue (integer micros plus explicit currency).
    pub revenue: Money,
    /// Impressions when every contributing row supplied them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impressions: Option<u64>,
    /// Contributing source rows.
    pub source_days: u32,
    /// Contributing rows with finality `final`.
    pub final_days: u32,
    /// Contributing rows with finality `provisional`.
    pub provisional_days: u32,
}

/// One value of a revenue breakdown dimension.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RevenueBreakdownRow {
    /// The dimension value (`applovin_max`, `rewarded`, `DE`, `rw_de_01`).
    pub dimension_value: String,
    /// Revenue for this value.
    pub revenue: Money,
    /// Impressions when every contributing row supplied them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impressions: Option<u64>,
    /// Contributing source rows.
    pub rows: u32,
    /// Share of the scope total in basis points; absent when the total is
    /// zero (a share needs a denominator).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_bp: Option<u32>,
}

/// One day of site-wide public page requests.
///
/// The `public_page_requested` family describes visits to the public site,
/// not a tenant's account: the event carries no publisher identity, so these
/// counts are deliberately NOT tenant-scoped. Only aggregate counts leave
/// the store: never rows, never query values, never a visitor identity.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PageRequestsDay {
    /// UTC day `YYYY-MM-DD`.
    pub day: String,
    /// Page requests that day (deduplicated by the stable source/event key).
    pub requests: u64,
    /// Requests answered with exactly HTTP 404 (`/unmatched` routes).
    pub not_found: u64,
    /// Requests per bounded bot classification, sorted by class name; never
    /// a claim about humans.
    pub bot_class_mix: Vec<(String, u64)>,
}

/// One experiment decision as a lift row.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LiftRow {
    /// Experiment identity.
    pub experiment_id: String,
    /// Change reference the decision is about, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_ref: Option<String>,
    /// Observed lift in basis points, when the engine produced one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_lift_bp: Option<i64>,
    /// Billable bound in basis points; present only for a qualifying Keep.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billable_bound_bp: Option<i64>,
    /// Decision outcome (`keep`, `rollback`, `void`, `extend`, `pending`).
    pub outcome: String,
    /// Registered metric the result was measured on; `None` when no
    /// registration event is stored (missing, not assumed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement_source: Option<String>,
    /// Source digest of the measurement, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measurement_version: Option<String>,
    /// When the decision was recorded (`YYYY-MM-DDThh:mm:ss.mmmZ`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<String>,
}

/// Fee line lifecycle state. Draft is not invoiced; invoiced is not
/// collected; pending is not paid (pages 47 and 77).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum FeeState {
    /// Computed with finalized sources, not yet invoiced.
    Estimated,
    /// Computed while the period still has provisional source days.
    Pending,
    /// Held for proof, recheck or dispute.
    Held,
    /// Issued on an invoice.
    Invoiced,
    /// Allocated verified payment received.
    Paid,
    /// Credited.
    Credited,
}

impl FeeState {
    /// Stable lower-case name.
    pub fn as_str(self) -> &'static str {
        match self {
            FeeState::Estimated => "estimated",
            FeeState::Pending => "pending",
            FeeState::Held => "held",
            FeeState::Invoiced => "invoiced",
            FeeState::Paid => "paid",
            FeeState::Credited => "credited",
        }
    }

    /// Parse a stored state name.
    pub fn parse(text: &str) -> Option<Self> {
        [
            FeeState::Estimated,
            FeeState::Pending,
            FeeState::Held,
            FeeState::Invoiced,
            FeeState::Paid,
            FeeState::Credited,
        ]
        .into_iter()
        .find(|state| state.as_str() == text)
    }
}

impl std::fmt::Display for FeeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One fee line with its invoice reference and proof hashes.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FeeRow {
    /// Change the fee is based on.
    pub change_id: String,
    /// Billing period `YYYY-MM`.
    pub period: String,
    /// Current state.
    pub state: FeeState,
    /// The fee amount in micros when it can be computed; `None` while held
    /// without a computable basis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<Money>,
    /// Eligible source amount the fee was computed from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_amount: Option<Money>,
    /// Billable lower bound in basis points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound_bp: Option<i64>,
    /// Agreed rate in basis points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_bp: Option<u16>,
    /// Active billed days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_days: Option<u32>,
    /// Invoice reference when issued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice_ref: Option<String>,
    /// Proof hashes linking the fee to its signed decision.
    pub proof_refs: Vec<String>,
}

/// The fee calculation table of one change (spec page 48): eligible scope
/// revenue, billable lower bound, attributed increment, agreed rate and the
/// lift fee before tax, all reproducible. When the arithmetic cannot be
/// reproduced (for example a rate outside the agreed contract), the derived
/// amounts are `None` and the reason is named in `unavailable`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FeeDetail {
    /// Change identity.
    pub change_id: String,
    /// Billing period `YYYY-MM`.
    pub period: String,
    /// Eligible scope revenue.
    pub eligible_revenue: Money,
    /// Conservative billable bound in basis points (the signed decision).
    pub conservative_bound_bp: i64,
    /// Agreed fee rate in basis points.
    pub agreed_rate_bp: u16,
    /// Attributed increment: R x (1 - 1 / F).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributed_increment: Option<Money>,
    /// Lift fee before tax: attributed increment x agreed rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_before_tax: Option<Money>,
    /// Active billed days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_days: Option<u32>,
    /// Proof hashes of the signed decision.
    pub proof_refs: Vec<String>,
    /// When the calculation was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calculated_at: Option<String>,
    /// Derived parts that could not be reproduced, with the reason.
    pub unavailable: Vec<String>,
}

/// A source with no fresh finalized day.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StaleSource {
    /// Platform.
    pub platform: String,
    /// Platform account.
    pub platform_account_id: String,
    /// Latest finalized source day; `None` when none is stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_final_day: Option<String>,
    /// As-of day minus the latest finalized day; `None` when there is no
    /// finalized day to measure from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_days: Option<i64>,
}

/// Data-quality badge behind every summary: matched sources, gaps, dedup
/// rejects, unmapped rows and stale sources. `None` means the backend cannot
/// report the item; the reason is named in `unavailable`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DataQuality {
    /// Tenant (or operator-wide) scope this badge describes.
    pub publisher_id: String,
    /// As-of time of the read.
    pub as_of: String,
    /// Distinct (platform, account, day) source rows contributing.
    pub matched_source_days: u64,
    /// Days with no stored row inside a requested range; `None` without a
    /// range (there is nothing to be missing from).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_days: Option<u64>,
    /// Provisional source rows.
    pub provisional_days: u64,
    /// Raw duplicate event rows pending merge: stored rows beyond one per
    /// `(source, event_id)` key that no merge has collapsed yet. The memory
    /// store collapses replays at insert time, so it reports a real zero;
    /// ClickHouse reports the physical surplus until a merge (or an
    /// `OPTIMIZE ... FINAL`) collapses it; `None` means the backend cannot
    /// report the count at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedup_rejects: Option<u64>,
    /// Source rows that carry no amount or no currency: they cannot enter a
    /// money total and are counted here instead of being zeroed.
    pub unmapped_rows: u64,
    /// Sources past the freshness threshold.
    pub stale_sources: Vec<StaleSource>,
    /// Names of metrics this backend cannot report, with the reason.
    pub unavailable: Vec<String>,
}

/// One event on an object timeline (merged by occurrence time, with the
/// received time kept available for delayed imports, page 58).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ObjectTimelineEntry {
    /// When the fact happened.
    pub occurred_at_utc: String,
    /// When the store received it.
    pub received_at_utc: String,
    /// Event type.
    pub event_type: EventType,
    /// Producing boundary.
    pub source: EventSource,
    /// Bounded outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    /// Bounded reason code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    /// Workflow correlation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Direct cause.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
}

/// The five operator queues.
///
/// The queues are as of the covered data: membership and ages are measured
/// against the covered-data horizon (the newest source event time the
/// snapshot covers), never against the wall clock or the requested as-of
/// time.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OperatorQueues {
    /// As-of time of the read. The covered-data horizon the queue ages are
    /// measured from may be earlier; it is the snapshot's `source_event_max`.
    pub as_of: String,
    /// Unresolved incidents.
    pub incidents: Vec<OperatorQueueRow>,
    /// Experiments with a completed window and no decision.
    pub due_decisions: Vec<OperatorQueueRow>,
    /// Accounts without movement past the staleness threshold.
    pub stale_accounts: Vec<OperatorQueueRow>,
    /// Job runs with failed or missed outcomes.
    pub overdue_jobs: Vec<OperatorQueueRow>,
    /// Held fees without a later release.
    pub held_fees: Vec<OperatorQueueRow>,
}

/// Freshness verdict of one source.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessStatus {
    /// The latest final day is within the threshold.
    Fresh,
    /// The latest final day is older than the threshold.
    Stale,
    /// No finalized day is stored for this source.
    MissingData,
}

impl FreshnessStatus {
    /// Stable lower-case name.
    pub fn as_str(self) -> &'static str {
        match self {
            FreshnessStatus::Fresh => "fresh",
            FreshnessStatus::Stale => "stale",
            FreshnessStatus::MissingData => "missing_data",
        }
    }
}

impl std::fmt::Display for FreshnessStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Freshness of one publisher/platform/account source: as-of day minus the
/// latest finalized source day.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IngestionHealthRow {
    /// Tenant.
    pub publisher_id: String,
    /// Platform.
    pub platform: String,
    /// Platform account.
    pub platform_account_id: String,
    /// Latest finalized source day; `None` when none is stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_final_day: Option<String>,
    /// As-of day minus the latest final day; `None` when there is no final
    /// day to measure from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_days: Option<i64>,
    /// Freshness verdict.
    pub status: FreshnessStatus,
}

/// One request cohort in the growth funnel.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GrowthCohortRow {
    /// Cohort month `YYYY-MM`.
    pub cohort_month: String,
    /// Distinct accepted lead identities (retry duplicates removed).
    pub accepted: u64,
    /// Distinct resolved account identities in the cohort.
    pub accepted_accounts: u64,
    /// Qualified accounts.
    pub qualified: u64,
    /// Accounts with validated read access.
    pub connected: u64,
    /// Accounts with a delivered first audit.
    pub audit_delivered: u64,
    /// Accounts with confirmed paid service.
    pub paid: u64,
    /// Whether the cohort cleared the 30-day follow-up horizon at the as-of.
    pub mature: bool,
    /// Qualification rate in basis points of resolved accounts (`None`
    /// without a denominator).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualification_rate_bp: Option<u32>,
}

/// One stage of the activation funnel.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FunnelStageRow {
    /// Stage name (`requested`, `qualified`, `connected`, `audit_delivered`,
    /// `paid`).
    pub stage: String,
    /// Accounts that reached this stage or a later one.
    pub accounts: u64,
}

/// The activation funnel of one cohort.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActivationFunnel {
    /// Cohort month `YYYY-MM`.
    pub cohort_month: String,
    /// Whether the cohort cleared the 30-day follow-up horizon.
    pub mature: bool,
    /// Stage counts, in funnel order.
    pub stages: Vec<FunnelStageRow>,
}

/// One experiment state in the fleet landscape.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExperimentLandscapeRow {
    /// Lifecycle state.
    pub state: String,
    /// Experiments in this state.
    pub count: u64,
    /// Reason of the oldest waiting experiment, when one is recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_waiting_reason: Option<String>,
    /// Age of the oldest experiment in this state, in days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_age_days: Option<i64>,
}

/// Effectiveness of a mature experiment start cohort.
///
/// Rates use explicit denominators and the spec's rule: pending and
/// in-flight tests cannot inflate completion rates (page 87, Cohort maturity
/// and outcomes).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExperimentEffectiveness {
    /// Cohort label (`YYYY-MM`).
    pub cohort_label: String,
    /// Whether the cohort cleared the maturity horizon.
    pub mature: bool,
    /// Started experiments.
    pub started: u64,
    /// Valid settled decisions (Kept + Rolled back): the rate denominator.
    pub valid_settled: u64,
    /// Kept decisions.
    pub kept: u64,
    /// Rolled back decisions.
    pub rolled_back: u64,
    /// Void decisions.
    pub voided: u64,
    /// Pending decisions.
    pub pending: u64,
    /// Decisions whose outcome label is unknown.
    pub unknown: u64,
    /// Started experiments with no decision yet.
    pub in_flight: u64,
    /// Kept / valid settled, in basis points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_rate_valid_bp: Option<u32>,
    /// Kept / started, in basis points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_rate_started_bp: Option<u32>,
    /// Median completion time (started to decided) in days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_p50_days: Option<i64>,
    /// P95 completion time in days (nearest rank).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_p95_days: Option<i64>,
    /// Populations kept outside the rate denominators, named so nothing is
    /// silently dropped.
    pub excluded_from_denominators: Vec<String>,
}
