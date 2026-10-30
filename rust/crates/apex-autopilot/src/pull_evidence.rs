//! The durable pull-evidence store.
//!
//! A day is only final when two *independent* reporting reads agree. The
//! store keys each observation by experiment instance, arm and calendar day,
//! keeps the first observation exactly as it was read, and accepts a second
//! observation only when it comes from a distinct reporting read at least 24
//! hours after the first. The accepted second observation and its own source
//! hash are persisted, so a later process can reproduce the finality decision
//! without reusing the first read.
//!
//! A3-V4 E6: finality is judged on the two **latest** reads, never against
//! the first read (which never changes). Each later accepted read replaces
//! the previous one, and the outgoing observation is kept as the penultimate
//! read, so the pair the arm-day exposes is always the latest two reads: any
//! two consecutive later reads that agree within the tolerance make the day
//! final, even when both disagree with the very first read.
//!
//! One file per experiment (named by a hash of namespace and experiment id)
//! is written atomically under the evidence root. An unreadable or malformed
//! file fails closed instead of resetting the evidence to an empty history.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use apex_lift::window::{ArmDay, PullObservation};
use sha2::{Digest, Sha256};

use crate::ops::ExperimentDays;

/// Configuration variable naming the durable evidence root.
pub const EVIDENCE_ROOT_ENV: &str = "APEX_PULL_EVIDENCE_ROOT";
/// The relative root used when the variable is absent.
pub const DEFAULT_EVIDENCE_ROOT: &str = ".apex/pull-evidence";
/// The minimum gap between the first and second reporting read.
pub const SECOND_PULL_MIN_SECS: i64 = apex_lift::window::SECOND_PULL_MIN_SECS;

/// Which arm an observation belongs to.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum EvidenceArm {
    Control,
    Test,
}

impl EvidenceArm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Test => "test",
        }
    }
}

/// What recording one observation did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationOutcome {
    /// The first observation for this arm-day.
    First,
    /// An accepted second observation from a later independent read.
    Second,
    /// The same reporting read replayed: the first read is never reused as
    /// finality evidence.
    SameRead,
    /// A later read, but less than 24 hours after the first.
    TooSoon,
    /// The arm-day's replayed latest read: it already is the accepted
    /// observation, so it adds no new finality evidence.
    AlreadyFinal,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredEntry {
    arm: EvidenceArm,
    day: String,
    first: PullObservation,
    second: Option<PullObservation>,
    /// The read the accepted second observation replaced: the earlier half of
    /// the two latest reads once a third read arrives. Absent while only two
    /// reads exist, and absent in documents written before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    penultimate: Option<PullObservation>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredDocument {
    experiment: String,
    entries: Vec<StoredEntry>,
}

impl StoredDocument {
    fn empty(experiment: &str) -> Self {
        Self {
            experiment: experiment.to_string(),
            entries: Vec::new(),
        }
    }
}

/// Resolve the evidence root from a configuration lookup, falling back to the
/// repository-relative default.
pub fn evidence_root_from_env(lookup: &impl Fn(&str) -> Option<String>) -> PathBuf {
    lookup(EVIDENCE_ROOT_ENV)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_EVIDENCE_ROOT))
}

/// The pull-evidence store for one platform namespace.
pub struct PullEvidenceStore {
    namespace: String,
    root: Option<PathBuf>,
    documents: BTreeMap<String, StoredDocument>,
}

impl std::fmt::Debug for PullEvidenceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PullEvidenceStore")
            .field("namespace", &self.namespace)
            .field("root", &self.root)
            .field("experiments", &self.documents.len())
            .finish()
    }
}

impl PullEvidenceStore {
    /// An in-memory store. Production uses [`Self::at`] so the evidence
    /// survives a process restart; tests that only exercise one process may
    /// use this.
    pub fn in_memory(namespace: &str) -> Self {
        Self {
            namespace: namespace.to_string(),
            root: None,
            documents: BTreeMap::new(),
        }
    }

