//! Experiments: the read-only projection of the runner's signed records.
//!
//! One experiment is assembled from the verified ledger (its
//! `experimentStarted` record, the signed `LiftRecord` payloads:
//! pre-registration, data pulls, guardrails and the result) plus, when one
//! exists, the runner's alert log (`APEX_ALERT_FILE`, the `FileAlertSink`
//! output): a `failedRollback` alert is the restore-failed incident, shown
//! with the frozen customer-facing message and a stable incident reference.
//!
//! States are observations, never guesses:
//!
//! - `Restore failed` when the alert log carries a failed rollback for the
//!   experiment;
//! - `Kept` / `Stopped` / `Voided` / `Extended` from the signed result
//!   record's decision;
//! - `Decision due` when the recorded analysis window is complete but no
//!   signed result exists;
//! - `Waiting for data` when the window is not complete, with the missing
//!   final days counted from the signed data-pull records;
//! - `Burn-in / measuring` while the registered burn-in has not elapsed;
//! - `Registered` when only the pre-registration exists (the platform
//!   experiment has not started).
//!
//! A positive interim estimate never turns a row green and never creates a
//! fee; only the recorded decision does.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use apex_autopilot::alerts::Alert;
use apex_core::PlatformId;
use apex_lift::records::{GuardrailRecord, LiftRecord};
use apex_proof::LedgerRecord;

use crate::error::ApiError;

/// The runner's alert log path (the `FileAlertSink` output), read-only. This
/// is the documented name the runner and the deployment units use.
pub const ALERT_FILE_ENV: &str = "APEX_ALERT_FILE";
/// The historical name of the same setting, read only as a fallback.
pub const ALERTS_FILE_ENV: &str = "APEX_ALERTS_FILE";
/// Read cap for the alert log.
pub const MAX_ALERTS_BYTES: u64 = 4 * 1024 * 1024;
const DAY_MS: i64 = 86_400_000;

/// Resolve the alert log path from the environment: `APEX_ALERT_FILE` (the
/// documented name, the one the runner writes) first, the historical
/// `APEX_ALERTS_FILE` as a fallback with one logged line when it is the only
/// name set.
pub fn alerts_file_from_env(lookup: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    if let Some(path) = lookup(ALERT_FILE_ENV).filter(|value| !value.trim().is_empty()) {
        return Some(path);
    }
    let fallback = lookup(ALERTS_FILE_ENV).filter(|value| !value.trim().is_empty())?;
    tracing::warn!(
        "alert log configured through {ALERTS_FILE_ENV}; the documented name is {ALERT_FILE_ENV}"
    );
    Some(fallback)
}

/// The lifecycle state of one experiment, exactly as the console renders it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExperimentStateView {
    /// Only the pre-registration exists.
    Registered,
    /// Started, burn-in not elapsed.
    BurnIn,
    /// The window is incomplete; `missing_days` final days are missing.
    WaitingForData {
        /// Final analysis days still missing, when computable.
        missing_days: Option<u32>,
    },
    /// The window is complete, no signed result yet.
    DecisionDue,
    /// One registered extension is in use.
    Extended,
    /// The decision rule passed.
    Kept,
    /// The test failed its rule or a harm guard fired.
    Stopped,
    /// The test cannot establish a valid result.
    Voided,
    /// The requested stop or restore did not complete.
    RestoreFailed,
}

impl ExperimentStateView {
    /// The short state label (the list column).
    pub fn label(&self) -> &'static str {
        match self {
            ExperimentStateView::Registered => "Registered",
            ExperimentStateView::BurnIn => "Burn-in / measuring",
            ExperimentStateView::WaitingForData { .. } => "Waiting for data",
            ExperimentStateView::DecisionDue => "Decision due",
            ExperimentStateView::Extended => "Extended",
            ExperimentStateView::Kept => "Kept",
            ExperimentStateView::Stopped => "Stopped",
            ExperimentStateView::Voided => "Voided",
            ExperimentStateView::RestoreFailed => "Restore failed",
        }
    }

    /// What becomes billable in this state (the frozen table, page 45).
    pub fn billing_behavior(&self) -> &'static str {
        match self {
            ExperimentStateView::Registered
            | ExperimentStateView::BurnIn
            | ExperimentStateView::WaitingForData { .. }
            | ExperimentStateView::DecisionDue => "Nothing.",
            ExperimentStateView::Extended => "Nothing until a later qualifying Keep.",
            ExperimentStateView::Kept => {
                "Only the recorded billable bound and applicable fee terms."
            }
            ExperimentStateView::Stopped => "No lift fee for this result.",
            ExperimentStateView::Voided => "Nothing.",
            ExperimentStateView::RestoreFailed => {
                "Do not infer a kept or restored outcome. Review disputed fees explicitly."
            }
        }
    }

    /// What the customer sees in this state (the frozen table, page 45).
    pub fn customer_sees(&self) -> &'static str {
        match self {
            ExperimentStateView::Registered => {
                "Test rules recorded. The platform experiment has not started."
            }
            ExperimentStateView::BurnIn => {
                "Data is accumulating. Show planned window and finalized-day count."
            }
            ExperimentStateView::WaitingForData { .. } | ExperimentStateView::DecisionDue => {
                "A required input is incomplete or reporting is paused. Show reason and next check."
            }
            ExperimentStateView::Extended => {
                "One registered extension is being used. Show the new final date."
            }
            ExperimentStateView::Kept => {
                "The decision rule passed. Show estimate, positive lower bound, guards and source."
            }
            ExperimentStateView::Stopped => {
                "The test failed its rule or a harm guard fired. Show confirmed platform outcome."
            }
            ExperimentStateView::Voided => {
                "The test cannot establish a valid result. Show edit, mismatch or incident reason."
            }
            ExperimentStateView::RestoreFailed => {
                "The requested stop or restore did not complete. Show incident and current known state."
            }
        }
    }

    /// The decision-rule row this state maps to (the decisions-are-states
    /// table of the design document).
    pub fn decision_state(&self) -> &'static str {
        match self {
            ExperimentStateView::Registered => "Registered",
            ExperimentStateView::BurnIn => "Burn-in / measuring",
            ExperimentStateView::WaitingForData { .. } => "Pending",
            ExperimentStateView::DecisionDue => "Pending",
            ExperimentStateView::Extended => "Extended",
            ExperimentStateView::Kept => "Kept",
            ExperimentStateView::Stopped => "Rolled back / stopped",
            ExperimentStateView::Voided => "Voided",
            ExperimentStateView::RestoreFailed => "Restore failed",
        }
    }
}

