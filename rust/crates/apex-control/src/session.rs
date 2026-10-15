//! In-memory operator sessions for the control plane.
//!
//! The store mirrors the publisher console's session discipline (bounded,
//! pruned on every use, opaque 128-bit ids, per-session CSRF, gone on
//! restart) but it is a separate implementation with a separate cookie
//! ([`SESSION_COOKIE`], `Path=/control`): a console cookie grants nothing
//! here and this cookie grants nothing there.
//!
//! A session exists only after both factors (password and authenticator
//! code). It records the operator identity and role, issued and expiry
//! times, the last activity, the device label used at sign-in and the moment
//! of the last full authentication, which gates the recent-auth (step-up)
//! check for sensitive capabilities.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::capabilities::Role;
use crate::signing::random_hex_32;

/// Name of the operator session cookie.
pub const SESSION_COOKIE: &str = "apex_operator_session";
/// Idle lifetime of one session in milliseconds: 30 minutes.
pub const SESSION_IDLE_MS: i64 = 30 * 60 * 1000;
/// Absolute lifetime of one session in milliseconds: 8 hours.
pub const SESSION_ABSOLUTE_MS: i64 = 8 * 60 * 60 * 1000;
/// Default upper bound of concurrent sessions kept in memory.
pub const DEFAULT_SESSION_CAPACITY: usize = 256;
/// Prefix of the session cookie path.
pub const SESSION_PATH: &str = "/control";

#[derive(Debug, Clone)]
struct SessionState {
    operator_id: String,
    operator_name: String,
    role: Role,
    csrf: String,
    issued_at_ms: i64,
    expires_at_ms: i64,
    last_seen_ms: i64,
    authenticated_at_ms: i64,
    device_label: String,
}

#[derive(Debug, Default)]
struct Inner {
    sessions: HashMap<String, SessionState>,
}

/// A bounded in-memory store of opaque operator sessions.
#[derive(Debug)]
pub struct OperatorSessions {
    inner: Mutex<Inner>,
    capacity: usize,
    idle_ms: i64,
    absolute_ms: i64,
    step_up_ms: i64,
}

/// What signing in hands back: the cookie value and the per-session CSRF
/// token.
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
    /// Operator identity from the configuration file.
    pub operator_id: String,
    /// Display name.
    pub operator_name: String,
    /// Effective role.
    pub role: Role,
    /// Per-session CSRF token.
    pub csrf: String,
    /// When the session was issued.
    pub issued_at_ms: i64,
    /// When the session expires absolutely.
    pub expires_at_ms: i64,
    /// Last activity.
    pub last_seen_ms: i64,
    /// The device label captured at sign-in (bounded, operator-supplied).
    pub device_label: String,
    /// When both factors were last verified for this session.
    pub authenticated_at_ms: i64,
}

impl SessionView {
    /// Whether a sensitive action may proceed: the last full authentication
    /// is inside the step-up window.
    pub fn recent_auth(&self, now_ms: i64, step_up_ms: i64) -> bool {
        now_ms.saturating_sub(self.authenticated_at_ms) <= step_up_ms
    }
}