    /// A durable store rooted at `root`. The directory is created on the
    /// first write; a malformed existing document fails closed.
    pub fn at(namespace: &str, root: impl Into<PathBuf>) -> Self {
        Self {
            namespace: namespace.to_string(),
            root: Some(root.into()),
            documents: BTreeMap::new(),
        }
    }

    /// Record one observation, persisting immediately. Returns what the store
    /// decided about it.
    pub fn observe(
        &mut self,
        experiment: &str,
        arm: EvidenceArm,
        day: &str,
        observation: PullObservation,
    ) -> Result<ObservationOutcome, String> {
        let outcome = self.apply(experiment, arm, day, observation)?;
        if matches!(
            outcome,
            ObservationOutcome::First | ObservationOutcome::Second
        ) {
            self.persist(experiment)?;
        }
        Ok(outcome)
    }

    /// Record a whole reporting read (both arms for every day it covers) and
    /// return the experiment's accumulated evidence. The document is
    /// persisted once, after every observation in the read is applied.
    pub fn record_read(
        &mut self,
        experiment: &str,
        observations: impl IntoIterator<Item = (EvidenceArm, String, PullObservation)>,
    ) -> Result<ExperimentDays, String> {
        let observations: Vec<_> = observations.into_iter().collect();
        if observations.is_empty() {
            // An empty reporting window is not an observation: return the
            // accumulated evidence without creating or rewriting a file.
            return Ok(self.experiment_days(experiment));
        }
        for (arm, day, observation) in observations {
            self.apply(experiment, arm, &day, observation)?;
        }
        // Persist once per read. Replayed reads write the same document, so
        // there is no partial state to observe either way.
        self.persist(experiment)?;
        Ok(self.experiment_days(experiment))
    }

    /// The accumulated arm-days for one experiment, in day order. A day is
    /// returned only when both arms have evidence, so no value is synthesized
    /// for an arm that never reported.
    pub fn experiment_days(&self, experiment: &str) -> ExperimentDays {
        let Some(document) = self.documents.get(experiment) else {
            return ExperimentDays {
                control: Vec::new(),
                test: Vec::new(),
            };
        };
        let mut by_day: BTreeMap<&str, (Option<&StoredEntry>, Option<&StoredEntry>)> =
            BTreeMap::new();
        for entry in &document.entries {
            let slot = by_day.entry(entry.day.as_str()).or_default();
            match entry.arm {
                EvidenceArm::Control => slot.0 = Some(entry),
                EvidenceArm::Test => slot.1 = Some(entry),
            }
        }
        let mut control = Vec::new();
        let mut test = Vec::new();
        for (day, (control_entry, test_entry)) in by_day {
            let (Some(control_entry), Some(test_entry)) = (control_entry, test_entry) else {
                continue;
            };
            control.push(arm_day(day, control_entry));
            test.push(arm_day(day, test_entry));
        }
        ExperimentDays { control, test }
    }

    /// The stored observations for one arm-day, cloned for inspection.
    pub fn observations(
        &self,
        experiment: &str,
        arm: EvidenceArm,
        day: &str,
    ) -> Option<(PullObservation, Option<PullObservation>)> {
        self.documents.get(experiment).and_then(|document| {
            document
                .entries
                .iter()
                .find(|entry| entry.arm == arm && entry.day == day)
                .map(|entry| (entry.first.clone(), entry.second.clone()))
        })
    }

