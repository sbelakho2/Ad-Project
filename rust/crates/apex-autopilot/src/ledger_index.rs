//! One shared, single-pass index over the signed ledger.
//!
//! The write cycle (`plan`), settlement (`settle`), and the daily running
//! scan all ask the same questions of the whole ledger: was this change
//! already applied, does this experiment have a signed pre-registration,
//! which platform id names which experiment, and is that experiment already
//! settled. Rebuilding those facts per proposal or per experiment makes a
//! pass O(proposals x records) or O(experiments x records): the daily loop
//! also appends a data pull per experiment, so the ledger keeps growing while
//! it is iterated.
//!
//! [`LedgerIndex`] is built once from a single `records()` read and updated
//! as the caller appends. The first-match semantics of the old per-experiment
//! scans are preserved: the first start, pre-registration, result, or applied
//! change for an id wins, and duplicate applied changes are kept in ledger
//! order so a rollback can retire exactly the sequence it names.

use std::collections::{BTreeMap, HashMap, HashSet};

use apex_core::{ExperimentSpec, PlatformId};
use apex_lift::decision::PreRegistration;
use apex_lift::records::LiftRecord;
use apex_proof::{ChangeKind, LedgerLog, LedgerRecord};

use crate::model::{Action, PlanError};

const DAY_MS: i64 = 86_400_000;

fn utc_day(at_unix_ms: i64) -> i64 {
    at_unix_ms.div_euclid(DAY_MS)
}

/// The applied change a `ChangeApplied` sequence names, in the shape the
/// settle path needs to reconstruct and roll back the action.
#[derive(Debug, Clone)]
pub(crate) struct AppliedChange {
    pub(crate) platform: PlatformId,
    pub(crate) ad_unit_id: String,
    pub(crate) change: ChangeKind,
}

/// One `experimentStarted` record in ledger order, carrying the last applied
/// change sequence seen before it. That is exactly the pairing the running
/// scan used to derive from a full second walk.
#[derive(Debug, Clone)]
pub(crate) struct Start {
    pub(crate) experiment_id: String,
    /// Immutable experiment instance named by the start record (falling back
    /// to the reusable platform name only for legacy records).
    pub(crate) instance_id: String,
    pub(crate) spec: ExperimentSpec,
    pub(crate) change_seq: Option<u64>,
    pub(crate) started_day: String,
    /// Ledger sequence of the start record, so terminal proofs can be
    /// ordered against the facts they depend on.
    pub(crate) seq: u64,
}

/// One `experimentCreateIntent` record: the durable promise that a platform
/// create call was about to happen for this exact request. The crash-recovery
/// replay needs every field of the original call, so a restarted process can
/// re-issue it byte-for-byte (the platform create is idempotent by name).
#[derive(Debug, Clone)]
pub(crate) struct CreateIntent {
    pub(crate) platform: PlatformId,
    /// The registered experiment name the create call used.
    pub(crate) experiment_id: String,
    pub(crate) ad_unit_id: String,
    pub(crate) idempotency_key: String,
    pub(crate) change: ChangeKind,
    pub(crate) prev_value: String,
    pub(crate) spec: ExperimentSpec,
}

