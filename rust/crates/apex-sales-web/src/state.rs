//! The server state: everything the router needs, injected once.
//!
//! [`WebState`] is `Clone` (it holds `Arc`s and small values) and pins the
//! clock, so tests and the fixture renderer run deterministic pages while
//! the binary uses the system clock. Two in-memory stores are bounded and
//! pruned on every write: owner sessions ([`crate::session`]) and the
//! idempotency/result memory ([`ActionMemory`]) that makes a replayed form
//! answer the original operation instead of duplicating its durable event.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use apex_metrics::Registry;

use crate::config::WebConfig;
use crate::session::OwnerSessions;

/// The build label stamped into startup lines.
pub const BUILD_LABEL: &str = "apex-sales-web-0.1.0";

/// How many idempotency keys and result pages are remembered at once.
pub const MAX_REMEMBERED_ACTIONS: usize = 256;

/// A source of "now" as Unix milliseconds.
pub trait Clock: Send + Sync + std::fmt::Debug {
    /// Current Unix time in milliseconds.
    fn now_unix_ms(&self) -> i64;
    /// Current Unix time in seconds, the engine's clock unit.
    fn now_unix_secs(&self) -> i64 {
        self.now_unix_ms().div_euclid(1000)
    }
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

/// What a key was seen with before.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RememberedAction {
    /// The fingerprint of the payload the key first submitted.
    pub fingerprint: String,
    /// The operation id the first run produced.
    pub operation_id: String,
}

/// The durable outcome of one executed action, as the result page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionResultRecord {
    /// The displayed operation id (`op_` + 12 hex).
    pub operation_id: String,
    /// The action kind (`approve`, `suppress`, `record`, `reset-health`).
    pub action: String,
    /// The target the action wrote.
    pub target: String,
    /// One factual sentence on what was written.
    pub detail: String,
    /// When the action executed.
    pub at_ms: i64,
    /// The confirming read link (a stable GET route).
    pub confirmed_link: String,
}

/// The outcome of claiming an idempotency key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// First submission: the caller may execute and then remember.
    Fresh,
    /// The same payload was already executed: redirect to its result page.
    Duplicate(String),
    /// The key exists with a different payload fingerprint.
    Conflict,
}

/// Bounded memory of executed actions: idempotency keys, result pages.
#[derive(Debug, Default)]
pub struct ActionMemory {
    by_key: BTreeMap<String, RememberedAction>,
    key_order: VecDeque<String>,
    outcomes: BTreeMap<String, ActionResultRecord>,
    outcome_order: VecDeque<String>,
}

impl ActionMemory {
    /// Claim a key for a payload fingerprint.
    pub fn claim(&mut self, key: &str, fingerprint: &str) -> Claim {
        match self.by_key.get(key) {
            Some(remembered) if remembered.fingerprint == fingerprint => {
                Claim::Duplicate(remembered.operation_id.clone())
            }
            Some(_) => Claim::Conflict,
            None => Claim::Fresh,
        }
    }

    /// Remember the result of a first submission under its key.
    pub fn remember(&mut self, key: &str, fingerprint: &str, record: ActionResultRecord) {
        let operation_id = record.operation_id.clone();
        self.by_key.insert(
            key.to_string(),
            RememberedAction {
                fingerprint: fingerprint.to_string(),
                operation_id: operation_id.clone(),
            },
        );
        self.key_order.push_back(key.to_string());
        self.outcomes.insert(operation_id.clone(), record);
        self.outcome_order.push_back(operation_id);
        self.prune();
    }

    /// The result page for an operation id, when it is still remembered.
    pub fn outcome(&self, operation_id: &str) -> Option<&ActionResultRecord> {
        self.outcomes.get(operation_id)
    }

    fn prune(&mut self) {
        while self.by_key.len() > MAX_REMEMBERED_ACTIONS {
            if let Some(oldest) = self.key_order.pop_front() {
                self.by_key.remove(&oldest);
            } else {
                break;
            }
        }
        while self.outcomes.len() > MAX_REMEMBERED_ACTIONS {
            if let Some(oldest) = self.outcome_order.pop_front() {
                self.outcomes.remove(&oldest);
            } else {
                break;
            }
        }
    }
}