/// The signed result of one experiment.
#[derive(Debug, Clone, PartialEq)]
pub struct ExperimentResultView {
    /// Point estimate in basis points.
    pub observed_lift_bp: Option<i64>,
    /// The engine's lower bound (the billable bound on a Keep).
    pub lower_90: f64,
    /// Lower bound at the interim look.
    pub lower_99: f64,
    /// Bootstrap lower bound.
    pub bootstrap_lower: f64,
    /// Signed decision (`keep`, `rollback`, `stop`, `void`, `extend`).
    pub decision: String,
    /// Billable lambda from the decision.
    pub lambda: f64,
    /// Daily log ratios the decision was made on.
    pub daily_log_ratios: Vec<f64>,
    /// Ledger sequence of the signed result record.
    pub seq: u64,
    /// Payload digest of the signed result record.
    pub payload_sha256: String,
    /// When the record was written.
    pub at_unix_ms: i64,
}

/// The restore-failed incident of one experiment (from the runner's alert
/// log).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncidentView {
    /// Stable console reference derived from the alert bytes.
    pub reference: String,
    /// When the alert was recorded.
    pub recorded_at_unix_ms: i64,
    /// The last known state, from the ledger's latest change for the scope.
    pub previous_state: String,
    /// When that state was recorded (the preceding change).
    pub previous_recorded_at_unix_ms: Option<i64>,
}

/// The frozen customer-facing message of the restore-failed state.
pub const RESTORE_FAILED_MESSAGE: &str =
    "The platform refused the stop request. Apex has not marked the test restored.";
/// The frozen headline of the recovery state.
pub const RESTORE_FAILED_HEADLINE: &str = "This test needs intervention";
/// The next step of the incident row (frozen copy).
pub const INCIDENT_NEXT_STEP: &str = "Operator investigating";
/// The next step of the previous-settings row (frozen copy).
pub const PREVIOUS_STATE_NEXT_STEP: &str = "Review before manual action";

/// One experiment as the console renders it.
#[derive(Debug, Clone, PartialEq)]
pub struct ExperimentView {
    /// Experiment identity.
    pub experiment_id: String,
    /// Platform the experiment started on.
    pub platform: Option<PlatformId>,
    /// Lifecycle state.
    pub state: ExperimentStateView,
    /// The registered scope (ad unit).
    pub scope: Option<String>,
    /// Registered metric label; `None` when no registration is stored.
    pub metric: Option<String>,
    /// Planned test-arm share (exploratory when 0.1).
    pub test_share: Option<f64>,
    /// Burn-in days.
    pub burn_in_days: Option<u32>,
    /// Analysis window days.
    pub window_days: Option<u32>,
    /// Engine version hash the pre-registration pinned.
    pub rule_version: Option<String>,
    /// Contract rate in basis points.
    pub rate_bp: Option<u16>,
    /// When the experiment started (ledger time).
    pub started_at_unix_ms: Option<i64>,
    /// Finalized data-pull records: their analysis days.
    pub data_pull_days: Vec<String>,
    /// The signed result, when one exists.
    pub result: Option<ExperimentResultView>,
    /// Guardrail records (fired checks), in order.
    pub guardrails: Vec<GuardrailRecord>,
    /// Registered guardrail thresholds as recorded (JSON), when present.
    pub guardrail_thresholds: Option<serde_json::Value>,
    /// Ledger sequence of the `experimentStarted` record.
    pub started_seq: Option<u64>,
    /// The restore-failed incident, when the alert log carries one.
    pub incident: Option<IncidentView>,
    /// The scope's latest change before the incident (previous settings row).
    pub previous_settings_recorded: bool,
}

impl ExperimentView {
    /// The metric definition the console shows: honest about what MAX can
    /// report (no invented ARPDAU or DAU).
    pub fn metric_definition(&self) -> String {
        match self.metric.as_deref() {
            Some(metric) => format!(
                "{metric}: platform-reported revenue divided by the allocated share. The \
                 allocation-normalized revenue metric is used because the platform has no user \
                 counts to divide by."
            ),
            None => "Not available (no pre-registration record is stored)".to_string(),
        }
    }

    /// The next step line of the list row.
    pub fn next_step(&self) -> String {
        match &self.state {
            ExperimentStateView::WaitingForData {
                missing_days: Some(days),
            } => format!("{days} final days missing"),
            ExperimentStateView::WaitingForData { missing_days: None } => "Waiting for data".into(),
            ExperimentStateView::DecisionDue => "The analysis window is complete".into(),
            ExperimentStateView::Kept => "View fee basis".into(),
            ExperimentStateView::Stopped => "No lift fee".into(),
            ExperimentStateView::RestoreFailed => "Open incident".into(),
            ExperimentStateView::Registered => {
                "Waiting for the platform experiment to start".into()
            }
            ExperimentStateView::BurnIn => "Data is accumulating".into(),
            ExperimentStateView::Extended => "Show the new final date".into(),
            ExperimentStateView::Voided => "Show the void reason".into(),
        }
    }

    /// True when this row matches the `finalized` list filter (a final
    /// outcome exists or is due; nothing is still accumulating).
    pub fn is_finalized(&self) -> bool {
        matches!(
            self.state,
            ExperimentStateView::Kept
                | ExperimentStateView::Stopped
                | ExperimentStateView::Voided
                | ExperimentStateView::DecisionDue
                | ExperimentStateView::RestoreFailed
        )
    }

    /// The allocation label, honest about the exploratory split.
    pub fn allocation_label(&self) -> String {
        match self.test_share {
            Some(share) if (share - 0.10).abs() < 1e-9 => "Exploratory 10% split".to_string(),
            Some(share) => format!("{}% test allocation", (share * 100.0).round() as i64),
            None => "Not available (no pre-registration record is stored)".to_string(),
        }
    }
}

