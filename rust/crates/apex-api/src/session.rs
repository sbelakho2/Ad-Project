//! In-memory browser sessions for the console.
//!
//! The token exchange on `POST /console/ledger` never keeps the publisher's
//! access token: it mints an opaque, short-lived session id and puts only that
//! id in an `HttpOnly`, host-only cookie scoped to `/`. The session is the
//! console's whole access boundary: every console page except the access
//! panel itself resolves its publisher from the live session, so one
//! deployment serves many publishers without ever falling back to a
//! deployment-wide default. The store here is the only place a session lives:
//! bounded, pruned on every use, and gone on process restart, so a leaked
//! cookie has a small window and no server-side secret to leak.
//!
//! - The cookie id and the CSRF token are 128 random bits hex-encoded; they
//!   carry no publisher data. The id is never displayed; the access page
//!   lists sessions by a separate public `handle` (64 random bits) so the
//!   cookie value cannot leak through the settings page.
//! - A session expires after [`SESSION_IDLE_MS`] of idleness (15 minutes);
//!   every successful read renews it.
//! - The store keeps at most its capacity; when full, the least recently used
//!   session is evicted first (oldest `last_seen_ms`, ties broken by the
//!   smallest session id so eviction is deterministic).
//! - Each session carries a per-session CSRF token for the session-end form
//!   and every other state-changing console action (settings, connections,
//!   proposal decisions).

use std::collections::{BTreeSet, HashMap};
use std::sync::Mutex;

use rand::RngCore;

/// Name of the session cookie.
pub const SESSION_COOKIE: &str = "apex_ledger_session";
/// Idle lifetime of one session in milliseconds: 15 minutes.
pub const SESSION_IDLE_MS: i64 = 15 * 60 * 1000;
/// Default upper bound of concurrent sessions kept in memory.
pub const DEFAULT_SESSION_CAPACITY: usize = 1024;

#[derive(Debug, Clone)]
struct SessionState {
    publisher: String,
    csrf: String,
    /// Public, non-secret identifier the access page can list and revoke by.
    handle: String,
    created_ms: i64,
    last_seen_ms: i64,
}

#[derive(Debug, Default)]
struct Inner {
    sessions: HashMap<String, SessionState>,
    /// `(last_seen_ms, session id)` in eviction order. The oldest entry is
    /// `first()`: least recently used, ties broken by the smallest id exactly
    /// as the previous linear scan did. Keeping the order here makes pruning
    /// cost the expired entries (not the whole store) and eviction cost a
    /// `log n` removal.
    by_age: BTreeSet<(i64, String)>,
    /// Entries dropped by idle expiry (tests and diagnostics).
    pruned: u64,
    /// Entries dropped because the store was at capacity (tests).
    evictions: u64,
}

/// A bounded in-memory store of opaque console sessions.
#[derive(Debug)]
pub struct LedgerSessions {
    inner: Mutex<Inner>,
    capacity: usize,
    idle_ms: i64,
}

/// What opening a session hands back: the cookie value and the per-session
/// CSRF token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedSession {
    /// The opaque session id stored in the cookie.
    pub id: String,
    /// The per-session CSRF token.
    pub csrf: String,
    /// The public, non-secret handle for lists and revocation.
    pub handle: String,
}

/// A live session read back on a later request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionView {
    /// The publisher this session is bound to.
    pub publisher: String,
    /// The per-session CSRF token.
    pub csrf: String,
    /// The public, non-secret handle for lists and revocation.
    pub handle: String,
}

/// One row of the access page: a live session, identified by its public
/// handle (never the cookie value).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    /// The public handle.
    pub handle: String,
    /// The publisher the session is bound to.
    pub publisher: String,
    /// When the session was opened.
    pub created_ms: i64,
    /// When the session was last used.
    pub last_seen_ms: i64,
}

