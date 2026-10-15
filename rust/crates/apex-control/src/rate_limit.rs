//! Server-side login rate limiting and lockout.
//!
//! Two counters bound every credential attempt: one per operator identity and
//! one per client, where the client key is the SHA-256 of the transport
//! identifier (a forwarded-for value or socket address) so the raw address
//! never appears in memory dumps, logs or events. After
//! [`AttemptLimiter::max_failures`] failures inside the window the key is
//! locked for the lockout span; a successful authentication clears that
//! operator's counter. The store is bounded and pruned on every use, like the
//! session store.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::signing::sha256_hex;

/// Failures allowed inside the window before a lockout.
pub const DEFAULT_MAX_FAILURES: u32 = 5;
/// The counting window for failures.
pub const DEFAULT_WINDOW_MS: i64 = 15 * 60 * 1000;
/// How long a locked key stays locked.
pub const DEFAULT_LOCKOUT_MS: i64 = 15 * 60 * 1000;
/// Default upper bound of distinct keys kept in memory.
pub const DEFAULT_LIMITER_CAPACITY: usize = 4096;

/// Whether a key may attempt right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitVerdict {
    /// The attempt may proceed.
    Allowed,
    /// The key is locked until `until_ms`.
    Locked {
        /// Unix milliseconds when the lockout ends.
        until_ms: i64,
    },
}

#[derive(Debug, Clone)]
struct Attempts {
    failures: u32,
    window_start_ms: i64,
    locked_until_ms: Option<i64>,
}

#[derive(Debug, Default)]
struct Inner {
    attempts: HashMap<String, Attempts>,
}

/// A bounded failure counter with lockout.
#[derive(Debug)]
pub struct AttemptLimiter {
    inner: Mutex<Inner>,
    capacity: usize,
    max_failures: u32,
    window_ms: i64,
    lockout_ms: i64,
}

impl AttemptLimiter {
    /// A limiter with explicit bounds.
    pub fn new(capacity: usize, max_failures: u32, window_ms: i64, lockout_ms: i64) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            capacity: capacity.max(1),
            max_failures: max_failures.max(1),
            window_ms: window_ms.max(1),
            lockout_ms: lockout_ms.max(1),
        }
    }

    /// The deployment defaults.
    pub fn bounded() -> Self {
        Self::new(
            DEFAULT_LIMITER_CAPACITY,
            DEFAULT_MAX_FAILURES,
            DEFAULT_WINDOW_MS,
            DEFAULT_LOCKOUT_MS,
        )
    }

    /// Whether `key` may attempt at `now_ms`.
    pub fn check(&self, key: &str, now_ms: i64) -> LimitVerdict {
        let mut inner = self.lock();
        inner.prune(now_ms, self.window_ms, self.lockout_ms);
        match inner.attempts.get(key) {
            Some(attempts) => match attempts.locked_until_ms {
                Some(until) if until > now_ms => LimitVerdict::Locked { until_ms: until },
                _ => LimitVerdict::Allowed,
            },
            None => LimitVerdict::Allowed,
        }
    }

    /// Record one failure for `key`; returns the verdict for the *next*
    /// attempt, so a caller can stop after the threshold immediately.
    pub fn record_failure(&self, key: &str, now_ms: i64) -> LimitVerdict {
        let mut inner = self.lock();
        inner.prune(now_ms, self.window_ms, self.lockout_ms);
        if !inner.attempts.contains_key(key) {
            inner.evict_to(self.capacity.saturating_sub(1));
        }
        let entry = inner.attempts.entry(key.to_string()).or_insert(Attempts {
            failures: 0,
            window_start_ms: now_ms,
            locked_until_ms: None,
        });
        if now_ms.saturating_sub(entry.window_start_ms) >= self.window_ms {
            entry.failures = 0;
            entry.window_start_ms = now_ms;
        }
        entry.failures = entry.failures.saturating_add(1);
        if entry.failures >= self.max_failures {
            entry.locked_until_ms = Some(now_ms.saturating_add(self.lockout_ms));
            entry.failures = 0;
            entry.window_start_ms = now_ms;
            return LimitVerdict::Locked {
                until_ms: now_ms.saturating_add(self.lockout_ms),
            };
        }
        LimitVerdict::Allowed
    }

    /// Clear the counter for `key` after a successful authentication.
    pub fn record_success(&self, key: &str) {
        self.lock().attempts.remove(key);
    }

    /// Number of keys currently tracked.
    pub fn len(&self) -> usize {
        self.lock().attempts.len()
    }

    /// True when no key is tracked.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Inner {
    fn prune(&mut self, now_ms: i64, window_ms: i64, lockout_ms: i64) {
        self.attempts.retain(|_, attempts| {
            if let Some(until) = attempts.locked_until_ms {
                return until > now_ms;
            }
            now_ms.saturating_sub(attempts.window_start_ms) < window_ms + lockout_ms
        });
    }

    fn evict_to(&mut self, capacity: usize) {
        while self.attempts.len() > capacity {
            let Some(oldest) = self
                .attempts
                .iter()
                .min_by_key(|(_, attempts)| (attempts.locked_until_ms, attempts.window_start_ms))
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.attempts.remove(&oldest);
        }
    }
}