/// Project every experiment from the verified records and the alert log.
pub fn project(
    records: &[LedgerRecord],
    alerts: &[Alert],
    now_unix_ms: i64,
) -> Vec<ExperimentView> {
    let mut views: Vec<ExperimentView> = Vec::new();
    // The id-to-index map keeps every lookup O(1): a linear scan per record
    // would make the projection quadratic in experiments × records.
    let mut index_by_id: HashMap<String, usize> = HashMap::new();
    // Indices, in view order, of the experiments a DataPull or Guardrail
    // belongs to: those with a recorded start and no result yet. Both
    // transitions are one-way (a start is only ever added, a result only ever
    // set), so a sorted set of open indices is exact and keeps the
    // most-recently-seen experiment one `next_back` away.
    let mut open: BTreeSet<usize> = BTreeSet::new();
    let index_of = |views: &mut Vec<ExperimentView>,
                    index_by_id: &mut HashMap<String, usize>,
                    id: &str|
     -> usize {
        if let Some(index) = index_by_id.get(id) {
            return *index;
        }
        views.push(ExperimentView {
            experiment_id: id.to_string(),
            platform: None,
            state: ExperimentStateView::Registered,
            scope: None,
            metric: None,
            test_share: None,
            burn_in_days: None,
            window_days: None,
            rule_version: None,
            rate_bp: None,
            started_at_unix_ms: None,
            data_pull_days: Vec::new(),
            result: None,
            guardrails: Vec::new(),
            guardrail_thresholds: None,
            started_seq: None,
            incident: None,
            previous_settings_recorded: false,
        });
        let index = views.len() - 1;
        index_by_id.insert(id.to_string(), index);
        index
    };

    for record in records {
        match record {
            LedgerRecord::ExperimentStarted {
                seq,
                at_unix_ms,
                platform,
                experiment_id,
                ..
            } => {
                let index = index_of(&mut views, &mut index_by_id, experiment_id);
                let view = &mut views[index];
                view.platform = Some(*platform);
                view.started_at_unix_ms = Some(*at_unix_ms);
                view.started_seq = Some(*seq);
                open.insert(index);
            }
            LedgerRecord::LiftRecord {
                seq,
                at_unix_ms,
                payload,
                payload_sha256,
                ..
            } => {
                let Ok(lift) = serde_json::from_str::<LiftRecord>(payload) else {
                    continue;
                };
                match lift {
                    LiftRecord::PreRegistration {
                        experiment_id,
                        scope,
                        split,
                        metric,
                        burn_in_days,
                        window_days,
                        code_version_hash,
                        rate_bp,
                        guardrail_thresholds,
                        ..
                    } => {
                        let index = index_of(&mut views, &mut index_by_id, &experiment_id);
                        let view = &mut views[index];
                        view.scope = Some(scope);
                        view.test_share = Some(split);
                        view.metric = Some(metric);
                        view.burn_in_days = Some(burn_in_days);
                        view.window_days = Some(window_days);
                        view.rule_version = Some(code_version_hash);
                        view.rate_bp = Some(rate_bp);
                        view.guardrail_thresholds = Some(guardrail_thresholds);
                    }
                    LiftRecord::DataPull { day, .. } => {
                        // Data pulls are keyed by experiment through the
                        // record order: the runner writes them inside the
                        // experiment's analysis flow, so the most recently
                        // seen experiment without a result owns them.
                        if let Some(&index) = open.iter().next_back() {
                            views[index].data_pull_days.push(day);
                        }
                    }
                    LiftRecord::Guardrail(guardrail) => {
                        if let Some(&index) = open.iter().next_back() {
                            views[index].guardrails.push(guardrail);
                        }
                    }
                    LiftRecord::Result {
                        experiment_id,
                        daily_log_ratios,
                        mean_log,
                        lower_90,
                        lower_99,
                        bootstrap_lower,
                        decision,
                        lambda,
                    } => {
                        let index = index_of(&mut views, &mut index_by_id, &experiment_id);
                        let view = &mut views[index];
                        let point_bp = if mean_log.is_finite() {
                            Some((mean_log.exp_m1() * 10_000.0).round() as i64)
                        } else {
                            None
                        };
                        view.result = Some(ExperimentResultView {
                            observed_lift_bp: point_bp,
                            lower_90,
                            lower_99,
                            bootstrap_lower,
                            decision,
                            lambda,
                            daily_log_ratios,
                            seq: *seq,
                            payload_sha256: payload_sha256.clone(),
                            at_unix_ms: *at_unix_ms,
                        });
                        open.remove(&index);
                    }
                    LiftRecord::MonthlyBilling(_) | LiftRecord::BillingChange(_) => {}
                }
            }
            _ => {}
        }
    }

    // The runner's failed-rollback alerts become the restore-failed
    // incidents; the reference is derived from the alert's own bytes so it is
    // stable across renders. The incident time is the newest record time for
    // the experiment, precomputed here in one pass so many alerts do not each
    // re-walk the ledger.
    let newest_by_experiment = newest_record_times(records);
    for alert in alerts {
        if let Alert::FailedRollback {
            experiment_id,
            detail,
        } = alert
        {
            let index = index_of(&mut views, &mut index_by_id, experiment_id);
            let reference = incident_reference(experiment_id, detail);
            let recorded_at = newest_by_experiment
                .get(experiment_id.as_str())
                .copied()
                .unwrap_or(now_unix_ms);
            views[index].incident = Some(IncidentView {
                reference,
                recorded_at_unix_ms: recorded_at,
                previous_state: "Test still reported active".to_string(),
                previous_recorded_at_unix_ms: None,
            });
        }
    }

    // The latest recorded change for an incident's scope is the "previous
    // settings are recorded" row of the recovery state. Incident views are
    // indexed by scope first so each change record touches only the views of
    // its own ad unit (a scan of every incident per record would be quadratic
    // in records × incidents).
    let mut incidents_by_scope: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, view) in views.iter().enumerate() {
        if view.incident.is_some() {
            if let Some(scope) = view.scope.clone() {
                incidents_by_scope.entry(scope).or_default().push(index);
            }
        }
    }
    for record in records {
        if let LedgerRecord::ChangeApplied {
            at_unix_ms,
            ad_unit_id,
            prev_value,
            ..
        } = record
        {
            if prev_value.trim().is_empty() {
                continue;
            }
            let Some(indices) = incidents_by_scope.get(ad_unit_id.as_str()) else {
                continue;
            };
            for &index in indices {
                let view = &mut views[index];
                let Some(incident) = &mut view.incident else {
                    continue;
                };
                if *at_unix_ms > incident.recorded_at_unix_ms {
                    continue;
                }
                view.previous_settings_recorded = true;
                if incident
                    .previous_recorded_at_unix_ms
                    .is_none_or(|current| *at_unix_ms > current)
                {
                    incident.previous_recorded_at_unix_ms = Some(*at_unix_ms);
                }
            }
        }
    }

    // Resolve the lifecycle states from what was observed.
    for view in &mut views {
        view.state = resolve_state(view, now_unix_ms);
    }
    views.sort_by(|a, b| a.experiment_id.cmp(&b.experiment_id));
    views
}