impl OperatorSessions {
    /// A store with explicit bounds.
    pub fn new(capacity: usize, idle_ms: i64, absolute_ms: i64, step_up_ms: i64) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            capacity: capacity.max(1),
            idle_ms: idle_ms.max(1),
            absolute_ms: absolute_ms.max(1),
            step_up_ms: step_up_ms.max(1),
        }
    }

    /// The deployment defaults.
    pub fn bounded() -> Self {
        Self::new(
            DEFAULT_SESSION_CAPACITY,
            SESSION_IDLE_MS,
            SESSION_ABSOLUTE_MS,
            crate::capabilities::STEP_UP_WINDOW_MS,
        )
    }

    /// Issue a session after both factors verified. Prunes expired entries
    /// and evicts the least recently used session when at capacity.
    pub fn open(
        &self,
        operator_id: &str,
        operator_name: &str,
        role: Role,
        device_label: &str,
        now_ms: i64,
    ) -> OpenedSession {
        let id = random_hex_32();
        let csrf = random_hex_32();
        let expires_at_ms = now_ms.saturating_add(self.absolute_ms);
        let mut inner = self.lock();
        inner.prune(now_ms, self.idle_ms);
        while inner.sessions.len() >= self.capacity {
            let Some(oldest) = inner
                .sessions
                .iter()
                .min_by_key(|(_, state)| state.last_seen_ms)
                .map(|(id, _)| id.clone())
            else {
                break;
            };
            inner.sessions.remove(&oldest);
        }
        inner.sessions.insert(
            id.clone(),
            SessionState {
                operator_id: operator_id.to_string(),
                operator_name: operator_name.to_string(),
                role,
                csrf: csrf.clone(),
                issued_at_ms: now_ms,
                expires_at_ms,
                last_seen_ms: now_ms,
                authenticated_at_ms: now_ms,
                device_label: device_label.to_string(),
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

    /// Mark both factors re-verified for a live session (the step-up flow).
    pub fn refresh_recent_auth(&self, id: &str, now_ms: i64) -> Option<SessionView> {
        let mut inner = self.lock();
        let state = inner.sessions.get_mut(id)?;
        if now_ms >= state.expires_at_ms
            || now_ms.saturating_sub(state.last_seen_ms) >= self.idle_ms
        {
            inner.sessions.remove(id);
            return None;
        }
        state.authenticated_at_ms = now_ms;
        state.last_seen_ms = now_ms;
        Some(state.view())
    }

    /// Drop one session. Returns whether it existed.
    pub fn close(&self, id: &str) -> bool {
        self.lock().sessions.remove(id).is_some()
    }

    /// Drop every live session of one operator; returns how many existed.
    /// Used by the Security `session_revoke` capability for one session and
    /// for incident response across all of them.
    pub fn close_for_operator(&self, operator_id: &str) -> usize {
        let mut inner = self.lock();
        let before = inner.sessions.len();
        inner
            .sessions
            .retain(|_, state| state.operator_id != operator_id);
        before - inner.sessions.len()
    }

    /// The live sessions, newest activity first, for `/control/access`.
    pub fn list(&self, now_ms: i64) -> Vec<SessionListRow> {
        let mut inner = self.lock();
        inner.prune(now_ms, self.idle_ms);
        let mut rows: Vec<SessionListRow> = inner
            .sessions
            .iter()
            .map(|(id, state)| SessionListRow {
                id_prefix: id.chars().take(8).collect(),
                operator_id: state.operator_id.clone(),
                operator_name: state.operator_name.clone(),
                role: state.role,
                device_label: state.device_label.clone(),
                issued_at_ms: state.issued_at_ms,
                expires_at_ms: state.expires_at_ms,
                last_seen_ms: state.last_seen_ms,
            })
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.last_seen_ms));
        rows
    }

    /// The live session whose id starts with `prefix`, when exactly one
    /// matches. The prefix shown on `/control/access` resolves here; an
    /// ambiguous prefix resolves to `None` and the revoke is refused rather
    /// than guessing.
    pub fn session_id_for_prefix(&self, prefix: &str, now_ms: i64) -> Option<String> {
        if prefix.is_empty() {
            return None;
        }
        let mut inner = self.lock();
        inner.prune(now_ms, self.idle_ms);
        let matches: Vec<&String> = inner
            .sessions
            .keys()
            .filter(|id| id.starts_with(prefix))
            .collect();
        match matches.as_slice() {
            [single] => Some((*single).clone()),
            _ => None,
        }
    }

    /// Number of sessions currently kept.
    pub fn len(&self) -> usize {
        self.lock().sessions.len()
    }

    /// True when no session is kept.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The step-up window in milliseconds.
    pub fn step_up_ms(&self) -> i64 {
        self.step_up_ms
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
            operator_id: self.operator_id.clone(),
            operator_name: self.operator_name.clone(),
            role: self.role,
            csrf: self.csrf.clone(),
            issued_at_ms: self.issued_at_ms,
            expires_at_ms: self.expires_at_ms,
            last_seen_ms: self.last_seen_ms,
            device_label: self.device_label.clone(),
            authenticated_at_ms: self.authenticated_at_ms,
        }
    }
}