/// The complete dependency set of the sales workspace.
#[derive(Clone, Debug)]
pub struct WebState {
    /// Deployment configuration.
    pub config: WebConfig,
    /// The unconfigured reason, when no owner token is set.
    pub unconfigured_reason: Option<String>,
    /// Owner sessions.
    pub sessions: Arc<OwnerSessions>,
    /// Idempotency keys and action result pages.
    pub actions: Arc<Mutex<ActionMemory>>,
    /// The clock boundary.
    pub clock: Arc<dyn Clock>,
    /// The bounded metrics registry of this origin.
    pub metrics: Arc<Registry>,
}

impl WebState {
    /// Build the state from a configuration.
    pub fn new(config: WebConfig) -> Self {
        let unconfigured_reason = config
            .token
            .is_none()
            .then(|| "APEX_SALES_WEB_TOKEN is not set".to_string());
        Self {
            config,
            unconfigured_reason,
            sessions: Arc::new(OwnerSessions::bounded()),
            actions: Arc::new(Mutex::new(ActionMemory::default())),
            clock: Arc::new(SystemClock),
            metrics: Arc::new(Registry::new()),
        }
    }

    /// Replace the clock (tests and fixtures pin a [`FixedClock`]).
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Current time in Unix milliseconds.
    pub fn now_ms(&self) -> i64 {
        self.clock.now_unix_ms()
    }

    /// Current time in Unix seconds, the engine's unit.
    pub fn now_secs(&self) -> i64 {
        self.clock.now_unix_secs()
    }

    /// Whether the owner gate is configured.
    pub fn is_configured(&self) -> bool {
        self.config.is_configured()
    }

    /// Run `f` over the action memory, recovering a poisoned lock instead of
    /// panicking: a panicked peer must not take the service down.
    pub fn with_actions<T>(&self, f: impl FnOnce(&mut ActionMemory) -> T) -> T {
        match self.actions.lock() {
            Ok(mut guard) => f(&mut guard),
            Err(poisoned) => f(&mut poisoned.into_inner()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(configured: bool) -> WebConfig {
        let mut env = apex_sales::cli::Env::new();
        if configured {
            env.insert("APEX_SALES_WEB_TOKEN".into(), "t".into());
        }
        WebConfig::from_env(&env).unwrap()
    }

    #[test]
    fn an_unset_token_starts_unconfigured() {
        let state = WebState::new(config(false));
        assert!(!state.is_configured());
        assert!(state.unconfigured_reason.is_some());
        assert!(state.sessions.is_empty());
    }

    #[test]
    fn fixed_clock_is_deterministic_and_secs_truncate() {
        let state =
            WebState::new(config(true)).with_clock(Arc::new(FixedClock::new(1_792_055_400_500)));
        assert_eq!(state.now_ms(), 1_792_055_400_500);
        assert_eq!(state.now_secs(), 1_792_055_400);
    }

    #[test]
    fn claims_are_fresh_then_duplicate_then_conflict() {
        let mut memory = ActionMemory::default();
        assert_eq!(memory.claim("k", "approve|t1"), Claim::Fresh);
        let record = ActionResultRecord {
            operation_id: "op_0123456789ab".into(),
            action: "approve".into(),
            target: "t1".into(),
            detail: "wrote".into(),
            at_ms: 1,
            confirmed_link: "/plan".into(),
        };
        memory.remember("k", "approve|t1", record.clone());
        assert_eq!(
            memory.claim("k", "approve|t1"),
            Claim::Duplicate("op_0123456789ab".into())
        );
        assert_eq!(memory.claim("k", "approve|t2"), Claim::Conflict);
        assert_eq!(memory.outcome("op_0123456789ab"), Some(&record));
        assert!(memory.outcome("op_ffffffffffff").is_none());
    }

    #[test]
    fn the_memory_is_bounded() {
        let mut memory = ActionMemory::default();
        for i in 0..(MAX_REMEMBERED_ACTIONS + 10) {
            let key = format!("k{i}");
            let id = format!("op_{i:012x}");
            memory.remember(
                &key,
                &format!("f{i}"),
                ActionResultRecord {
                    operation_id: id.clone(),
                    action: "record".into(),
                    target: "x".into(),
                    detail: "d".into(),
                    at_ms: i as i64,
                    confirmed_link: "/replies".into(),
                },
            );
        }
        assert!(memory.outcome("op_000000000000").is_none(), "oldest pruned");
        assert!(memory
            .outcome(&format!("op_{:012x}", MAX_REMEMBERED_ACTIONS + 9))
            .is_some());
    }
}
