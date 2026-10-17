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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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

use crate::fixtures::DemoData;

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

/// The read-only durable files the runner keeps.
#[derive(Debug, Clone)]
pub struct DurableReader {
    /// Directory of `<publisher>.jsonl` ledgers.
    pub ledger_root: PathBuf,
    /// Invoice register JSONL path.
    pub invoice_register: PathBuf,
    /// Parsed ledgers keyed by path, reused while the file identity (length,
    /// mtime and on Unix ctime and inode) is unchanged, so repeated page
    /// reads do not re-parse the whole directory.
    cache: Arc<Mutex<HashMap<PathBuf, CachedLedger>>>,
}

/// One parsed publisher ledger: the publisher name and its parsed lines.
type LedgerLines = (String, Arc<Vec<serde_json::Value>>);

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

impl DurableReader {
    /// A reader over the given roots.
    pub fn new(ledger_root: impl Into<PathBuf>, invoice_register: impl Into<PathBuf>) -> Self {
        Self {
            ledger_root: ledger_root.into(),
            invoice_register: invoice_register.into(),
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Every `<publisher>.jsonl` ledger file, as (publisher, lines).
    fn ledger_files(&self) -> Result<Vec<LedgerLines>, Unavailable> {
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
        let mut files = Vec::new();
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
            let metadata = std::fs::metadata(&path).map_err(|error| {
                Unavailable::new(format!("the ledger {name} could not be read: {error}"))
            })?;
            let stamp = FileStamp::of(&metadata);
            let cached = {
                let cache = self
                    .cache
                    .lock()
                    .map_err(|_| Unavailable::new("ledger cache lock poisoned"))?;
                cache
                    .get(&path)
                    .filter(|cached| cached.stamp == stamp)
                    .map(|cached| cached.lines.clone())
            };
            let lines = match cached {
                Some(lines) => lines,
                None => {
                    let raw = std::fs::read_to_string(&path).map_err(|error| {
                        Unavailable::new(format!("the ledger {name} could not be read: {error}"))
                    })?;
                    let mut lines = Vec::new();
                    for (index, line) in raw.lines().enumerate() {
                        if line.trim().is_empty() {
                            continue;
                        }
                        let value: serde_json::Value =
                            serde_json::from_str(line).map_err(|error| {
                                Unavailable::new(format!(
                                    "the ledger {name} line {} did not parse: {error}",
                                    index + 1
                                ))
                            })?;
                        lines.push(value);
                    }
                    let lines = Arc::new(lines);
                    let mut cache = self
                        .cache
                        .lock()
                        .map_err(|_| Unavailable::new("ledger cache lock poisoned"))?;
                    cache.insert(
                        path,
                        CachedLedger {
                            stamp,
                            lines: lines.clone(),
                        },
                    );
                    lines
                }
            };
            files.push((publisher.to_string(), lines));
        }
        if files.is_empty() {
            return Err(Unavailable::new(format!(
                "the ledger root {} holds no publisher ledgers",
                self.ledger_root.display()
            )));
        }
        files.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(files)
    }

    /// The experiment records the ledgers carry, read-only.
    pub fn experiments(&self) -> Result<Vec<DurableExperiment>, Unavailable> {
        let files = self.ledger_files()?;
        let mut experiments: Vec<DurableExperiment> = Vec::new();
        // Experiment ids are only unique per publisher: two publishers may
        // reuse one id, so the merge index is keyed by (publisher, id).
        let mut index: HashMap<(String, String), usize> = HashMap::new();
        for (publisher, lines) in &files {
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
        Ok(experiments)
    }

    /// The invoice register, read-only.
    pub fn invoices(&self) -> Result<Vec<InvoiceRecord>, Unavailable> {
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
        Ok(invoices)
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

    /// Durable invoice register.
    pub fn invoices(&self) -> Result<Vec<InvoiceRecord>, Unavailable> {
        match self {
            ControlData::Fixtures(demo) => Ok(demo.invoices()),
            ControlData::Live(live) => live.durable.invoices(),
        }
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

    /// A money helper for pages: micros plus explicit currency.
    pub fn money(micros: i64, currency: &str) -> Money {
        Money::new(micros, currency)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        std::fs::write(
            &ledger,
            "{\"publisherId\":\"northstar\",\"type\":\"liftRecord\",\"experimentId\":\"exp_1\"}\n",
        )
        .expect("write");
        let reader = DurableReader::new(&dir, dir.join("invoices.jsonl"));

        let first = reader.ledger_files().expect("first read");
        assert_eq!(first.len(), 1);
        let second = reader.ledger_files().expect("second read");
        // The unchanged file reuses the cached parse (same allocation).
        assert!(Arc::ptr_eq(&first[0].1, &second[0].1));

        std::fs::write(
            &ledger,
            "{\"publisherId\":\"northstar\",\"type\":\"liftRecord\",\"experimentId\":\"exp_1\"}\n{\"publisherId\":\"northstar\",\"type\":\"liftRecord\",\"experimentId\":\"exp_2\"}\n",
        )
        .expect("append");
        let third = reader.ledger_files().expect("third read");
        assert_eq!(third[0].1.len(), 2, "the changed file re-parses");
        assert!(!Arc::ptr_eq(&second[0].1, &third[0].1));
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
        let original =
            "{\"publisherId\":\"northstar\",\"type\":\"liftRecord\",\"experimentId\":\"exp_1\"}\n";
        let replacement =
            "{\"publisherId\":\"northstar\",\"type\":\"liftRecord\",\"experimentId\":\"exp_2\"}\n";
        assert_eq!(original.len(), replacement.len(), "same length");
        std::fs::write(&ledger, original).expect("write");
        let reader = DurableReader::new(&dir, dir.join("invoices.jsonl"));

        let first = reader.ledger_files().expect("first read");
        assert_eq!(
            first[0].1[0]
                .get("experimentId")
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
            !Arc::ptr_eq(&first[0].1, &second[0].1),
            "the restored-mtime rewrite re-parses"
        );
        assert_eq!(
            second[0].1[0]
                .get("experimentId")
                .and_then(|value| value.as_str()),
            Some("exp_2"),
            "the new content is parsed"
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
        let lines = [
            r#"{"type":"experimentStarted","seq":2,"atUnixMs":1788426060000,"platform":"applovin_max","experimentId":"exp_0246","spec":{"name":"de-floor-raise","adUnitId":"rw_de_01","metric":"ecpm","variants":2,"testGroupAllocation":25,"changeDescription":"raise the DE floor"},"prevHash":"0","hash":"0"}"#,
            r#"{"type":"experimentObserved","seq":4,"atUnixMs":1790842140000,"platform":"applovin_max","experimentId":"exp_0246","result":{"experimentId":"exp_0246","state":"completed","results":[],"liftPercentBp":120,"confidence":8800}}"#,
        ];
        std::fs::write(dir.join("northstar.jsonl"), lines.join("\n")).expect("write");
        let reader = DurableReader::new(&dir, dir.join("invoices.jsonl"));
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
        let alice = [
            r#"{"type":"experimentStarted","atUnixMs":1000,"experimentId":"u1","spec":{"name":"alice-started"}}"#,
            r#"{"type":"experimentObserved","atUnixMs":2000,"experimentId":"u1","result":{"state":"running","liftPercentBp":50,"confidence":7000}}"#,
        ];
        let bob = [
            r#"{"type":"experimentStarted","atUnixMs":1500,"experimentId":"u1","spec":{"name":"bob-started"}}"#,
            r#"{"type":"experimentObserved","atUnixMs":2500,"experimentId":"u1","result":{"state":"stopped","liftPercentBp":200,"confidence":9000}}"#,
        ];
        std::fs::write(dir.join("alice.jsonl"), alice.join("\n")).expect("write alice");
        std::fs::write(dir.join("bob.jsonl"), bob.join("\n")).expect("write bob");
        let reader = DurableReader::new(&dir, dir.join("invoices.jsonl"));
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
        std::fs::write(
            dir.join("alice.jsonl"),
            r#"{"type":"experimentStarted","atUnixMs":1000,"experimentId":"u1","spec":{"name":"alice-started"}}"#,
        )
        .expect("write alice");
        std::fs::write(
            dir.join("bob.jsonl"),
            [
                r#"{"type":"experimentStarted","atUnixMs":1500,"experimentId":"u1","spec":{"name":"bob-started"}}"#,
                r#"{"type":"experimentObserved","atUnixMs":2500,"experimentId":"u1","result":{"state":"stopped","liftPercentBp":200,"confidence":9000}}"#,
            ]
            .join("\n"),
        )
        .expect("write bob");
        let reader = DurableReader::new(&dir, dir.join("invoices.jsonl"));
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
}