/// The ledger facts a pass needs, indexed once.
#[derive(Debug, Default)]
pub(crate) struct LedgerIndex {
    /// First `ChangeApplied` per sequence, for the settle lookup.
    changes: HashMap<u64, AppliedChange>,
    /// Every sequence the ledger carries, so a settle lookup can tell a
    /// missing sequence from a sequence holding another record kind.
    sequences: HashSet<u64>,
    /// Applied changes by idempotency key, in ledger order (a key may repeat
    /// with a different value; the payload decides, never the key alone).
    applied: HashMap<String, Vec<(ChangeKind, u64)>>,
    /// Every applied change in ledger order, `(seq, at_unix_ms)`, for the
    /// daily-cap count.
    applied_order: Vec<(u64, i64)>,
    /// Change sequences a rollback retired.
    rolled_back: HashSet<u64>,
    /// The last applied sequence observed so far, used to pair a start with
    /// the change it tests.
    last_change_seq: Option<u64>,
    /// First pre-registration sequence per experiment name (the write
    /// cycle's in-flight gate).
    pre_registrations: HashMap<String, u64>,
    /// Number of pre-registrations seen per experiment name (the deterministic
    /// attempt counter that makes a restart's instance id distinct).
    pre_registration_counts: HashMap<String, u64>,
    /// Every name with a pre-registration record (or a parsed one).
    pre_registered_names: HashSet<String>,
    /// Every instance with a pre-registration record, even when the record is
    /// too sparse to reconstruct (the running scan only needs existence).
    pre_registered_instances: HashSet<String>,
    /// First reconstructible pre-registration per immutable instance id
    /// (settlement reads the signed record back, never a caller's copy).
    prereg_by_instance: HashMap<String, PreRegistration>,
    /// First pre-registered prior value per instance (the running scan's
    /// settings-change check).
    before_values: BTreeMap<String, String>,
    /// First start name per platform experiment id.
    started_names: HashMap<String, String>,
    /// First start day per platform experiment id.
    started_days: BTreeMap<String, String>,
    /// Promotion deadline per `(experiment id, change sequence)`.
    promotion_deadlines: HashMap<(String, u64), i64>,
    /// Every start in ledger order, paired with the last applied sequence
    /// seen before it.
    starts: Vec<Start>,
    /// Experiment ids carrying an `experimentObserved` record.
    observed: HashSet<String>,
    /// Latest observation sequence per experiment name, so an observation
    /// from a previous instance of a reused name never completes the new one.
    observed_seq: HashMap<String, u64>,
    /// Experiment instances carrying a signed lift `Result` (any decision).
    result_instances: HashSet<String>,
    /// First signed lift `Result` payload per experiment instance, with its
    /// sequence. Crash recovery reads the exact signed decision back from it.
    result_records: HashMap<String, (u64, LiftRecord)>,
    /// The first pre-registration sequence per immutable instance id.
    prereg_seq_by_instance: HashMap<String, u64>,
    /// The latest durable holdout state per immutable experiment instance.
    holdout_states: HashMap<String, apex_lift::holdout::HoldoutState>,
    /// The signed payload digest of the first pre-registration per instance.
    prereg_hashes: HashMap<String, String>,
    /// The signed payload digest of the first result per instance.
    result_hashes: HashMap<String, String>,
    /// The first lift payload that did not parse, for fail-closed scans.
    parse_failure: Option<(u64, String)>,
    /// Every experiment-create intent in ledger order.
    create_intents: Vec<CreateIntent>,
    /// Experiment names whose create the platform confirmed, with the
    /// external id the adapter returned.
    created_experiments: HashMap<String, String>,
    /// Experiment names whose create attempt is resolved as never-replayable.
    abandoned_experiments: HashSet<String>,
}

impl LedgerIndex {
    /// Build the index from one `records()` read. A malformed signed payload
    /// is retained as a typed failure instead of aborting: the write cycle
    /// and settlement historically skipped unparseable lift records, while
    /// the running scan refuses them. The caller decides which discipline
    /// applies.
    pub(crate) fn build(ledger: &dyn LedgerLog) -> Self {
        let mut index = Self::default();
        for record in ledger.records() {
            index.observe(record);
        }
        index
    }

