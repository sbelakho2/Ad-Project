//! Owner sessions for the sales workspace.
//!
//! A session exists only after the owner token verified in constant time.
//! The store is bounded in memory, prunes on every use, issues opaque
//! 128-bit ids with a per-session CSRF token, and is gone on restart.
//!
//! The cookie ([`SESSION_COOKIE`] = `apex_sales_session`) is host-only (no
//! `Domain` attribute), `HttpOnly`, `SameSite=Lax`, `Path=/`, gains
//! `Secure` when the request arrived over HTTPS as reported by
//! `X-Forwarded-Proto`, and carries the 8-hour absolute lifetime as its
//! `Max-Age`; the server additionally expires a session idle for more than
//! [`SESSION_IDLE_MS`].

use std::collections::HashMap;
use std::sync::Mutex;

use crate::signing::random_hex_32;

/// Name of the owner session cookie.
pub const SESSION_COOKIE: &str = "apex_sales_session";
/// Idle lifetime of one session in milliseconds: 60 minutes.
pub const SESSION_IDLE_MS: i64 = 60 * 60 * 1000;
/// Absolute lifetime of one session in milliseconds: 8 hours.
pub const SESSION_ABSOLUTE_MS: i64 = 8 * 60 * 60 * 1000;
/// Default upper bound of concurrent sessions kept in memory.
pub const DEFAULT_SESSION_CAPACITY: usize = 128;

#[derive(Debug, Clone)]
struct SessionState {
    csrf: String,
    issued_at_ms: i64,
    expires_at_ms: i64,
    last_seen_ms: i64,
}

#[derive(Debug, Default)]
struct Inner {
    sessions: HashMap<String, SessionState>,
}

/// A bounded in-memory store of opaque owner sessions.
#[derive(Debug)]
pub struct OwnerSessions {
    inner: Mutex<Inner>,
    capacity: usize,
    idle_ms: i64,
    absolute_ms: i64,
}

/// What opening a session hands back: the cookie value and the per-session
/// CSRF token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedSession {
    /// The opaque session id stored in the cookie.
    pub id: String,
    /// The per-session CSRF token.
    pub csrf: String,
    /// Unix milliseconds when the session expires absolutely.
    pub expires_at_ms: i64,
}

/// A live session read back on a later request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionView {
    /// Per-session CSRF token.
    pub csrf: String,
    /// When the session was issued.
    pub issued_at_ms: i64,
    /// When the session expires absolutely.
    pub expires_at_ms: i64,
    /// Last activity.
    pub last_seen_ms: i64,
}

impl OwnerSessions {
    /// A store with explicit bounds.
    pub fn new(capacity: usize, idle_ms: i64, absolute_ms: i64) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            capacity: capacity.max(1),
            idle_ms: idle_ms.max(1),
            absolute_ms: absolute_ms.max(1),
        }
    }

    /// The deployment defaults.
    pub fn bounded() -> Self {
        Self::new(
            DEFAULT_SESSION_CAPACITY,
            SESSION_IDLE_MS,
            SESSION_ABSOLUTE_MS,
        )
    }

    /// Issue a session after the owner token verified. Prunes expired
    /// entries and evicts the least recently used session at capacity.
    pub fn open(&self, now_ms: i64) -> OpenedSession {
        let id = random_hex_32();
        let csrf = random_hex_32();
        let expires_at_ms = now_ms.saturating_add(self.absolute_ms);
        let mut inner = self.lock();
        inner.prune(now_ms, self.idle_ms);
        while inner.sessions.len() >= self.capacity {
            let Some(oldest) = inner
                .sessions
                .iter()
                .min_by_key(|(id, state)| (state.last_seen_ms, id.as_str()))
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            inner.sessions.remove(&oldest);
        }
        inner.sessions.insert(
            id.clone(),
            SessionState {
                csrf: csrf.clone(),
                issued_at_ms: now_ms,
                expires_at_ms,
                last_seen_ms: now_ms,
            },
        );
        OpenedSession {
            id,
            csrf,
            expires_at_ms,
        }
    }

    /// Look up a live session, renewing its idle window. An expired or
    /// unknown id returns `None` and the entry is dropped.
    pub fn touch(&self, id: &str, now_ms: i64) -> Option<SessionView> {
        let mut inner = self.lock();
        let state = inner.sessions.get_mut(id)?;
        let idle_expired = now_ms.saturating_sub(state.last_seen_ms) >= self.idle_ms;
        if idle_expired || now_ms >= state.expires_at_ms {
            inner.sessions.remove(id);
            return None;
        }
        state.last_seen_ms = now_ms;
        Some(state.view())
    }

    /// Drop one session. Returns whether it existed.
    pub fn close(&self, id: &str) -> bool {
        self.lock().sessions.remove(id).is_some()
    }

    /// Number of sessions currently kept.
    pub fn len(&self) -> usize {
        self.lock().sessions.len()
    }

    /// True when no session is kept.
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

impl SessionState {
    fn view(&self) -> SessionView {
        SessionView {
            csrf: self.csrf.clone(),
            issued_at_ms: self.issued_at_ms,
            expires_at_ms: self.expires_at_ms,
            last_seen_ms: self.last_seen_ms,
        }
    }
}

