//! The read layer: analytics read models, durable runner files and the
//! verified-operation evidence.
//!
//! Two honest modes exist and both serve the same page code:
//!
//! - [`ControlData::Fixtures`] carries the deterministic dataset the
//!   fixture renderer and the tests use. It never invents a live claim: it
//!   is the "illustrative data" dataset of the design document, marked
//!   `ILLUSTRATIVE DATA` on every page.
//! - [`ControlData::Live`] reads the analytics store through
//!   [`apex_analytics::AnalyticsReader`] and the durable files the runner
//!   keeps. When the store or a file is absent or unreachable, every read
//!   returns [`Unavailable`] with the reason, and the page renders the
//!   honest not-available state (page 97: "No healthy or zero-error claim
//!   can be made from missing telemetry"). Nothing is fabricated.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use apex_analytics::queries::{
    ActivationFunnel, AnalyticsReader, ClickHouseReader, DataQuality, ExperimentEffectiveness,
    ExperimentLandscapeRow, FeeDetail, FeeRow, GrowthCohortRow, IngestionHealthRow, LiftRow, Money,
    ObjectTimelineEntry, OperatorQueues, QueryScope,
};
use apex_analytics::{ClickHouseConfig, ClickHouseStore, UtcMillis};
use apex_connectors::admob::ADMOB_ENDPOINTS;
use apex_connectors::levelplay::LEVELPLAY_ENDPOINTS;
use apex_connectors::max::MAX_ENDPOINTS;
use apex_core::endpoint::EndpointMeta;
use apex_proof::{Ledger, VerifyingKey};

use crate::fixtures::{CustomerRow, DemoData};

/// Data that could not be read, with the reason. The reason is shown to the
/// operator; it never contains credentials or query bodies.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{reason}")]
pub struct Unavailable {
    /// Why the read is not available.
    pub reason: String,
}

impl Unavailable {
    /// A named absence.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

/// Read cap for one verified ledger file: larger files are refused before
/// any bytes are read.
pub const MAX_LEDGER_BYTES: u64 = 32 * 1024 * 1024;

/// One verified publisher ledger summary (customers list/detail).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPublisher {
    /// Publisher identity (the ledger file stem).
    pub publisher: String,
    /// Number of records that passed chain verification.
    pub record_count: u64,
    /// Timestamp of the last verified record.
    pub last_record_at_ms: Option<i64>,
    /// Platforms present in verified records, sorted.
    pub platforms: Vec<String>,
    /// Verified record count per platform, in the same order as `platforms`.
    pub platform_counts: Vec<(String, u64)>,
    /// Why this ledger is not verified, when it is present but refused.
    pub error: Option<String>,
}

/// One verified record as the customer detail shows it (bounded fields only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRecordView {
    /// Chain sequence.
    pub seq: u64,
    /// Record time (Unix milliseconds).
    pub at_ms: i64,
    /// Record kind (`changeApplied`, `experimentStarted`, ...).
    pub kind: String,
    /// Platform wire name, when the record has one.
    pub platform: Option<String>,
    /// Bounded, non-secret detail line.
    pub detail: String,
}

/// One platform snapshot under the runner snapshot root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotPlatformView {
    /// Platform wire name (the file stem).
    pub platform: String,
    /// `fetchedAtUnixMs` from the snapshot, when parsed.
    pub fetched_at_ms: Option<i64>,
    /// Age in milliseconds at the requested as-of time.
    pub age_ms: Option<i64>,
    /// Ad units in the parsed snapshot.
    pub ad_units: Option<usize>,
    /// Why the snapshot could not be read/parsed.
    pub error: Option<String>,
}

/// The snapshots held for one publisher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublisherSnapshots {
    /// Publisher identity (the directory name).
    pub publisher: String,
    /// Per-platform snapshots, sorted by platform.
    pub platforms: Vec<SnapshotPlatformView>,
}

/// One live audit publisher row: the snapshot inputs the runner left and the
/// eligibility that follows from them, without claiming a pass or a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditPublisherView {
    /// Publisher identity.
    pub publisher: String,
    /// Per-platform snapshot reads.
    pub platforms: Vec<SnapshotPlatformView>,
    /// `Eligible inputs`, `Incomplete inputs` or `Inputs unavailable`.
    pub eligibility: String,
    /// Why eligibility is not established, when it is not.
    pub eligibility_reason: Option<String>,
}

/// The sync state of the copied ledger data, read from the sync-timer marker
/// file (`APEX_CONTROL_SYNC_MARKER`). The three states are exhaustive and
/// honest: recorded, not recorded (no path configured or no file yet), and
/// unreadable (present but not a trustworthy marker). A malformed marker is
/// never a panic and never a fabricated age.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerCopy {
    /// The marker records when the copy was last synced.
    Recorded {
        /// The sync time, unix milliseconds.
        synced_at_ms: i64,
    },
    /// No marker configured or present: the honest "sync age not recorded".
    NotRecorded,
    /// The marker exists but cannot be trusted; the reason is shown.
    Unreadable {
        /// Why the marker cannot be read.
        reason: String,
    },
}

/// The closed set of marker sources the sync timer may record.
const SYNC_MARKER_SOURCES: [&str; 3] = ["ledger", "invoices", "leads"];

/// The marker file shape: `{"synced_at_unix_ms": ..., "source": "..."}`.
#[derive(Debug, serde::Deserialize)]
struct SyncMarkerFile {
    synced_at_unix_ms: i64,
    source: String,
}

/// Read the sync marker. `None` (env unset) and an absent file are the
/// honest not-recorded state; a file that does not parse, carries an unknown
/// source, or records a time ahead of `as_of_ms` is a typed unavailable with
/// the reason, never a panic and never a guess.
pub fn read_sync_marker(path: Option<&Path>, as_of_ms: i64) -> LedgerCopy {
    let Some(path) = path else {
        return LedgerCopy::NotRecorded;
    };
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return LedgerCopy::NotRecorded
        }
        Err(error) => {
            return LedgerCopy::Unreadable {
                reason: format!("the sync marker could not be read: {error}"),
            }
        }
    };
    let marker: SyncMarkerFile = match serde_json::from_str(&raw) {
        Ok(marker) => marker,
        Err(error) => {
            return LedgerCopy::Unreadable {
                reason: format!("the sync marker did not parse: {error}"),
            }
        }
    };
    if !SYNC_MARKER_SOURCES.contains(&marker.source.as_str()) {
        return LedgerCopy::Unreadable {
            reason: format!(
                "the sync marker names an unknown source '{source}'",
                source = marker.source
            ),
        };
    }
    if marker.synced_at_unix_ms > as_of_ms {
        return LedgerCopy::Unreadable {
            reason: "the sync marker records a time in the future".to_string(),
        };
    }
    LedgerCopy::Recorded {
        synced_at_ms: marker.synced_at_unix_ms,
    }
}

impl LedgerCopy {
    /// The one factual system-page line: the UTC sync time and the age in
    /// whole hours for a recorded marker, the honest absent state otherwise.
    pub fn system_line(&self, as_of_ms: i64) -> String {
        match self {
            LedgerCopy::Recorded { synced_at_ms } => format!(
                "Ledger copy synced: {} (age {} h)",
                apex_ui::format::utc_datetime(*synced_at_ms),
                as_of_ms.saturating_sub(*synced_at_ms).max(0) / 3_600_000
            ),
            LedgerCopy::NotRecorded => "Ledger copy synced: not recorded".to_string(),
            LedgerCopy::Unreadable { reason } => {
                format!("Ledger copy synced: not recorded ({reason})")
            }
        }
    }
}

/// The analytics endpoint bound to the configured store.
#[derive(Debug, Clone)]
pub struct AnalyticsSource {
    reader: ClickHouseReader,
}

impl AnalyticsSource {
    /// Build from an optional store configuration. `None` means no analytics
    /// store is configured; the reason string then explains the absence.
    pub fn build(config: Option<&ClickHouseConfig>) -> Result<Option<Self>, Unavailable> {
        let Some(config) = config else {
            return Ok(None);
        };
        let store = ClickHouseStore::new(config.clone()).map_err(|error| {
            Unavailable::new(format!(
                "the analytics store configuration is invalid: {error}"
            ))
        })?;
        Ok(Some(Self {
            reader: ClickHouseReader::new(store),
        }))
    }

    /// The shared scope used for operator-wide reads.
    fn scope(as_of: UtcMillis) -> QueryScope {
        QueryScope::operator_wide(as_of)
    }

    async fn queues(&self, as_of: UtcMillis) -> Result<OperatorQueues, Unavailable> {
        self.reader
            .operator_queues(&Self::scope(as_of))
            .await
            .map_err(|error| Unavailable::new(format!("the analytics store read failed: {error}")))
    }

    async fn ingestion_health(
        &self,
        as_of: UtcMillis,
    ) -> Result<Vec<IngestionHealthRow>, Unavailable> {
        self.reader
            .ingestion_health(&Self::scope(as_of))
            .await
            .map_err(|error| Unavailable::new(format!("the analytics store read failed: {error}")))
    }

    async fn data_quality(&self, as_of: UtcMillis) -> Result<DataQuality, Unavailable> {
        self.reader
            .data_quality(&Self::scope(as_of))
            .await
            .map_err(|error| Unavailable::new(format!("the analytics store read failed: {error}")))
    }

    async fn growth_cohorts(&self, as_of: UtcMillis) -> Result<Vec<GrowthCohortRow>, Unavailable> {
        self.reader
            .growth_cohorts(&Self::scope(as_of))
            .await
            .map_err(|error| Unavailable::new(format!("the analytics store read failed: {error}")))
    }

    async fn activation_funnel(
        &self,
        as_of: UtcMillis,
        cohort_month: &str,
    ) -> Result<ActivationFunnel, Unavailable> {
        self.reader
            .activation_funnel(&Self::scope(as_of), cohort_month)
            .await
            .map_err(|error| Unavailable::new(format!("the analytics store read failed: {error}")))
    }

    async fn experiment_landscape(
        &self,
        as_of: UtcMillis,
    ) -> Result<Vec<ExperimentLandscapeRow>, Unavailable> {
        self.reader
            .experiment_landscape(&Self::scope(as_of))
            .await
            .map_err(|error| Unavailable::new(format!("the analytics store read failed: {error}")))
    }

    async fn experiment_effectiveness(
        &self,
        as_of: UtcMillis,
        cohort_month: &str,
    ) -> Result<ExperimentEffectiveness, Unavailable> {
        self.reader
            .experiment_effectiveness(&Self::scope(as_of), cohort_month)
            .await
            .map_err(|error| Unavailable::new(format!("the analytics store read failed: {error}")))
    }

    async fn fee_rows(&self, as_of: UtcMillis) -> Result<Vec<FeeRow>, Unavailable> {
        self.reader
            .fee_rows(&Self::scope(as_of))
            .await
            .map_err(|error| Unavailable::new(format!("the analytics store read failed: {error}")))
    }

    async fn fee_detail(
        &self,
        as_of: UtcMillis,
        change_id: &str,
    ) -> Result<Option<FeeDetail>, Unavailable> {
        self.reader
            .fee_detail(&Self::scope(as_of), change_id)
            .await
            .map_err(|error| Unavailable::new(format!("the analytics store read failed: {error}")))
    }

    async fn lift_rows(&self, as_of: UtcMillis) -> Result<Vec<LiftRow>, Unavailable> {
        self.reader
            .lift_rows(&Self::scope(as_of))
            .await
            .map_err(|error| Unavailable::new(format!("the analytics store read failed: {error}")))
    }

    async fn object_timeline(
        &self,
        as_of: UtcMillis,
        object_type: &str,
        object_id: &str,
    ) -> Result<Vec<ObjectTimelineEntry>, Unavailable> {
        self.reader
            .object_timeline(&Self::scope(as_of), object_type, object_id)
            .await
            .map_err(|error| Unavailable::new(format!("the analytics store read failed: {error}")))
    }

    /// The site-request total over the last 30 calendar days (including the
    /// as-of day), summed from the per-day edge access-log counts.
    async fn site_requests_last_30_days(&self, as_of: UtcMillis) -> Result<u64, Unavailable> {
        let to_day = as_of.to_day_string();
        let from = UtcMillis::from_unix_millis(
            as_of
                .unix_millis()
                .saturating_sub(29 * apex_analytics::timestamp::MILLIS_PER_DAY),
        );
        let days = self
            .reader
            .page_requests_daily(&Self::scope(as_of), &from.to_day_string(), &to_day)
            .await
            .map_err(|error| {
                Unavailable::new(format!("the analytics store read failed: {error}"))
            })?;
        Ok(days.iter().map(|day| day.requests).sum())
    }
}

/// One experiment read from a durable runner ledger line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableExperiment {
    /// Publisher whose ledger carries the record.
    pub publisher: String,
    /// Experiment identity.
    pub experiment_id: String,
    /// Human name from the registered spec, when present.
    pub name: Option<String>,
    /// Registered metric, when present.
    pub metric: Option<String>,
    /// Registered change description, when present.
    pub change_description: Option<String>,
    /// When the experiment started (Unix milliseconds).
    pub started_at_ms: Option<i64>,
    /// Latest observed platform state (`running`, `stopped`, `completed`).
    pub state: Option<String>,
    /// Latest observed lift in basis points.
    pub lift_bp: Option<i64>,
    /// Latest observed confidence in basis points.
    pub confidence: Option<i64>,
    /// When the latest observation was recorded.
    pub observed_at_ms: Option<i64>,
}

/// One invoice line read from the durable register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceLineRecord {
    /// Line sequence in the invoice.
    pub seq: u64,
    /// Experiment or change the line bills.
    pub experiment_id: String,
    /// Proven lift in micros.
    pub proven_lift: i64,
    /// Agreed rate in basis points.
    pub rate_bp: u16,
    /// Net fee in micros.
    pub fee_micros: i64,
    /// VAT in micros.
    pub vat_micros: i64,
    /// Gross in micros.
    pub gross_micros: i64,
}

/// One invoice read from the durable register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceRecord {
    /// Publisher.
    pub publisher_id: String,
    /// Billing period `YYYY-MM`.
    pub period: String,
    /// Period start (inclusive).
    pub start_unix_ms: i64,
    /// Period end (exclusive).
    pub end_unix_ms: i64,
    /// Lines.
    pub line_items: Vec<InvoiceLineRecord>,
    /// Net total in micros.
    pub total_micros: i64,
    /// VAT rate in basis points.
    pub vat_rate_bp: u16,
    /// VAT total in micros.
    pub vat_micros: i64,
    /// Gross total in micros.
    pub gross_micros: i64,
}

impl InvoiceRecord {
    /// The invoice identity used across pages: `INV-<period>-<publisher>`.
    pub fn invoice_id(period: &str, publisher: &str) -> String {
        format!("INV-{period}-{publisher}")
    }

    /// This invoice's identity.
    pub fn id(&self) -> String {
        Self::invoice_id(&self.period, &self.publisher_id)
    }
}

/// The default number of days the live ingestion coverage matrix spans.
pub const COVERAGE_WINDOW_DAYS: u32 = 14;
/// The longest accepted coverage window.
pub const MAX_COVERAGE_WINDOW_DAYS: u32 = 90;
/// The currency the control host's micros-denominated sources are rendered
/// in. Neither the verified ledger fee records nor the invoice register carry
/// a currency field, so the operator plane states the deployment's currency
/// rather than guessing per row.
pub const CONTROL_CURRENCY: &str = "USD";

/// The record kinds that count as audit evidence in the coverage matrix: the
/// signed engine output (`liftRecord`), the billing proof and the rollback.
/// The ledger has no dedicated "audit" record type, so the audit column
/// counts these verifiable evidence records explicitly.
pub fn is_audit_record_kind(kind: &str) -> bool {
    matches!(kind, "liftRecord" | "billingProof" | "rollbackPerformed")
}

/// One day cell of the live ingestion coverage matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageDayCell {
    /// Calendar day `YYYY-MM-DD` (UTC).
    pub day: String,
    /// Verified ledger records observed for the row on this day.
    pub records: u64,
    /// Of those, the audit-evidence records (see [`is_audit_record_kind`]).
    pub audits: u64,
    /// The latest verified record time on this day.
    pub last_verified_at_ms: Option<i64>,
}