/// Derive the lifecycle state from the observed records.
fn resolve_state(view: &ExperimentView, now_unix_ms: i64) -> ExperimentStateView {
    if view.incident.is_some() {
        return ExperimentStateView::RestoreFailed;
    }
    if let Some(result) = &view.result {
        return match result.decision.as_str() {
            "keep" => ExperimentStateView::Kept,
            "rollback" | "stop" => ExperimentStateView::Stopped,
            "void" => ExperimentStateView::Voided,
            "extend" => ExperimentStateView::Extended,
            _ => ExperimentStateView::DecisionDue,
        };
    }
    let Some(started_at) = view.started_at_unix_ms else {
        return ExperimentStateView::Registered;
    };
    let burn_in_days = i64::from(view.burn_in_days.unwrap_or(0));
    let window_days = i64::from(view.window_days.unwrap_or(0));
    if window_days == 0 {
        // No registration record: nothing can be asserted beyond "started".
        return ExperimentStateView::WaitingForData { missing_days: None };
    }
    let elapsed_days = now_unix_ms.saturating_sub(started_at).max(0) / DAY_MS;
    if elapsed_days < burn_in_days {
        return ExperimentStateView::BurnIn;
    }
    // Only pulls inside the analysis window (after the burn-in) count as
    // final days; a burn-in pull is data accumulation, not a final day.
    let window_start_day = utc_day_string(started_at.saturating_add(burn_in_days * DAY_MS));
    let pulled = view
        .data_pull_days
        .iter()
        .filter(|day| day.as_str() >= window_start_day.as_str())
        .count() as i64;
    let missing = (window_days - pulled).max(0);
    if elapsed_days >= burn_in_days + window_days && missing == 0 {
        ExperimentStateView::DecisionDue
    } else {
        ExperimentStateView::WaitingForData {
            missing_days: Some(u32::try_from(missing).unwrap_or(u32::MAX)),
        }
    }
}

/// The UTC day (`YYYY-MM-DD`) of a Unix-millisecond timestamp.
fn utc_day_string(unix_ms: i64) -> String {
    apex_analytics::UtcMillis::from_unix_millis(unix_ms).to_day_string()
}

