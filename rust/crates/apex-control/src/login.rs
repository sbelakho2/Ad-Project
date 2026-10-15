//! The two-step sign-in state machine.
//!
//! Step one verifies the password and mints a short-lived pending login. Step
//! two verifies the authenticator code against that pending login and only
//! then issues a session. Pending logins are bounded, expire after five
//! minutes and are consumed on success, so a completed code step cannot be
//! replayed and an abandoned password step cannot be resumed hours later.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::signing::random_hex_32;

/// How long a pending login between the two factors stays valid.
pub const PENDING_TTL_MS: i64 = 5 * 60 * 1000;
/// Default bound of concurrent pending logins.
pub const DEFAULT_PENDING_CAPACITY: usize = 512;

#[derive(Debug, Clone)]
struct Pending {
    operator_id: String,
    created_at_ms: i64,
    /// The safe GET view to return to after sign-in or step-up.
    next: Option<String>,
    /// Whether this is a step-up re-authentication of an existing session.
    reauth: bool,
}

#[derive(Debug, Default)]
struct Inner {
    pending: HashMap<String, Pending>,
}

/// A bounded store of pending logins between the two factors.
#[derive(Debug)]
pub struct PendingLogins {
    inner: Mutex<Inner>,
    capacity: usize,
    ttl_ms: i64,
}

impl PendingLogins {
    /// A store with explicit bounds.
    pub fn new(capacity: usize, ttl_ms: i64) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            capacity: capacity.max(1),
            ttl_ms: ttl_ms.max(1),
        }
    }

    /// The deployment default.
    pub fn bounded() -> Self {
        Self::new(DEFAULT_PENDING_CAPACITY, PENDING_TTL_MS)
    }

    /// Open a pending login for `operator_id` after the password verified.
    /// `next` is the safe view to return to after both factors; it is
    /// validated by the caller and only control-plane GET paths are accepted.
    pub fn open(&self, operator_id: &str, next: Option<&str>, now_ms: i64) -> String {
        self.open_with_reason(operator_id, next, false, now_ms)
    }

    /// Open a pending login, marking step-up re-authentications separately so
    /// the code step can refresh an existing session instead of minting a
    /// second one.
    pub fn open_with_reason(
        &self,
        operator_id: &str,
        next: Option<&str>,
        reauth: bool,
        now_ms: i64,
    ) -> String {
        let token = random_hex_32();
        let mut inner = self.lock();
        inner.prune(now_ms, self.ttl_ms);
        while inner.pending.len() >= self.capacity {
            let Some(oldest) = inner
                .pending
                .iter()
                .min_by_key(|(_, pending)| pending.created_at_ms)
                .map(|(token, _)| token.clone())
            else {
                break;
            };
            inner.pending.remove(&oldest);
        }
        inner.pending.insert(
            token.clone(),
            Pending {
                operator_id: operator_id.to_string(),
                created_at_ms: now_ms,
                next: next.map(str::to_string),
                reauth,
            },
        );
        token
    }

    /// Look up a pending login without consuming it (a wrong authenticator
    /// code can be retried until the TTL ends).
    pub fn get(&self, token: &str, now_ms: i64) -> Option<PendingView> {
        let mut inner = self.lock();
        inner.prune(now_ms, self.ttl_ms);
        inner.pending.get(token).map(|pending| PendingView {
            operator_id: pending.operator_id.clone(),
            next: pending.next.clone(),
            reauth: pending.reauth,
        })
    }

    /// Consume a pending login after the second factor verified.
    pub fn consume(&self, token: &str, now_ms: i64) -> Option<PendingView> {
        let mut inner = self.lock();
        inner.prune(now_ms, self.ttl_ms);
        inner.pending.remove(token).map(|pending| PendingView {
            operator_id: pending.operator_id,
            next: pending.next,
            reauth: pending.reauth,
        })
    }

    /// Drop one pending login.
    pub fn close(&self, token: &str) -> bool {
        self.lock().pending.remove(token).is_some()
    }

    /// Number of pending logins kept.
    pub fn len(&self) -> usize {
        self.lock().pending.len()
    }

    /// True when no pending login is kept.
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
    fn prune(&mut self, now_ms: i64, ttl_ms: i64) {
        self.pending
            .retain(|_, pending| now_ms.saturating_sub(pending.created_at_ms) < ttl_ms);
    }
}