    fn apply(
        &mut self,
        experiment: &str,
        arm: EvidenceArm,
        day: &str,
        observation: PullObservation,
    ) -> Result<ObservationOutcome, String> {
        if observation.source_sha256.trim().is_empty() {
            return Err(format!(
                "pull evidence for {experiment}/{}/{day} has no source hash",
                arm.as_str()
            ));
        }
        let document = self.document(experiment)?;
        if let Some(entry) = document
            .entries
            .iter_mut()
            .find(|entry| entry.arm == arm && entry.day == day)
        {
            // The exact read replayed is never finality evidence.
            if (
                entry.first.observed_at_unix_s,
                entry.first.source_sha256.as_str(),
            ) == (
                observation.observed_at_unix_s,
                observation.source_sha256.as_str(),
            ) {
                return Ok(ObservationOutcome::SameRead);
            }
            if let Some(second) = &entry.second {
                // A later read replaces a disagreeing second once it is itself
                // a day on; two consecutive agreeing reads make the day final.
                if (second.observed_at_unix_s, second.source_sha256.as_str())
                    == (
                        observation.observed_at_unix_s,
                        observation.source_sha256.as_str(),
                    )
                {
                    return Ok(ObservationOutcome::AlreadyFinal);
                }
                if observation
                    .observed_at_unix_s
                    .saturating_sub(second.observed_at_unix_s) // adversarial-ok: clock delta
                    < SECOND_PULL_MIN_SECS
                {
                    return Ok(ObservationOutcome::TooSoon);
                }
                // The outgoing read becomes the penultimate half of the latest
                // pair: finality compares the two latest reads, never the
                // never-changing first read.
                entry.penultimate = entry.second.take();
                entry.second = Some(observation);
                return Ok(ObservationOutcome::Second);
            }
            if observation
                .observed_at_unix_s
                .saturating_sub(entry.first.observed_at_unix_s)
                < SECOND_PULL_MIN_SECS
            {
                return Ok(ObservationOutcome::TooSoon);
            }
            entry.second = Some(observation);
            return Ok(ObservationOutcome::Second);
        }
        document.entries.push(StoredEntry {
            arm,
            day: day.to_string(),
            first: observation,
            second: None,
            penultimate: None,
        });
        Ok(ObservationOutcome::First)
    }

    fn document(&mut self, experiment: &str) -> Result<&mut StoredDocument, String> {
        if !self.documents.contains_key(experiment) {
            let document = match &self.root {
                Some(root) => {
                    let path = path_for(root, &self.namespace, experiment);
                    match std::fs::read_to_string(&path) {
                        Ok(raw) => {
                            let document: StoredDocument =
                                serde_json::from_str(&raw).map_err(|error| {
                                    format!(
                                        "pull evidence at {} is not valid JSON: {error}",
                                        path.display()
                                    )
                                })?;
                            if document.experiment != experiment {
                                return Err(format!(
                                    "pull evidence at {} belongs to '{}', not '{experiment}'",
                                    path.display(),
                                    document.experiment
                                ));
                            }
                            document
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            StoredDocument::empty(experiment)
                        }
                        Err(error) => {
                            return Err(format!(
                                "pull evidence at {} could not be read: {error}",
                                path.display()
                            ))
                        }
                    }
                }
                None => StoredDocument::empty(experiment),
            };
            self.documents.insert(experiment.to_string(), document);
        }
        self.documents
            .get_mut(experiment)
            .ok_or_else(|| "pull evidence document vanished".to_string())
    }