impl LedgerSessions {
    /// A store keeping at most `capacity` sessions, each idle-expiring after
    /// `idle_ms` milliseconds.
    pub fn new(capacity: usize, idle_ms: i64) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            capacity: capacity.max(1),
            idle_ms: idle_ms.max(1),
        }
    }

    /// Open a session bound to `publisher`, pruning expired entries and
    /// evicting the least recently used session when at capacity.
    pub fn open(&self, publisher: &str, now_unix_ms: i64) -> OpenedSession {
        let id = random_hex_32();
        let csrf = random_hex_32();
        let handle = random_hex_16();
        let mut inner = self.lock();
        inner.prune(now_unix_ms, self.idle_ms);
        while inner.sessions.len() >= self.capacity {
            // Oldest `last_seen_ms` first; equal millisecond timestamps are
            // broken by the smallest session id so the victim is
            // deterministic instead of arbitrary map order.
            if !inner.evict_oldest() {
                break;
            }
        }
        inner.by_age.insert((now_unix_ms, id.clone()));
        inner.sessions.insert(
            id.clone(),
            SessionState {
                publisher: publisher.to_string(),
                csrf: csrf.clone(),
                handle: handle.clone(),
                created_ms: now_unix_ms,
                last_seen_ms: now_unix_ms,
            },
        );
        OpenedSession { id, csrf, handle }
    }

    /// Look up a live session, renewing its idle window. An expired or unknown
    /// id returns `None` and the entry is dropped.
    pub fn touch(&self, id: &str, now_unix_ms: i64) -> Option<SessionView> {
        let mut inner = self.lock();
        let expired = {
            let state = inner.sessions.get(id)?;
            now_unix_ms.saturating_sub(state.last_seen_ms) >= self.idle_ms
        };
        if expired {
            inner.remove(id);
            return None;
        }
        let previous = {
            let state = inner.sessions.get_mut(id)?;
            let previous = state.last_seen_ms;
            state.last_seen_ms = now_unix_ms;
            let view = SessionView {
                publisher: state.publisher.clone(),
                csrf: state.csrf.clone(),
                handle: state.handle.clone(),
            };
            (previous, view)
        };
        let (previous, view) = previous;
        if previous != now_unix_ms {
            inner.by_age.remove(&(previous, id.to_string()));
            inner.by_age.insert((now_unix_ms, id.to_string()));
        }
        Some(view)
    }

    /// Every live session, pruned of expired entries, ordered by first seen.
    /// Identified by the public handle only.
    pub fn summaries(&self, now_unix_ms: i64) -> Vec<SessionSummary> {
        let mut inner = self.lock();
        inner.prune(now_unix_ms, self.idle_ms);
        let mut out: Vec<SessionSummary> = inner
            .sessions
            .values()
            .map(|state| SessionSummary {
                handle: state.handle.clone(),
                publisher: state.publisher.clone(),
                created_ms: state.created_ms,
                last_seen_ms: state.last_seen_ms,
            })
            .collect();
        out.sort_by(|a, b| {
            a.created_ms
                .cmp(&b.created_ms)
                .then_with(|| a.handle.cmp(&b.handle))
        });
        out
    }

    /// Revoke the session `handle` when it is bound to `publisher`. Sessions
    /// of any other publisher are not touchable from this page.
    pub fn revoke_for(&self, publisher: &str, handle: &str) -> bool {
        let mut inner = self.lock();
        let victim = inner
            .sessions
            .iter()
            .find(|(_, state)| state.handle == handle && state.publisher == publisher)
            .map(|(id, _)| id.clone());
        match victim {
            Some(id) => inner.remove(&id).is_some(),
            None => false,
        }
    }

    /// Drop a session. Returns whether it existed.
    pub fn close(&self, id: &str) -> bool {
        self.lock().remove(id).is_some()
    }

    /// Number of sessions currently kept.
    pub fn len(&self) -> usize {
        self.lock().sessions.len()
    }

    /// True when no session is kept.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Lock the store. A poisoned lock (another thread panicked while holding
    /// it) recovers the data rather than panicking this request path.
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Inner {
    /// Drop every session whose idle window closed at `now_unix_ms`. The age
    /// index is ordered, so this visits exactly the expired prefix and stops
    /// at the first live entry instead of scanning the whole store.
    fn prune(&mut self, now_unix_ms: i64, idle_ms: i64) {
        while let Some((last_seen_ms, id)) = self.by_age.first().cloned() {
            if now_unix_ms.saturating_sub(last_seen_ms) < idle_ms {
                break;
            }
            self.by_age.remove(&(last_seen_ms, id.clone()));
            if self.sessions.remove(&id).is_some() {
                self.pruned += 1;
            }
        }
    }

    /// Drop the least recently used session (smallest id on a timestamp tie).
    fn evict_oldest(&mut self) -> bool {
        let Some((last_seen_ms, id)) = self.by_age.first().cloned() else {
            return false;
        };
        self.by_age.remove(&(last_seen_ms, id.clone()));
        if self.sessions.remove(&id).is_some() {
            self.evictions += 1;
            true
        } else {
            false
        }
    }

    /// Remove one session from both the map and the age index.
    fn remove(&mut self, id: &str) -> Option<SessionState> {
        let state = self.sessions.remove(id)?;
        self.by_age.remove(&(state.last_seen_ms, id.to_string()));
        Some(state)
    }
}