/// One publisher/platform row of the live ingestion coverage matrix, built
/// from the verified ledgers and the runner snapshot files the control host
/// already reads. It never claims a finalized reporting day: it reports what
/// was observed and says so when a day carries no data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoveragePlatformRow {
    /// Publisher identity.
    pub publisher: String,
    /// Platform wire name, or `-` for platform-agnostic records.
    pub platform: String,
    /// One cell per day in the window, in ascending day order.
    pub days: Vec<CoverageDayCell>,
    /// Verified ledger records in the window.
    pub total_records: u64,
    /// Audit-evidence records in the window.
    pub total_audits: u64,
    /// The latest verified ledger record time in the window.
    pub last_verified_at_ms: Option<i64>,
    /// Whether a parseable snapshot is present for the row.
    pub snapshot_present: bool,
    /// `fetchedAtUnixMs` of the snapshot, when present.
    pub snapshot_fetched_at_ms: Option<i64>,
    /// Snapshot age at the as-of time, when present.
    pub snapshot_age_ms: Option<i64>,
    /// Ad units in the snapshot, when present.
    pub snapshot_ad_units: Option<usize>,
    /// Why the snapshot could not be read/parsed (including an absent root).
    pub snapshot_error: Option<String>,
}

/// One registered decision check: the requirement, the observed value from
/// the verified records and the verdict. It is shown on the experiment fleet
/// and detail pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionCheck {
    /// Check name.
    pub check: String,
    /// The registered requirement.
    pub registered: String,
    /// The observed value from the verified ledger.
    pub observed: String,
    /// `Pass`, `Fail`, `Pending` or `Not settled`.
    pub verdict: String,
}

/// One month of the live service economics, computed from the verified ledger
/// fee records plus the invoice register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveEconomicsMonth {
    /// Month label `YYYY-MM`.
    pub month: String,
    /// Currency the micros are rendered in (the deployment currency).
    pub currency: String,
    /// Net invoiced fees (`totalMicros` of the month's invoices).
    pub net_micros: i64,
    /// Gross invoiced fees (`grossMicros`, net plus VAT).
    pub gross_micros: i64,
    /// Credit notes; `None` because neither source carries a credit record.
    pub credits_micros: Option<i64>,
    /// Verified ledger fee micros (`billingProof.feeMicros`).
    pub ledger_fee_micros: i64,
    /// Number of verified ledger fee records.
    pub ledger_fee_records: u64,
    /// Number of invoice register lines in the month.
    pub invoice_records: u64,
}

impl LiveEconomicsMonth {
    /// An empty month: no verified fee record and no invoice line.
    fn empty(month: String) -> Self {
        Self {
            month,
            currency: CONTROL_CURRENCY.to_string(),
            net_micros: 0,
            gross_micros: 0,
            credits_micros: None,
            ledger_fee_micros: 0,
            ledger_fee_records: 0,
            invoice_records: 0,
        }
    }

    /// True when neither source carried a record for this month.
    pub fn is_empty(&self) -> bool {
        self.ledger_fee_records == 0 && self.invoice_records == 0
    }
}

/// One row of the invoice payment timeline. Every payment stage is an
/// `Option`: `None` means the stage is genuinely not recorded by any source,
/// which renders as "Not recorded" rather than a fabricated pending state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoicePaymentRow {
    /// Event time, when the row comes from a time-stamped record.
    pub at_ms: Option<i64>,
    /// Event description.
    pub event_code: String,
    /// Masked reference (invoice id or experiment id); never a secret.
    pub reference_masked: Option<String>,
    /// Amount in micros, when the source carries one.
    pub amount_micros: Option<i64>,
    /// Currency of the amount.
    pub currency: String,
    /// Payment received stage, when a payment record exists.
    pub received: Option<bool>,
    /// HMAC verification stage.
    pub hmac_verified: Option<bool>,
    /// Deduplication stage.
    pub deduplicated: Option<bool>,
    /// Reconciliation stage.
    pub reconciled: Option<bool>,
    /// Allocation stage.
    pub allocated: Option<bool>,
    /// Why a stage is not recorded.
    pub note: Option<String>,
}

impl From<&crate::fixtures::PaymentEvent> for InvoicePaymentRow {
    fn from(payment: &crate::fixtures::PaymentEvent) -> Self {
        Self {
            at_ms: Some(payment.at_ms),
            event_code: payment.event_code.clone(),
            reference_masked: Some(payment.psp_reference_masked.clone()),
            amount_micros: Some(payment.amount.micros),
            currency: payment.amount.currency.clone(),
            received: Some(payment.received),
            hmac_verified: Some(payment.hmac_verified),
            deduplicated: Some(payment.deduplicated),
            reconciled: Some(payment.reconciled),
            allocated: Some(payment.allocated),
            note: None,
        }
    }
}

/// The payment timeline of one invoice: the register-derived rows plus any
/// verified ledger references, and an explicit row when no payment or
/// reconciliation record exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoicePayments {
    /// Invoice identity.
    pub invoice_id: String,
    /// The timeline rows, ascending by event time (undated rows last).
    pub rows: Vec<InvoicePaymentRow>,
    /// Why the verified ledger could not be read, when it could not.
    pub ledger_note: Option<String>,
}

/// The read-only durable files the runner keeps.
#[derive(Debug, Clone)]
pub struct DurableReader {
    /// Directory of `<publisher>.jsonl` verified ledgers.
    pub ledger_root: PathBuf,
    /// Verified public keys, keyed by publisher.
    pub ledger_keys: std::collections::HashMap<String, VerifyingKey>,
    /// Runner snapshot root (`<publisher>/<platform>.json`).
    pub snapshot_root: PathBuf,
    /// Invoice register JSONL path.
    pub invoice_register: PathBuf,
    /// Parsed ledgers keyed by path, reused while the file identity (length,
    /// mtime and on Unix ctime and inode) is unchanged, so repeated page
    /// reads do not re-parse the whole directory.
    cache: Arc<Mutex<HashMap<PathBuf, CachedLedger>>>,
    /// The parsed invoice register, reused while its file identity is
    /// unchanged. One finance page can read the register several times (one
    /// form per invoice capability), and the file rarely changes between the
    /// monthly syncs.
    invoice_cache: Arc<Mutex<Option<CachedInvoices>>>,
    /// The merged experiment projection, reused while the directory scan's
    /// per-file identities are unchanged. Page reads used to replay every
    /// cached ledger line on every request (20k records at the reference
    /// size) even though no file had changed.
    experiments_cache: Arc<Mutex<Option<CachedExperiments>>>,
    #[cfg(test)]
    invoice_parses: Arc<AtomicU64>,
    #[cfg(test)]
    experiment_builds: Arc<AtomicU64>,
    #[cfg(test)]
    ledger_stats: Arc<AtomicU64>,
}

/// One parsed publisher ledger: the publisher name and its parsed lines.
type LedgerLines = (String, Arc<Vec<serde_json::Value>>);

/// The identity of one ledger-directory scan: every path still present with
/// its hardened file stamp. Appending, rewriting or removing any ledger
/// changes this key, so an external write invalidates the projection.
type LedgerStamp = Vec<(PathBuf, FileStamp)>;

/// The result of one directory scan: the cached lines plus the identity that
/// the derived caches key on.
struct LedgerScan {
    files: Vec<LedgerLines>,
    stamp: LedgerStamp,
}

/// A durable file's cache identity: byte length, modification time (`None`
/// when the filesystem does not expose one) and, on Unix, the ctime and
/// inode. An unprivileged writer cannot restore the ctime, so a same-length
/// rewrite that puts the mtime back (`touch -t`) still invalidates the entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileStamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    ctime: i64,
    #[cfg(unix)]
    ctime_nsec: i64,
    #[cfg(unix)]
    ino: u64,
}

impl FileStamp {
    /// The stamp of `metadata`.
    pub(crate) fn of(metadata: &std::fs::Metadata) -> Self {
        let len = metadata.len();
        let modified = metadata.modified().ok();
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            Self {
                len,
                modified,
                ctime: metadata.ctime(),
                ctime_nsec: metadata.ctime_nsec(),
                ino: metadata.ino(),
            }
        }
        #[cfg(not(unix))]
        {
            Self { len, modified }
        }
    }
}

/// One cached ledger parse.
#[derive(Debug, Clone)]
struct CachedLedger {
    stamp: FileStamp,
    lines: Arc<Vec<serde_json::Value>>,
}

/// One ledger file seen during a scan, with its verification result.
struct ScannedLedger {
    publisher: String,
    path: PathBuf,
    stamp: Option<FileStamp>,
    lines: Result<Arc<Vec<serde_json::Value>>, String>,
}

/// The subset of a runner snapshot the console reads. Unknown fields are
/// ignored; a file without the two fields is refused as malformed.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotFile {
    fetched_at_unix_ms: i64,
    ad_units: Vec<serde_json::Value>,
}

/// One cached invoice-register parse.
#[derive(Debug, Clone)]
struct CachedInvoices {
    stamp: FileStamp,
    invoices: Arc<Vec<InvoiceRecord>>,
}

/// One cached experiment projection.
#[derive(Debug, Clone)]
struct CachedExperiments {
    stamp: LedgerStamp,
    experiments: Arc<Vec<DurableExperiment>>,
}

