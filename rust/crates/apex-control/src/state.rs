//! The server state: everything the router needs, injected once.
//!
//! [`ControlState`] is `Clone` (it holds `Arc`s and small values) and pins
//! the clock, so tests and the fixture renderer run deterministic operators
//! while the binary uses the system clock. The startup rules:
//!
//! - a configured operator file that is absent starts the service in the
//!   logged unconfigured state (no session can be minted); a configured file
//!   that exists but is invalid refuses startup;
//! - the journal is read once to rebuild the idempotency index, so a
//!   restarted process still answers a duplicate POST with the original
//!   result page;
//! - the analytics emitter is optional and only built when the store is
//!   configured.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use apex_analytics::UtcMillis;

use crate::config::{ConfigError, ControlConfig, Operators};
use crate::data::ControlData;
use crate::events::EventEmitter;
use crate::governance::GovernanceStore;
use crate::jobs::JobTransport;
use crate::journal::{ActionJournal, JournalIndex, RememberedResult};
use crate::login::PendingLogins;
use crate::rate_limit::AttemptLimiter;
use crate::session::OperatorSessions;
use crate::versions::ObjectVersions;

/// The build label stamped into release health and emitted events.
pub const BUILD_LABEL: &str = "apex-control-0.1.0";

/// The most idempotency keys that keep a claim lock entry at once. The map
/// self-cleans: idle entries are pruned when a new key arrives at the cap,
/// so a burst of one-shot keys cannot grow it without bound.
const MAX_SUBMIT_CLAIMS: usize = 1_024;

/// A source of "now" as Unix milliseconds.
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// Current Unix time in milliseconds.
    fn now_unix_ms(&self) -> i64;
}

/// Wall-clock time as reported by the operating system.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

/// A clock frozen at one instant; deterministic tests and fixtures use it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FixedClock {
    now_unix_ms: i64,
}

impl FixedClock {
    /// A clock that always reports `now_unix_ms`.
    pub const fn new(now_unix_ms: i64) -> Self {
        Self { now_unix_ms }
    }
}

impl Clock for FixedClock {
    fn now_unix_ms(&self) -> i64 {
        self.now_unix_ms
    }
}

/// Startup failures that stop the binary.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StateError {
    /// The operator configuration was refused.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The journal could not be read back at startup.
    #[error("the action journal could not be read at startup: {0}")]
    Journal(String),
    /// The operator-jobs transport could not be built.
    #[error("the operator-jobs transport could not be built: {0}")]
    Jobs(String),
}

/// The complete dependency set of the operator control plane.
#[derive(Clone)]
pub struct ControlState {
    /// Deployment configuration.
    pub config: ControlConfig,
    /// Operator directory (possibly empty, the unconfigured state).
    pub operators: Arc<Operators>,
    /// The unconfigured reason, when no operator file is present.
    pub unconfigured_reason: Option<String>,
    /// Operator sessions.
    pub sessions: Arc<OperatorSessions>,
    /// Pending logins between the two factors.
    pub pending: Arc<PendingLogins>,
    /// Login rate limiting and lockout.
    pub limiter: Arc<AttemptLimiter>,
    /// The append-only action journal.
    pub journal: ActionJournal,
    /// The idempotency index built from the journal.
    pub index: Arc<Mutex<JournalIndex>>,
    /// Per-idempotency-key submission claims. One submission at a time runs
    /// per key, so the duplicate check and the durable records are one
    /// critical section ([`ControlState::claim_submission`]).
    claims: Arc<tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// Durable operator jobs (local folder or the split-host endpoint).
    pub jobs: JobTransport,
    /// Object versions for stale-state checks.
    pub versions: Arc<ObjectVersions>,
    /// The read layer.
    pub data: Arc<ControlData>,
    /// Policy and intervention registers.
    pub governance: Arc<GovernanceStore>,
    /// The clock boundary.
    pub clock: Arc<dyn Clock>,
    /// The bounded metrics registry (the shared `apex-metrics` registry with
    /// this origin's bounded vocabularies; see [`crate::telemetry`]).
    pub metrics: Arc<apex_metrics::Registry>,
    /// Analytics emission, when the store is configured.
    pub emitter: Option<EventEmitter>,
}

impl std::fmt::Debug for ControlState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ControlState")
            .field("config", &self.config)
            .field("operators", &self.operators.len())
            .field("unconfigured", &self.unconfigured_reason.is_some())
            .field("sessions", &self.sessions.len())
            .field("clock", &self.clock)
            .finish_non_exhaustive()
    }
}