/// What a pending login reveals: the operator it belongs to, the safe
/// return path and whether it is a step-up, never the password or a session
/// handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingView {
    /// Operator identity the password verified for.
    pub operator_id: String,
    /// Optional safe GET view to return to.
    pub next: Option<String>,
    /// Whether this pending login is a step-up of an existing session.
    pub reauth: bool,
}

/// Whether `next` is an acceptable post-login destination: only control-plane
/// GET paths, never a scheme, host or anything outside `/control/`.
pub fn safe_next(next: &str) -> Option<&str> {
    let next = next.trim();
    if next.starts_with("/control/")
        && !next.contains("//")
        && !next.contains("..")
        && !next.contains('\\')
        && !next.contains(['\r', '\n', '"', '\'', '<', '>', ' '])
    {
        Some(next)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_792_000_000_000;

    #[test]
    fn open_get_consume_roundtrip() {
        let pending = PendingLogins::bounded();
        let token = pending.open("maya", Some("/control/incidents/inc_018"), T0);
        assert!(crate::signing::is_random_hex_32(&token));
        let view = pending.get(&token, T0 + 1_000).expect("pending");
        assert_eq!(view.operator_id, "maya");
        assert_eq!(view.next.as_deref(), Some("/control/incidents/inc_018"));
        assert!(!view.reauth);
        let step_up = pending.open_with_reason("maya", None, true, T0 + 1_500);
        assert!(pending.get(&step_up, T0 + 1_500).expect("step-up").reauth);
        let consumed = pending.consume(&token, T0 + 2_000).expect("consumed");
        assert_eq!(consumed.operator_id, "maya");
        assert!(!consumed.reauth);
        assert!(pending.get(&token, T0 + 2_000).is_none(), "one shot");
        assert!(pending.consume(&token, T0 + 2_000).is_none());
    }

    #[test]
    fn pending_logins_expire_after_the_ttl() {
        let pending = PendingLogins::new(8, 60_000);
        let token = pending.open("sam", None, T0);
        assert!(pending.get(&token, T0 + 59_999).is_some());
        assert!(pending.get(&token, T0 + 60_000).is_none());
        assert!(pending.is_empty(), "expired entries are dropped");
    }

    #[test]
    fn the_store_is_bounded_and_evicts_oldest() {
        let pending = PendingLogins::new(2, 60_000);
        let first = pending.open("a", None, T0);
        let _second = pending.open("b", None, T0 + 1);
        let _third = pending.open("c", None, T0 + 2);
        assert_eq!(pending.len(), 2);
        assert!(pending.get(&first, T0 + 3).is_none(), "oldest evicted");
    }

    #[test]
    fn safe_next_accepts_only_control_get_paths() {
        assert_eq!(
            safe_next("/control/incidents/inc_018"),
            Some("/control/incidents/inc_018")
        );
        assert_eq!(
            safe_next("/control/finance?view=analytics"),
            Some("/control/finance?view=analytics")
        );
        assert_eq!(safe_next("https://evil.example/control/x"), None);
        assert_eq!(safe_next("//evil.example"), None);
        assert_eq!(safe_next("/control/../etc/passwd"), None);
        assert_eq!(safe_next("/console/audit"), None);
        assert_eq!(safe_next("/control/incidents\\.."), None);
        assert_eq!(safe_next("/control/incidents\nSet-Cookie: x"), None);
        assert_eq!(safe_next(""), None);
    }
}