impl DurableReader {
    /// A reader over the given roots.
    pub fn new(ledger_root: impl Into<PathBuf>, invoice_register: impl Into<PathBuf>) -> Self {
        Self {
            ledger_root: ledger_root.into(),
            ledger_keys: std::collections::HashMap::new(),
            snapshot_root: PathBuf::from(".apex/snapshots"),
            invoice_register: invoice_register.into(),
            cache: Arc::new(Mutex::new(HashMap::new())),
            invoice_cache: Arc::new(Mutex::new(None)),
            experiments_cache: Arc::new(Mutex::new(None)),
            #[cfg(test)]
            invoice_parses: Arc::new(AtomicU64::new(0)),
            #[cfg(test)]
            experiment_builds: Arc::new(AtomicU64::new(0)),
            #[cfg(test)]
            ledger_stats: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Attach the verified public-key allow-list. A publisher without a key
    /// is reported unverified instead of being read.
    #[must_use]
    pub fn with_ledger_keys(
        mut self,
        keys: std::collections::HashMap<String, VerifyingKey>,
    ) -> Self {
        self.ledger_keys = keys;
        self
    }

    /// Attach the runner snapshot root.
    #[must_use]
    pub fn with_snapshot_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.snapshot_root = root.into();
        self
    }

    /// The number of invoice-register parses this reader performed (test seam
    /// proving the cache is hit; a fresh reader counts from zero).
    #[cfg(test)]
    pub(crate) fn invoice_parse_count(&self) -> u64 {
        self.invoice_parses.load(Ordering::SeqCst)
    }

    /// The number of cached ledger parses (test seam proving removed files
    /// are pruned).
    #[cfg(test)]
    pub(crate) fn ledger_cache_len(&self) -> usize {
        self.cache.lock().map(|cache| cache.len()).unwrap_or(0)
    }

    /// The number of experiment projections this reader built (test seam
    /// proving a cached scan does not replay the records).
    #[cfg(test)]
    pub(crate) fn experiment_build_count(&self) -> u64 {
        self.experiment_builds.load(Ordering::SeqCst)
    }

    /// The number of ledger file stamps taken (one `stat` per present
    /// ledger), the per-request read cost the cache cannot remove: external
    /// writes must be observed.
    #[cfg(test)]
    pub(crate) fn ledger_stat_count(&self) -> u64 {
        self.ledger_stats.load(Ordering::SeqCst)
    }

    /// Scan the ledger directory once. Each publisher's lines are verified
    /// against its configured public key; a publisher without a key or with a
    /// chain that does not verify is carried as a typed error, never read as
    /// unverified data.
    fn ledger_scan(&self) -> Result<Vec<ScannedLedger>, Unavailable> {
        let entries = match std::fs::read_dir(&self.ledger_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Unavailable::new(format!(
                    "no ledger root is present at {}",
                    self.ledger_root.display()
                )))
            }
            Err(error) => {
                return Err(Unavailable::new(format!(
                    "the ledger root {} could not be read: {error}",
                    self.ledger_root.display()
                )))
            }
        };
        let mut ledgers = Vec::new();
        // Files still present on this scan. Entries for removed ledgers are
        // pruned below, so a rotating ledger directory cannot grow the cache
        // without bound.
        let mut seen: HashSet<PathBuf> = HashSet::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                Unavailable::new(format!(
                    "the ledger root {} could not be read: {error}",
                    self.ledger_root.display()
                ))
            })?;
            let name = entry.file_name().to_string_lossy().to_string();
            let Some(publisher) = name.strip_suffix(".jsonl") else {
                continue;
            };
            let path = entry.path();
            seen.insert(path.clone());
            if !is_valid_publisher(publisher) {
                ledgers.push(ScannedLedger {
                    publisher: publisher.to_string(),
                    path: path.clone(),
                    stamp: None,
                    lines: Err(
                        "the file name is not a valid publisher id (^[A-Za-z0-9_-]{1,64}$)"
                            .to_string(),
                    ),
                });
                continue;
            }
            let metadata = match std::fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    ledgers.push(ScannedLedger {
                        publisher: publisher.to_string(),
                        path: path.clone(),
                        stamp: None,
                        lines: Err(format!("the ledger could not be read: {error}")),
                    });
                    continue;
                }
            };
            #[cfg(test)]
            self.ledger_stats.fetch_add(1, Ordering::SeqCst);
            let file_stamp = FileStamp::of(&metadata);
            let cached = {
                let cache = self
                    .cache
                    .lock()
                    .map_err(|_| Unavailable::new("ledger cache lock poisoned"))?;
                cache
                    .get(&path)
                    .filter(|cached| cached.stamp == file_stamp)
                    .map(|cached| cached.lines.clone())
            };
            let lines = match cached {
                Some(lines) => Ok(lines),
                None => self.verify_ledger(publisher, &path, &metadata, file_stamp),
            };
            ledgers.push(ScannedLedger {
                publisher: publisher.to_string(),
                path,
                stamp: Some(file_stamp),
                lines,
            });
        }
        {
            let mut cache = self
                .cache
                .lock()
                .map_err(|_| Unavailable::new("ledger cache lock poisoned"))?;
            cache.retain(|path, _| seen.contains(path));
        }
        Ok(ledgers)
    }

    /// Verify one ledger and cache the parsed records under its file stamp.
    fn verify_ledger(
        &self,
        publisher: &str,
        path: &Path,
        metadata: &std::fs::Metadata,
        stamp: FileStamp,
    ) -> Result<Arc<Vec<serde_json::Value>>, String> {
        let Some(key) = self.ledger_keys.get(publisher) else {
            return Err(format!(
                "no verifying key is configured for publisher {publisher} (set APEX_LEDGER_KEYS)"
            ));
        };
        if metadata.len() > MAX_LEDGER_BYTES {
            return Err(format!(
                "the ledger is {} bytes, above the {MAX_LEDGER_BYTES}-byte read cap",
                metadata.len()
            ));
        }
        let records = Ledger::read_verified(path, key)
            .map_err(|error| format!("chain verification failed: {error}"))?;
        let mut lines = Vec::with_capacity(records.len());
        for record in records {
            let value = serde_json::to_value(&record)
                .map_err(|error| format!("a verified record could not be rendered: {error}"))?;
            lines.push(value);
        }
        let lines = Arc::new(lines);
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| "ledger cache lock poisoned".to_string())?;
        cache.insert(
            path.to_path_buf(),
            CachedLedger {
                stamp,
                lines: Arc::clone(&lines),
            },
        );
        Ok(lines)
    }

    fn ledger_files(&self) -> Result<LedgerScan, Unavailable> {
        let scan = self.ledger_scan()?;
        let mut files = Vec::new();
        let mut stamp: LedgerStamp = Vec::new();
        for ledger in scan {
            let ScannedLedger {
                publisher,
                path,
                stamp: file_stamp,
                lines,
            } = ledger;
            match lines {
                Ok(lines) => {
                    if let Some(file_stamp) = file_stamp {
                        stamp.push((path, file_stamp));
                    }
                    files.push((publisher, lines));
                }
                Err(reason) => {
                    return Err(Unavailable::new(format!(
                        "the ledger {publisher} did not verify: {reason}"
                    )))
                }
            }
        }
        if files.is_empty() {
            return Err(Unavailable::new(format!(
                "the ledger root {} holds no publisher ledgers",
                self.ledger_root.display()
            )));
        }
        files.sort_by(|left, right| left.0.cmp(&right.0));
        stamp.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(LedgerScan { files, stamp })
    }

    /// Every publisher ledger with its verification outcome. The customers
    /// list uses this to show verified counts and refusals together.
    pub fn publisher_summaries(&self) -> Result<Vec<VerifiedPublisher>, Unavailable> {
        let mut out = Vec::new();
        for ledger in self.ledger_scan()? {
            match ledger.lines {
                Ok(lines) => {
                    let last = lines
                        .iter()
                        .filter_map(|line| line.get("atUnixMs").and_then(|value| value.as_i64()))
                        .max();
                    let mut counts: std::collections::BTreeMap<String, u64> =
                        std::collections::BTreeMap::new();
                    for line in lines.iter() {
                        if let Some(platform) =
                            line.get("platform").and_then(|value| value.as_str())
                        {
                            *counts.entry(platform.to_string()).or_default() += 1;
                        }
                    }
                    let platform_counts: Vec<(String, u64)> = counts.into_iter().collect();
                    let platforms: Vec<String> = platform_counts
                        .iter()
                        .map(|(platform, _)| platform.clone())
                        .collect();
                    out.push(VerifiedPublisher {
                        publisher: ledger.publisher,
                        record_count: u64::try_from(lines.len()).unwrap_or(u64::MAX),
                        last_record_at_ms: last,
                        platforms,
                        platform_counts,
                        error: None,
                    });
                }
                Err(reason) => out.push(VerifiedPublisher {
                    publisher: ledger.publisher,
                    record_count: 0,
                    last_record_at_ms: None,
                    platforms: Vec::new(),
                    platform_counts: Vec::new(),
                    error: Some(reason),
                }),
            }
        }
        out.sort_by(|left, right| left.publisher.cmp(&right.publisher));
        Ok(out)
    }

    /// The verified records of one publisher, bounded to display fields.
    pub fn verified_records(
        &self,
        publisher: &str,
    ) -> Result<Vec<VerifiedRecordView>, Unavailable> {
        if !is_valid_publisher(publisher) {
            return Err(Unavailable::new(
                "the publisher id is not a valid ledger file name",
            ));
        }
        let scan = self.ledger_scan()?;
        let Some(ledger) = scan
            .into_iter()
            .find(|ledger| ledger.publisher == publisher)
        else {
            return Err(Unavailable::new(format!(
                "no ledger file is present for publisher {publisher}"
            )));
        };
        let lines = ledger
            .lines
            .map_err(|reason| Unavailable::new(format!("the ledger did not verify: {reason}")))?;
        Ok(lines
            .iter()
            .map(|line| VerifiedRecordView {
                seq: line
                    .get("seq")
                    .and_then(|value| value.as_u64())
                    .unwrap_or(0),
                at_ms: line
                    .get("atUnixMs")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(0),
                kind: line
                    .get("type")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                platform: line
                    .get("platform")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                detail: record_detail(line),
            })
            .collect())
    }

    /// The runner snapshots under `<root>/<publisher>/<platform>.json`.
    /// A missing root is a typed absence; a malformed file is carried as an
    /// error on its own row, never as a fabricated snapshot.
    pub fn publisher_snapshots(
        &self,
        as_of_ms: i64,
    ) -> Result<Vec<PublisherSnapshots>, Unavailable> {
        let entries = match std::fs::read_dir(&self.snapshot_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Unavailable::new(format!(
                    "no runner snapshot root is present at {}",
                    self.snapshot_root.display()
                )))
            }
            Err(error) => {
                return Err(Unavailable::new(format!(
                    "the runner snapshot root {} could not be read: {error}",
                    self.snapshot_root.display()
                )))
            }
        };
        let mut publishers = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                Unavailable::new(format!(
                    "the runner snapshot root {} could not be read: {error}",
                    self.snapshot_root.display()
                ))
            })?;
            if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                continue;
            }
            let publisher = entry.file_name().to_string_lossy().to_string();
            if !is_valid_publisher(&publisher) {
                continue;
            }
            let mut platforms = Vec::new();
            let platform_entries = match std::fs::read_dir(entry.path()) {
                Ok(entries) => entries,
                Err(error) => {
                    platforms.push(SnapshotPlatformView {
                        platform: "-".to_string(),
                        fetched_at_ms: None,
                        age_ms: None,
                        ad_units: None,
                        error: Some(format!(
                            "the publisher directory could not be read: {error}"
                        )),
                    });
                    publishers.push(PublisherSnapshots {
                        publisher,
                        platforms,
                    });
                    continue;
                }
            };
            for file in platform_entries {
                let file = match file {
                    Ok(file) => file,
                    Err(error) => {
                        platforms.push(SnapshotPlatformView {
                            platform: "-".to_string(),
                            fetched_at_ms: None,
                            age_ms: None,
                            ad_units: None,
                            error: Some(format!("a snapshot entry could not be read: {error}")),
                        });
                        continue;
                    }
                };
                let name = file.file_name().to_string_lossy().to_string();
                let Some(platform) = name.strip_suffix(".json") else {
                    continue;
                };
                if !is_valid_publisher(platform) {
                    continue;
                }
                let path = file.path();
                let parsed = match std::fs::read_to_string(&path) {
                    Ok(raw) => serde_json::from_str::<SnapshotFile>(&raw)
                        .map_err(|error| format!("the snapshot did not parse: {error}")),
                    Err(error) => Err(format!("the snapshot could not be read: {error}")),
                };
                match parsed {
                    Ok(snapshot) => platforms.push(SnapshotPlatformView {
                        platform: platform.to_string(),
                        fetched_at_ms: Some(snapshot.fetched_at_unix_ms),
                        age_ms: Some(as_of_ms.saturating_sub(snapshot.fetched_at_unix_ms).max(0)),
                        ad_units: Some(snapshot.ad_units.len()),
                        error: None,
                    }),
                    Err(reason) => platforms.push(SnapshotPlatformView {
                        platform: platform.to_string(),
                        fetched_at_ms: None,
                        age_ms: None,
                        ad_units: None,
                        error: Some(reason),
                    }),
                }
            }
            platforms.sort_by(|left, right| left.platform.cmp(&right.platform));
            publishers.push(PublisherSnapshots {
                publisher,
                platforms,
            });
        }
        if publishers.is_empty() {
            return Err(Unavailable::new(format!(
                "the runner snapshot root {} holds no publisher snapshots",
                self.snapshot_root.display()
            )));
        }
        publishers.sort_by(|left, right| left.publisher.cmp(&right.publisher));
        Ok(publishers)
    }

    /// The experiment records the ledgers carry, read-only.
    pub fn experiments(&self) -> Result<Vec<DurableExperiment>, Unavailable> {
        Ok((*self.experiments_shared()?).clone())
    }

    /// The merged experiment projection without cloning it per call: the
    /// returned handle is shared while the directory stamp is unchanged.
    pub fn experiments_shared(&self) -> Result<Arc<Vec<DurableExperiment>>, Unavailable> {
        let scan = self.ledger_files()?;
        if let Some(cached) = self.cached_experiments(&scan.stamp)? {
            return Ok(cached);
        }
        let files = &scan.files;
        let mut experiments: Vec<DurableExperiment> = Vec::new();
        // Experiment ids are only unique per publisher: two publishers may
        // reuse one id, so the merge index is keyed by (publisher, id).
        let mut index: HashMap<(String, String), usize> = HashMap::new();
        for (publisher, lines) in files {
            for line in lines.iter() {
                let kind = line.get("type").and_then(|value| value.as_str());
                let at = line.get("atUnixMs").and_then(|value| value.as_i64());
                match kind {
                    Some("experimentStarted") => {
                        let Some(experiment_id) =
                            line.get("experimentId").and_then(|value| value.as_str())
                        else {
                            continue;
                        };
                        let spec = line.get("spec");
                        let position = experiments.len();
                        index
                            .entry((publisher.clone(), experiment_id.to_string()))
                            .or_insert(position);
                        experiments.push(DurableExperiment {
                            publisher: publisher.clone(),
                            experiment_id: experiment_id.to_string(),
                            name: spec
                                .and_then(|spec| spec.get("name"))
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                            metric: spec
                                .and_then(|spec| spec.get("metric"))
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                            change_description: spec
                                .and_then(|spec| spec.get("changeDescription"))
                                .and_then(|value| value.as_str())
                                .map(str::to_string),
                            started_at_ms: at,
                            state: None,
                            lift_bp: None,
                            confidence: None,
                            observed_at_ms: None,
                        });
                    }
                    Some("experimentObserved") => {
                        let Some(experiment_id) =
                            line.get("experimentId").and_then(|value| value.as_str())
                        else {
                            continue;
                        };
                        let result = line.get("result");
                        let state = result
                            .and_then(|result| result.get("state"))
                            .and_then(|value| value.as_str())
                            .map(str::to_string);
                        let lift_bp = result
                            .and_then(|result| result.get("liftPercentBp"))
                            .and_then(|value| value.as_i64());
                        let confidence = result
                            .and_then(|result| result.get("confidence"))
                            .and_then(|value| value.as_i64());
                        let key = (publisher.clone(), experiment_id.to_string());
                        if let Some(&position) = index.get(&key) {
                            let existing = &mut experiments[position];
                            existing.state = state;
                            existing.lift_bp = lift_bp;
                            existing.confidence = confidence;
                            existing.observed_at_ms = at;
                        } else {
                            let position = experiments.len();
                            index.entry(key).or_insert(position);
                            experiments.push(DurableExperiment {
                                publisher: publisher.clone(),
                                experiment_id: experiment_id.to_string(),
                                name: None,
                                metric: None,
                                change_description: None,
                                started_at_ms: None,
                                state,
                                lift_bp,
                                confidence,
                                observed_at_ms: at,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        if experiments.is_empty() {
            return Err(Unavailable::new(
                "no experiment records are present in the durable ledgers",
            ));
        }
        experiments.sort_by(|left, right| left.experiment_id.cmp(&right.experiment_id));
        #[cfg(test)]
        self.experiment_builds.fetch_add(1, Ordering::SeqCst);
        let experiments = Arc::new(experiments);
        let mut cache = self
            .experiments_cache
            .lock()
            .map_err(|_| Unavailable::new("experiment cache lock poisoned"))?;
        *cache = Some(CachedExperiments {
            stamp: scan.stamp,
            experiments: Arc::clone(&experiments),
        });
        Ok(experiments)
    }

    fn cached_experiments(
        &self,
        stamp: &LedgerStamp,
    ) -> Result<Option<Arc<Vec<DurableExperiment>>>, Unavailable> {
        let cache = self
            .experiments_cache
            .lock()
            .map_err(|_| Unavailable::new("experiment cache lock poisoned"))?;
        Ok(cache
            .as_ref()
            .filter(|cached| cached.stamp == *stamp)
            .map(|cached| Arc::clone(&cached.experiments)))
    }

    /// The invoice register, read-only.
    ///
    /// The parsed register is reused while its file identity (length, mtime
    /// and on Unix ctime and inode) is unchanged, so the several reads one
    /// finance page makes do not re-parse the whole file each time. A stale
    /// cache is impossible: the same identity rules as the ledger cache apply.
    pub fn invoices(&self) -> Result<Vec<InvoiceRecord>, Unavailable> {
        Ok((*self.invoices_shared()?).clone())
    }

    /// One invoice by identity, cloned alone instead of the whole register.
    pub fn invoice_by_id(&self, id: &str) -> Result<Option<InvoiceRecord>, Unavailable> {
        Ok(self
            .invoices_shared()?
            .iter()
            .find(|invoice| invoice.id() == id)
            .cloned())
    }

    /// The parsed register without cloning it per call.
    pub fn invoices_shared(&self) -> Result<Arc<Vec<InvoiceRecord>>, Unavailable> {
        let stamp = match std::fs::metadata(&self.invoice_register) {
            Ok(metadata) => Some(FileStamp::of(&metadata)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Unavailable::new(format!(
                    "no invoice register is present at {}",
                    self.invoice_register.display()
                )))
            }
            // A stat that fails for another reason is not fatal: the read
            // below reports the real error (and the parse is not cached).
            Err(_) => None,
        };
        if let Some(stamp) = stamp {
            let cached = {
                let cache = self
                    .invoice_cache
                    .lock()
                    .map_err(|_| Unavailable::new("invoice cache lock poisoned"))?;
                cache
                    .as_ref()
                    .filter(|cached| cached.stamp == stamp)
                    .map(|cached| cached.invoices.clone())
            };
            if let Some(invoices) = cached {
                return Ok(invoices);
            }
        }
        let raw = match std::fs::read_to_string(&self.invoice_register) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(Unavailable::new(format!(
                    "no invoice register is present at {}",
                    self.invoice_register.display()
                )))
            }
            Err(error) => {
                return Err(Unavailable::new(format!(
                    "the invoice register {} could not be read: {error}",
                    self.invoice_register.display()
                )))
            }
        };
        let mut invoices = Vec::new();
        for (index, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            invoices.push(parse_invoice_line(line).map_err(|reason| {
                Unavailable::new(format!(
                    "the invoice register line {} did not parse: {reason}",
                    index + 1
                ))
            })?);
        }
        invoices.sort_by_key(|invoice| invoice.id());
        #[cfg(test)]
        self.invoice_parses.fetch_add(1, Ordering::SeqCst);
        let invoices = Arc::new(invoices);
        if let Some(stamp) = stamp {
            let mut cache = self
                .invoice_cache
                .lock()
                .map_err(|_| Unavailable::new("invoice cache lock poisoned"))?;
            *cache = Some(CachedInvoices {
                stamp,
                invoices: Arc::clone(&invoices),
            });
        }
        Ok(invoices)
    }

    /// The live per-day ingestion coverage matrix: every publisher/platform
    /// seen in the verified ledgers or the runner snapshots, one cell per day
    /// in the window. Records come from the verified ledger chain; audits
    /// count the audit-evidence record kinds ([`is_audit_record_kind`]); the
    /// snapshot columns report presence and age. A day with no observed data
    /// is an explicit zero cell, never a fabricated verdict.
    pub fn ingestion_coverage(
        &self,
        as_of_ms: i64,
        window_days: u32,
    ) -> Result<Vec<CoveragePlatformRow>, Unavailable> {
        let window_days = window_days.clamp(1, MAX_COVERAGE_WINDOW_DAYS);
        let end_day = as_of_ms
            .div_euclid(apex_analytics::timestamp::MILLIS_PER_DAY)
            .max(0);
        let start_day = end_day.saturating_sub(i64::from(window_days) - 1);
        let days: Vec<(i64, String)> = (start_day..=end_day)
            .map(|index| (index, day_string(index)))
            .collect();

        let ledgers = self.ledger_files();
        let snapshots = self.publisher_snapshots(as_of_ms);
        if ledgers.is_err() && snapshots.is_err() {
            let mut reason = String::from("no coverage source is available: ");
            if let Err(error) = &ledgers {
                reason.push_str(&error.reason);
            }
            if let Err(error) = &snapshots {
                if !reason.ends_with(": ") {
                    reason.push_str("; ");
                }
                reason.push_str(&error.reason);
            }
            return Err(Unavailable::new(reason));
        }

        #[derive(Default)]
        struct Accumulator {
            per_day: HashMap<i64, (u64, u64, Option<i64>)>,
            last_at_ms: Option<i64>,
            total_records: u64,
            total_audits: u64,
        }
        let mut rows: BTreeMap<(String, String), Accumulator> = BTreeMap::new();
        if let Ok(files) = &ledgers {
            for (publisher, lines) in files.files.iter() {
                for line in lines.iter() {
                    let kind = line.get("type").and_then(|value| value.as_str());
                    let audit = kind.is_some_and(is_audit_record_kind);
                    let platform = line
                        .get("platform")
                        .and_then(|value| value.as_str())
                        .filter(|platform| is_valid_publisher(platform))
                        .unwrap_or("-")
                        .to_string();
                    let accumulator = rows.entry((publisher.clone(), platform)).or_default();
                    accumulator.total_records += 1;
                    if audit {
                        accumulator.total_audits += 1;
                    }
                    if let Some(at) = line.get("atUnixMs").and_then(|value| value.as_i64()) {
                        accumulator.last_at_ms =
                            Some(accumulator.last_at_ms.map_or(at, |last| last.max(at)));
                        let day = at
                            .div_euclid(apex_analytics::timestamp::MILLIS_PER_DAY)
                            .max(0);
                        let cell = accumulator.per_day.entry(day).or_insert((0, 0, None));
                        cell.0 += 1;
                        if audit {
                            cell.1 += 1;
                        }
                        cell.2 = Some(cell.2.map_or(at, |last| last.max(at)));
                    }
                }
            }
        }

        let mut snapshots_by_row: BTreeMap<(String, String), SnapshotPlatformView> =
            BTreeMap::new();
        let mut snapshot_problem: Option<String> = None;
        match &snapshots {
            Ok(publishers) => {
                for publisher in publishers {
                    for platform in &publisher.platforms {
                        if platform.platform == "-" {
                            snapshot_problem = platform.error.clone();
                            continue;
                        }
                        snapshots_by_row.insert(
                            (publisher.publisher.clone(), platform.platform.clone()),
                            platform.clone(),
                        );
                    }
                }
            }
            Err(error) => snapshot_problem = Some(error.reason.clone()),
        }

        let mut keys: BTreeSet<(String, String)> = rows.keys().cloned().collect();
        keys.extend(snapshots_by_row.keys().cloned());
        if keys.is_empty() {
            return Err(Unavailable::new(
                "the verified ledgers and the runner snapshots hold no publisher/platform row in this window",
            ));
        }

        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            let accumulator = rows.remove(&key);
            let snapshot = snapshots_by_row.remove(&key);
            let (total_records, total_audits, last_verified_at_ms, cells) = match accumulator {
                Some(accumulator) => {
                    let cells = days
                        .iter()
                        .map(|(index, day)| {
                            let (records, audits, last) = accumulator
                                .per_day
                                .get(index)
                                .copied()
                                .unwrap_or((0, 0, None));
                            CoverageDayCell {
                                day: day.clone(),
                                records,
                                audits,
                                last_verified_at_ms: last,
                            }
                        })
                        .collect();
                    (
                        accumulator.total_records,
                        accumulator.total_audits,
                        accumulator.last_at_ms,
                        cells,
                    )
                }
                None => (
                    0,
                    0,
                    None,
                    days.iter()
                        .map(|(_, day)| CoverageDayCell {
                            day: day.clone(),
                            records: 0,
                            audits: 0,
                            last_verified_at_ms: None,
                        })
                        .collect(),
                ),
            };
            let snapshot_error = snapshot
                .as_ref()
                .and_then(|view| view.error.clone())
                .or_else(|| snapshot_problem.clone());
            let (snapshot_present, snapshot_fetched_at_ms, snapshot_age_ms, snapshot_ad_units) =
                match snapshot {
                    Some(view) => (
                        view.error.is_none(),
                        view.fetched_at_ms,
                        view.age_ms,
                        view.ad_units,
                    ),
                    None => (false, None, None, None),
                };
            out.push(CoveragePlatformRow {
                publisher: key.0,
                platform: key.1,
                days: cells,
                total_records,
                total_audits,
                last_verified_at_ms,
                snapshot_present,
                snapshot_fetched_at_ms,
                snapshot_age_ms,
                snapshot_ad_units,
                snapshot_error,
            });
        }
        Ok(out)
    }

    /// The decision checks of one experiment, derived from its verified
    /// ledger records: the registration (`experimentStarted`), the observed
    /// result / signed decision (`experimentObserved` / `liftRecord`) and the
    /// settle state (`billingProof`). Whatever the records actually carry is
    /// what is shown; a missing record is an explicit not-recorded verdict.
    pub fn experiment_decision_checks(
        &self,
        publisher: &str,
        experiment_id: &str,
    ) -> Result<Vec<DecisionCheck>, Unavailable> {
        if !is_valid_publisher(publisher) {
            return Err(Unavailable::new(
                "the publisher id is not a valid ledger file name",
            ));
        }
        let scan = self.ledger_scan()?;
        let Some(ledger) = scan
            .into_iter()
            .find(|ledger| ledger.publisher == publisher)
        else {
            return Err(Unavailable::new(format!(
                "no ledger file is present for publisher {publisher}"
            )));
        };
        let lines = ledger
            .lines
            .map_err(|reason| Unavailable::new(format!("the ledger did not verify: {reason}")))?;
        let matching: Vec<&serde_json::Value> = lines
            .iter()
            .filter(|line| {
                line.get("experimentId").and_then(|value| value.as_str()) == Some(experiment_id)
            })
            .collect();
        if matching.is_empty() {
            return Err(Unavailable::new(format!(
                "no verified ledger record names experiment {experiment_id}"
            )));
        }

        let started = matching.iter().find(|line| {
            line.get("type").and_then(|value| value.as_str()) == Some("experimentStarted")
        });
        let spec = started.and_then(|line| line.get("spec"));
        let name = spec
            .and_then(|spec| spec.get("name"))
            .and_then(|value| value.as_str());
        let metric = spec
            .and_then(|spec| spec.get("metric"))
            .and_then(|value| value.as_str());
        let ad_unit = spec
            .and_then(|spec| spec.get("adUnitId"))
            .and_then(|value| value.as_str());

        let observation = matching
            .iter()
            .filter(|line| {
                line.get("type").and_then(|value| value.as_str()) == Some("experimentObserved")
            })
            .max_by_key(|line| {
                line.get("atUnixMs")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(0)
            });
        let state = observation
            .and_then(|line| line.get("result"))
            .and_then(|result| result.get("state"))
            .and_then(|value| value.as_str());
        let lift_bp = observation
            .and_then(|line| line.get("result"))
            .and_then(|result| result.get("liftPercentBp"))
            .and_then(|value| value.as_i64());
        let lift_records = matching
            .iter()
            .filter(|line| line.get("type").and_then(|value| value.as_str()) == Some("liftRecord"))
            .count();
        let proofs: Vec<&serde_json::Value> = matching
            .iter()
            .filter(|line| {
                line.get("type").and_then(|value| value.as_str()) == Some("billingProof")
            })
            .copied()
            .collect();
        let fee_micros: i64 = proofs
            .iter()
            .filter_map(|line| line.get("feeMicros").and_then(|value| value.as_i64()))
            .fold(0i64, i64::saturating_add);

        let registered = match (name, metric, ad_unit) {
            (Some(name), Some(metric), Some(ad_unit)) => {
                format!("spec name {name}, metric {metric}, ad unit {ad_unit}")
            }
            _ => "experimentStarted record".to_string(),
        };
        let registration_verdict = if started.is_some() { "Pass" } else { "Fail" };

        let decision_done =
            lift_records > 0 || state.is_some_and(|state| matches!(state, "completed" | "stopped"));
        let observed_decision = match (state, lift_bp) {
            (Some(state), Some(lift)) => format!("{state}, lift {lift} bp"),
            (Some(state), None) => state.to_string(),
            (None, _) if lift_records > 0 => "signed lift record only".to_string(),
            (None, _) => "no observation or decision record".to_string(),
        };
        let decision_observed = if lift_records > 0 {
            format!("{observed_decision}; {lift_records} signed lift record(s)")
        } else {
            observed_decision
        };

        let settle_observed = if proofs.is_empty() {
            "no billing proof".to_string()
        } else {
            format!("{} billing proof(s), fee {fee_micros} micros", proofs.len())
        };

        Ok(vec![
            DecisionCheck {
                check: "Registration".to_string(),
                registered,
                observed: if started.is_some() {
                    "registered in the verified ledger".to_string()
                } else {
                    "no experimentStarted record".to_string()
                },
                verdict: registration_verdict.to_string(),
            },
            DecisionCheck {
                check: "Plan / decision".to_string(),
                registered: "an observed result or the signed lift record".to_string(),
                observed: decision_observed,
                verdict: if decision_done {
                    "Pass".to_string()
                } else {
                    "Pending".to_string()
                },
            },
            DecisionCheck {
                check: "Settle".to_string(),
                registered: "a billing proof".to_string(),
                observed: settle_observed,
                verdict: if proofs.is_empty() {
                    "Not settled".to_string()
                } else {
                    "Pass".to_string()
                },
            },
        ])
    }

    /// The monthly service economics from the verified ledger fee records
    /// (`billingProof`) plus the invoice register. Net and gross come from the
    /// register (net is the invoiced net, gross adds VAT); the verified ledger
    /// fee is shown beside them. Credits are `None`: neither source carries a
    /// credit record. Every month between the first and last observed month is
    /// present, so an empty month is explicit.
    pub fn monthly_economics(&self) -> Result<Vec<LiveEconomicsMonth>, Unavailable> {
        let ledgers = self.ledger_files();
        let invoices = self.invoices();
        let mut months: BTreeMap<String, LiveEconomicsMonth> = BTreeMap::new();
        if let Ok(files) = &ledgers {
            for (_publisher, lines) in files.files.iter() {
                for line in lines.iter() {
                    if line.get("type").and_then(|value| value.as_str()) != Some("billingProof") {
                        continue;
                    }
                    let Some(at) = line.get("atUnixMs").and_then(|value| value.as_i64()) else {
                        continue;
                    };
                    let fee = line
                        .get("feeMicros")
                        .and_then(|value| value.as_i64())
                        .unwrap_or(0);
                    let month = UtcMillis::from_unix_millis(at).to_month_string();
                    let entry = months
                        .entry(month.clone())
                        .or_insert_with(|| LiveEconomicsMonth::empty(month));
                    entry.ledger_fee_micros = entry.ledger_fee_micros.saturating_add(fee);
                    entry.ledger_fee_records += 1;
                }
            }
        }
        if let Ok(register) = &invoices {
            for invoice in register {
                let entry = months
                    .entry(invoice.period.clone())
                    .or_insert_with(|| LiveEconomicsMonth::empty(invoice.period.clone()));
                entry.net_micros = entry.net_micros.saturating_add(invoice.total_micros);
                entry.gross_micros = entry.gross_micros.saturating_add(invoice.gross_micros);
                entry.invoice_records += 1;
            }
        }
        if months.is_empty() {
            let reason = match (&ledgers, &invoices) {
                (Err(ledger), Err(register)) => format!(
                    "no verified ledger fee records ({}) and no invoice register ({})",
                    ledger.reason, register.reason
                ),
                (Err(ledger), Ok(_)) => {
                    format!("no verified ledger fee records: {}", ledger.reason)
                }
                (Ok(_), Err(register)) => format!("no invoice register: {}", register.reason),
                (Ok(_), Ok(_)) => {
                    "no billing proof record and no invoice line is present".to_string()
                }
            };
            return Err(Unavailable::new(reason));
        }

        let first = months.keys().next().cloned().unwrap_or_default();
        let last = months.keys().next_back().cloned().unwrap_or_default();
        let mut out = Vec::new();
        let mut cursor = Some(first);
        while let Some(month) = cursor {
            let is_last = month == last;
            let entry = months
                .remove(&month)
                .unwrap_or_else(|| LiveEconomicsMonth::empty(month.clone()));
            out.push(entry);
            if is_last {
                break;
            }
            cursor = next_month(&month);
        }
        Ok(out)
    }

    /// The payment timeline of `invoice`: the register-derived issuance row,
    /// any verified ledger fee proofs for the invoice's lines, and an explicit
    /// row stating that no payment or reconciliation record exists (neither
    /// source carries one). It never claims a payment stage that is not
    /// recorded.
    pub fn invoice_payments(&self, invoice: &InvoiceRecord) -> InvoicePayments {
        let mut rows = vec![InvoicePaymentRow {
            at_ms: Some(invoice.end_unix_ms),
            event_code: "Invoice issued".to_string(),
            reference_masked: Some(invoice.id()),
            amount_micros: Some(invoice.gross_micros),
            currency: CONTROL_CURRENCY.to_string(),
            received: None,
            hmac_verified: None,
            deduplicated: None,
            reconciled: None,
            allocated: None,
            note: Some(
                "Issued state from the invoice register; the register carries no payment stage."
                    .to_string(),
            ),
        }];
        let mut ledger_note = None;
        match self.ledger_files() {
            Ok(files) => {
                let experiments: HashSet<&str> = invoice
                    .line_items
                    .iter()
                    .map(|line| line.experiment_id.as_str())
                    .collect();
                for (publisher, lines) in files.files.iter() {
                    if publisher != &invoice.publisher_id {
                        continue;
                    }
                    for line in lines.iter() {
                        if line.get("type").and_then(|value| value.as_str()) != Some("billingProof")
                        {
                            continue;
                        }
                        let experiment = line.get("experimentId").and_then(|value| value.as_str());
                        if !experiment.is_some_and(|id| experiments.contains(id)) {
                            continue;
                        }
                        rows.push(InvoicePaymentRow {
                            at_ms: line.get("atUnixMs").and_then(|value| value.as_i64()),
                            event_code: "Verified fee proof".to_string(),
                            reference_masked: experiment.map(str::to_string),
                            amount_micros: line.get("feeMicros").and_then(|value| value.as_i64()),
                            currency: CONTROL_CURRENCY.to_string(),
                            received: None,
                            hmac_verified: None,
                            deduplicated: None,
                            reconciled: None,
                            allocated: None,
                            note: Some(
                                "Settle evidence from a verified billing proof; it is not a payment event."
                                    .to_string(),
                            ),
                        });
                    }
                }
            }
            Err(error) => ledger_note = Some(error.reason),
        }
        rows.push(InvoicePaymentRow {
            at_ms: None,
            event_code: "Payment received".to_string(),
            reference_masked: None,
            amount_micros: None,
            currency: CONTROL_CURRENCY.to_string(),
            received: None,
            hmac_verified: None,
            deduplicated: None,
            reconciled: None,
            allocated: None,
            note: Some(
                "No payment or reconciliation record exists: the verified ledger has no payment/reconciliation record type and the invoice register carries no payment state. Nothing is claimed as received, verified, deduplicated, reconciled or allocated."
                    .to_string(),
            ),
        });
        rows.sort_by_key(|row| row.at_ms.unwrap_or(i64::MAX));
        InvoicePayments {
            invoice_id: invoice.id(),
            rows,
            ledger_note,
        }
    }
}

/// The calendar day string (`YYYY-MM-DD`) of a day index.
fn day_string(day_index: i64) -> String {
    UtcMillis::from_unix_millis(day_index.saturating_mul(apex_analytics::timestamp::MILLIS_PER_DAY))
        .to_day_string()
}

/// The next `YYYY-MM` month label, when the label parses.
fn next_month(month: &str) -> Option<String> {
    let (year, month_number) = month.split_once('-')?;
    let year: i32 = year.parse().ok()?;
    let month_number: u32 = month_number.parse().ok()?;
    if !(1..=12).contains(&month_number) {
        return None;
    }
    if month_number == 12 {
        Some(format!("{:04}-01", year.checked_add(1)?))
    } else {
        Some(format!("{year:04}-{:02}", month_number + 1))
    }
}

/// Parse one register line, mirroring the `apex-billing` serialization.
fn parse_invoice_line(line: &str) -> Result<InvoiceRecord, String> {
    let value: serde_json::Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
    let publisher_id = value
        .get("publisherId")
        .and_then(|v| v.as_str())
        .ok_or("missing publisherId")?
        .to_string();
    let period = value.get("period").ok_or("missing period")?;
    let year_month = period
        .get("yearMonth")
        .and_then(|v| v.as_str())
        .ok_or("missing period.yearMonth")?
        .to_string();
    let start_unix_ms = period
        .get("startUnixMs")
        .and_then(|v| v.as_i64())
        .ok_or("missing period.startUnixMs")?;
    let end_unix_ms = period
        .get("endUnixMs")
        .and_then(|v| v.as_i64())
        .ok_or("missing period.endUnixMs")?;
    let mut line_items = Vec::new();
    for item in value
        .get("lineItems")
        .and_then(|v| v.as_array())
        .ok_or("missing lineItems")?
    {
        line_items.push(InvoiceLineRecord {
            seq: item.get("seq").and_then(|v| v.as_u64()).ok_or("line seq")?,
            experiment_id: item
                .get("experimentId")
                .and_then(|v| v.as_str())
                .ok_or("line experimentId")?
                .to_string(),
            proven_lift: item
                .get("provenLift")
                .and_then(|v| v.as_i64())
                .ok_or("line provenLift")?,
            rate_bp: item
                .get("rateBp")
                .and_then(|v| v.as_u64())
                .and_then(|v| u16::try_from(v).ok())
                .ok_or("line rateBp")?,
            fee_micros: item
                .get("feeMicros")
                .and_then(|v| v.as_i64())
                .ok_or("line feeMicros")?,
            vat_micros: item.get("vatMicros").and_then(|v| v.as_i64()).unwrap_or(0),
            gross_micros: item
                .get("grossMicros")
                .and_then(|v| v.as_i64())
                .ok_or("line grossMicros")?,
        });
    }
    Ok(InvoiceRecord {
        publisher_id,
        period: year_month,
        start_unix_ms,
        end_unix_ms,
        line_items,
        total_micros: value
            .get("totalMicros")
            .and_then(|v| v.as_i64())
            .ok_or("missing totalMicros")?,
        vat_rate_bp: value
            .get("vatRateBp")
            .and_then(|v| v.as_u64())
            .and_then(|v| u16::try_from(v).ok())
            .unwrap_or(0),
        vat_micros: value.get("vatMicros").and_then(|v| v.as_i64()).unwrap_or(0),
        gross_micros: value
            .get("grossMicros")
            .and_then(|v| v.as_i64())
            .ok_or("missing grossMicros")?,
    })
}

/// One evidence entry from `APEX_CONTROL_OPERATIONS`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationEvidence {
    /// Platform (`applovin_max`, `levelplay`, `admob`).
    pub platform: String,
    /// Endpoint/action name matching the connector's endpoint metadata.
    pub action: String,
    /// Whether a sandbox call confirmed the operation.
    #[serde(default)]
    pub sandbox_evidence: bool,
    /// Allowed deployment description.
    #[serde(default)]
    pub allowed_deployment: Option<String>,
    /// Credential scope description.
    #[serde(default)]
    pub credential_scope: Option<String>,
    /// Whether an approval is required before the operation runs.
    #[serde(default)]
    pub approval_required: bool,
    /// Release version the operation was verified in.
    #[serde(default)]
    pub release_version: Option<String>,
}

/// The evidence file, loaded once.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OperationsEvidence {
    entries: Vec<OperationEvidence>,
}