impl ControlState {
    /// Build the state from a configuration and a read layer, applying the
    /// startup rules described on the module.
    pub fn new(config: ControlConfig, data: ControlData) -> Result<Self, StateError> {
        let (operators, unconfigured_reason) = match &config.operators_path {
            Some(path) if path.exists() => (Operators::load(path)?, None),
            Some(path) => (
                Operators::empty(),
                Some(format!(
                    "no operator configuration file is present at {}",
                    path.display()
                )),
            ),
            None => (
                Operators::empty(),
                Some("APEX_CONTROL_OPERATORS is not set".to_string()),
            ),
        };
        let journal = ActionJournal::new(config.journal_path.clone());
        let read = journal
            .read()
            .map_err(|error| StateError::Journal(error.to_string()))?;
        let index = JournalIndex::from_entries(&read.entries);
        let emitter =
            EventEmitter::build(config.clickhouse.as_ref(), &config.environment, BUILD_LABEL)
                .map_err(|error| StateError::Journal(format!("analytics emitter: {error}")))?;
        let jobs = match &config.jobs_remote {
            Some(remote) => {
                JobTransport::remote(remote).map_err(|error| StateError::Jobs(error.to_string()))?
            }
            None => JobTransport::local(config.jobs_dir.clone()),
        };
        Ok(Self {
            sessions: Arc::new(OperatorSessions::bounded()),
            pending: Arc::new(PendingLogins::bounded()),
            limiter: Arc::new(AttemptLimiter::bounded()),
            journal,
            index: Arc::new(Mutex::new(index)),
            claims: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            jobs,
            versions: Arc::new(ObjectVersions::bounded()),
            governance: Arc::new(GovernanceStore::new(
                config.policies_path.clone(),
                config.interventions_path.clone(),
            )),
            clock: Arc::new(SystemClock),
            metrics: Arc::new(apex_metrics::Registry::new()),
            data: Arc::new(data),
            unconfigured_reason,
            operators: Arc::new(operators),
            emitter,
            config,
        })
    }

    /// Replace the clock (tests and the fixture renderer pin a
    /// [`FixedClock`]).
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Replace the read layer.
    #[must_use]
    pub fn with_data(mut self, data: ControlData) -> Self {
        self.data = Arc::new(data);
        self
    }

    /// Replace the operator directory and clear the unconfigured reason.
    /// The fixture renderer swaps in the illustrative operators; a
    /// deployment serves the directory loaded from its configuration file.
    #[must_use]
    pub fn with_operators(mut self, operators: Operators) -> Self {
        self.operators = Arc::new(operators);
        self.unconfigured_reason = None;
        self
    }

    /// Current time in Unix milliseconds.
    pub fn now_ms(&self) -> i64 {
        self.clock.now_unix_ms()
    }

    /// The as-of time of every read on this request.
    pub fn as_of(&self) -> UtcMillis {
        UtcMillis::from_unix_millis(self.now_ms())
    }

    /// The environment chip label.
    pub fn environment(&self) -> &str {
        &self.config.environment
    }

    /// Whether the deployment is configured (operators present).
    pub fn is_configured(&self) -> bool {
        !self.operators.is_empty()
    }