/// 32 lowercase hex characters from 16 random bytes.
fn random_hex_32() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// 16 lowercase hex characters from 8 random bytes: the public session handle.
fn random_hex_16() -> String {
    let mut bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// The `Set-Cookie` value for a session id. `secure` adds the `Secure`
/// attribute (true when the request arrived over HTTPS). The cookie is
/// host-only (no `Domain` attribute), `HttpOnly` and `SameSite=Lax`, scoped to
/// `/` so a browser sends it to the console overview at `/` as well as every
/// `/console` surface. Browsers never send a `Path=/console` cookie to `/`,
/// which made the overview unreachable from a browser; the host-only boundary
/// (not a path boundary) keeps it on this deployment's origin. Every route
/// still enforces the session, so the wider path grants no additional access.
pub fn session_cookie(id: &str, secure: bool) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE}={id}; Path=/; Max-Age={}; HttpOnly; SameSite=Lax",
        SESSION_IDLE_MS / 1000
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// The `Set-Cookie` value that clears the session cookie. The `Path` must
/// match the minted cookie's `Path=/` or the browser would keep the old one.
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

/// A well-formed session id: exactly 32 lowercase hex characters.
pub fn is_session_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A well-formed public session handle: exactly 16 lowercase hex characters.
pub fn is_session_handle(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Whether the request reached the server over HTTPS, as reported by the TLS
/// terminating proxy. The **last** `X-Forwarded-Proto` entry is the hop the
/// trusted proxy appended; earlier entries are client-supplied and cannot
/// change the verdict. An absent header is not secure (the direct peer speaks
/// plain HTTP here).
pub fn request_is_secure(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next_back())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("https"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_792_000_000_000;

    #[test]
    fn open_touch_close_roundtrip_renews_the_idle_window() {
        let sessions = LedgerSessions::new(8, SESSION_IDLE_MS);
        let opened = sessions.open("northstar", T0);
        assert!(is_session_id(&opened.id));
        assert!(is_session_id(&opened.csrf));
        assert!(is_session_handle(&opened.handle));
        assert_ne!(opened.id, opened.csrf);
        let view = sessions.touch(&opened.id, T0 + 60_000).expect("live");
        assert_eq!(view.publisher, "northstar");
        assert_eq!(view.csrf, opened.csrf);
        assert_eq!(view.handle, opened.handle);
        // Renewed: still live 14 minutes after the touch.
        assert!(sessions.touch(&opened.id, T0 + 60_000 + 840_000).is_some());
        assert!(sessions.close(&opened.id));
        assert!(!sessions.close(&opened.id));
        assert!(sessions.touch(&opened.id, T0).is_none());
        assert!(sessions.is_empty());
    }

    #[test]
    fn summaries_list_sessions_without_exposing_the_cookie_id() {
        let sessions = LedgerSessions::new(8, SESSION_IDLE_MS);
        let first = sessions.open("northstar", T0);
        let second = sessions.open("northstar", T0 + 1_000);
        let summaries = sessions.summaries(T0 + 2_000);
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].handle, first.handle);
        assert_eq!(summaries[0].created_ms, T0);
        assert_eq!(summaries[0].last_seen_ms, T0);
        assert_eq!(summaries[1].handle, second.handle);
        // The opaque cookie id never appears in the summary.
        let rendered = format!("{summaries:?}");
        assert!(!rendered.contains(&first.id), "{rendered}");
        assert!(!rendered.contains(&second.id), "{rendered}");
    }

    #[test]
    fn revoke_for_is_scoped_to_the_publishers_own_sessions() {
        let sessions = LedgerSessions::new(8, SESSION_IDLE_MS);
        let mine = sessions.open("northstar", T0);
        let mine_id = mine.id.clone();
        let theirs = sessions.open("other", T0);
        // A handle of another publisher cannot be revoked from this page.
        assert!(!sessions.revoke_for("northstar", &theirs.handle));
        assert!(sessions.touch(&theirs.id, T0).is_some());
        // Our own handle revokes our own session.
        assert!(sessions.revoke_for("northstar", &mine.handle));
        assert!(sessions.touch(&mine_id, T0).is_none());
        assert!(!sessions.revoke_for("northstar", &mine.handle));
        // Unknown handles are refused, and nothing else is touched.
        assert!(!sessions.revoke_for("northstar", "0123456789abcdef"));
        assert!(sessions.touch(&theirs.id, T0 + 1).is_some());
    }

    #[test]
    fn revoke_for_requires_the_exact_handle_and_publisher_pair() {
        let sessions = LedgerSessions::new(8, SESSION_IDLE_MS);
        let opened = sessions.open("northstar", T0);
        assert!(!sessions.revoke_for("northstar2", &opened.handle));
        assert!(!sessions.revoke_for("", &opened.handle));
        assert!(sessions.touch(&opened.id, T0).is_some());
        assert!(sessions.revoke_for("northstar", &opened.handle));
        assert!(sessions.is_empty());
    }

    #[test]
    fn sessions_expire_after_the_idle_window() {
        let sessions = LedgerSessions::new(8, SESSION_IDLE_MS);
        let opened = sessions.open("northstar", T0);
        // Just inside the window: live, and the read renews the idle clock.
        assert!(sessions
            .touch(&opened.id, T0 + SESSION_IDLE_MS - 1)
            .is_some());
        assert!(sessions.touch(&opened.id, T0 + SESSION_IDLE_MS).is_some());
        // A session that is never touched again expires exactly at the
        // window.
        let fresh = sessions.open("other", T0 + SESSION_IDLE_MS);
        assert!(sessions
            .touch(&fresh.id, T0 + 2 * SESSION_IDLE_MS)
            .is_none());
        // The expired entry is dropped, not merely hidden.
        assert_eq!(sessions.len(), 1);
        assert!(sessions.touch(&fresh.id, T0 + 1).is_none());
    }

    #[test]
    fn a_clock_that_moves_backwards_does_not_expire_a_session() {
        let sessions = LedgerSessions::new(8, SESSION_IDLE_MS);
        let opened = sessions.open("pub", T0);
        assert!(sessions.touch(&opened.id, T0 - 5_000).is_some());
    }

    #[test]
    fn capacity_evicts_the_least_recently_used_session() {
        let sessions = LedgerSessions::new(2, SESSION_IDLE_MS);
        let first = sessions.open("one", T0);
        let second = sessions.open("two", T0 + 1_000);
        assert!(sessions.touch(&first.id, T0 + 2_000).is_some());
        let third = sessions.open("three", T0 + 3_000);
        assert_eq!(sessions.len(), 2);
        assert!(sessions.touch(&second.id, T0 + 3_000).is_none(), "evicted");
        assert_eq!(
            sessions
                .touch(&first.id, T0 + 3_000)
                .expect("kept")
                .publisher,
            "one"
        );
        assert_eq!(
            sessions
                .touch(&third.id, T0 + 3_000)
                .expect("kept")
                .publisher,
            "three"
        );
    }

    #[test]
    fn equal_millisecond_sessions_evict_the_smallest_id_deterministically() {
        let sessions = LedgerSessions::new(2, SESSION_IDLE_MS);
        let first = sessions.open("one", T0);
        let second = sessions.open("two", T0);
        // Same-millisecond opens make the two candidates indistinguishable by
        // last_seen_ms; the smallest id is the deterministic victim.
        let victim = if first.id < second.id {
            first.id.clone()
        } else {
            second.id.clone()
        };
        let survivor = if first.id < second.id {
            second.id.clone()
        } else {
            first.id.clone()
        };
        let third = sessions.open("three", T0);
        assert_eq!(sessions.len(), 2);
        assert!(
            sessions.touch(&victim, T0).is_none(),
            "the lexicographically smallest id is evicted on a tie"
        );
        assert!(
            sessions.touch(&survivor, T0).is_some(),
            "the other session survives"
        );
        assert!(sessions.touch(&third.id, T0).is_some());
    }

    /// Churn at a 100k-session capacity: opening a session must cost the
    /// expired prefix plus one eviction, never a scan of the whole store.
    /// The deterministic counters (pruned/evicted, not wall-clock time) show
    /// that one extra open at capacity evicts exactly one entry, and one
    /// clock advance past the idle window prunes exactly the 100k stored
    /// sessions.
    #[test]
    fn one_hundred_thousand_session_churn_does_bounded_work() {
        let sessions = LedgerSessions::new(100_000, SESSION_IDLE_MS);
        let mut ids = Vec::with_capacity(100_000);
        for index in 0..100_000 {
            ids.push(sessions.open("pub", T0 + index).id);
        }
        assert_eq!(sessions.len(), 100_000);

        // At capacity: exactly one eviction (the oldest), no prune scan.
        let extra = sessions.open("pub", T0 + 100_000);
        {
            let inner = sessions.lock();
            assert_eq!(inner.evictions, 1, "one open at capacity evicts one");
            assert_eq!(inner.pruned, 0, "nothing was expired yet");
        }
        assert_eq!(sessions.len(), 100_000);
        assert!(
            sessions.touch(&ids[0], T0 + 100_000).is_none(),
            "the oldest session is the eviction victim"
        );
        assert!(sessions.touch(&extra.id, T0 + 100_000).is_some());

        // Past the idle window every stored session is expired. One open
        // prunes them all and leaves exactly the new session.
        let later = T0 + 100_000 + SESSION_IDLE_MS + 1;
        let fresh = sessions.open("pub", later);
        {
            let inner = sessions.lock();
            assert_eq!(
                inner.pruned, 100_000,
                "exactly the expired entries were visited and dropped"
            );
            assert_eq!(inner.evictions, 1, "no further eviction was needed");
            assert_eq!(inner.sessions.len(), 1);
        }
        assert!(sessions.touch(&fresh.id, later).is_some());
        assert_eq!(sessions.len(), 1, "only the new session remains");
    }

    #[test]
    fn ids_and_csrf_tokens_are_unique_and_unpredictable_shape() {
        let sessions = LedgerSessions::new(64, SESSION_IDLE_MS);
        let mut ids = std::collections::BTreeSet::new();
        for index in 0..64 {
            let opened = sessions.open("pub", T0 + index);
            assert!(is_session_id(&opened.id));
            assert!(is_session_id(&opened.csrf));
            assert!(ids.insert(opened.id), "duplicate session id");
        }
    }

    #[test]
    fn cookie_is_host_only_http_only_lax_and_secure_only_when_asked() {
        let plain = session_cookie("abc", false);
        assert!(plain.contains("HttpOnly"));
        assert!(plain.contains("SameSite=Lax"));
        assert!(plain.contains("Path=/;"));
        assert!(!plain.contains("Secure"));
        assert!(!plain.contains("Domain="));
        assert!(plain.contains("Max-Age=900"));
        let secure = session_cookie("abc", true);
        assert!(secure.ends_with("; Secure"));
        let cleared = cleared_session_cookie();
        assert!(cleared.contains("Max-Age=0"));
        assert!(cleared.contains("Path=/;"));
        assert!(!cleared.contains("Domain="));
    }

    /// A browser sends a cookie only when its `Path` path-matches the request
    /// (RFC 6265 section 5.1.4). The minted cookie's `Path=/` therefore
    /// reaches the overview at `/`, every `/console/*` page and any other path
    /// on the same host; the `Domain`-less cookie is host-only and is never
    /// sent to another origin.
    #[test]
    fn session_cookie_path_reaches_the_overview_and_the_console() {
        let cookie = session_cookie("abc", false);
        let path = cookie
            .split(';')
            .find_map(|part| part.trim().strip_prefix("Path="))
            .expect("the cookie carries a Path");
        assert_eq!(path, "/");
        for request in ["/", "/console", "/console/ledger", "/console/ledger?page=2"] {
            assert!(
                cookie_path_matches(path, request),
                "browser would send the session cookie to {request}"
            );
        }
        assert!(
            cookie_path_matches(path, "/healthz"),
            "a non-console path on the same host receives the host-only cookie"
        );
        assert!(!cookie.contains("Domain="), "host-only: {cookie}");
    }

    /// RFC 6265 path-match, used by the browser-semantics test above.
    fn cookie_path_matches(cookie_path: &str, request_path: &str) -> bool {
        if request_path == cookie_path {
            return true;
        }
        if !request_path.starts_with(cookie_path) {
            return false;
        }
        cookie_path.ends_with('/') || request_path.as_bytes()[cookie_path.len()] == b'/'
    }

    #[test]
    fn cookie_parsing_reads_only_the_session_cookie() {
        assert_eq!(
            session_id_from_cookie(Some("apex_theme=dark; apex_ledger_session=deadbeef")),
            Some("deadbeef".to_string())
        );
        assert_eq!(
            session_id_from_cookie(Some("apex_ledger_session=value#1")).as_deref(),
            Some("value#1")
        );
        assert_eq!(session_id_from_cookie(Some("apex_theme=dark")), None);
        assert_eq!(session_id_from_cookie(Some("apex_ledger_session=")), None);
        assert_eq!(session_id_from_cookie(None), None);
    }

    #[test]
    fn session_id_shape_is_exact() {
        assert!(is_session_id(&"a".repeat(32)));
        assert!(is_session_id("0123456789abcdef0123456789abcdef"));
        assert!(!is_session_id(&"A".repeat(32)));
        assert!(!is_session_id(&"g".repeat(32)));
        assert!(!is_session_id(&"a".repeat(31)));
        assert!(!is_session_id(&"a".repeat(33)));
        assert!(!is_session_id(""));
    }

    #[test]
    fn session_handle_shape_is_exact() {
        assert!(is_session_handle("0123456789abcdef"));
        assert!(is_session_handle(&"a".repeat(16)));
        assert!(!is_session_handle(&"A".repeat(16)));
        assert!(!is_session_handle(&"g".repeat(16)));
        assert!(!is_session_handle(&"a".repeat(15)));
        assert!(!is_session_handle(&"a".repeat(17)));
        assert!(!is_session_handle(""));
    }

    #[test]
    fn https_detection_reads_the_last_forwarded_proto_value() {
        let mut headers = axum::http::HeaderMap::new();
        assert!(!request_is_secure(&headers), "absent header stays insecure");
        headers.insert("x-forwarded-proto", "https".parse().expect("value"));
        assert!(request_is_secure(&headers));
        headers.insert("x-forwarded-proto", "HTTPS".parse().expect("value"));
        assert!(request_is_secure(&headers));
        headers.insert("x-forwarded-proto", "http".parse().expect("value"));
        assert!(!request_is_secure(&headers));
        // A spoofed first entry cannot turn a plain-HTTP last hop secure, and
        // a spoofed plain first entry cannot hide a secure last hop.
        headers.insert("x-forwarded-proto", "https, http".parse().expect("value"));
        assert!(!request_is_secure(&headers));
        headers.insert("x-forwarded-proto", "http, https".parse().expect("value"));
        assert!(request_is_secure(&headers));
    }
}