impl OperationsEvidence {
    /// Build from parsed entries (the fixture dataset and tests).
    pub fn from_entries(entries: Vec<OperationEvidence>) -> Self {
        Self { entries }
    }

    /// Load the evidence file; an absent file is an empty evidence set (every
    /// operation then remains unavailable), a malformed file is an error the
    /// matrix displays instead of guessing.
    pub fn load(path: &Path) -> Result<Self, Unavailable> {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => {
                return Err(Unavailable::new(format!(
                    "the operation evidence file {} could not be read: {error}",
                    path.display()
                )))
            }
        };
        let entries: Vec<OperationEvidence> = serde_json::from_str(&raw).map_err(|error| {
            Unavailable::new(format!(
                "the operation evidence file {} did not parse: {error}",
                path.display()
            ))
        })?;
        Ok(Self { entries })
    }

    /// The evidence for one platform/action pair.
    pub fn find(&self, platform: &str, action: &str) -> Option<&OperationEvidence> {
        self.entries
            .iter()
            .find(|entry| entry.platform == platform && entry.action == action)
    }

    /// Number of evidence entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no evidence is on file.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One row of the verified-operation matrix (page 72).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessRow {
    /// Platform name.
    pub platform: String,
    /// Action/endpoint name.
    pub action: String,
    /// Adapter implemented: `Yes (documented)` or `Verify live`.
    pub adapter_implemented: String,
    /// Sandbox evidence: `Confirmed`, `Not confirmed` or `Not on file`.
    pub sandbox_evidence: String,
    /// Allowed deployment or `Not on file`.
    pub allowed_deployment: String,
    /// Credential scope or `Not on file`.
    pub credential_scope: String,
    /// Approval requirement (`Required`/`Not required`) or `Not on file`.
    pub approval_requirement: String,
    /// Release version or `Not on file`.
    pub release_version: String,
    /// Whether the action is available: the endpoint is writable and every
    /// evidence column is on file. Missing evidence means unavailable.
    pub available: bool,
}