/// The hashed client key for a transport identifier: `sha256:<hex>`. The raw
/// identifier is never retained.
pub fn client_key(raw_client: &str) -> String {
    format!("client:sha256:{}", sha256_hex(raw_client.trim().as_bytes()))
}

/// The per-operator key. Operator ids come from the configuration file, not
/// from request input, but hashing keeps every limiter key uniform.
pub fn operator_key(operator_id: &str) -> String {
    format!("operator:sha256:{}", sha256_hex(operator_id.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_792_000_000_000;

    #[test]
    fn lockout_after_the_threshold_and_until_expiry() {
        let limiter = AttemptLimiter::new(16, 3, 60_000, 300_000);
        assert_eq!(limiter.check("k", T0), LimitVerdict::Allowed);
        assert_eq!(limiter.record_failure("k", T0), LimitVerdict::Allowed);
        assert_eq!(limiter.record_failure("k", T0 + 1), LimitVerdict::Allowed);
        assert_eq!(
            limiter.record_failure("k", T0 + 2),
            LimitVerdict::Locked {
                until_ms: T0 + 2 + 300_000
            }
        );
        assert_eq!(
            limiter.check("k", T0 + 3),
            LimitVerdict::Locked {
                until_ms: T0 + 2 + 300_000
            }
        );
        assert_eq!(limiter.check("k", T0 + 2 + 300_000), LimitVerdict::Allowed);
    }

    #[test]
    fn the_window_resets_stale_failures() {
        let limiter = AttemptLimiter::new(16, 3, 60_000, 300_000);
        assert_eq!(limiter.record_failure("k", T0), LimitVerdict::Allowed);
        assert_eq!(limiter.record_failure("k", T0 + 1), LimitVerdict::Allowed);
        // Two failures, then a quiet window longer than the window span.
        assert_eq!(
            limiter.record_failure("k", T0 + 120_000),
            LimitVerdict::Allowed
        );
        assert_eq!(limiter.check("k", T0 + 120_001), LimitVerdict::Allowed);
    }

    #[test]
    fn success_clears_the_counter_and_keys_are_independent() {
        let limiter = AttemptLimiter::new(16, 2, 60_000, 300_000);
        let _ = limiter.record_failure("a", T0);
        assert_eq!(limiter.record_failure("b", T0), LimitVerdict::Allowed);
        assert_eq!(
            limiter.record_failure("a", T0 + 1),
            LimitVerdict::Locked {
                until_ms: T0 + 1 + 300_000
            }
        );
        limiter.record_success("a");
        assert_eq!(limiter.check("a", T0 + 2), LimitVerdict::Allowed);
        // b still carries its single failure.
        assert_eq!(
            limiter.record_failure("b", T0 + 2),
            LimitVerdict::Locked {
                until_ms: T0 + 2 + 300_000
            }
        );
    }

    #[test]
    fn the_store_is_bounded_and_prunes_completed_entries() {
        let limiter = AttemptLimiter::new(2, 5, 60_000, 300_000);
        for index in 0..10 {
            let _ = limiter.record_failure(&format!("k{index}"), T0 + index);
        }
        assert!(limiter.len() <= 2, "bounded: {}", limiter.len());
        // Long after every window the entries are pruned to nothing.
        assert_eq!(limiter.check("k9", T0 + 10_000_000), LimitVerdict::Allowed);
        assert!(limiter.is_empty(), "{}", limiter.len());
    }

    #[test]
    fn hashed_client_keys_never_contain_the_raw_identifier() {
        let key = client_key("203.0.113.7\n");
        assert!(key.starts_with("client:sha256:"));
        assert!(!key.contains("203.0.113.7"));
        assert_ne!(client_key("203.0.113.7"), client_key("203.0.113.8"));
        let operator = operator_key("maya");
        assert!(operator.starts_with("operator:sha256:"));
        assert!(!operator.contains("maya"));
    }
}