    /// Claim the submission slot for one idempotency key. The returned
    /// guard serializes the duplicate check and the durable records for the
    /// key, so a concurrent submission with the same key waits and then
    /// observes the original result instead of executing a second time.
    /// Distinct keys never block each other; idle lock entries are pruned
    /// once the map reaches [`MAX_SUBMIT_CLAIMS`].
    pub async fn claim_submission(
        &self,
        idempotency_key: &str,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut claims = self.claims.lock().await;
            if claims.len() >= MAX_SUBMIT_CLAIMS {
                claims.retain(|_, lock| Arc::strong_count(lock) > 1);
            }
            match claims.get(idempotency_key) {
                // A lock with another holder or waiter is joined, so every
                // active submission for the key shares one mutex.
                Some(existing) if Arc::strong_count(existing) > 1 => Arc::clone(existing),
                // An idle entry is replaced: under the map lock its strong
                // count is one (the map itself), so no one is holding it.
                _ => {
                    let fresh = Arc::new(tokio::sync::Mutex::new(()));
                    claims.insert(idempotency_key.to_string(), Arc::clone(&fresh));
                    fresh
                }
            }
        };
        lock.lock_owned().await
    }

    /// The result remembered for an idempotency key, when any; the payload
    /// fingerprint distinguishes a duplicate from a reused key.
    pub fn remembered_result(&self, idempotency_key: &str) -> Option<RememberedResult> {
        let index = match self.index.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        index.result_for(idempotency_key).cloned()
    }

    /// Remember a result id and its payload fingerprint for an idempotency
    /// key.
    pub fn remember_result(
        &self,
        idempotency_key: &str,
        payload_fingerprint: &str,
        result_id: &str,
    ) {
        let mut index = match self.index.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        index.remember(idempotency_key, payload_fingerprint, result_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(dir: &std::path::Path, operators: bool) -> ControlConfig {
        ControlConfig {
            bind: "127.0.0.1:0".parse().expect("addr"),
            operators_path: operators.then(|| dir.join("operators.json")),
            journal_path: dir.join("journal.jsonl"),
            jobs_dir: dir.join("jobs"),
            jobs_remote: None,
            sync_marker: None,
            environment: "TEST".to_string(),
            ledger_root: dir.join("ledgers"),
            invoice_register: dir.join("invoices.jsonl"),
            operations_path: dir.join("operations.json"),
            policies_path: dir.join("policies.jsonl"),
            interventions_path: dir.join("interventions.jsonl"),
            clickhouse: None,
        }
    }

    #[test]
    fn an_absent_operator_file_starts_unconfigured() {
        let dir = std::env::temp_dir().join(format!("apex-control-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state =
            ControlState::new(config_with(&dir, true), ControlData::fixtures()).expect("state");
        assert!(!state.is_configured());
        assert!(state.unconfigured_reason.is_some());
        assert_eq!(state.environment(), "TEST");
        assert!(state.emitter.is_none());
        // No session can be minted without operators.
        assert_eq!(state.sessions.len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_rebuilds_the_idempotency_index_from_the_journal() {
        let dir = std::env::temp_dir().join(format!("apex-control-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = config_with(&dir, false);
        let journal = ActionJournal::new(config.journal_path.clone());
        let entry = crate::journal::ActionJournalEntry {
            at_ms: 1,
            actor: "maya".to_string(),
            role: "operator".to_string(),
            capability: "retry_read".to_string(),
            capability_class: "ingestion".to_string(),
            target: "acct".to_string(),
            outcome: "requested".to_string(),
            reason: String::new(),
            before_digest: "a".repeat(64),
            after_digest: "b".repeat(64),
            idempotency_key: crate::signing::idempotency_fingerprint("key1"),
            payload_fingerprint: Some(crate::signing::payload_fingerprint(
                "retry_read",
                "acct",
                &serde_json::json!({}),
            )),
            result_id: Some("res_1".to_string()),
            job_id: None,
            correlation_id: "c".to_string(),
        };
        journal.append(&entry).expect("append");
        let state = ControlState::new(config, ControlData::fixtures()).expect("state");
        let remembered = state.remembered_result("key1").expect("remembered");
        assert_eq!(remembered.result_id, "res_1");
        assert!(remembered.matches(&crate::signing::payload_fingerprint(
            "retry_read",
            "acct",
            &serde_json::json!({})
        )));
        assert!(!remembered.matches("sha256:other"));
        assert_eq!(state.remembered_result("key2"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_configured_but_invalid_operator_file_refuses_startup() {
        let dir = std::env::temp_dir().join(format!("apex-control-badop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("operators.json"), "{ not json }").expect("write");
        let error = ControlState::new(config_with(&dir, true), ControlData::fixtures())
            .expect_err("invalid operators");
        assert!(matches!(error, StateError::Config(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn one_submission_claims_a_key_serially_and_distinct_keys_do_not_block() {
        let state = ControlState::new(
            config_with(&std::env::temp_dir(), false),
            ControlData::fixtures(),
        )
        .expect("state");
        let first = state.claim_submission("key-one").await;
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let same_key = tokio::spawn({
            let state = state.clone();
            async move {
                started_tx.send(()).expect("started");
                state.claim_submission("key-one").await;
                "same-key"
            }
        });
        started_rx.await.expect("started signal");
        tokio::task::yield_now().await;
        assert!(
            !same_key.is_finished(),
            "a second claim of the same key must wait"
        );
        // A distinct key is never blocked by a held claim.
        let other = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            state.claim_submission("key-two"),
        )
        .await
        .expect("distinct keys do not block");
        drop(other);
        drop(first);
        assert_eq!(same_key.await.expect("join"), "same-key");
    }

    #[test]
    fn fixed_clock_is_deterministic() {
        let state = ControlState::new(
            config_with(&std::env::temp_dir(), false),
            ControlData::fixtures(),
        )
        .expect("state")
        .with_clock(Arc::new(FixedClock::new(1_792_055_400_000)));
        assert_eq!(state.now_ms(), 1_792_055_400_000);
        assert_eq!(state.as_of().unix_millis(), 1_792_055_400_000);
    }
}