/// The platform/action pairs of the connector layer, with the evidence
/// applied. The adapter column is read from `apex-connectors` itself.
pub fn readiness_rows(evidence: &OperationsEvidence) -> Vec<ReadinessRow> {
    let all: [(&str, &[EndpointMeta]); 3] = [
        ("applovin_max", &MAX_ENDPOINTS[..]),
        ("levelplay", &LEVELPLAY_ENDPOINTS[..]),
        ("admob", &ADMOB_ENDPOINTS[..]),
    ];
    let mut rows = Vec::new();
    for (platform, endpoints) in all {
        for endpoint in endpoints {
            let evidence = evidence.find(platform, endpoint.name);
            let adapter_implemented = match endpoint.verification {
                apex_core::endpoint::SourceVerification::Documented => "Yes (documented)",
                apex_core::endpoint::SourceVerification::VerifyLive => "Verify live",
            }
            .to_string();
            let sandbox_evidence = match evidence {
                Some(entry) if entry.sandbox_evidence => "Confirmed".to_string(),
                Some(_) => "Not confirmed".to_string(),
                None => "Not on file".to_string(),
            };
            let allowed_deployment = evidence
                .and_then(|entry| entry.allowed_deployment.clone())
                .unwrap_or_else(|| "Not on file".to_string());
            let credential_scope = evidence
                .and_then(|entry| entry.credential_scope.clone())
                .unwrap_or_else(|| "Not on file".to_string());
            let approval_requirement = match evidence {
                Some(entry) if entry.approval_required => "Required".to_string(),
                Some(_) => "Not required".to_string(),
                None => "Not on file".to_string(),
            };
            let release_version = evidence
                .and_then(|entry| entry.release_version.clone())
                .unwrap_or_else(|| "Not on file".to_string());
            let evidence_complete = evidence.is_some_and(|entry| {
                entry.sandbox_evidence
                    && entry.allowed_deployment.is_some()
                    && entry.credential_scope.is_some()
                    && entry.release_version.is_some()
            });
            rows.push(ReadinessRow {
                platform: platform.to_string(),
                action: endpoint.name.to_string(),
                adapter_implemented,
                sandbox_evidence,
                allowed_deployment,
                credential_scope,
                approval_requirement,
                release_version,
                available: endpoint.writable() && evidence_complete,
            });
        }
    }
    rows
}

/// The whole read layer.
#[derive(Debug, Clone)]
pub enum ControlData {
    /// The deterministic fixture dataset (illustrative data).
    Fixtures(Box<DemoData>),
    /// Live reads.
    Live(Box<LiveData>),
}

/// The live read sources.
#[derive(Debug, Clone)]
pub struct LiveData {
    /// Analytics store, when configured.
    pub analytics: Option<AnalyticsSource>,
    /// Durable runner files.
    pub durable: DurableReader,
    /// Verified-operation evidence, or the reason it is not on file.
    pub operations: Result<OperationsEvidence, Unavailable>,
    /// The sync-timer marker file, when `APEX_CONTROL_SYNC_MARKER` is set.
    pub sync_marker: Option<PathBuf>,
}

/// The default reason shown when no analytics store is configured.
pub const NO_ANALYTICS: &str =
    "APEX_CLICKHOUSE_HTTP and APEX_CLICKHOUSE_DB are not set, so no read model is configured";

impl ControlData {
    /// The illustrative dataset wrapped for injection.
    pub fn fixtures() -> Self {
        ControlData::Fixtures(Box::default())
    }

    /// Whether this is the illustrative dataset (drives the page banner).
    pub fn is_fixtures(&self) -> bool {
        matches!(self, ControlData::Fixtures(_))
    }

    /// The freshness reads served by the fixture dataset (test seam).
    #[cfg(test)]
    pub(crate) fn test_ingestion_read_count(&self) -> u64 {
        match self {
            ControlData::Fixtures(demo) => demo.test_ingestion_read_count(),
            ControlData::Live(_) => 0,
        }
    }

    /// The five operator queues.
    pub async fn queues(&self, as_of: UtcMillis) -> Result<OperatorQueues, Unavailable> {
        if let ControlData::Fixtures(demo) = self {
            return Ok(demo.queues());
        }
        let ControlData::Live(live) = self else {
            return Err(Unavailable::new(NO_ANALYTICS));
        };
        match &live.analytics {
            Some(analytics) => analytics.queues(as_of).await,
            None => Err(Unavailable::new(NO_ANALYTICS)),
        }
    }