impl Inner {
    fn prune(&mut self, now_ms: i64, idle_ms: i64) {
        self.sessions.retain(|_, state| {
            now_ms < state.expires_at_ms && now_ms.saturating_sub(state.last_seen_ms) < idle_ms
        });
    }
}

/// The `Set-Cookie` value for a session id. `secure` adds the `Secure`
/// attribute. The cookie is host-only, `HttpOnly`, `SameSite=Lax`, scoped to
/// `/` and carries the 8-hour absolute lifetime as `Max-Age`.
pub fn session_cookie(id: &str, secure: bool) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE}={id}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax",
        SESSION_ABSOLUTE_MS / 1000
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// The `Set-Cookie` value that clears the session cookie.
pub fn cleared_session_cookie() -> String {
    format!("{SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax")
}

/// Read the session id from a `Cookie` header value.
pub fn session_id_from_cookie(cookie_header: Option<&str>) -> Option<String> {
    let header = cookie_header?;
    let prefix = format!("{SESSION_COOKIE}=");
    for part in header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix(&prefix) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Whether the request reached the server over HTTPS, as reported by the TLS
/// terminating proxy (`X-Forwarded-Proto: https` as the first value).
pub fn request_is_secure(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("https"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_792_000_000_000;

    #[test]
    fn open_touch_close_roundtrip_records_the_session_fields() {
        let store = OwnerSessions::bounded();
        let opened = store.open(T0);
        assert!(crate::signing::is_random_hex_32(&opened.id));
        assert!(crate::signing::is_random_hex_32(&opened.csrf));
        assert_ne!(opened.id, opened.csrf);
        assert_eq!(opened.expires_at_ms, T0 + SESSION_ABSOLUTE_MS);
        let view = store.touch(&opened.id, T0 + 60_000).expect("live");
        assert_eq!(view.csrf, opened.csrf);
        assert_eq!(view.issued_at_ms, T0);
        assert_eq!(view.last_seen_ms, T0 + 60_000);
        assert!(store.close(&opened.id));
        assert!(!store.close(&opened.id));
        assert!(store.is_empty());
    }

    #[test]
    fn sessions_expire_on_idle_and_on_the_absolute_cap() {
        let store = OwnerSessions::bounded();
        let idle = store.open(T0);
        assert!(store.touch(&idle.id, T0 + SESSION_IDLE_MS - 1).is_some());
        assert!(store.touch(&idle.id, T0 + 2 * SESSION_IDLE_MS).is_none());
        // Even with activity inside the idle window, the absolute expiry
        // ends the session (a one-minute cap keeps the test short).
        let short = OwnerSessions::new(8, SESSION_IDLE_MS, 60_000);
        let absolute = short.open(T0);
        assert!(short.touch(&absolute.id, T0 + 59_999).is_some());
        assert!(short.touch(&absolute.id, T0 + 60_000).is_none());
    }

    #[test]
    fn capacity_evicts_the_least_recently_used_session_deterministically() {
        let store = OwnerSessions::new(2, SESSION_IDLE_MS, SESSION_ABSOLUTE_MS);
        let first = store.open(T0);
        let second = store.open(T0);
        let victim = std::cmp::min(first.id.clone(), second.id.clone());
        let survivor = if victim == first.id {
            second.id.clone()
        } else {
            first.id.clone()
        };
        let third = store.open(T0);
        assert_eq!(store.len(), 2);
        assert!(store.touch(&victim, T0).is_none(), "smallest id evicted");
        assert!(store.touch(&survivor, T0).is_some());
        assert!(store.touch(&third.id, T0).is_some());
    }

    #[test]
    fn cookie_is_host_only_http_only_lax_and_long_lived() {
        let plain = session_cookie("abc", false);
        assert!(plain.contains("apex_sales_session=abc"));
        assert!(plain.contains("Path=/"));
        assert!(plain.contains("HttpOnly"));
        assert!(plain.contains("SameSite=Lax"));
        assert!(plain.contains("Max-Age=28800"));
        assert!(!plain.contains("Secure"));
        assert!(!plain.contains("Domain="));
        let secure = session_cookie("abc", true);
        assert!(secure.ends_with("; Secure"));
        let cleared = cleared_session_cookie();
        assert!(cleared.contains("Max-Age=0"));
    }

    #[test]
    fn cookie_parsing_reads_only_the_sales_cookie() {
        assert_eq!(
            session_id_from_cookie(Some("apex_theme=dark; apex_sales_session=deadbeef")),
            Some("deadbeef".to_string())
        );
        assert_eq!(
            session_id_from_cookie(Some("apex_operator_session=x")),
            None
        );
        assert_eq!(session_id_from_cookie(Some("apex_sales_session=")), None);
        assert_eq!(session_id_from_cookie(None), None);
    }

    #[test]
    fn forwarded_proto_selects_secure() {
        let mut headers = axum::http::HeaderMap::new();
        assert!(!request_is_secure(&headers));
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        assert!(request_is_secure(&headers));
        headers.insert("x-forwarded-proto", "https, http".parse().unwrap());
        assert!(request_is_secure(&headers));
        headers.insert("x-forwarded-proto", "http".parse().unwrap());
        assert!(!request_is_secure(&headers));
    }
}