    /// Fold one record into the index. Appending callers pass the sealed
    /// record the ledger returned, so the index never lags the ledger it
    /// serves.
    pub(crate) fn observe(&mut self, record: &LedgerRecord) {
        #[cfg(test)]
        metrics::record_visit();
        self.sequences.insert(record.seq());
        match record {
            LedgerRecord::ChangeApplied {
                seq,
                at_unix_ms,
                platform,
                ad_unit_id,
                change,
                idempotency_key,
                ..
            } => {
                self.last_change_seq =
                    Some(self.last_change_seq.map_or(*seq, |seen| seen.max(*seq)));
                self.applied_order.push((*seq, *at_unix_ms));
                #[cfg(test)]
                metrics::index_entry();
                self.changes.entry(*seq).or_insert_with(|| AppliedChange {
                    platform: *platform,
                    ad_unit_id: ad_unit_id.clone(),
                    change: change.clone(),
                });
                self.applied
                    .entry(idempotency_key.clone())
                    .or_default()
                    .push((change.clone(), *seq));
            }
            LedgerRecord::ExperimentStarted {
                seq,
                at_unix_ms,
                experiment_id,
                spec,
                ..
            } => {
                #[cfg(test)]
                metrics::index_entry();
                self.started_names
                    .entry(experiment_id.clone())
                    .or_insert_with(|| spec.name.clone());
                let started_day = crate::calendar::format_day(utc_day(*at_unix_ms));
                self.started_days
                    .entry(experiment_id.clone())
                    .or_insert_with(|| started_day.clone());
                self.starts.push(Start {
                    experiment_id: experiment_id.clone(),
                    instance_id: instance_id_of_spec(spec),
                    spec: spec.clone(),
                    change_seq: self.last_change_seq,
                    started_day,
                    seq: *seq,
                });
            }
            LedgerRecord::ExperimentObserved {
                seq, experiment_id, ..
            } => {
                #[cfg(test)]
                metrics::index_entry();
                self.observed.insert(experiment_id.clone());
                self.observed_seq
                    .entry(experiment_id.clone())
                    .and_modify(|existing| *existing = (*existing).max(*seq))
                    .or_insert(*seq);
            }
            LedgerRecord::PromotionPending {
                experiment_id,
                change_seq,
                deadline_unix_ms,
                ..
            } => {
                #[cfg(test)]
                metrics::index_entry();
                self.promotion_deadlines
                    .entry((experiment_id.clone(), *change_seq))
                    .or_insert(*deadline_unix_ms);
            }
            LedgerRecord::RollbackPerformed {
                rolled_back_seq, ..
            } => {
                #[cfg(test)]
                metrics::index_entry();
                self.rolled_back.insert(*rolled_back_seq);
            }
            LedgerRecord::ExperimentCreateIntent {
                platform,
                experiment_id,
                ad_unit_id,
                idempotency_key,
                change,
                prev_value,
                spec,
                ..
            } => {
                #[cfg(test)]
                metrics::index_entry();
                self.create_intents.push(CreateIntent {
                    platform: *platform,
                    experiment_id: experiment_id.clone(),
                    ad_unit_id: ad_unit_id.clone(),
                    idempotency_key: idempotency_key.clone(),
                    change: change.clone(),
                    prev_value: prev_value.clone(),
                    spec: spec.clone(),
                });
            }
            LedgerRecord::ExperimentCreated {
                experiment_id,
                external_id,
                ..
            } => {
                #[cfg(test)]
                metrics::index_entry();
                self.created_experiments
                    .insert(experiment_id.clone(), external_id.clone());
            }
            LedgerRecord::ExperimentCreateAbandoned { experiment_id, .. } => {
                #[cfg(test)]
                metrics::index_entry();
                self.abandoned_experiments.insert(experiment_id.clone());
            }
            LedgerRecord::AaQualification { .. } => {}
            LedgerRecord::LiftRecord {
                seq,
                payload,
                payload_sha256,
                ..
            } => {
                let parsed = match parse_lift_record(payload) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        if self.parse_failure.is_none() {
                            self.parse_failure = Some((*seq, error.to_string()));
                        }
                        return;
                    }
                };
                match &parsed {
                    LiftRecord::PreRegistration {
                        experiment_id,
                        instance_id,
                        ..
                    } => {
                        #[cfg(test)]
                        metrics::index_entry();
                        self.pre_registered_names.insert(experiment_id.clone());
                        self.pre_registrations
                            .entry(experiment_id.clone())
                            .or_insert(*seq);
                        *self
                            .pre_registration_counts
                            .entry(experiment_id.clone())
                            .or_insert(0) += 1;
                        let instance = instance_of_record(instance_id, experiment_id);
                        self.pre_registered_instances.insert(instance.clone());
                        self.prereg_seq_by_instance
                            .entry(instance.clone())
                            .or_insert(*seq);
                        self.prereg_hashes
                            .entry(instance.clone())
                            .or_insert_with(|| payload_sha256.clone());
                        if let Ok(prereg) = PreRegistration::from_record(&parsed) {
                            self.before_values
                                .entry(instance.clone())
                                .or_insert_with(|| prereg.before_value.clone());
                            self.prereg_by_instance.entry(instance).or_insert(prereg);
                        }
                    }
                    LiftRecord::Result {
                        experiment_id,
                        instance_id,
                        ..
                    } => {
                        #[cfg(test)]
                        metrics::index_entry();
                        let instance = instance_of_record(instance_id, experiment_id);
                        self.result_instances.insert(instance.clone());
                        self.result_hashes
                            .entry(instance.clone())
                            .or_insert_with(|| payload_sha256.clone());
                        self.result_records
                            .entry(instance)
                            .or_insert_with(|| (*seq, parsed.clone()));
                    }
                    LiftRecord::HoldoutState(state) => {
                        #[cfg(test)]
                        metrics::index_entry();
                        // Last write wins: a rollback record supersedes the
                        // opening state and a replay never re-opens it.
                        self.holdout_states
                            .insert(state.change_id.clone(), state.clone());
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// True when the same change (equal idempotency key and equal payload)
    /// is already in the ledger and not rolled back. A rerun must never
    /// re-apply it, and a rerun with a different value under a colliding key
    /// must never be mistaken for it.
    pub(crate) fn already_applied(&self, key: &str, action: &Action) -> bool {
        let expected = crate::plan::change_kind_of(action);
        self.applied.get(key).is_some_and(|records| {
            records
                .iter()
                .any(|(change, seq)| *change == expected && !self.rolled_back.contains(seq))
        })
    }

    /// True when the ledger already carries the signed pre-registration for
    /// `experiment_id` (a rerun must not stack duplicate registrations).
    pub(crate) fn pre_registration_exists(&self, experiment_id: &str) -> bool {
        self.pre_registrations.contains_key(experiment_id)
    }

    /// The first signed pre-registration sequence for `experiment_id`. A
    /// retry that appends a duplicate still reports the first registration,
    /// exactly as the old per-name scan did.
    pub(crate) fn pre_registration_seq(&self, experiment_id: &str) -> Option<u64> {
        self.pre_registrations.get(experiment_id).copied()
    }

    /// How many pre-registrations this ledger already carries for `name`.
    /// The write cycle uses it as the deterministic attempt number.
    pub(crate) fn pre_registration_attempts(&self, name: &str) -> u64 {
        self.pre_registration_counts.get(name).copied().unwrap_or(0)
    }

    /// True when any change was ever applied under `key`, live or rolled
    /// back. A pre-registration without such a record means a prior attempt
    /// may have started the experiment before its bookkeeping was written.
    pub(crate) fn prior_change_exists(&self, key: &str) -> bool {
        self.applied.contains_key(key)
    }

    /// Applied changes at or before `now_unix_ms` on the UTC `day`, excluding
    /// sequences a rollback retired. The daily cap is counted from this
    /// index, never from caller state.
    pub(crate) fn applied_on_day(&self, day: i64, now_unix_ms: i64) -> usize {
        self.applied_order
            .iter()
            .filter(|(seq, at_unix_ms)| {
                *at_unix_ms <= now_unix_ms
                    && utc_day(*at_unix_ms) == day
                    && !self.rolled_back.contains(seq)
            })
            .count()
    }

    /// The first applied change with this sequence.
    pub(crate) fn change(&self, seq: u64) -> Option<&AppliedChange> {
        self.changes.get(&seq)
    }

    /// Whether the ledger carries any record at this sequence.
    pub(crate) fn contains_seq(&self, seq: u64) -> bool {
        self.sequences.contains(&seq)
    }

    /// The experiment name pinned by the first `experimentStarted` record for
    /// this platform id.
    pub(crate) fn started_experiment_name(&self, experiment_id: &str) -> Option<&str> {
        self.started_names.get(experiment_id).map(String::as_str)
    }

    /// The signed pre-registration for the instance started by
    /// `experiment_id` at `change_seq`. The change sequence disambiguates a
    /// reused platform id (MAX returns the ad unit id for every run).
    pub(crate) fn signed_pre_registration(
        &self,
        experiment_id: &str,
        change_seq: u64,
    ) -> Result<PreRegistration, PlanError> {
        let start = self
            .start(experiment_id, change_seq)
            .ok_or_else(|| PlanError::NoBaseline {
                reason: format!("experiment {experiment_id} was never started in this ledger"),
            })?;
        self.prereg_by_instance
            .get(&start.instance_id)
            .cloned()
            .ok_or_else(|| PlanError::NoBaseline {
                reason: format!(
                    "no signed pre-registration for experiment instance {}",
                    start.instance_id
                ),
            })
    }

    /// The start record for `experiment_id` paired with `change_seq`.
    pub(crate) fn start(&self, experiment_id: &str, change_seq: u64) -> Option<&Start> {
        self.starts.iter().rev().find(|start| {
            start.experiment_id == experiment_id && start.change_seq == Some(change_seq)
        })
    }

    /// The typed refusal when `experiment_id` already has a terminal record:
    /// an `experimentObserved`, a rollback of its change, or a signed lift
    /// `Result` carrying its registered name. A result that was written after
    /// both its pre-registration and its start but has no completion marker is
    /// a pending side effect, not a settled experiment: the settle path
    /// reconciles it instead of refusing it.
    pub(crate) fn settled_refusal(
        &self,
        experiment_id: &str,
        change_seq: u64,
    ) -> Option<PlanError> {
        let instance = self
            .start(experiment_id, change_seq)
            .map(|start| start.instance_id.clone())
            .or_else(|| {
                self.started_experiment_name(experiment_id)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let start_seq = self
            .start(experiment_id, change_seq)
            .map(|start| start.seq)
            .unwrap_or(0);
        let completed = self.terminal_complete(experiment_id, change_seq, start_seq);
        // A result that predates the start or the pre-registration is the
        // historical "result without its evidence" shape: refuse it exactly
        // as before rather than treating it as recoverable.
        let unordered = self.result_records.get(&instance).is_some_and(|(seq, _)| {
            !self.result_is_ordered(experiment_id, &instance, change_seq, *seq)
        });
        let settled = completed || unordered;
        settled.then(|| {
            PlanError::Validation(format!("experiment {experiment_id} is already settled"))
        })
    }

    /// Whether an experiment carries the completion marker of a terminal
    /// decision: an observation (keep/void) or a rollback of its change.
    pub(crate) fn terminal_complete(
        &self,
        experiment_id: &str,
        change_seq: u64,
        start_seq: u64,
    ) -> bool {
        self.observed_seq
            .get(experiment_id)
            .is_some_and(|seq| *seq > start_seq)
            || self.rolled_back.contains(&change_seq)
    }

    /// Whether a signed terminal `Result` exists but its remote side effect
    /// has no completion marker yet.
    pub(crate) fn has_pending_promotion(&self, experiment_id: &str, change_seq: u64) -> bool {
        self.promotion_deadlines
            .contains_key(&(experiment_id.to_string(), change_seq))
    }

    pub(crate) fn pending_terminal(
        &self,
        experiment_id: &str,
        instance_id: &str,
        change_seq: u64,
    ) -> bool {
        let Some((seq, _)) = self.result_records.get(instance_id) else {
            return false;
        };
        let Some(start) = self.start(experiment_id, change_seq) else {
            return false;
        };
        self.result_is_ordered(experiment_id, instance_id, change_seq, *seq)
            && !self.terminal_complete(experiment_id, change_seq, start.seq)
    }

    /// The signed terminal result written for `instance_id`, when it was
    /// written after both the start and the pre-registration.
    pub(crate) fn result_record(&self, instance_id: &str) -> Option<(&LiftRecord, u64)> {
        self.result_records
            .get(instance_id)
            .map(|(seq, record)| (record, *seq))
    }

    /// Result ordering against the facts it depends on. A result before its
    /// start or pre-registration is never reconciled: it is the hostile
    /// shape the scans already refuse.
    fn result_is_ordered(
        &self,
        experiment_id: &str,
        instance_id: &str,
        change_seq: u64,
        result_seq: u64,
    ) -> bool {
        let Some(start) = self.start(experiment_id, change_seq) else {
            return false;
        };
        let Some(prereg_seq) = self.prereg_seq_by_instance.get(instance_id).copied() else {
            return false;
        };
        result_seq > start.seq && result_seq > prereg_seq
    }

    /// The first lift payload that did not parse, if any. The running scan
    /// fails closed on this; the write and settle paths skip it exactly as
    /// their record walks always did.
    pub(crate) fn parse_failure(&self) -> Option<(u64, &str)> {
        self.parse_failure
            .as_ref()
            .map(|(seq, detail)| (*seq, detail.as_str()))
    }

    /// Every start in ledger order.
    pub(crate) fn starts(&self) -> &[Start] {
        &self.starts
    }

    pub(crate) fn is_pre_registered(&self, instance_id: &str) -> bool {
        self.pre_registered_instances.contains(instance_id)
    }

    pub(crate) fn is_observed(&self, experiment_id: &str) -> bool {
        self.observed.contains(experiment_id)
    }

    pub(crate) fn is_rolled_back(&self, change_seq: u64) -> bool {
        self.rolled_back.contains(&change_seq)
    }

    pub(crate) fn has_result(&self, instance_id: &str) -> bool {
        self.result_instances.contains(instance_id)
    }

    pub(crate) fn before_value(&self, instance_id: &str) -> Option<&str> {
        self.before_values.get(instance_id).map(String::as_str)
    }

    /// The signed pre-registration for one immutable instance.
    pub(crate) fn pre_registration(&self, instance_id: &str) -> Option<&PreRegistration> {
        self.prereg_by_instance.get(instance_id)
    }

    /// The durable holdout state for one immutable instance, when a keep has
    /// opened one.
    pub(crate) fn holdout_state(
        &self,
        instance_id: &str,
    ) -> Option<&apex_lift::holdout::HoldoutState> {
        self.holdout_states.get(instance_id)
    }

    /// The signed payload digest of the pre-registration.
    pub(crate) fn pre_registration_hash(&self, instance_id: &str) -> Option<&str> {
        self.prereg_hashes.get(instance_id).map(String::as_str)
    }

    /// The signed payload digest of the keep result.
    pub(crate) fn result_hash(&self, instance_id: &str) -> Option<&str> {
        self.result_hashes.get(instance_id).map(String::as_str)
    }

    /// First start day per platform experiment id.
    /// The promotion deadline recorded for a kept-but-not-yet-promoted test.
    pub(crate) fn promotion_deadline(&self, experiment_id: &str, change_seq: u64) -> Option<i64> {
        self.promotion_deadlines
            .get(&(experiment_id.to_string(), change_seq))
            .copied()
    }

    pub(crate) fn started_days(&self) -> &BTreeMap<String, String> {
        &self.started_days
    }

    /// Every create intent that is neither confirmed nor abandoned, in ledger
    /// order. The recovery replay re-issues exactly these.
    pub(crate) fn pending_creates(&self) -> Vec<&CreateIntent> {
        self.create_intents
            .iter()
            .filter(|intent| {
                !self.created_experiments.contains_key(&intent.experiment_id)
                    && !self.abandoned_experiments.contains(&intent.experiment_id)
            })
            .collect()
    }

    /// Every create intent the platform confirmed but whose start record
    /// never landed (a crash inside the confirmation bookkeeping). The
    /// recovery completes the durable records without a second platform call.
    pub(crate) fn created_but_unstarted(&self) -> Vec<&CreateIntent> {
        self.create_intents
            .iter()
            .filter(|intent| {
                self.created_experiments.contains_key(&intent.experiment_id)
                    && !self.abandoned_experiments.contains(&intent.experiment_id)
                    && !self.any_start_with_name(&intent.experiment_id)
            })
            .collect()
    }

    /// The external experiment id the adapter returned for a confirmed create.
    pub(crate) fn created_external_id(&self, experiment_id: &str) -> Option<&String> {
        self.created_experiments.get(experiment_id)
    }

    /// Whether the platform confirmed this experiment's create.
    #[cfg(test)]
    pub(crate) fn is_created(&self, experiment_id: &str) -> bool {
        self.created_experiments.contains_key(experiment_id)
    }

    /// Whether this experiment's create attempt is resolved as never-replayable.
    #[cfg(test)]
    pub(crate) fn is_abandoned(&self, experiment_id: &str) -> bool {
        self.abandoned_experiments.contains(experiment_id)
    }

    /// Whether any `experimentStarted` record used this registered experiment
    /// name (the adapter's external id may differ from the name).
    pub(crate) fn any_start_with_name(&self, name: &str) -> bool {
        self.starts.iter().any(|start| start.spec.name == name)
    }
}

fn instance_of_record(instance_id: &str, experiment_id: &str) -> String {
    if instance_id.trim().is_empty() {
        experiment_id.to_string()
    } else {
        instance_id.to_string()
    }
}

fn instance_id_of_spec(spec: &ExperimentSpec) -> String {
    instance_of_record(&spec.instance_id, &spec.name)
}

/// The signed lift payload parse, counted under `cfg(test)` so the scaling
/// tests can prove one parse per payload rather than one per experiment.
pub(crate) fn parse_lift_record(payload: &str) -> Result<LiftRecord, serde_json::Error> {
    #[cfg(test)]
    metrics::parse();
    serde_json::from_str(payload)
}

#[cfg(test)]
pub(crate) fn parse_call_count() -> u64 {
    metrics::parse_calls()
}

#[cfg(test)]
pub(crate) fn reset_parse_call_count() {
    metrics::reset_parses();
}

#[cfg(test)]
pub(crate) fn reset_index_metrics() {
    metrics::reset();
}

#[cfg(test)]
pub(crate) fn index_metrics() -> (u64, u64, u64) {
    metrics::snapshot()
}

/// Test-only counters for the index's structural work: records visited,
/// payloads parsed, and owned index entries inserted (the allocation proxy
/// the scaling report quotes).
#[cfg(test)]
mod metrics {
    use std::cell::Cell;

    thread_local! {
        static RECORD_VISITS: Cell<u64> = const { Cell::new(0) };
        static PARSE_CALLS: Cell<u64> = const { Cell::new(0) };
        static INDEX_ENTRIES: Cell<u64> = const { Cell::new(0) };
    }

    pub(super) fn record_visit() {
        RECORD_VISITS.with(|count| count.set(count.get() + 1));
    }

    pub(super) fn parse() {
        PARSE_CALLS.with(|count| count.set(count.get() + 1));
    }

    pub(super) fn index_entry() {
        INDEX_ENTRIES.with(|count| count.set(count.get() + 1));
    }

    pub(super) fn parse_calls() -> u64 {
        PARSE_CALLS.with(Cell::get)
    }

    pub(super) fn reset_parses() {
        PARSE_CALLS.with(|count| count.set(0));
    }

    pub(super) fn snapshot() -> (u64, u64, u64) {
        (
            RECORD_VISITS.with(Cell::get),
            PARSE_CALLS.with(Cell::get),
            INDEX_ENTRIES.with(Cell::get),
        )
    }

    pub(super) fn reset() {
        RECORD_VISITS.with(|count| count.set(0));
        PARSE_CALLS.with(|count| count.set(0));
        INDEX_ENTRIES.with(|count| count.set(0));
    }
}