    /// Ingestion freshness per source.
    pub async fn ingestion_health(
        &self,
        as_of: UtcMillis,
    ) -> Result<Vec<IngestionHealthRow>, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo.ingestion_health()),
            ControlData::Live(live) => match &live.analytics {
                Some(analytics) => analytics.ingestion_health(as_of).await,
                None => Err(Unavailable::new(NO_ANALYTICS)),
            },
        }
    }

    /// The data-quality badge.
    pub async fn data_quality(&self, as_of: UtcMillis) -> Result<DataQuality, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo.data_quality()),
            ControlData::Live(live) => match &live.analytics {
                Some(analytics) => analytics.data_quality(as_of).await,
                None => Err(Unavailable::new(NO_ANALYTICS)),
            },
        }
    }

    /// Growth cohorts by request month.
    pub async fn growth_cohorts(
        &self,
        as_of: UtcMillis,
    ) -> Result<Vec<GrowthCohortRow>, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo.growth_cohorts()),
            ControlData::Live(live) => match &live.analytics {
                Some(analytics) => analytics.growth_cohorts(as_of).await,
                None => Err(Unavailable::new(NO_ANALYTICS)),
            },
        }
    }

    /// The activation funnel of one cohort.
    pub async fn activation_funnel(
        &self,
        as_of: UtcMillis,
        cohort_month: &str,
    ) -> Result<ActivationFunnel, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => demo.activation_funnel(cohort_month),
            ControlData::Live(live) => match &live.analytics {
                Some(analytics) => analytics.activation_funnel(as_of, cohort_month).await,
                None => Err(Unavailable::new(NO_ANALYTICS)),
            },
        }
    }

    /// The experiment fleet landscape.
    pub async fn experiment_landscape(
        &self,
        as_of: UtcMillis,
    ) -> Result<Vec<ExperimentLandscapeRow>, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo.experiment_landscape()),
            ControlData::Live(live) => match &live.analytics {
                Some(analytics) => analytics.experiment_landscape(as_of).await,
                None => Err(Unavailable::new(NO_ANALYTICS)),
            },
        }
    }

    /// The effectiveness of one mature start cohort.
    pub async fn experiment_effectiveness(
        &self,
        as_of: UtcMillis,
        cohort_month: &str,
    ) -> Result<ExperimentEffectiveness, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => demo.experiment_effectiveness(cohort_month),
            ControlData::Live(live) => match &live.analytics {
                Some(analytics) => {
                    analytics
                        .experiment_effectiveness(as_of, cohort_month)
                        .await
                }
                None => Err(Unavailable::new(NO_ANALYTICS)),
            },
        }
    }

    /// Fee rows.
    pub async fn fee_rows(&self, as_of: UtcMillis) -> Result<Vec<FeeRow>, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo.fee_rows()),
            ControlData::Live(live) => match &live.analytics {
                Some(analytics) => analytics.fee_rows(as_of).await,
                None => Err(Unavailable::new(NO_ANALYTICS)),
            },
        }
    }

    /// One reproducible fee calculation.
    pub async fn fee_detail(
        &self,
        as_of: UtcMillis,
        change_id: &str,
    ) -> Result<Option<FeeDetail>, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo.fee_detail(change_id)),
            ControlData::Live(live) => match &live.analytics {
                Some(analytics) => analytics.fee_detail(as_of, change_id).await,
                None => Err(Unavailable::new(NO_ANALYTICS)),
            },
        }
    }

    /// Decision lift rows.
    pub async fn lift_rows(&self, as_of: UtcMillis) -> Result<Vec<LiftRow>, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo.lift_rows()),
            ControlData::Live(live) => match &live.analytics {
                Some(analytics) => analytics.lift_rows(as_of).await,
                None => Err(Unavailable::new(NO_ANALYTICS)),
            },
        }
    }

    /// One object timeline, merged by occurrence time.
    pub async fn object_timeline(
        &self,
        as_of: UtcMillis,
        object_type: &str,
        object_id: &str,
    ) -> Result<Vec<ObjectTimelineEntry>, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo.object_timeline(object_type, object_id)),
            ControlData::Live(live) => match &live.analytics {
                Some(analytics) => {
                    analytics
                        .object_timeline(as_of, object_type, object_id)
                        .await
                }
                None => Err(Unavailable::new(NO_ANALYTICS)),
            },
        }
    }

    /// Site requests over the last 30 days from the edge access log. `None`
    /// when the read is unavailable (no analytics store configured or the
    /// store did not answer), so the growth page renders nothing rather
    /// than a fabricated zero.
    pub async fn site_requests_last_30_days(&self, as_of: UtcMillis) -> Option<u64> {
        match self {
            ControlData::Fixtures(demo) => Some(demo.site_requests_last_30_days()),
            ControlData::Live(live) => match &live.analytics {
                Some(analytics) => analytics.site_requests_last_30_days(as_of).await.ok(),
                None => None,
            },
        }
    }

    /// Durable experiment records.
    pub fn durable_experiments(&self) -> Result<Vec<DurableExperiment>, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo.durable_experiments()),
            ControlData::Live(live) => live.durable.experiments(),
        }
    }

    /// One durable experiment by id, cloned alone instead of the whole
    /// projection.
    pub fn experiment_by_id(&self, id: &str) -> Result<Option<DurableExperiment>, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo
                .durable_experiments()
                .into_iter()
                .find(|experiment| experiment.experiment_id == id)),
            ControlData::Live(live) => Ok(live
                .durable
                .experiments_shared()?
                .iter()
                .find(|experiment| experiment.experiment_id == id)
                .cloned()),
        }
    }

    /// The durable experiments of one publisher, cloned alone.
    pub fn experiments_for_publisher(
        &self,
        publisher: &str,
    ) -> Result<Vec<DurableExperiment>, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo
                .durable_experiments()
                .into_iter()
                .filter(|experiment| experiment.publisher == publisher)
                .collect()),
            ControlData::Live(live) => Ok(live
                .durable
                .experiments_shared()?
                .iter()
                .filter(|experiment| experiment.publisher == publisher)
                .cloned()
                .collect()),
        }
    }

    /// Durable invoice register.
    pub fn invoices(&self) -> Result<Vec<InvoiceRecord>, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo.invoices()),
            ControlData::Live(live) => live.durable.invoices(),
        }
    }

    /// One durable invoice by identity, cloned alone.
    pub fn invoice_by_id(&self, id: &str) -> Result<Option<InvoiceRecord>, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo
                .invoices()
                .into_iter()
                .find(|invoice| invoice.id() == id)),
            ControlData::Live(live) => live.durable.invoice_by_id(id),
        }
    }

    /// The customer inventory: verified ledger publishers with their counts
    /// and last verified record time, unioned with snapshot publishers. The
    /// fixtures dataset is the illustration; live mode never fabricates a
    /// publisher.
    pub fn customer_rows(&self, as_of_ms: i64) -> Result<Vec<CustomerRow>, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo.customers()),
            ControlData::Live(live) => {
                let summaries = live.durable.publisher_summaries();
                let snapshots = live.durable.publisher_snapshots(as_of_ms);
                if summaries.is_err() && snapshots.is_err() {
                    let mut reason = String::from("no account projection source is available: ");
                    if let Err(error) = &summaries {
                        reason.push_str(&error.reason);
                    }
                    if let Err(error) = &snapshots {
                        if !reason.ends_with(": ") {
                            reason.push_str("; ");
                        }
                        reason.push_str(&error.reason);
                    }
                    return Err(Unavailable::new(reason));
                }
                let mut publishers: std::collections::BTreeMap<
                    String,
                    (Option<VerifiedPublisher>, bool),
                > = std::collections::BTreeMap::new();
                if let Ok(rows) = summaries {
                    for row in rows {
                        let publisher = row.publisher.clone();
                        publishers.entry(publisher).or_default().0 = Some(row);
                    }
                }
                if let Ok(rows) = snapshots {
                    for row in rows {
                        publishers.entry(row.publisher).or_default().1 = true;
                    }
                }
                if publishers.is_empty() {
                    return Err(Unavailable::new(
                        "no ledger or snapshot publisher is present in this deployment",
                    ));
                }
                let experiments = live.durable.experiments().unwrap_or_default();
                let invoices = live.durable.invoices();
                Ok(publishers
                    .into_iter()
                    .map(|(publisher, (summary, has_snapshot))| {
                        let running_tests = experiments
                            .iter()
                            .filter(|experiment| {
                                experiment.publisher == publisher
                                    && experiment.state.as_deref() == Some("running")
                            })
                            .count() as u64;
                        let invoice_count = invoices
                            .as_ref()
                            .map(|rows| {
                                rows.iter()
                                    .filter(|invoice| invoice.publisher_id == publisher)
                                    .count()
                            })
                            .unwrap_or(0);
                        let billing_state = match &invoices {
                            Ok(_) if invoice_count > 0 => {
                                format!("{invoice_count} invoice record(s)")
                            }
                            Ok(_) => "No invoice record".to_string(),
                            Err(_) => "No invoice register configured".to_string(),
                        };
                        let (
                            last_activity,
                            stage_age_days,
                            lifecycle,
                            health,
                            platform_readiness,
                        ) = match summary {
                            Some(VerifiedPublisher {
                                record_count,
                                last_record_at_ms,
                                ref platform_counts,
                                error: None,
                                ..
                            }) => {
                                let last = last_record_at_ms
                                    .map(apex_ui::format::utc_datetime)
                                    .unwrap_or_else(|| "no record".to_string());
                                let mut readiness = if platform_counts.is_empty() {
                                    vec![format!(
                                        "ledger: {record_count} verified record(s)"
                                    )]
                                } else {
                                    platform_counts
                                        .iter()
                                        .map(|(platform, count)| {
                                            format!("{platform}: {count} verified record(s)")
                                        })
                                        .collect()
                                };
                                readiness.push(format!("last verified: {last}"));
                                (
                                    format!(
                                        "{record_count} verified record(s); last verified record {last}"
                                    ),
                                    last_record_at_ms
                                        .map(|at| {
                                            as_of_ms.saturating_sub(at).max(0)
                                                / apex_analytics::timestamp::MILLIS_PER_DAY
                                        })
                                        .unwrap_or(0),
                                    "ledger_verified".to_string(),
                                    if has_snapshot {
                                        "Verified ledger; snapshot present".to_string()
                                    } else {
                                        "Verified ledger".to_string()
                                    },
                                    readiness,
                                )
                            }
                            Some(VerifiedPublisher {
                                error: Some(reason),
                                ..
                            }) => (
                                format!("Ledger did not verify: {reason}"),
                                0,
                                "ledger_unverified".to_string(),
                                "Ledger unverified".to_string(),
                                vec!["ledger: unverified".to_string()],
                            ),
                            None => (
                                if has_snapshot {
                                    "Snapshot present; no ledger record".to_string()
                                } else {
                                    "No verified source".to_string()
                                },
                                0,
                                "snapshot_only".to_string(),
                                "No verified ledger".to_string(),
                                vec!["snapshot: present".to_string()],
                            ),
                        };
                        CustomerRow {
                            account_id: publisher.clone(),
                            publisher_id: publisher.clone(),
                            display_name: publisher,
                            lifecycle,
                            platform_readiness,
                            health,
                            owner: None,
                            stage_age_days,
                            last_activity,
                            contact_masked: None,
                            running_tests,
                            billing_state,
                        }
                    })
                    .collect())
            }
        }
    }

    /// The verified records of one live publisher, for the customer detail.
    pub fn customer_records(
        &self,
        publisher: &str,
    ) -> Result<Vec<VerifiedRecordView>, Unavailable> {
        match self {
            ControlData::Fixtures(_) => Err(Unavailable::new(
                "the illustrative dataset has no verified ledger record source",
            )),
            ControlData::Live(live) => live.durable.verified_records(publisher),
        }
    }

    /// Invoices for one live publisher.
    pub fn customer_invoices(&self, publisher: &str) -> Result<Vec<InvoiceRecord>, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo
                .invoices()
                .into_iter()
                .filter(|invoice| invoice.publisher_id == publisher)
                .collect()),
            ControlData::Live(live) => Ok(live
                .durable
                .invoices()?
                .into_iter()
                .filter(|invoice| invoice.publisher_id == publisher)
                .collect()),
        }
    }

    /// Live audit inputs from the runner snapshot root, with an honest
    /// eligibility state derived only from what is present.
    pub fn audit_publishers(&self, as_of_ms: i64) -> Result<Vec<AuditPublisherView>, Unavailable> {
        let publishers = match self {
            ControlData::Fixtures(_) => {
                return Err(Unavailable::new(
                    "the illustrative dataset has no snapshot root",
                ))
            }
            ControlData::Live(live) => live.durable.publisher_snapshots(as_of_ms)?,
        };
        Ok(publishers
            .into_iter()
            .map(|publisher| {
                let mut reason = None;
                let mut present = 0usize;
                let mut ad_units = 0usize;
                for platform in &publisher.platforms {
                    match (&platform.error, platform.ad_units) {
                        (Some(error), _) => {
                            reason.get_or_insert_with(|| format!("{}: {error}", platform.platform));
                        }
                        (None, Some(count)) => {
                            present += 1;
                            ad_units += count;
                        }
                        (None, None) => {}
                    }
                }
                let (eligibility, eligibility_reason) = if let Some(reason) = reason {
                    ("Inputs unavailable".to_string(), Some(reason))
                } else if present == 0 {
                    (
                        "Incomplete inputs".to_string(),
                        Some("no platform snapshot parsed".to_string()),
                    )
                } else if ad_units == 0 {
                    (
                        "Incomplete inputs".to_string(),
                        Some("the snapshots hold no ad units".to_string()),
                    )
                } else {
                    (
                        format!("Eligible inputs: {present} platform snapshot(s)"),
                        None,
                    )
                };
                AuditPublisherView {
                    publisher: publisher.publisher,
                    platforms: publisher.platforms,
                    eligibility,
                    eligibility_reason,
                }
            })
            .collect())
    }

    /// The copied-ledger sync state. Fixtures carry one illustrative recorded
    /// marker two hours before the pinned demo clock; the live mode reads the
    /// sync marker file fresh on every request, so a timer update is visible
    /// without a restart.
    pub fn ledger_copy(&self, as_of_ms: i64) -> LedgerCopy {
        match self {
            ControlData::Fixtures(_) => LedgerCopy::Recorded {
                synced_at_ms: crate::fixtures::DEMO_NOW_MS - 7_200_000,
            },
            ControlData::Live(live) => read_sync_marker(live.sync_marker.as_deref(), as_of_ms),
        }
    }

    /// The verified-operation evidence.
    pub fn operations_evidence(&self) -> Result<OperationsEvidence, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo.operations_evidence()),
            ControlData::Live(live) => live.operations.clone(),
        }
    }

    /// The verified-operation matrix rows.
    pub fn readiness_rows(&self) -> Vec<ReadinessRow> {
        let evidence = self.operations_evidence().unwrap_or_default();
        readiness_rows(&evidence)
    }

    /// The decision checks of one experiment from the verified ledger records.
    pub fn decision_checks(
        &self,
        publisher: &str,
        experiment_id: &str,
    ) -> Result<Vec<DecisionCheck>, Unavailable> {
        match self {
            ControlData::Fixtures(_) => Err(Unavailable::new(
                "the illustrative dataset has no verified decision-check source",
            )),
            ControlData::Live(live) => live
                .durable
                .experiment_decision_checks(publisher, experiment_id),
        }
    }

    /// The live per-day ingestion coverage matrix.
    pub fn ingestion_coverage(
        &self,
        as_of_ms: i64,
        window_days: u32,
    ) -> Result<Vec<CoveragePlatformRow>, Unavailable> {
        match self {
            ControlData::Fixtures(_) => Err(Unavailable::new(
                "the illustrative dataset has no verified ledger coverage source",
            )),
            ControlData::Live(live) => live.durable.ingestion_coverage(as_of_ms, window_days),
        }
    }

    /// The live monthly service economics.
    pub fn monthly_economics(&self) -> Result<Vec<LiveEconomicsMonth>, Unavailable> {
        match self {
            ControlData::Fixtures(_) => Err(Unavailable::new(
                "the illustrative dataset has no verified ledger fee source",
            )),
            ControlData::Live(live) => live.durable.monthly_economics(),
        }
    }

    /// The payment timeline of one invoice.
    pub fn invoice_payments(
        &self,
        invoice: &InvoiceRecord,
    ) -> Result<InvoicePayments, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => {
                let rows = demo
                    .payments()
                    .iter()
                    .filter(|payment| payment.invoice_id == invoice.id())
                    .map(InvoicePaymentRow::from)
                    .collect();
                Ok(InvoicePayments {
                    invoice_id: invoice.id(),
                    rows,
                    ledger_note: None,
                })
            }
            ControlData::Live(live) => Ok(live.durable.invoice_payments(invoice)),
        }
    }

    /// A money helper for pages: micros plus explicit currency.
    pub fn money(micros: i64, currency: &str) -> Money {
        Money::new(micros, currency)
    }
}