    fn persist(&self, experiment: &str) -> Result<(), String> {
        let Some(root) = &self.root else {
            return Ok(());
        };
        let document = self
            .documents
            .get(experiment)
            .ok_or_else(|| "pull evidence document vanished".to_string())?;
        std::fs::create_dir_all(root)
            .map_err(|error| format!("pull evidence directory {}: {error}", root.display()))?;
        let path = path_for(root, &self.namespace, experiment);
        // Two workers must not interleave load -> mutate -> rename here: a
        // later daily read from another process would be lost.
        let _state_lock = crate::file_lock::StateLock::acquire(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let bytes = serde_json::to_vec_pretty(document)
            .map_err(|error| format!("pull evidence serialization: {error}"))?;
        let name = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("pull-evidence.json");
        let sequence = TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = path.with_file_name(format!(".{name}.{}.{sequence}.tmp", std::process::id()));
        let write = (|| -> Result<(), String> {
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp)
                .map_err(|error| format!("pull evidence create {}: {error}", tmp.display()))?;
            use std::io::Write;
            file.write_all(&bytes)
                .map_err(|error| format!("pull evidence write {}: {error}", tmp.display()))?;
            file.sync_all()
                .map_err(|error| format!("pull evidence sync {}: {error}", tmp.display()))?;
            std::fs::rename(&tmp, &path)
                .map_err(|error| format!("pull evidence rename {}: {error}", path.display()))?;
            #[cfg(unix)]
            if let Some(parent) = path.parent() {
                std::fs::File::open(parent)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|error| {
                        format!("pull evidence directory sync {}: {error}", parent.display())
                    })?;
            }
            Ok(())
        })();
        if write.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        write
    }
}

static TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn path_for(root: &Path, namespace: &str, experiment: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(namespace.as_bytes());
    hasher.update(b"\0");
    hasher.update(experiment.as_bytes());
    let digest = hasher.finalize();
    let name: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    root.join(format!("{name}.json"))
}