/// A stable incident reference from the alert's own bytes: the console's
/// reference for this unresolved fact.
pub fn incident_reference(experiment_id: &str, detail: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(experiment_id.as_bytes());
    hasher.update([0x1f]);
    hasher.update(detail.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("inc_{}", &digest[..10])
}

/// The newest record time per experiment id, from one pass over the ledger.
///
/// This is the single-pass form of "the incident time: the newest record time
/// for the experiment": each record that belongs to an experiment (a start, an
/// observation, or a lift payload naming it) advances that experiment's
/// newest time, and the map is consulted once per alert instead of re-walking
/// the whole ledger per alert.
fn newest_record_times(records: &[LedgerRecord]) -> HashMap<String, i64> {
    let mut newest: HashMap<String, i64> = HashMap::new();
    for record in records {
        let belongs: Option<String> = match record {
            LedgerRecord::ExperimentStarted {
                experiment_id: id, ..
            }
            | LedgerRecord::ExperimentObserved {
                experiment_id: id, ..
            } => Some(id.clone()),
            LedgerRecord::LiftRecord { payload, .. } => {
                match serde_json::from_str::<LiftRecord>(payload) {
                    Ok(LiftRecord::PreRegistration {
                        experiment_id: id, ..
                    })
                    | Ok(LiftRecord::Result {
                        experiment_id: id, ..
                    }) => Some(id),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(id) = belongs {
            newest
                .entry(id)
                .and_modify(|time| *time = (*time).max(record.at_unix_ms()))
                .or_insert_with(|| record.at_unix_ms());
        }
    }
    newest
}

/// One history entry: a signed record of the experiment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperimentHistoryEntry {
    /// Ledger sequence.
    pub seq: u64,
    /// Record time.
    pub at_unix_ms: i64,
    /// A short record label (`experimentStarted`, `preRegistration`, ...).
    pub label: &'static str,
}

/// The signed records of one experiment, in ledger order.
pub fn history(records: &[LedgerRecord], experiment_id: &str) -> Vec<ExperimentHistoryEntry> {
    let mut out = Vec::new();
    for record in records {
        let label = match record {
            LedgerRecord::ExperimentStarted {
                experiment_id: id, ..
            } if id == experiment_id => Some("experimentStarted"),
            LedgerRecord::ExperimentObserved {
                experiment_id: id, ..
            } if id == experiment_id => Some("experimentObserved"),
            LedgerRecord::LiftRecord { payload, .. } => {
                match serde_json::from_str::<LiftRecord>(payload) {
                    Ok(LiftRecord::PreRegistration {
                        experiment_id: id, ..
                    }) if id == experiment_id => Some("preRegistration"),
                    Ok(LiftRecord::DataPull { .. }) => None,
                    Ok(LiftRecord::Guardrail(_)) => None,
                    Ok(LiftRecord::Result {
                        experiment_id: id, ..
                    }) if id == experiment_id => Some("result"),
                    Ok(LiftRecord::MonthlyBilling(_)) | Ok(LiftRecord::BillingChange(_)) => None,
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(label) = label {
            out.push(ExperimentHistoryEntry {
                seq: record.seq(),
                at_unix_ms: record.at_unix_ms(),
                label,
            });
        }
    }
    out.sort_by_key(|entry| entry.seq);
    out
}

/// The detail-page chip label: the list state label, with the pending state
/// named as the design document names it on the detail page.
pub fn detail_chip_label(state: &ExperimentStateView) -> &'static str {
    match state {
        ExperimentStateView::WaitingForData { .. } | ExperimentStateView::DecisionDue => {
            "Awaiting final data"
        }
        other => other.label(),
    }
}

/// The console label of a platform.
pub fn platform_label(platform: Option<PlatformId>) -> &'static str {
    match platform {
        Some(PlatformId::AppLovinMax) => "AppLovin MAX",
        Some(PlatformId::LevelPlay) => "LevelPlay",
        Some(PlatformId::Admob) => "AdMob",
        Some(PlatformId::Billing) => "Billing",
        None => "Not available",
    }
}

/// Read the runner's alert log (the `FileAlertSink` JSONL). A missing file is
/// an empty log; a malformed line is a logged internal error, never a
/// fabricated incident.
pub fn read_alerts(path: &Path) -> Result<Vec<Alert>, ApiError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(ApiError::Internal(format!("alert log stat: {error}"))),
    };
    crate::sources::ensure_within_cap(metadata.len(), MAX_ALERTS_BYTES)?;
    let raw = std::fs::read_to_string(path)
        .map_err(|error| ApiError::Internal(format!("alert log read: {error}")))?;
    let mut alerts = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let alert: Alert = serde_json::from_str(line).map_err(|error| {
            ApiError::Internal(format!("alert log parse at line {}: {error}", index + 1))
        })?;
        alerts.push(alert);
    }
    Ok(alerts)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use apex_core::{ExperimentMetric, ExperimentSpec};
    use sha2::{Digest, Sha256};

    use super::*;

    const NOW: i64 = 1_792_055_400_000;

    fn seal_all(drafts: Vec<LedgerRecord>) -> Vec<LedgerRecord> {
        let mut ledger =
            apex_proof::InMemoryLedger::new(apex_proof::Keypair::from_bytes(&[9u8; 32]));
        drafts
            .into_iter()
            .map(|draft| ledger.append(draft).expect("seal"))
            .collect()
    }

    fn lift_record(payload: &str) -> LedgerRecord {
        let digest = hex::encode(Sha256::digest(payload.as_bytes()));
        LedgerRecord::lift_record(1_792_000_000_000, &digest, payload)
    }

    fn started(id: &str, at: i64) -> LedgerRecord {
        LedgerRecord::experiment_started(
            at,
            PlatformId::AppLovinMax,
            id,
            ExperimentSpec {
                name: format!("test-{id}"),
                ad_unit_id: "rw_de_01".into(),
                metric: ExperimentMetric::Ecpm,
                variants: 2,
                test_group_allocation: 25,
                change_description: "raise the DE floor".into(),
            },
        )
    }

    fn preregistration(id: &str) -> LedgerRecord {
        lift_record(&format!(
            r#"{{"type":"preRegistration","experiment_id":"{id}","scope":"rw_de_01","before_value":{{"DE":4000000}},"after_value":{{"DE":3200000}},"split":0.25,"metric":"RevenuePerUser","burn_in_days":3,"window_days":14,"looks":1,"extension_allowed":true,"guardrail_thresholds":{{"harmPoint":-0.05,"fillRewarded":0.02,"fillBanner":0.05,"splitChiSquare":10.83}},"code_version_hash":"apex-lift-test","rate_bp":2000}}"#
        ))
    }

    fn result_record(id: &str, decision: &str) -> LedgerRecord {
        lift_record(&format!(
            r#"{{"type":"result","experiment_id":"{id}","daily_log_ratios":[0.03,0.02,0.028],"mean_log":0.026,"lower_90":0.012,"lower_99":0.008,"bootstrap_lower":0.011,"decision":"{decision}","lambda":0.012}}"#
        ))
    }

    fn data_pull(day: &str) -> LedgerRecord {
        lift_record(&format!(
            r#"{{"type":"dataPull","day":"{day}","rows_sha256":"ab","rows":[]}}"#
        ))
    }

    #[test]
    fn a_started_experiment_with_a_registration_is_measuring_then_waiting() {
        let records = seal_all(vec![
            started("exp_0247", NOW - 4 * DAY_MS),
            preregistration("exp_0247"),
            // Inside the analysis window (burn-in 3 days): one final day.
            data_pull("2026-10-14"),
            // A burn-in pull does not count as a final day.
            data_pull("2026-10-12"),
        ]);
        let views = project(&records, &[], NOW);
        assert_eq!(views.len(), 1);
        let view = &views[0];
        assert_eq!(
            view.state,
            ExperimentStateView::WaitingForData {
                missing_days: Some(13)
            }
        );
        assert_eq!(view.scope.as_deref(), Some("rw_de_01"));
        assert_eq!(view.allocation_label(), "25% test allocation");
        assert_eq!(view.next_step(), "13 final days missing");
        assert!(view
            .metric_definition()
            .contains("allocation-normalized revenue"));
        assert!(view.metric_definition().contains("no user counts"));
    }

    #[test]
    fn the_window_completing_only_after_the_registered_days_marks_a_decision_due() {
        let records = seal_all(vec![
            started("exp_0247", NOW - 20 * DAY_MS),
            preregistration("exp_0247"),
        ]);
        let views = project(&records, &[], NOW);
        // No pulls at all: still waiting, with the whole window missing.
        assert_eq!(
            views[0].state,
            ExperimentStateView::WaitingForData {
                missing_days: Some(14)
            }
        );
    }

    #[test]
    fn burn_in_is_reported_while_it_runs() {
        let records = seal_all(vec![
            started("exp_1", NOW - DAY_MS),
            preregistration("exp_1"),
        ]);
        let views = project(&records, &[], NOW);
        assert_eq!(views[0].state, ExperimentStateView::BurnIn);
        assert_eq!(views[0].state.decision_state(), "Burn-in / measuring");
    }

    #[test]
    fn missing_final_days_are_counted_from_the_analysis_window() {
        let records = seal_all(vec![
            started("exp_2", NOW - 20 * DAY_MS),
            preregistration("exp_2"),
            data_pull("2026-09-28"),
            data_pull("2026-09-29"),
        ]);
        let views = project(&records, &[], NOW);
        // 20 - 3 = 17 final days expected, clamped to the 14-day window; 2 pulled.
        assert_eq!(
            views[0].state,
            ExperimentStateView::WaitingForData {
                missing_days: Some(12)
            }
        );
        assert_eq!(views[0].next_step(), "12 final days missing");
    }

    #[test]
    fn a_complete_window_without_a_result_is_a_decision_due() {
        let records = seal_all(vec![
            started("exp_3", NOW - 20 * DAY_MS),
            preregistration("exp_3"),
        ]);
        let mut drafts = records;
        for day in 0..14 {
            drafts.push(data_pull(&format!("2026-10-{day:02}")));
        }
        let sealed = seal_all(drafts);
        let views = project(&sealed, &[], NOW);
        assert_eq!(views[0].state, ExperimentStateView::DecisionDue);
        assert!(views[0].is_finalized());
    }

    #[test]
    fn a_kept_result_carries_the_estimate_bounds_and_signed_reference() {
        let records = seal_all(vec![
            started("exp_0246", NOW - 30 * DAY_MS),
            preregistration("exp_0246"),
            result_record("exp_0246", "keep"),
        ]);
        let views = project(&records, &[], NOW);
        let view = &views[0];
        assert_eq!(view.state, ExperimentStateView::Kept);
        let result = view.result.as_ref().expect("result");
        assert_eq!(result.decision, "keep");
        assert_eq!(result.daily_log_ratios.len(), 3);
        assert_eq!(result.payload_sha256.len(), 64);
        assert!(result.observed_lift_bp.unwrap() > 200);
        assert_eq!(view.next_step(), "View fee basis");
        assert!(view
            .state
            .billing_behavior()
            .contains("recorded billable bound"));
    }

    #[test]
    fn rollback_and_void_decisions_map_to_their_states() {
        for (decision, expected) in [
            ("rollback", ExperimentStateView::Stopped),
            ("void", ExperimentStateView::Voided),
            ("extend", ExperimentStateView::Extended),
            ("pending", ExperimentStateView::DecisionDue),
        ] {
            let records = seal_all(vec![
                started("exp_x", NOW - 30 * DAY_MS),
                preregistration("exp_x"),
                result_record("exp_x", decision),
            ]);
            assert_eq!(project(&records, &[], NOW)[0].state, expected, "{decision}");
        }
    }

    #[test]
    fn an_exploratory_split_is_labeled_and_never_billed() {
        let payload = String::from(
            r#"{"type":"preRegistration","experiment_id":"exp_x","scope":"u","before_value":1,"after_value":2,"split":0.1,"metric":"RevenuePerUser","burn_in_days":3,"window_days":14,"looks":1,"extension_allowed":true,"guardrail_thresholds":{"harmPoint":-0.05,"fillRewarded":0.02,"fillBanner":0.05,"splitChiSquare":10.83},"code_version_hash":"c","rate_bp":2000}"#,
        );
        let records = seal_all(vec![started("exp_x", NOW - DAY_MS), lift_record(&payload)]);
        let views = project(&records, &[], NOW);
        assert_eq!(views[0].allocation_label(), "Exploratory 10% split");
    }

    #[test]
    fn a_failed_rollback_alert_becomes_the_restore_failed_state() {
        let records = seal_all(vec![
            started("exp_0239", NOW - 14 * DAY_MS),
            preregistration("exp_0239"),
            LedgerRecord::change_applied(
                NOW - 13 * DAY_MS,
                PlatformId::AppLovinMax,
                "rw_de_01",
                apex_proof::ChangeKind::CountryFloorSet {
                    country: "DE".into(),
                    floor_ecpm: 4_000_000,
                },
                "key-1",
                "3200000",
            ),
        ]);
        let alerts = vec![Alert::FailedRollback {
            experiment_id: "exp_0239".into(),
            detail: "rollback also failed for max-floor:rw_de_01:DE:2".into(),
        }];
        let views = project(&records, &alerts, NOW);
        let view = &views[0];
        assert_eq!(view.state, ExperimentStateView::RestoreFailed);
        let incident = view.incident.as_ref().expect("incident");
        assert!(incident.reference.starts_with("inc_"));
        assert_eq!(incident.reference.len(), 14);
        assert!(view.previous_settings_recorded);
        assert!(incident.previous_recorded_at_unix_ms.is_some());
        assert!(view.state.billing_behavior().contains("disputed fees"));
        assert_eq!(view.next_step(), "Open incident");
        // The reference is stable across renders.
        let again = project(&records, &alerts, NOW);
        assert_eq!(
            again[0].incident.as_ref().expect("incident").reference,
            incident.reference
        );
    }

    #[test]
    fn guardrails_and_data_pulls_attach_to_the_open_experiment() {
        let guardrail = lift_record(
            r#"{"type":"guardrail","day":"2026-10-10","guardrail":"fastHarm","inputs":{"pointLift":-0.06},"action":"stopped"}"#,
        );
        let records = seal_all(vec![
            started("exp_1", NOW - 6 * DAY_MS),
            preregistration("exp_1"),
            guardrail,
            data_pull("2026-10-10"),
        ]);
        let views = project(&records, &[], NOW);
        assert_eq!(views[0].guardrails.len(), 1);
        assert_eq!(views[0].guardrails[0].guardrail, "fastHarm");
        assert_eq!(views[0].guardrails[0].action, "stopped");
        assert_eq!(views[0].data_pull_days, vec!["2026-10-10".to_string()]);
    }

    #[test]
    fn experiments_are_sorted_and_unrelated_records_are_ignored() {
        let records = seal_all(vec![
            started("exp_b", NOW - DAY_MS),
            started("exp_a", NOW - DAY_MS),
        ]);
        let views = project(&records, &[], NOW);
        assert_eq!(views[0].experiment_id, "exp_a");
        assert_eq!(views[1].experiment_id, "exp_b");
        assert!(project(&[], &[], NOW).is_empty());
    }

    #[test]
    fn alert_log_reads_jsonl_and_missing_files_are_empty() {
        let dir = std::env::temp_dir().join(format!("apex-alerts-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(read_alerts(&dir.join("missing.jsonl"))
            .expect("missing")
            .is_empty());
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("alerts.jsonl");
        std::fs::write(
            &path,
            "{\"kind\":\"failedRollback\",\"experiment_id\":\"exp_0239\",\"detail\":\"refused\"}\n",
        )
        .expect("write");
        let alerts = read_alerts(&path).expect("read");
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0], Alert::FailedRollback { .. }));
        std::fs::write(&path, "not json\n").expect("write");
        assert!(matches!(read_alerts(&path), Err(ApiError::Internal(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn alert_file_env_prefers_the_documented_name_with_a_fallback() {
        let both = |key: &str| match key {
            ALERT_FILE_ENV => Some("/documented.jsonl".to_string()),
            ALERTS_FILE_ENV => Some("/fallback.jsonl".to_string()),
            _ => None,
        };
        assert_eq!(
            alerts_file_from_env(&both).as_deref(),
            Some("/documented.jsonl")
        );
        let fallback_only = |key: &str| match key {
            ALERTS_FILE_ENV => Some("/fallback.jsonl".to_string()),
            _ => None,
        };
        assert_eq!(
            alerts_file_from_env(&fallback_only).as_deref(),
            Some("/fallback.jsonl"),
            "the runner's documented name is primary; the old name still resolves"
        );
        assert_eq!(alerts_file_from_env(&|_| None), None);
        let blank = |key: &str| match key {
            ALERT_FILE_ENV => Some("  ".to_string()),
            ALERTS_FILE_ENV => Some("/fallback.jsonl".to_string()),
            _ => None,
        };
        assert_eq!(
            alerts_file_from_env(&blank).as_deref(),
            Some("/fallback.jsonl"),
            "a blank primary falls through to the fallback"
        );
    }

    #[test]
    fn frozen_copy_is_pinned() {
        assert_eq!(
            RESTORE_FAILED_MESSAGE,
            "The platform refused the stop request. Apex has not marked the test restored."
        );
        assert_eq!(RESTORE_FAILED_HEADLINE, "This test needs intervention");
        assert_eq!(INCIDENT_NEXT_STEP, "Operator investigating");
        assert_eq!(PREVIOUS_STATE_NEXT_STEP, "Review before manual action");
    }

    /// The pre-fix projection, kept verbatim as the naive reference: linear
    /// `index_of`, reverse scans for pulls/guardrails, a per-alert ledger
    /// walk and a per-record incident scan. The optimized projection must
    /// agree with it record-for-record at n = 10_000.
    fn project_naive(
        records: &[LedgerRecord],
        alerts: &[Alert],
        now_unix_ms: i64,
    ) -> Vec<ExperimentView> {
        let mut views: Vec<ExperimentView> = Vec::new();
        let index_of = |views: &mut Vec<ExperimentView>, id: &str| -> usize {
            if let Some(index) = views.iter().position(|view| view.experiment_id == id) {
                return index;
            }
            views.push(ExperimentView {
                experiment_id: id.to_string(),
                platform: None,
                state: ExperimentStateView::Registered,
                scope: None,
                metric: None,
                test_share: None,
                burn_in_days: None,
                window_days: None,
                rule_version: None,
                rate_bp: None,
                started_at_unix_ms: None,
                data_pull_days: Vec::new(),
                result: None,
                guardrails: Vec::new(),
                guardrail_thresholds: None,
                started_seq: None,
                incident: None,
                previous_settings_recorded: false,
            });
            views.len() - 1
        };
        for record in records {
            match record {
                LedgerRecord::ExperimentStarted {
                    seq,
                    at_unix_ms,
                    platform,
                    experiment_id,
                    ..
                } => {
                    let index = index_of(&mut views, experiment_id);
                    let view = &mut views[index];
                    view.platform = Some(*platform);
                    view.started_at_unix_ms = Some(*at_unix_ms);
                    view.started_seq = Some(*seq);
                }
                LedgerRecord::LiftRecord {
                    seq,
                    at_unix_ms,
                    payload,
                    payload_sha256,
                    ..
                } => {
                    let Ok(lift) = serde_json::from_str::<LiftRecord>(payload) else {
                        continue;
                    };
                    match lift {
                        LiftRecord::PreRegistration {
                            experiment_id,
                            scope,
                            split,
                            metric,
                            burn_in_days,
                            window_days,
                            code_version_hash,
                            rate_bp,
                            guardrail_thresholds,
                            ..
                        } => {
                            let index = index_of(&mut views, &experiment_id);
                            let view = &mut views[index];
                            view.scope = Some(scope);
                            view.test_share = Some(split);
                            view.metric = Some(metric);
                            view.burn_in_days = Some(burn_in_days);
                            view.window_days = Some(window_days);
                            view.rule_version = Some(code_version_hash);
                            view.rate_bp = Some(rate_bp);
                            view.guardrail_thresholds = Some(guardrail_thresholds);
                        }
                        LiftRecord::DataPull { day, .. } => {
                            if let Some(view) = views.iter_mut().rev().find(|view| {
                                view.result.is_none() && view.started_at_unix_ms.is_some()
                            }) {
                                view.data_pull_days.push(day);
                            }
                        }
                        LiftRecord::Guardrail(guardrail) => {
                            if let Some(view) = views.iter_mut().rev().find(|view| {
                                view.started_at_unix_ms.is_some() && view.result.is_none()
                            }) {
                                view.guardrails.push(guardrail);
                            }
                        }
                        LiftRecord::Result {
                            experiment_id,
                            daily_log_ratios,
                            mean_log,
                            lower_90,
                            lower_99,
                            bootstrap_lower,
                            decision,
                            lambda,
                        } => {
                            let index = index_of(&mut views, &experiment_id);
                            let view = &mut views[index];
                            let point_bp = if mean_log.is_finite() {
                                Some((mean_log.exp_m1() * 10_000.0).round() as i64)
                            } else {
                                None
                            };
                            view.result = Some(ExperimentResultView {
                                observed_lift_bp: point_bp,
                                lower_90,
                                lower_99,
                                bootstrap_lower,
                                decision,
                                lambda,
                                daily_log_ratios,
                                seq: *seq,
                                payload_sha256: payload_sha256.clone(),
                                at_unix_ms: *at_unix_ms,
                            });
                        }
                        LiftRecord::MonthlyBilling(_) | LiftRecord::BillingChange(_) => {}
                    }
                }
                _ => {}
            }
        }
        for alert in alerts {
            if let Alert::FailedRollback {
                experiment_id,
                detail,
            } = alert
            {
                let index = index_of(&mut views, experiment_id);
                let reference = incident_reference(experiment_id, detail);
                let recorded_at = recorded_at_of_naive(records, experiment_id, now_unix_ms);
                views[index].incident = Some(IncidentView {
                    reference,
                    recorded_at_unix_ms: recorded_at,
                    previous_state: "Test still reported active".to_string(),
                    previous_recorded_at_unix_ms: None,
                });
            }
        }
        for record in records {
            if let LedgerRecord::ChangeApplied {
                at_unix_ms,
                ad_unit_id,
                prev_value,
                ..
            } = record
            {
                if prev_value.trim().is_empty() {
                    continue;
                }
                for view in views.iter_mut().filter(|view| view.incident.is_some()) {
                    if view.scope.as_deref() != Some(ad_unit_id.as_str()) {
                        continue;
                    }
                    let Some(incident) = &mut view.incident else {
                        continue;
                    };
                    if *at_unix_ms > incident.recorded_at_unix_ms {
                        continue;
                    }
                    view.previous_settings_recorded = true;
                    if incident
                        .previous_recorded_at_unix_ms
                        .is_none_or(|current| *at_unix_ms > current)
                    {
                        incident.previous_recorded_at_unix_ms = Some(*at_unix_ms);
                    }
                }
            }
        }
        for view in &mut views {
            view.state = resolve_state(view, now_unix_ms);
        }
        views.sort_by(|a, b| a.experiment_id.cmp(&b.experiment_id));
        views
    }

    fn recorded_at_of_naive(
        records: &[LedgerRecord],
        experiment_id: &str,
        now_unix_ms: i64,
    ) -> i64 {
        let mut newest: Option<i64> = None;
        for record in records {
            let belongs = match record {
                LedgerRecord::ExperimentStarted {
                    experiment_id: id, ..
                }
                | LedgerRecord::ExperimentObserved {
                    experiment_id: id, ..
                } => id == experiment_id,
                LedgerRecord::LiftRecord { payload, .. } => {
                    serde_json::from_str::<LiftRecord>(payload)
                        .map(|lift| match lift {
                            LiftRecord::PreRegistration {
                                experiment_id: id, ..
                            }
                            | LiftRecord::Result {
                                experiment_id: id, ..
                            } => id == experiment_id,
                            _ => false,
                        })
                        .unwrap_or(false)
                }
                _ => false,
            };
            if belongs {
                newest = Some(newest.map_or(record.at_unix_ms(), |t| t.max(record.at_unix_ms())));
            }
        }
        newest.unwrap_or(now_unix_ms)
    }

    #[test]
    fn the_projection_agrees_with_the_naive_reference_at_ten_thousand_records() {
        // 1_000 experiments x 10 records: starts, registrations, pulls
        // (including a burn-in pull before the analysis window), guardrails,
        // results for half, and billing records that the projection ignores.
        let mut drafts = Vec::with_capacity(10_000);
        for e in 0..1_000 {
            let id = format!("exp_{e:04}");
            drafts.push(started(&id, NOW - 30 * DAY_MS));
            drafts.push(preregistration(&id));
            drafts.push(data_pull("2026-09-01")); // burn-in pull
            drafts.push(data_pull("2026-10-10"));
            drafts.push(data_pull("2026-10-11"));
            drafts.push(lift_record(
                r#"{"type":"guardrail","day":"2026-10-12","guardrail":"fastHarm","inputs":{"pointLift":-0.061},"action":"rollback"}"#,
            ));
            if e % 2 == 0 {
                drafts.push(result_record(&id, if e % 4 == 0 { "keep" } else { "stop" }));
            } else {
                drafts.push(data_pull("2026-10-12"));
            }
            drafts.push(data_pull("2026-10-13"));
            drafts.push(lift_record(
                r#"{"type":"monthlyBilling","changeId":"ch_1","month":"2026-10","scopeRevenue":100,"lambda":0.03,"factor":1.03,"incremental":10,"logShare":1.0,"rateBp":2000,"feeMicros":1000,"billedDays":30,"monthDays":30}"#,
            ));
            drafts.push(preregistration(&id));
        }
        assert_eq!(drafts.len(), 10_000);
        let alerts: Vec<Alert> = (0..20)
            .map(|i| Alert::FailedRollback {
                experiment_id: format!("exp_{:04}", i * 47),
                detail: format!("rollback call failed: {i}"),
            })
            .collect();

        let fixed = project(&drafts, &alerts, NOW);
        let naive = project_naive(&drafts, &alerts, NOW);
        assert_eq!(fixed.len(), 1_000);
        assert_eq!(
            fixed, naive,
            "the optimized projection must agree with the naive reference"
        );
        // The incident rows really are populated (the naive reference walks
        // the ledger per alert, so agreement is not vacuous).
        assert_eq!(
            fixed.iter().filter(|view| view.incident.is_some()).count(),
            20
        );
    }
}