/// `^[A-Za-z0-9_-]{1,64}$`: the only names that become ledger file names.
fn is_valid_publisher(publisher: &str) -> bool {
    !publisher.is_empty()
        && publisher.len() <= 64
        && publisher
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// A bounded, non-secret description of one verified ledger record.
fn record_detail(line: &serde_json::Value) -> String {
    let kind = line
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    match kind {
        "changeApplied" => format!(
            "ad unit {} changed",
            line.get("adUnitId")
                .and_then(|value| value.as_str())
                .unwrap_or("(not named)")
        ),
        "experimentStarted" => format!(
            "experiment {} started",
            line.get("experimentId")
                .and_then(|value| value.as_str())
                .unwrap_or("(not named)")
        ),
        "experimentObserved" => format!(
            "experiment {} observed ({})",
            line.get("experimentId")
                .and_then(|value| value.as_str())
                .unwrap_or("(not named)"),
            line.get("result")
                .and_then(|result| result.get("state"))
                .and_then(|value| value.as_str())
                .unwrap_or("state not recorded")
        ),
        "rollbackPerformed" => format!(
            "sequence {} rolled back",
            line.get("rolledBackSeq")
                .and_then(|value| value.as_u64())
                .unwrap_or(0)
        ),
        "billingProof" => format!(
            "billing proof for experiment {}",
            line.get("experimentId")
                .and_then(|value| value.as_str())
                .unwrap_or("(not named)")
        ),
        "liftRecord" => "signed lift record".to_string(),
        _ => kind.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_core::{
        ExperimentMetric, ExperimentResult, ExperimentSpec, ExperimentState, PlatformId,
    };
    use apex_proof::{Keypair, Ledger, LedgerRecord};

    fn test_key() -> Keypair {
        Keypair::from_bytes(&[7u8; 32])
    }

    fn write_records(path: &Path, key: &Keypair, records: Vec<LedgerRecord>) {
        let mut ledger = Ledger::open(path, key.clone()).expect("open ledger");
        for record in records {
            ledger.append(record).expect("append record");
        }
    }

    fn reader_for(dir: &Path, publishers: &[&str], key: &Keypair) -> DurableReader {
        let mut keys = std::collections::HashMap::new();
        for publisher in publishers {
            keys.insert((*publisher).to_string(), key.verifying_key());
        }
        DurableReader::new(dir, dir.join("invoices.jsonl")).with_ledger_keys(keys)
    }

    fn lift(at: i64, payload: &str) -> LedgerRecord {
        LedgerRecord::lift_record(at, &"aa".repeat(32), payload)
    }

    fn started(at: i64, experiment_id: &str, name: &str) -> LedgerRecord {
        LedgerRecord::experiment_started(
            at,
            PlatformId::AppLovinMax,
            experiment_id,
            ExperimentSpec {
                name: name.to_string(),
                ad_unit_id: "unit".to_string(),
                metric: ExperimentMetric::Ecpm,
                variants: 2,
                change_description: "change".to_string(),
                test_group_allocation: 25,
            },
        )
    }

    fn observed(
        at: i64,
        experiment_id: &str,
        state: ExperimentState,
        lift_bp: i64,
        confidence: i32,
    ) -> LedgerRecord {
        LedgerRecord::experiment_observed(
            at,
            PlatformId::AppLovinMax,
            experiment_id,
            ExperimentResult {
                experiment_id: experiment_id.to_string(),
                state,
                results: Vec::new(),
                lift_percent_bp: Some(lift_bp),
                confidence: Some(confidence),
                winner_variant_id: None,
            },
        )
    }

    fn billing(at: i64, publisher: &str, experiment_id: &str, fee: i64) -> LedgerRecord {
        // The proof is validated: `fee = proven_lift * rate_bp / 10_000`, so
        // the proven lift is derived from the fee under the 2_000 bp rate.
        let proven_lift = fee.saturating_mul(5);
        LedgerRecord::billing_proof(at, publisher, experiment_id, proven_lift, 2_000, fee)
    }

    fn at(time: &str) -> i64 {
        UtcMillis::parse_rfc3339(time)
            .expect("timestamp")
            .unix_millis()
    }

    fn write_snapshot(
        root: &Path,
        publisher: &str,
        platform: &str,
        fetched_at_ms: i64,
        units: usize,
    ) {
        let dir = root.join(publisher);
        std::fs::create_dir_all(&dir).expect("snapshot dir");
        let units: Vec<serde_json::Value> = (0..units)
            .map(|_| serde_json::json!({"adUnitId": "u"}))
            .collect();
        std::fs::write(
            dir.join(format!("{platform}.json")),
            serde_json::json!({"fetchedAtUnixMs": fetched_at_ms, "adUnits": units}).to_string(),
        )
        .expect("snapshot");
    }

    fn unique(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "apex-control-{label}-{}-{}",
            std::process::id(),
            crate::signing::random_hex_32()
        ))
    }

    #[test]
    fn ledger_parses_are_cached_until_the_file_changes() {
        let dir = std::env::temp_dir().join(format!(
            "apex-control-cache-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let ledger = dir.join("northstar.jsonl");
        let key = test_key();
        write_records(&ledger, &key, vec![lift(1, "exp_1")]);
        let reader = reader_for(&dir, &["northstar"], &key);

        let first = reader.ledger_files().expect("first read");
        assert_eq!(first.files.len(), 1);
        let second = reader.ledger_files().expect("second read");
        // The unchanged file reuses the cached parse (same allocation).
        assert!(Arc::ptr_eq(&first.files[0].1, &second.files[0].1));

        let mut signed = Ledger::open(&ledger, key.clone()).expect("open");
        signed.append(lift(2, "exp_2")).expect("external append");
        let third = reader.ledger_files().expect("third read");
        assert_eq!(third.files[0].1.len(), 2, "the changed file re-parses");
        assert!(!Arc::ptr_eq(&second.files[0].1, &third.files[0].1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A same-length rewrite whose mtime is restored (`touch -t`) must still
    /// re-parse: the Unix ctime is part of the cache key. A pair-keyed cache
    /// serves the stale lines here; this test fails on it.
    #[test]
    fn ledger_cache_detects_a_same_length_rewrite_with_restored_mtime() {
        let dir = std::env::temp_dir().join(format!(
            "apex-control-ctime-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let ledger = dir.join("northstar.jsonl");
        let key = test_key();
        write_records(&ledger, &key, vec![lift(1, "exp_1")]);
        let replacement_dir = dir.join("replacement");
        std::fs::create_dir_all(&replacement_dir).expect("replacement dir");
        let replacement_path = replacement_dir.join("replacement.jsonl");
        write_records(&replacement_path, &key, vec![lift(1, "exp_2")]);
        let original = std::fs::read(&ledger).expect("original bytes");
        let replacement = std::fs::read(&replacement_path).expect("replacement bytes");
        assert_eq!(original.len(), replacement.len(), "same length");
        let reader = reader_for(&dir, &["northstar"], &key);

        let first = reader.ledger_files().expect("first read");
        assert_eq!(
            first.files[0].1[0]
                .get("payload")
                .and_then(|value| value.as_str()),
            Some("exp_1")
        );
        let modified = std::fs::metadata(&ledger)
            .expect("stat")
            .modified()
            .expect("mtime");
        std::fs::write(&ledger, replacement).expect("rewrite");
        std::fs::File::options()
            .write(true)
            .open(&ledger)
            .expect("open")
            .set_modified(modified)
            .expect("restore mtime");
        assert_eq!(
            std::fs::metadata(&ledger)
                .expect("stat")
                .modified()
                .expect("mtime"),
            modified,
            "the mtime is restored to the pair-keyed value"
        );

        let second = reader.ledger_files().expect("second read");
        assert!(
            !Arc::ptr_eq(&first.files[0].1, &second.files[0].1),
            "the restored-mtime rewrite re-parses"
        );
        assert_eq!(
            second.files[0].1[0]
                .get("payload")
                .and_then(|value| value.as_str()),
            Some("exp_2"),
            "the new content is parsed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invoice_parses_are_cached_until_the_register_changes() {
        let dir = std::env::temp_dir().join(format!(
            "apex-control-invoice-cache-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let register = dir.join("invoices.jsonl");
        let line = |publisher: &str| {
            format!(
                "{{\"publisherId\":\"{publisher}\",\"period\":{{\"yearMonth\":\"2026-10\",\"startUnixMs\":1,\"endUnixMs\":2}},\"lineItems\":[],\"totalMicros\":0,\"vatRateBp\":0,\"vatMicros\":0,\"grossMicros\":0}}\n"
            )
        };
        std::fs::write(&register, line("northstar")).expect("write");
        let reader = DurableReader::new(dir.join("ledgers"), &register);

        let first = reader.invoices().expect("first read");
        assert_eq!(first.len(), 1);
        let second = reader.invoices().expect("second read");
        assert_eq!(second, first);
        assert_eq!(reader.invoice_parse_count(), 1, "the register parsed once");

        let mut changed = line("northstar");
        changed.push_str(&line("rival"));
        std::fs::write(&register, changed).expect("append");
        let third = reader.invoices().expect("third read");
        assert_eq!(third.len(), 2, "the changed register re-parses");
        assert_eq!(reader.invoice_parse_count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ledger_cache_prunes_files_removed_from_the_directory() {
        let dir = std::env::temp_dir().join(format!(
            "apex-control-ledger-prune-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let key = test_key();
        write_records(&dir.join("northstar.jsonl"), &key, vec![lift(1, "exp_1")]);
        write_records(&dir.join("rival.jsonl"), &key, vec![lift(2, "exp_2")]);
        let reader = reader_for(&dir, &["northstar", "rival"], &key);

        assert_eq!(reader.ledger_files().expect("read").files.len(), 2);
        assert_eq!(reader.ledger_cache_len(), 2);
        std::fs::remove_file(dir.join("rival.jsonl")).expect("remove");
        assert_eq!(reader.ledger_files().expect("re-read").files.len(), 1);
        assert_eq!(
            reader.ledger_cache_len(),
            1,
            "the removed ledger's parse is pruned"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invoice_lines_parse_the_billing_serialization() {
        let line = r#"{"publisherId":"northstar","period":{"yearMonth":"2026-10","startUnixMs":1,"endUnixMs":2},"lineItems":[{"seq":11,"experimentId":"exp_0247","provenLift":10000000000,"rateBp":2000,"feeMicros":300000000,"vatMicros":60000000,"grossMicros":360000000}],"totalMicros":300000000,"vatRateBp":2000,"vatMicros":60000000,"grossMicros":360000000}"#;
        let record = parse_invoice_line(line).expect("parse");
        assert_eq!(record.publisher_id, "northstar");
        assert_eq!(record.period, "2026-10");
        assert_eq!(record.line_items.len(), 1);
        assert_eq!(record.line_items[0].fee_micros, 300_000_000);
        assert_eq!(record.id(), "INV-2026-10-northstar");
    }

    #[test]
    fn a_malformed_invoice_line_names_the_missing_field() {
        let error = parse_invoice_line(r#"{"publisherId":"x"}"#).expect_err("missing");
        assert!(error.contains("period"), "{error}");
    }

    #[test]
    fn durable_reads_name_the_absent_roots() {
        let dir = std::env::temp_dir().join(format!("apex-control-nodata-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let reader = DurableReader::new(dir.join("ledgers"), dir.join("invoices.jsonl"));
        let error = reader.experiments().expect_err("absent");
        assert!(error.reason.contains("no ledger root"));
        let error = reader.invoices().expect_err("absent");
        assert!(error.reason.contains("no invoice register"));
    }

    #[test]
    fn durable_experiments_merge_started_and_observed_records() {
        let dir = std::env::temp_dir().join(format!("apex-control-ledger-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let key = test_key();
        write_records(
            &dir.join("northstar.jsonl"),
            &key,
            vec![
                started(1_788_426_060_000, "exp_0246", "de-floor-raise"),
                observed(
                    1_790_842_140_000,
                    "exp_0246",
                    ExperimentState::Completed,
                    120,
                    8800,
                ),
            ],
        );
        let reader = reader_for(&dir, &["northstar"], &key);
        let experiments = reader.experiments().expect("experiments");
        assert_eq!(experiments.len(), 1);
        let experiment = &experiments[0];
        assert_eq!(experiment.experiment_id, "exp_0246");
        assert_eq!(experiment.name.as_deref(), Some("de-floor-raise"));
        assert_eq!(experiment.state.as_deref(), Some("completed"));
        assert_eq!(experiment.lift_bp, Some(120));
        assert_eq!(experiment.confidence, Some(8800));
        assert_eq!(experiment.observed_at_ms, Some(1_790_842_140_000));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn durable_experiments_keep_the_same_id_separate_across_publishers() {
        let dir = std::env::temp_dir().join(format!(
            "apex-control-ledger-shared-id-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let key = test_key();
        write_records(
            &dir.join("alice.jsonl"),
            &key,
            vec![
                started(1_000, "u1", "alice-started"),
                observed(2_000, "u1", ExperimentState::Running, 50, 7000),
            ],
        );
        write_records(
            &dir.join("bob.jsonl"),
            &key,
            vec![
                started(1_500, "u1", "bob-started"),
                observed(2_500, "u1", ExperimentState::Stopped, 200, 9000),
            ],
        );
        let reader = reader_for(&dir, &["alice", "bob"], &key);
        let experiments = reader.experiments().expect("experiments");
        assert_eq!(experiments.len(), 2, "one row per publisher, not a merge");
        let alice_row = experiments
            .iter()
            .find(|experiment| experiment.publisher == "alice")
            .expect("alice row");
        let bob_row = experiments
            .iter()
            .find(|experiment| experiment.publisher == "bob")
            .expect("bob row");
        assert_eq!(alice_row.experiment_id, "u1");
        assert_eq!(bob_row.experiment_id, "u1");
        assert_eq!(alice_row.name.as_deref(), Some("alice-started"));
        assert_eq!(alice_row.state.as_deref(), Some("running"));
        assert_eq!(alice_row.lift_bp, Some(50));
        assert_eq!(alice_row.confidence, Some(7000));
        assert_eq!(alice_row.observed_at_ms, Some(2000));
        assert_eq!(bob_row.name.as_deref(), Some("bob-started"));
        assert_eq!(bob_row.state.as_deref(), Some("stopped"));
        assert_eq!(bob_row.lift_bp, Some(200));
        assert_eq!(bob_row.confidence, Some(9000));
        assert_eq!(bob_row.observed_at_ms, Some(2500));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn durable_experiments_do_not_cross_update_a_shared_id() {
        let dir = std::env::temp_dir().join(format!(
            "apex-control-ledger-shared-id-only-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        // Alice only starts; bob starts and is observed. Alice's row must
        // stay untouched (state None) while bob's row carries the result.
        let key = test_key();
        write_records(
            &dir.join("alice.jsonl"),
            &key,
            vec![started(1_000, "u1", "alice-started")],
        );
        write_records(
            &dir.join("bob.jsonl"),
            &key,
            vec![
                started(1_500, "u1", "bob-started"),
                observed(2_500, "u1", ExperimentState::Stopped, 200, 9000),
            ],
        );
        let reader = reader_for(&dir, &["alice", "bob"], &key);
        let experiments = reader.experiments().expect("experiments");
        assert_eq!(experiments.len(), 2);
        let alice_row = experiments
            .iter()
            .find(|experiment| experiment.publisher == "alice")
            .expect("alice row");
        let bob_row = experiments
            .iter()
            .find(|experiment| experiment.publisher == "bob")
            .expect("bob row");
        assert_eq!(alice_row.state, None);
        assert_eq!(alice_row.lift_bp, None);
        assert_eq!(alice_row.observed_at_ms, None);
        assert_eq!(bob_row.state.as_deref(), Some("stopped"));
        assert_eq!(bob_row.lift_bp, Some(200));
        assert_eq!(bob_row.observed_at_ms, Some(2500));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn readiness_marks_everything_unavailable_without_evidence() {
        let rows = readiness_rows(&OperationsEvidence::default());
        assert_eq!(
            rows.len(),
            MAX_ENDPOINTS.len() + LEVELPLAY_ENDPOINTS.len() + ADMOB_ENDPOINTS.len()
        );
        for row in &rows {
            assert!(!row.available, "{}/{}", row.platform, row.action);
            assert_eq!(row.sandbox_evidence, "Not on file");
            assert_eq!(row.release_version, "Not on file");
        }
        let max_list = rows
            .iter()
            .find(|row| row.platform == "applovin_max" && row.action == "updateFloors")
            .expect("updateFloors");
        assert_eq!(max_list.adapter_implemented, "Verify live");
        let reads = rows
            .iter()
            .find(|row| row.platform == "applovin_max" && row.action == "listAdUnits")
            .expect("listAdUnits");
        assert_eq!(reads.adapter_implemented, "Yes (documented)");
    }

    #[test]
    fn readiness_requires_complete_evidence_to_claim_availability() {
        let evidence = OperationsEvidence {
            entries: vec![OperationEvidence {
                platform: "applovin_max".to_string(),
                action: "updateFloors".to_string(),
                sandbox_evidence: true,
                allowed_deployment: Some("production".to_string()),
                credential_scope: Some("publisher management key".to_string()),
                approval_required: true,
                release_version: Some("0.9.0".to_string()),
            }],
        };
        let rows = readiness_rows(&evidence);
        let row = rows
            .iter()
            .find(|row| row.platform == "applovin_max" && row.action == "updateFloors")
            .expect("row");
        // The endpoint metadata still says verify-live, so the action cannot
        // be available although evidence exists; the matrix shows both
        // facts.
        assert_eq!(row.adapter_implemented, "Verify live");
        assert_eq!(row.sandbox_evidence, "Confirmed");
        assert_eq!(row.approval_requirement, "Required");
        assert!(!row.available);
        // An endpoint with no evidence remains unavailable regardless.
        let list = rows
            .iter()
            .find(|row| row.platform == "applovin_max" && row.action == "listAdUnits")
            .expect("row");
        assert!(!list.available);
    }

    #[tokio::test]
    async fn fixtures_mode_serves_the_demo_dataset_and_live_mode_is_honest() {
        let fixtures = ControlData::fixtures();
        assert!(fixtures.is_fixtures());
        let live = ControlData::Live(Box::new(LiveData {
            analytics: None,
            durable: DurableReader::new("/nonexistent-ledgers", "/nonexistent-invoices.jsonl"),
            operations: Ok(OperationsEvidence::default()),
            sync_marker: None,
        }));
        assert!(!live.is_fixtures());
        let as_of = UtcMillis::from_unix_millis(0);
        let error = live.queues(as_of).await.expect_err("no analytics");
        assert!(
            error.reason.contains("APEX_CLICKHOUSE_HTTP"),
            "{}",
            error.reason
        );
        let error = live.durable_experiments().expect_err("no ledgers");
        assert!(error.reason.contains("no ledger root"));
    }

    #[test]
    fn the_sync_marker_line_is_exact_or_honestly_absent() {
        // A fixed clock: 2026-10-16T15:00:00Z.
        let as_of = 1_792_162_800_000i64;
        let synced = 1_792_152_000_000i64; // 2026-10-16T12:00:00Z
        let dir = std::env::temp_dir().join(format!(
            "apex-control-syncmarker-{}-{}",
            std::process::id(),
            crate::signing::random_hex_32()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("sync.json");

        // Present: the exact line, with whole-hour age.
        std::fs::write(
            &path,
            format!("{{\"synced_at_unix_ms\":{synced}, \"source\":\"ledger\"}}"),
        )
        .expect("write");
        let state = read_sync_marker(Some(&path), as_of);
        assert_eq!(
            state,
            LedgerCopy::Recorded {
                synced_at_ms: synced
            }
        );
        assert_eq!(
            state.system_line(as_of),
            "Ledger copy synced: 16 Oct 2026, 12:00 UTC (age 3 h)"
        );

        // The age floors to whole hours.
        let one_ms_late = LedgerCopy::Recorded {
            synced_at_ms: synced + 1,
        };
        assert_eq!(
            one_ms_late.system_line(as_of),
            "Ledger copy synced: 16 Oct 2026, 12:00 UTC (age 2 h)"
        );

        // Unset path and an absent file: the honest not-recorded state.
        assert_eq!(read_sync_marker(None, as_of), LedgerCopy::NotRecorded);
        assert_eq!(
            read_sync_marker(Some(&dir.join("absent.json")), as_of),
            LedgerCopy::NotRecorded
        );
        assert_eq!(
            LedgerCopy::NotRecorded.system_line(as_of),
            "Ledger copy synced: not recorded"
        );

        // Malformed shapes are typed unreadable states, never a panic.
        for (label, body) in [
            ("not json", "{ not json".to_string()),
            ("missing time", "{\"source\":\"ledger\"}".to_string()),
            (
                "unknown source",
                format!("{{\"synced_at_unix_ms\":{synced},\"source\":\"customers\"}}"),
            ),
            (
                "future time",
                format!(
                    "{{\"synced_at_unix_ms\":{},\"source\":\"ledger\"}}",
                    as_of + 1
                ),
            ),
        ] {
            std::fs::write(&path, body).expect("write");
            let state = read_sync_marker(Some(&path), as_of);
            assert!(
                matches!(state, LedgerCopy::Unreadable { .. }),
                "{label}: {state:?}"
            );
            let line = state.system_line(as_of);
            assert!(line.contains("not recorded"), "{label}: {line}");
        }

        // An extreme recorded time cannot overflow the age computation.
        std::fs::write(
            &path,
            format!(
                "{{\"synced_at_unix_ms\":{},\"source\":\"ledger\"}}",
                i64::MIN
            ),
        )
        .expect("write");
        let state = read_sync_marker(Some(&path), as_of);
        assert_eq!(
            state,
            LedgerCopy::Recorded {
                synced_at_ms: i64::MIN
            }
        );
        assert!(state.system_line(as_of).starts_with("Ledger copy synced: "));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fixtures_carry_a_deterministic_recorded_sync_marker() {
        let as_of = crate::fixtures::DEMO_NOW_MS;
        let fixtures = ControlData::fixtures();
        assert_eq!(
            fixtures.ledger_copy(as_of),
            LedgerCopy::Recorded {
                synced_at_ms: as_of - 7_200_000
            }
        );
        let live = ControlData::Live(Box::new(LiveData {
            analytics: None,
            durable: DurableReader::new("/nonexistent-ledgers", "/nonexistent-invoices.jsonl"),
            operations: Ok(OperationsEvidence::default()),
            sync_marker: None,
        }));
        assert_eq!(live.ledger_copy(as_of), LedgerCopy::NotRecorded);
    }

    #[tokio::test]
    async fn site_requests_are_a_fixture_value_and_absent_without_analytics() {
        let as_of = UtcMillis::from_unix_millis(0);
        let fixtures = ControlData::fixtures();
        assert_eq!(
            fixtures.site_requests_last_30_days(as_of).await,
            Some(18_240)
        );
        // No analytics store configured: the read is absent, never zero.
        let live = ControlData::Live(Box::new(LiveData {
            analytics: None,
            durable: DurableReader::new("/nonexistent-ledgers", "/nonexistent-invoices.jsonl"),
            operations: Ok(OperationsEvidence::default()),
            sync_marker: None,
        }));
        assert_eq!(live.site_requests_last_30_days(as_of).await, None);
    }

    /// At the reference size (2k ledgers, 20k records) the merged projection
    /// is built once; every later page read reuses the same allocation until
    /// a ledger's hardened stamp changes. Removing a ledger prunes its parse.
    #[test]
    fn experiment_projection_is_cached_at_two_thousand_ledgers_and_invalidated() {
        let dir = std::env::temp_dir().join(format!(
            "apex-control-projection-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let ledgers = dir.join("ledgers");
        std::fs::create_dir_all(&ledgers).expect("dir");
        let key = test_key();
        let mut keys = std::collections::HashMap::new();
        for publisher in 0..2_000usize {
            let name = format!("publisher{publisher}");
            keys.insert(name.clone(), key.verifying_key());
            let mut records = vec![started(
                (publisher * 100) as i64,
                &format!("exp_{publisher}"),
                "E",
            )];
            for observation in 1..=9usize {
                records.push(observed(
                    (publisher * 100 + observation) as i64,
                    &format!("exp_{publisher}"),
                    ExperimentState::Running,
                    10,
                    90,
                ));
            }
            write_records(&ledgers.join(format!("{name}.jsonl")), &key, records);
        }
        let reader =
            DurableReader::new(&ledgers, dir.join("invoices.jsonl")).with_ledger_keys(keys);

        let started_at = std::time::Instant::now();
        let first = reader.experiments().expect("first projection");
        let first_elapsed = started_at.elapsed();
        assert_eq!(first.len(), 2_000);
        assert_eq!(reader.experiment_build_count(), 1);
        assert_eq!(reader.ledger_cache_len(), 2_000);

        let shared_a = reader.experiments_shared().expect("cached");
        let shared_b = reader.experiments_shared().expect("cached again");
        assert!(Arc::ptr_eq(&shared_a, &shared_b), "one shared projection");
        assert_eq!(reader.experiment_build_count(), 1, "no rebuild on read");
        let stats_after_first = reader.ledger_stat_count();
        let cached_started = std::time::Instant::now();
        for _ in 0..10 {
            let shared = reader.experiments_shared().expect("cached read");
            assert!(Arc::ptr_eq(&shared_a, &shared));
        }
        let cached_ms = cached_started.elapsed().as_secs_f64() * 1_000.0 / 10.0;
        let stats_per_cached_read = (reader.ledger_stat_count() - stats_after_first) / 10;
        assert_eq!(
            stats_per_cached_read, 2_000,
            "one stat per present ledger keeps external writes visible"
        );

        // An external append changes one ledger's stamp and invalidates.
        let ledger = ledgers.join("publisher0.jsonl");
        let mut signed = Ledger::open(&ledger, key.clone()).expect("open ledger");
        signed
            .append(started(999_999, "exp_new", "New"))
            .expect("append");
        assert_eq!(reader.experiments().expect("rebuilt").len(), 2_001);
        assert_eq!(reader.experiment_build_count(), 2);

        // Removing a ledger prunes its cached parse (no unbounded index).
        std::fs::remove_file(ledgers.join("publisher1.jsonl")).expect("remove");
        assert_eq!(reader.experiments().expect("pruned").len(), 2_000);
        assert_eq!(reader.experiment_build_count(), 3);
        assert_eq!(reader.ledger_cache_len(), 1_999);

        eprintln!(
            "experiment_projection files=2000 records=20000 build_ms={:.2} cached_read_ms={cached_ms:.2} stats_per_read={stats_per_cached_read} builds={} cache_entries={}",
            first_elapsed.as_secs_f64() * 1_000.0,
            reader.experiment_build_count(),
            reader.ledger_cache_len()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ingestion_coverage_reports_records_by_day_and_snapshot_presence() {
        let dir = unique("coverage");
        let _ = std::fs::remove_dir_all(&dir);
        let ledgers = dir.join("ledgers");
        std::fs::create_dir_all(&ledgers).expect("dir");
        let snapshots = dir.join("snapshots");
        let key = test_key();
        let day_one = at("2026-10-14T10:00:00Z");
        let day_two = at("2026-10-15T09:00:00Z");
        write_records(
            &ledgers.join("northstar.jsonl"),
            &key,
            vec![
                started(day_one, "exp_cov", "coverage"),
                observed(day_one, "exp_cov", ExperimentState::Running, 10, 90),
                billing(day_two, "northstar", "exp_cov", 500_000),
            ],
        );
        write_snapshot(
            &snapshots,
            "northstar",
            "applovin_max",
            day_two - 3_600_000,
            2,
        );
        write_snapshot(&snapshots, "northstar", "levelplay", day_two - 7_200_000, 1);
        let reader = reader_for(&ledgers, &["northstar"], &key).with_snapshot_root(&snapshots);
        let as_of = at("2026-10-15T09:10:00Z");
        let rows = reader.ingestion_coverage(as_of, 14).expect("coverage");
        assert_eq!(
            rows.len(),
            3,
            "two ledger platforms plus a snapshot-only row"
        );

        let max_row = rows
            .iter()
            .find(|row| row.publisher == "northstar" && row.platform == "applovin_max")
            .expect("max row");
        assert_eq!(max_row.days.len(), 14, "one cell per day in the window");
        let day_one_cell = max_row
            .days
            .iter()
            .find(|cell| cell.day == "2026-10-14")
            .expect("day one cell");
        assert_eq!(day_one_cell.records, 2, "started + observed");
        assert_eq!(day_one_cell.audits, 0);
        assert_eq!(day_one_cell.last_verified_at_ms, Some(day_one));
        let day_two_cell = max_row
            .days
            .iter()
            .find(|cell| cell.day == "2026-10-15")
            .expect("day two cell");
        assert_eq!(day_two_cell.records, 0, "no ledger record that day");
        assert_eq!(day_two_cell.last_verified_at_ms, None);
        assert!(max_row.snapshot_present);
        assert_eq!(max_row.snapshot_ad_units, Some(2));
        // as_of is 09:10 and the applovin snapshot was fetched at 08:00.
        assert_eq!(max_row.snapshot_age_ms, Some(4_200_000));
        assert_eq!(max_row.total_records, 2);

        // The platform-agnostic billing proof lands on the `-` row.
        let billing_row = rows
            .iter()
            .find(|row| row.platform == "-")
            .expect("billing row");
        assert_eq!(billing_row.total_records, 1);
        assert_eq!(
            billing_row.total_audits, 1,
            "billingProof is audit evidence"
        );
        assert!(!billing_row.snapshot_present);
        // A snapshot-only platform is present with explicit zero ledger cells.
        let levelplay = rows
            .iter()
            .find(|row| row.platform == "levelplay")
            .expect("levelplay row");
        assert_eq!(levelplay.total_records, 0);
        assert!(levelplay.snapshot_present);
        assert!(levelplay.days.iter().all(|cell| cell.records == 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ingestion_coverage_is_honest_about_missing_roots() {
        let dir = unique("coverage-absent");
        let _ = std::fs::remove_dir_all(&dir);
        let reader = DurableReader::new(dir.join("ledgers"), dir.join("invoices.jsonl"))
            .with_snapshot_root(dir.join("snapshots"));
        let error = reader
            .ingestion_coverage(at("2026-10-15T09:10:00Z"), 14)
            .expect_err("no roots");
        assert!(
            error.reason.contains("no coverage source"),
            "{}",
            error.reason
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn decision_checks_cover_registered_decided_and_settled() {
        let dir = unique("decisions");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let key = test_key();

        // Registered only.
        write_records(
            &dir.join("northstar.jsonl"),
            &key,
            vec![started(1_000, "exp_reg", "registered-only")],
        );
        let reader = reader_for(&dir, &["northstar"], &key);
        let checks = reader
            .experiment_decision_checks("northstar", "exp_reg")
            .expect("registered");
        assert_eq!(checks.len(), 3);
        assert_eq!(checks[0].check, "Registration");
        assert_eq!(checks[0].verdict, "Pass");
        assert_eq!(checks[1].verdict, "Pending", "no result yet");
        assert_eq!(checks[2].verdict, "Not settled");

        // Decided: an observed final result.
        write_records(
            &dir.join("orbit.jsonl"),
            &key,
            vec![
                started(1_000, "exp_dec", "decided"),
                observed(2_000, "exp_dec", ExperimentState::Completed, 120, 9_800),
            ],
        );
        let reader = reader_for(&dir, &["northstar", "orbit"], &key);
        let checks = reader
            .experiment_decision_checks("orbit", "exp_dec")
            .expect("decided");
        assert_eq!(checks[1].verdict, "Pass");
        assert!(
            checks[1].observed.contains("completed"),
            "{}",
            checks[1].observed
        );
        assert_eq!(checks[2].verdict, "Not settled");

        // Settled: a billing proof.
        write_records(
            &dir.join("comet.jsonl"),
            &key,
            vec![
                started(1_000, "exp_set", "settled"),
                observed(2_000, "exp_set", ExperimentState::Completed, 120, 9_800),
                billing(3_000, "comet", "exp_set", 600_000),
            ],
        );
        let reader = reader_for(&dir, &["northstar", "orbit", "comet"], &key);
        let checks = reader
            .experiment_decision_checks("comet", "exp_set")
            .expect("settled");
        assert_eq!(checks[2].verdict, "Pass");
        assert!(
            checks[2].observed.contains("600000"),
            "{}",
            checks[2].observed
        );

        // An unknown experiment and an invalid publisher are typed errors.
        assert!(reader
            .experiment_decision_checks("comet", "exp_missing")
            .is_err());
        assert!(reader.experiment_decision_checks("../evil", "x").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn monthly_economics_matches_the_hand_computed_fixture() {
        let dir = unique("economics");
        let _ = std::fs::remove_dir_all(&dir);
        let ledgers = dir.join("ledgers");
        std::fs::create_dir_all(&ledgers).expect("dir");
        let key = test_key();
        let july = at("2026-07-15T00:00:00Z");
        assert_eq!(
            UtcMillis::from_unix_millis(july).to_month_string(),
            "2026-07"
        );
        write_records(
            &ledgers.join("northstar.jsonl"),
            &key,
            vec![
                billing(july, "northstar", "exp_1", 100_000),
                billing(july, "northstar", "exp_2", 250_000),
            ],
        );
        std::fs::write(
            dir.join("invoices.jsonl"),
            concat!(
                r#"{"publisherId":"northstar","period":{"yearMonth":"2026-09","startUnixMs":1,"endUnixMs":2},"lineItems":[],"totalMicros":500000,"vatRateBp":2000,"vatMicros":100000,"grossMicros":600000}"#,
                "\n",
                r#"{"publisherId":"northstar","period":{"yearMonth":"2026-09","startUnixMs":1,"endUnixMs":2},"lineItems":[],"totalMicros":250000,"vatRateBp":2000,"vatMicros":50000,"grossMicros":300000}"#,
                "\n"
            ),
        )
        .expect("register");
        let mut reader = reader_for(&ledgers, &["northstar"], &key);
        reader.invoice_register = dir.join("invoices.jsonl");
        let months = reader.monthly_economics().expect("economics");
        assert_eq!(
            months
                .iter()
                .map(|month| month.month.as_str())
                .collect::<Vec<_>>(),
            vec!["2026-07", "2026-08", "2026-09"],
            "the empty month is explicit"
        );
        let july_row = &months[0];
        assert_eq!(july_row.ledger_fee_micros, 350_000, "100000 + 250000");
        assert_eq!(july_row.ledger_fee_records, 2);
        assert_eq!(july_row.net_micros, 0, "no invoice in July");
        assert_eq!(july_row.credits_micros, None, "no credit record exists");
        let august_row = &months[1];
        assert!(august_row.is_empty(), "August carries no record");
        let september_row = &months[2];
        assert_eq!(september_row.net_micros, 750_000, "500000 + 250000");
        assert_eq!(september_row.gross_micros, 900_000, "600000 + 300000");
        assert_eq!(september_row.invoice_records, 2);
        assert_eq!(september_row.ledger_fee_micros, 0);
        assert_eq!(september_row.currency, "USD");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn monthly_economics_names_both_missing_sources() {
        let dir = unique("economics-absent");
        let _ = std::fs::remove_dir_all(&dir);
        let reader = DurableReader::new(dir.join("ledgers"), dir.join("invoices.jsonl"));
        let error = reader.monthly_economics().expect_err("no sources");
        assert!(
            error.reason.contains("no verified ledger"),
            "{}",
            error.reason
        );
        assert!(
            error.reason.contains("no invoice register"),
            "{}",
            error.reason
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invoice_payments_are_built_from_the_register_and_ledger() {
        let dir = unique("invoice-payments");
        let _ = std::fs::remove_dir_all(&dir);
        let ledgers = dir.join("ledgers");
        std::fs::create_dir_all(&ledgers).expect("dir");
        let key = test_key();
        write_records(
            &ledgers.join("northstar.jsonl"),
            &key,
            vec![billing(5_000, "northstar", "exp_inv", 300_000)],
        );
        let invoice = InvoiceRecord {
            publisher_id: "northstar".to_string(),
            period: "2026-10".to_string(),
            start_unix_ms: 1,
            end_unix_ms: 2,
            line_items: vec![InvoiceLineRecord {
                seq: 1,
                experiment_id: "exp_inv".to_string(),
                proven_lift: 1_000,
                rate_bp: 2_000,
                fee_micros: 300_000,
                vat_micros: 60_000,
                gross_micros: 360_000,
            }],
            total_micros: 300_000,
            vat_rate_bp: 2_000,
            vat_micros: 60_000,
            gross_micros: 360_000,
        };
        let reader = reader_for(&ledgers, &["northstar"], &key);
        let payments = reader.invoice_payments(&invoice);
        assert_eq!(payments.invoice_id, "INV-2026-10-northstar");
        assert_eq!(payments.rows.len(), 3, "issued + fee proof + absence row");
        assert_eq!(payments.ledger_note, None);
        let settled = payments
            .rows
            .iter()
            .find(|row| row.event_code == "Verified fee proof")
            .expect("fee proof");
        assert_eq!(settled.amount_micros, Some(300_000));
        assert_eq!(settled.reference_masked.as_deref(), Some("exp_inv"));
        let payment = payments
            .rows
            .iter()
            .find(|row| row.event_code == "Payment received")
            .expect("payment row");
        assert_eq!(payment.received, None, "a stage with no source says so");
        assert!(payment.note.is_some());
        assert!(payment
            .note
            .as_deref()
            .is_some_and(|note| note.contains("No payment")));

        // A missing ledger still yields the register row and the honest note.
        let absent = DurableReader::new(dir.join("absent"), dir.join("absent.jsonl"));
        let payments = absent.invoice_payments(&invoice);
        assert!(payments.ledger_note.is_some());
        assert!(payments
            .rows
            .iter()
            .any(|row| row.event_code == "Invoice issued"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