/// One row of the `/control/access` session list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionListRow {
    /// First characters of the opaque session id (never the whole handle).
    pub id_prefix: String,
    /// Operator identity.
    pub operator_id: String,
    /// Display name.
    pub operator_name: String,
    /// Effective role.
    pub role: Role,
    /// Bounded device label captured at sign-in.
    pub device_label: String,
    /// Issued time.
    pub issued_at_ms: i64,
    /// Absolute expiry.
    pub expires_at_ms: i64,
    /// Last activity.
    pub last_seen_ms: i64,
}

impl Inner {
    fn prune(&mut self, now_ms: i64, idle_ms: i64) {
        self.sessions.retain(|_, state| {
            now_ms < state.expires_at_ms && now_ms.saturating_sub(state.last_seen_ms) < idle_ms
        });
    }
}

/// The `Set-Cookie` value for a session id. `secure` adds the `Secure`
/// attribute. The cookie is host-only, `HttpOnly`, `SameSite=Lax` and scoped
/// to `/control` so no customer surface ever sees it.
pub fn session_cookie(id: &str, secure: bool) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE}={id}; Path={SESSION_PATH}; Max-Age={}; HttpOnly; SameSite=Lax",
        SESSION_IDLE_MS / 1000
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// The `Set-Cookie` value that clears the session cookie.
pub fn cleared_session_cookie() -> String {
    format!("{SESSION_COOKIE}=; Path={SESSION_PATH}; Max-Age=0; HttpOnly; SameSite=Lax")
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

/// A well-formed operator session id: exactly 32 lowercase hex characters.
pub fn is_session_id(value: &str) -> bool {
    crate::signing::is_random_hex_32(value)
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

    fn sessions() -> OperatorSessions {
        OperatorSessions::new(8, SESSION_IDLE_MS, SESSION_ABSOLUTE_MS, 600_000)
    }

    #[test]
    fn open_touch_close_roundtrip_records_the_session_fields() {
        let store = sessions();
        let opened = store.open("maya", "Maya", Role::Operator, "Chrome on macOS", T0);
        assert!(is_session_id(&opened.id));
        assert!(is_session_id(&opened.csrf));
        assert_ne!(opened.id, opened.csrf);
        assert_eq!(opened.expires_at_ms, T0 + SESSION_ABSOLUTE_MS);
        let view = store.touch(&opened.id, T0 + 60_000).expect("live");
        assert_eq!(view.operator_id, "maya");
        assert_eq!(view.operator_name, "Maya");
        assert_eq!(view.role, Role::Operator);
        assert_eq!(view.csrf, opened.csrf);
        assert_eq!(view.device_label, "Chrome on macOS");
        assert_eq!(view.issued_at_ms, T0);
        assert_eq!(view.last_seen_ms, T0 + 60_000);
        assert!(view.recent_auth(T0 + 600_000, 600_000));
        assert!(store.close(&opened.id));
        assert!(!store.close(&opened.id));
        assert!(store.is_empty());
    }

    #[test]
    fn sessions_expire_on_idle_and_on_the_absolute_cap() {
        let store = sessions();
        let idle = store.open("maya", "Maya", Role::Operator, "d", T0);
        assert!(store.touch(&idle.id, T0 + SESSION_IDLE_MS - 1).is_some());
        assert!(store.touch(&idle.id, T0 + 2 * SESSION_IDLE_MS).is_none());
        // Even with activity inside the idle window, the absolute expiry
        // ends the session (a one-minute cap keeps the test short).
        let short = OperatorSessions::new(8, SESSION_IDLE_MS, 60_000, 600_000);
        let absolute = short.open("sam", "Sam", Role::Operator, "d", T0);
        assert!(short.touch(&absolute.id, T0 + 59_999).is_some());
        assert!(short.touch(&absolute.id, T0 + 60_000).is_none());
    }

    #[test]
    fn recent_auth_window_is_renewed_only_by_refresh() {
        let store = OperatorSessions::new(8, SESSION_IDLE_MS, SESSION_ABSOLUTE_MS, 600_000);
        let opened = store.open("maya", "Maya", Role::Operator, "d", T0);
        let view = store.touch(&opened.id, T0 + 700_000).expect("live");
        assert!(!view.recent_auth(T0 + 700_000, 600_000));
        let refreshed = store
            .refresh_recent_auth(&opened.id, T0 + 700_000)
            .expect("live");
        assert!(refreshed.recent_auth(T0 + 700_000, 600_000));
        assert!(!refreshed.recent_auth(T0 + 700_000 + 600_001, 600_000));
    }

    #[test]
    fn close_for_operator_revokes_every_session_of_one_operator() {
        let store = sessions();
        let _first = store.open("maya", "Maya", Role::Operator, "a", T0);
        let _second = store.open("maya", "Maya", Role::Operator, "b", T0 + 1);
        let other = store.open("sam", "Sam", Role::Finance, "c", T0 + 2);
        assert_eq!(store.close_for_operator("maya"), 2);
        assert_eq!(store.close_for_operator("maya"), 0);
        assert!(store.touch(&other.id, T0 + 3).is_some());
    }

    #[test]
    fn capacity_evicts_the_least_recently_used_session() {
        let store = OperatorSessions::new(2, SESSION_IDLE_MS, SESSION_ABSOLUTE_MS, 600_000);
        let first = store.open("a", "A", Role::Observer, "d", T0);
        let second = store.open("b", "B", Role::Observer, "d", T0 + 1_000);
        assert!(store.touch(&first.id, T0 + 2_000).is_some());
        let third = store.open("c", "C", Role::Observer, "d", T0 + 3_000);
        assert_eq!(store.len(), 2);
        assert!(store.touch(&second.id, T0 + 3_000).is_none(), "evicted");
        assert!(store.touch(&first.id, T0 + 3_000).is_some());
        assert!(store.touch(&third.id, T0 + 3_000).is_some());
    }

    #[test]
    fn list_reports_the_display_fields_newest_first() {
        let store = sessions();
        let older = store.open("maya", "Maya", Role::Operator, "Chrome", T0);
        let newer = store.open("sam", "Sam", Role::Finance, "Firefox", T0 + 1_000);
        assert!(store.touch(&older.id, T0 + 2_000).is_some());
        assert!(store.touch(&newer.id, T0 + 3_000).is_some());
        let rows = store.list(T0 + 3_001);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].operator_name, "Sam");
        assert_eq!(rows[0].role, Role::Finance);
        assert_eq!(rows[0].device_label, "Firefox");
        assert_eq!(rows[1].operator_name, "Maya");
        assert_eq!(rows[1].id_prefix.len(), 8);
        assert!(!rows[0].id_prefix.is_empty());
    }

    #[test]
    fn cookie_is_host_only_http_only_lax_and_control_scoped() {
        let plain = session_cookie("abc", false);
        assert!(plain.contains("apex_operator_session=abc"));
        assert!(plain.contains("Path=/control"));
        assert!(plain.contains("HttpOnly"));
        assert!(plain.contains("SameSite=Lax"));
        assert!(plain.contains("Max-Age=1800"));
        assert!(!plain.contains("Secure"));
        assert!(!plain.contains("Domain="));
        let secure = session_cookie("abc", true);
        assert!(secure.ends_with("; Secure"));
        let cleared = cleared_session_cookie();
        assert!(cleared.contains("Max-Age=0"));
    }

    #[test]
    fn cookie_parsing_reads_only_the_operator_cookie() {
        assert_eq!(
            session_id_from_cookie(Some(
                "apex_theme=dark; apex_operator_session=deadbeef; apex_ledger_session=x"
            )),
            Some("deadbeef".to_string())
        );
        assert_eq!(session_id_from_cookie(Some("apex_ledger_session=x")), None);
        assert_eq!(session_id_from_cookie(Some("apex_operator_session=")), None);
        assert_eq!(session_id_from_cookie(None), None);
    }

    #[test]
    fn session_id_shape_is_exact() {
        assert!(is_session_id(&"a".repeat(32)));
        assert!(!is_session_id(&"A".repeat(32)));
        assert!(!is_session_id(&"a".repeat(33)));
    }
}