/// The accumulated arm-day: the first and latest reporting reads stay stored
/// exactly as read, but the pair the analysis sees is the two **latest** reads
/// (penultimate, latest) once a third read arrives, so finality is never
/// judged against the never-changing first read.
fn arm_day(day: &str, entry: &StoredEntry) -> ArmDay {
    let earlier = entry.penultimate.as_ref().unwrap_or(&entry.first);
    let later = entry.second.as_ref();
    ArmDay {
        day: day.to_string(),
        revenue: earlier.revenue,
        users: earlier.users,
        second_pull_revenue: later.map(|second| second.revenue),
        second_pull_users: later.and_then(|second| second.users),
        fill: earlier.fill,
        second_pull_fill: later.and_then(|second| second.fill),
        first_observed_at_unix_s: Some(earlier.observed_at_unix_s),
        second_observed_at_unix_s: later.map(|second| second.observed_at_unix_s),
        first_source_sha256: Some(earlier.source_sha256.clone()),
        second_source_sha256: later.map(|second| second.source_sha256.clone()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use apex_core::Micros;
    use apex_lift::window::arm_day_source_sha256;

    fn observation(
        revenue: Micros,
        users: Option<u64>,
        observed_at: i64,
        marker: &str,
    ) -> PullObservation {
        PullObservation {
            revenue,
            users,
            fill: None,
            observed_at_unix_s: observed_at,
            source_sha256: arm_day_source_sha256("2026-10-01", marker, revenue, users, None),
        }
    }

    fn read(
        store: &mut PullEvidenceStore,
        experiment: &str,
        observed_at: i64,
        control_revenue: Micros,
        control_users: Option<u64>,
        test_revenue: Micros,
        test_users: Option<u64>,
    ) -> ExperimentDays {
        store
            .record_read(
                experiment,
                [
                    (
                        EvidenceArm::Control,
                        "2026-10-01".to_string(),
                        observation(control_revenue, control_users, observed_at, "control"),
                    ),
                    (
                        EvidenceArm::Test,
                        "2026-10-01".to_string(),
                        observation(test_revenue, test_users, observed_at, "test"),
                    ),
                ],
            )
            .expect("record read")
    }

    #[test]
    fn a_first_read_is_never_finality_evidence() {
        let mut store = PullEvidenceStore::in_memory("test");
        let days = read(&mut store, "exp", 1_000, 100, Some(10), 120, Some(12));
        assert_eq!(days.control[0].second_pull_revenue, None);
        assert_eq!(days.test[0].second_pull_revenue, None);
        assert_eq!(
            store
                .observations("exp", EvidenceArm::Control, "2026-10-01")
                .expect("stored")
                .1,
            None
        );
    }

    #[test]
    fn the_same_read_replayed_is_rejected() {
        let mut store = PullEvidenceStore::in_memory("test");
        let first = observation(100, Some(10), 1_000, "control");
        assert_eq!(
            store
                .observe("exp", EvidenceArm::Control, "2026-10-01", first.clone())
                .expect("first"),
            ObservationOutcome::First
        );
        assert_eq!(
            store
                .observe("exp", EvidenceArm::Control, "2026-10-01", first)
                .expect("same read"),
            ObservationOutcome::SameRead
        );
        assert!(
            store
                .observations("exp", EvidenceArm::Control, "2026-10-01")
                .expect("stored")
                .1
                .is_none(),
            "the first read is not reused as the second"
        );
    }

    #[test]
    fn a_second_read_inside_twenty_four_hours_is_not_accepted() {
        let mut store = PullEvidenceStore::in_memory("test");
        let first = observation(100, Some(10), 1_000, "control");
        store
            .observe("exp", EvidenceArm::Control, "2026-10-01", first)
            .expect("first");
        let soon = observation(120, Some(12), 1_000 + SECOND_PULL_MIN_SECS - 1, "control");
        assert_eq!(
            store
                .observe("exp", EvidenceArm::Control, "2026-10-01", soon)
                .expect("soon"),
            ObservationOutcome::TooSoon
        );
        let (first_stored, second_stored) = store
            .observations("exp", EvidenceArm::Control, "2026-10-01")
            .expect("stored");
        assert_eq!(first_stored.revenue, 100);
        assert!(second_stored.is_none());
    }

    #[test]
    fn a_delayed_independent_read_becomes_the_persisted_second_observation() {
        let root = std::env::temp_dir().join(format!(
            "apex-pull-evidence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let experiment = "exp::durable";
        {
            let mut store = PullEvidenceStore::at("test", &root);
            let days = read(&mut store, experiment, 1_000, 100, Some(10), 120, Some(12));
            assert_eq!(days.control[0].second_pull_revenue, None);
        }
        {
            let mut store = PullEvidenceStore::at("test", &root);
            let days = read(
                &mut store,
                experiment,
                1_000 + SECOND_PULL_MIN_SECS,
                100,
                Some(10),
                120,
                Some(12),
            );
            assert_eq!(days.control[0].second_pull_revenue, Some(100));
            assert_eq!(days.control[0].second_pull_users, Some(10));
            assert_eq!(days.test[0].second_pull_revenue, Some(120));
            assert!(days.control[0].second_observed_at_unix_s.is_some());
            assert!(days.control[0].second_source_sha256.is_some());
            assert_eq!(
                store
                    .observe(
                        experiment,
                        EvidenceArm::Control,
                        "2026-10-01",
                        observation(999, Some(99), 1_000 + 2 * SECOND_PULL_MIN_SECS, "control"),
                    )
                    .expect("third read"),
                ObservationOutcome::Second,
                "a later disagreeing read replaces the second observation"
            );
            let days = read(
                &mut store,
                experiment,
                1_000 + 2 * SECOND_PULL_MIN_SECS,
                999,
                Some(99),
                120,
                Some(12),
            );
            assert_eq!(days.control[0].second_pull_revenue, Some(999));
            assert_eq!(
                store
                    .observe(
                        experiment,
                        EvidenceArm::Control,
                        "2026-10-01",
                        observation(999, Some(99), 1_000 + 2 * SECOND_PULL_MIN_SECS, "control"),
                    )
                    // adversarial-ok: test assertion
                    .expect("replayed third read"),
                ObservationOutcome::AlreadyFinal
            );
        }
        std::fs::remove_dir_all(&root).expect("clean");
    }

    #[test]
    fn a_later_read_with_the_same_hash_is_not_a_replay() {
        let mut store = PullEvidenceStore::in_memory("test");
        read(&mut store, "exp", 1_000, 100, Some(10), 120, Some(12));
        // Same content at a later time: it is still the second observation,
        // not the identical first-read replay.
        let days = read(
            &mut store,
            "exp",
            1_000 + SECOND_PULL_MIN_SECS,
            100,
            Some(10),
            120,
            Some(12),
        );
        assert_eq!(days.control[0].second_pull_revenue, Some(100));
    }

    #[test]
    fn finality_rides_the_two_latest_reads_not_the_first() {
        // A3-V4 E6: reads 1 and 2 disagree, reads 2 and 3 agree. The day is
        // final because the two latest reads agree, even though the first
        // read (which never changes) disagrees with both.
        let mut store = PullEvidenceStore::in_memory("test");
        let mut at = 1_000;
        read(&mut store, "exp", at, 100, Some(10), 120, Some(12));
        at += SECOND_PULL_MIN_SECS;
        read(&mut store, "exp", at, 200, Some(20), 120, Some(12));
        let days = store.experiment_days("exp");
        assert!(
            !apex_lift::window::day_is_final(&days.control[0]),
            "the latest pair (100, 200) disagrees"
        );
        at += SECOND_PULL_MIN_SECS;
        read(&mut store, "exp", at, 200, Some(20), 120, Some(12));
        let days = store.experiment_days("exp");
        let control = &days.control[0];
        assert_eq!(
            control.revenue, 200,
            "the analyzed value is the penultimate read"
        );
        assert_eq!(control.second_pull_revenue, Some(200));
        assert!(
            apex_lift::window::day_is_final(control),
            "reads 2 and 3 agree: the day is final despite the first read"
        );
        // The first read is retained as the audit record, never as the pair.
        let (first, _) = store
            .observations("exp", EvidenceArm::Control, "2026-10-01")
            .expect("stored");
        assert_eq!(first.revenue, 100);

        // A newer disagreeing read unseats finality; the next agreeing read
        // restores it.
        at += SECOND_PULL_MIN_SECS;
        read(&mut store, "exp", at, 300, Some(30), 120, Some(12));
        let days = store.experiment_days("exp");
        assert!(
            !apex_lift::window::day_is_final(&days.control[0]),
            "the latest pair (200, 300) disagrees"
        );
        at += SECOND_PULL_MIN_SECS;
        read(&mut store, "exp", at, 300, Some(30), 120, Some(12));
        let days = store.experiment_days("exp");
        assert!(
            apex_lift::window::day_is_final(&days.control[0]),
            "any two consecutive later reads that agree make the day final"
        );
    }

    #[test]
    fn an_empty_reporting_window_is_not_an_observation() {
        let mut store = PullEvidenceStore::in_memory("test");
        let empty = store
            .record_read("exp", std::iter::empty())
            .expect("empty read");
        assert!(empty.control.is_empty() && empty.test.is_empty());
        let days = read(&mut store, "exp", 1_000, 100, Some(10), 120, Some(12));
        assert_eq!(days.control.len(), 1);
        let again = store
            .record_read("exp", std::iter::empty())
            .expect("empty read");
        assert_eq!(again.control.len(), 1, "the stored day is still returned");
        assert_eq!(again.control[0].second_pull_revenue, None);
    }

    #[test]
    fn a_malformed_document_fails_closed() {
        let root = std::env::temp_dir().join(format!(
            "apex-pull-evidence-bad-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("dir");
        let path = path_for(&root, "test", "exp");
        std::fs::write(&path, "{not json}").expect("write");
        let mut store = PullEvidenceStore::at("test", &root);
        let error = store
            .observe(
                "exp",
                EvidenceArm::Control,
                "2026-10-01",
                observation(100, None, 1_000, "control"),
            )
            .expect_err("malformed");
        assert!(error.contains("not valid JSON"), "{error}");
        std::fs::remove_dir_all(&root).expect("clean");
    }
}
