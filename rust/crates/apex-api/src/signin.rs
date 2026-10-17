//! Self-serve sign-in: the workspace seam and the one-time link.
//!
//! The free-audit form is the front door: when `POST /free-audit` accepts a
//! submission, two things happen beside storing the lead.
//!
//! 1. **A workspace exists.** The publisher key is derived exactly as the
//!    lead storage derives it ([`crate::leads::publisher_slug`] over the
//!    studio name), and [`WorkspaceTokens::ensure`] creates or extends that
//!    publisher's entry in the deployment's ledger-token file
//!    (`APEX_LEDGER_TOKENS_FILE`, the operator's existing per-publisher
//!    token model). The studio-to-slug mapping itself is recorded through
//!    the existing lead store: the stored lead row carries the slug as its
//!    `publisher` and the studio name as its `displayName`. No second
//!    identity store exists.
//! 2. **A one-time sign-in link is issued.** [`SigninStore`] appends a row
//!    to `signin.jsonl` under the leads root with `at`, `publisher`,
//!    `token_hash` (SHA-256 hex of the opaque token), `expires_at` and
//!    `used_at`; the raw token exists only in the email and in memory. The
//!    email goes through the transactional sink (`APEX_NOTIFY_SINK=email`
//!    builds [`apex_notify::sink::EmailSink`]); without a mailer the link is
//!    logged as a warning and the audit success contract is kept — a saved
//!    lead is success even when the mail path fails.
//!
//! `GET /console/signin?token=<opaque>` consumes a row: a valid, unused,
//! unexpired token mints the **existing** console session (the same
//! [`crate::session::LedgerSessions`] and cookie the ledger-token exchange
//! uses) and answers `303` to `/`. Single use is enforced while the store
//! lock is held, and it applies to minting only: sessions already minted
//! live out their own idle window. The token is never echoed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use apex_metrics::vocab::TemplateClass;
use apex_notify::sink::{DeliveryMessage, EmailSink, NotificationSink, FROM_ENV, SINK_ENV};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::ApiError;

/// The durable one-time link log, under the leads root.
pub const SIGNIN_FILE_NAME: &str = "signin.jsonl";
/// The route that consumes a one-time link.
pub const SIGNIN_PATH: &str = "/console/signin";
/// The query parameter carrying the opaque link token.
pub const SIGNIN_TOKEN_PARAM: &str = "token";
/// Length of the opaque token: 32 lowercase hex characters (128 random bits),
/// the same shape as a console session id.
pub const SIGNIN_TOKEN_HEX_LEN: usize = 32;
/// Largest sign-in log the store reads; a larger file is refused instead of
/// silently serving a truncated view of it.
pub const MAX_SIGNIN_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// Subject of the sign-in email.
pub const SIGNIN_EMAIL_SUBJECT: &str = "Your Apex sign-in link";
/// The "if this wasn't you" line of the email body.
pub const SIGNIN_EMAIL_WRONG_PERSON: &str = "If this wasn't you, you can ignore this email.";
/// The public console base URL environment variable (optional; when unset the
/// email carries the root-relative link).
pub const CONSOLE_URL_ENV: &str = "APEX_CONSOLE_URL";
/// Most recent warnings [`WarningLog`] keeps for diagnostics and tests.
const MAX_WARNING_LINES: usize = 64;

/// One appended sign-in row. The raw token never appears here: only its
/// SHA-256 hex digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SigninRow {
    /// When the link was created, epoch milliseconds.
    pub at: i64,
    /// The publisher key the link signs in.
    pub publisher: String,
    /// SHA-256 hex of the opaque token.
    pub token_hash: String,
    /// Absolute expiry, epoch milliseconds.
    pub expires_at: i64,
    /// When the link was consumed, epoch milliseconds; `null` while unused.
    #[serde(default)]
    pub used_at: Option<i64>,
}

/// A freshly created link: the raw token (memory only) and its durable facts.
#[derive(Clone)]
pub struct CreatedSignin {
    /// The opaque token placed in the link. Never stored.
    pub token: String,
    /// The SHA-256 hex digest that *is* stored.
    pub token_hash: String,
    /// Absolute expiry, epoch milliseconds.
    pub expires_at_unix_ms: i64,
}

impl std::fmt::Debug for CreatedSignin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreatedSignin")
            .field("token", &"<opaque>")
            .field("token_hash", &self.token_hash)
            .field("expires_at_unix_ms", &self.expires_at_unix_ms)
            .finish()
    }
}

/// 32 lowercase hex characters from 16 random bytes.
fn random_token() -> String {
    let mut bytes = [0u8; SIGNIN_TOKEN_HEX_LEN / 2];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// SHA-256 hex of a token.
fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// The exact shape of a sign-in token: exactly 32 lowercase hex characters.
pub fn is_signin_token(value: &str) -> bool {
    value.len() == SIGNIN_TOKEN_HEX_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// The append-only `signin.jsonl` log under the leads root.
///
/// One lock guards both appends and the read-modify-write that marks a row
/// used, so a token can be consumed exactly once even under concurrent
/// requests. Reads never truncate: a malformed line is skipped and an
/// oversized file is a typed error, never a silently short view.
#[derive(Debug)]
pub struct SigninStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl SigninStore {
    /// The store at `<leads_root>/signin.jsonl`.
    pub fn new(leads_root: impl Into<PathBuf>) -> Self {
        Self {
            path: leads_root.into().join(SIGNIN_FILE_NAME),
            lock: Mutex::new(()),
        }
    }

    /// The durable log path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one row for `publisher` and return the raw token (memory only)
    /// together with the stored hash and expiry.
    pub fn create(
        &self,
        publisher: &str,
        now_unix_ms: i64,
        ttl_ms: i64,
    ) -> Result<CreatedSignin, ApiError> {
        if !crate::is_valid_publisher(publisher) {
            return Err(ApiError::BadRequest("invalid_publisher"));
        }
        let token = random_token();
        let hash = token_hash(&token);
        let expires_at = now_unix_ms.saturating_add(ttl_ms);
        let row = SigninRow {
            at: now_unix_ms,
            publisher: publisher.to_string(),
            token_hash: hash.clone(),
            expires_at,
            used_at: None,
        };
        let _guard = self.lock();
        self.append_row(&row)?;
        Ok(CreatedSignin {
            token,
            token_hash: hash,
            expires_at_unix_ms: expires_at,
        })
    }

    /// Consume `token`: on a valid, unused, unexpired row mark `used_at`
    /// under the same lock and return the publisher. Anything else — wrong
    /// shape, unknown, used or expired — is `None`, and the file is not
    /// touched.
    pub fn consume(&self, token: &str, now_unix_ms: i64) -> Result<Option<String>, ApiError> {
        if !is_signin_token(token) {
            return Ok(None);
        }
        let hash = token_hash(token);
        let _guard = self.lock();
        let mut rows = self.read_rows()?;
        let Some(index) = rows.iter().rposition(|row| row.token_hash == hash) else {
            return Ok(None);
        };
        let row = &rows[index];
        if row.used_at.is_some() || now_unix_ms >= row.expires_at {
            return Ok(None);
        }
        let publisher = row.publisher.clone();
        rows[index].used_at = Some(now_unix_ms);
        self.write_rows(&rows)?;
        Ok(Some(publisher))
    }

    /// Every parsed row, in file order. Malformed lines are skipped.
    pub fn read_rows(&self) -> Result<Vec<SigninRow>, ApiError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ApiError::Internal(format!(
                    "sign-in store read {}: {error}",
                    self.path.display()
                )))
            }
        };
        if bytes.len() as u64 > MAX_SIGNIN_FILE_BYTES {
            return Err(ApiError::Internal(format!(
                "sign-in store {} exceeds {MAX_SIGNIN_FILE_BYTES} bytes",
                self.path.display()
            )));
        }
        let text = String::from_utf8(bytes).map_err(|_| {
            ApiError::Internal(format!(
                "sign-in store {} is not UTF-8",
                self.path.display()
            ))
        })?;
        let mut rows = Vec::new();
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            if let Ok(row) = serde_json::from_str::<SigninRow>(line) {
                rows.push(row);
            }
        }
        Ok(rows)
    }

    /// Append one serialized row (mode 0600 on unix).
    fn append_row(&self, row: &SigninRow) -> Result<(), ApiError> {
        let mut line = serde_json::to_string(row)
            .map_err(|error| ApiError::Internal(format!("sign-in row serialization: {error}")))?;
        line.push('\n');
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| ApiError::Internal(format!("sign-in store directory: {error}")))?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options.open(&self.path).map_err(|error| {
            ApiError::Internal(format!(
                "sign-in store open {}: {error}",
                self.path.display()
            ))
        })?;
        file.write_all(line.as_bytes()).map_err(|error| {
            ApiError::Internal(format!(
                "sign-in store write {}: {error}",
                self.path.display()
            ))
        })
    }

    /// Replace the whole log atomically (write a sibling temp file, rename
    /// over the target), so a crash never leaves a half-written row.
    fn write_rows(&self, rows: &[SigninRow]) -> Result<(), ApiError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| ApiError::Internal(format!("sign-in store directory: {error}")))?;
        }
        let mut body = String::new();
        for row in rows {
            let line = serde_json::to_string(row).map_err(|error| {
                ApiError::Internal(format!("sign-in row serialization: {error}"))
            })?;
            body.push_str(&line);
            body.push('\n');
        }
        let mut tmp_name = self.path.clone().into_os_string();
        tmp_name.push(".tmp");
        let tmp = PathBuf::from(tmp_name);
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options.open(&tmp).map_err(|error| {
            ApiError::Internal(format!("sign-in store temp {}: {error}", tmp.display()))
        })?;
        file.write_all(body.as_bytes())
            .map_err(|error| ApiError::Internal(format!("sign-in store temp write: {error}")))?;
        drop(file);
        std::fs::rename(&tmp, &self.path).map_err(|error| {
            ApiError::Internal(format!(
                "sign-in store rename {}: {error}",
                self.path.display()
            ))
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        match self.lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// The per-publisher ledger-token file (`APEX_LEDGER_TOKENS_FILE`) behind the
/// self-serve workspace: a JSON object of publisher to token, exactly the
/// operator's existing format. [`ensure`](Self::ensure) creates or extends an
/// entry without touching any other one.
#[derive(Debug)]
pub struct WorkspaceTokens {
    path: PathBuf,
    inner: Mutex<WorkspaceState>,
}

#[derive(Debug, Default)]
struct WorkspaceState {
    loaded: bool,
    tokens: BTreeMap<String, String>,
}

impl WorkspaceTokens {
    /// A workspace over the token file at `path` (loaded lazily).
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            inner: Mutex::new(WorkspaceState::default()),
        }
    }

    /// The token file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Ensure `publisher` has a ledger token, creating the file and the entry
    /// when needed; an existing entry (operator- or self-serve-created) is
    /// returned unchanged. Returns the publisher's token.
    pub fn ensure(&self, publisher: &str) -> Result<String, ApiError> {
        if !crate::is_valid_publisher(publisher) {
            return Err(ApiError::BadRequest("invalid_publisher"));
        }
        let mut state = self.lock();
        self.load(&mut state)?;
        if let Some(token) = state.tokens.get(publisher) {
            return Ok(token.clone());
        }
        let token = random_workspace_token();
        state.tokens.insert(publisher.to_string(), token.clone());
        self.write(&state.tokens)?;
        Ok(token)
    }

    /// The stored token for `publisher`, when the file holds one. An
    /// unreadable file yields `None` (fail closed).
    pub fn token(&self, publisher: &str) -> Option<String> {
        let mut state = self.lock();
        self.load(&mut state).ok()?;
        state.tokens.get(publisher).cloned()
    }

    /// Whether no publisher token is recorded. An unreadable file counts as
    /// empty so gates fail closed.
    pub fn is_empty(&self) -> bool {
        let mut state = self.lock();
        if self.load(&mut state).is_err() {
            return true;
        }
        state.tokens.is_empty()
    }

    /// Read the JSON object once per store instance.
    fn load(&self, state: &mut WorkspaceState) -> Result<(), ApiError> {
        if state.loaded {
            return Ok(());
        }
        let raw = match std::fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                state.loaded = true;
                return Ok(());
            }
            Err(error) => {
                return Err(ApiError::Internal(format!(
                    "ledger tokens read {}: {error}",
                    self.path.display()
                )))
            }
        };
        let tokens: BTreeMap<String, String> = serde_json::from_str(&raw).map_err(|error| {
            ApiError::Internal(format!(
                "ledger tokens {} are not a JSON object: {error}",
                self.path.display()
            ))
        })?;
        state.tokens = tokens;
        state.loaded = true;
        Ok(())
    }

    /// Write the whole object atomically (temp file plus rename, mode 0600).
    fn write(&self, tokens: &BTreeMap<String, String>) -> Result<(), ApiError> {
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    ApiError::Internal(format!("ledger tokens directory: {error}"))
                })?;
            }
        }
        let mut body = serde_json::to_string_pretty(tokens)
            .map_err(|error| ApiError::Internal(format!("ledger tokens serialization: {error}")))?;
        body.push('\n');
        let mut tmp_name = self.path.clone().into_os_string();
        tmp_name.push(".tmp");
        let tmp = PathBuf::from(tmp_name);
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options.open(&tmp).map_err(|error| {
            ApiError::Internal(format!("ledger tokens temp {}: {error}", tmp.display()))
        })?;
        file.write_all(body.as_bytes())
            .map_err(|error| ApiError::Internal(format!("ledger tokens temp write: {error}")))?;
        drop(file);
        std::fs::rename(&tmp, &self.path).map_err(|error| {
            ApiError::Internal(format!(
                "ledger tokens rename {}: {error}",
                self.path.display()
            ))
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, WorkspaceState> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// 64 lowercase hex characters from 32 random bytes: the self-serve token
/// written beside the operator's own entries.
fn random_workspace_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// A bounded, in-memory record of the sign-in flow's operator warnings.
///
/// Every recorded line is also emitted through `tracing::warn!`, so a
/// deployment log carries it; the bounded copy is the seam tests use to
/// capture the warning. The raw token is never part of a warning.
#[derive(Debug, Default)]
pub struct WarningLog {
    lines: Mutex<Vec<String>>,
}

impl WarningLog {
    /// An empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one operator-facing warning: logged and kept (the newest
    /// [`MAX_WARNING_LINES`] lines).
    pub fn record(&self, message: impl Into<String>) {
        let message = message.into();
        tracing::warn!("{message}");
        let mut lines = match self.lines.lock() {
            Ok(lines) => lines,
            Err(poisoned) => poisoned.into_inner(),
        };
        if lines.len() >= MAX_WARNING_LINES {
            lines.remove(0);
        }
        lines.push(message);
    }

    /// A copy of the recorded lines, oldest first.
    pub fn lines(&self) -> Vec<String> {
        match self.lines.lock() {
            Ok(lines) => lines.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

/// The sign-in half of the free-audit flow: link creation, mail delivery and
/// consumption, over one leads root.
#[derive(Debug)]
pub struct SigninService {
    store: SigninStore,
    ttl_ms: i64,
    public_base: Option<String>,
    mailer: Option<Arc<dyn NotificationSink>>,
    warnings: Arc<WarningLog>,
}

impl SigninService {
    /// A service over `<leads_root>/signin.jsonl` with the default TTL, no
    /// mailer and a fresh warning log.
    pub fn open(leads_root: impl Into<PathBuf>) -> Self {
        Self {
            store: SigninStore::new(leads_root),
            ttl_ms: ttl_ms_from_hours(crate::config::DEFAULT_SIGNIN_TTL_HOURS),
            public_base: None,
            mailer: None,
            warnings: Arc::new(WarningLog::new()),
        }
    }

    /// Set the link lifetime. Callers pass a value already validated by
    /// [`crate::config::signin_ttl_hours`]; the constructor clamps to the
    /// documented bounds as defense in depth.
    #[must_use]
    pub fn with_ttl_hours(mut self, hours: i64) -> Self {
        self.ttl_ms = ttl_ms_from_hours(hours);
        self
    }

    /// Set the public console base the emailed link starts with (for example
    /// `https://console.apexmediation.ee`). Unset keeps the link
    /// root-relative.
    #[must_use]
    pub fn with_public_base(mut self, base: impl Into<String>) -> Self {
        let base = base.into();
        let base = base.trim().trim_end_matches('/').to_string();
        self.public_base = if base.is_empty() { None } else { Some(base) };
        self
    }

    /// Deliver links through `mailer` (the transactional email sink).
    #[must_use]
    pub fn with_mailer(mut self, mailer: Arc<dyn NotificationSink>) -> Self {
        self.mailer = Some(mailer);
        self
    }

    /// Record warnings into `warnings` (tests capture the copy).
    #[must_use]
    pub fn with_warnings(mut self, warnings: Arc<WarningLog>) -> Self {
        self.warnings = warnings;
        self
    }

    /// The configured link lifetime in milliseconds.
    pub fn ttl_ms(&self) -> i64 {
        self.ttl_ms
    }

    /// The durable log.
    pub fn store(&self) -> &SigninStore {
        &self.store
    }

    /// Consume a link token (the route's entry point): `Some(publisher)` on
    /// the single successful use, `None` for anything else.
    pub fn consume(&self, token: &str, now_unix_ms: i64) -> Result<Option<String>, ApiError> {
        self.store.consume(token, now_unix_ms)
    }

    /// The link a token travels in.
    pub fn link_for(&self, token: &str) -> String {
        match &self.public_base {
            Some(base) => format!("{base}{SIGNIN_PATH}?{SIGNIN_TOKEN_PARAM}={token}"),
            None => format!("{SIGNIN_PATH}?{SIGNIN_TOKEN_PARAM}={token}"),
        }
    }

    /// Create the durable single-use row for `publisher` and send the link to
    /// `email`. The email is best effort by contract: an absent mailer or a
    /// delivery failure records a warning and never fails the caller, so a
    /// saved lead stays a success even when the mail path fails.
    pub async fn issue_link(
        &self,
        publisher: &str,
        email: &str,
        now_unix_ms: i64,
    ) -> Result<CreatedSignin, ApiError> {
        let created = self.store.create(publisher, now_unix_ms, self.ttl_ms)?;
        self.deliver(publisher, email, &created).await;
        Ok(created)
    }

    /// Best-effort delivery of one created link.
    async fn deliver(&self, publisher: &str, email: &str, created: &CreatedSignin) {
        let link = self.link_for(&created.token);
        let expires = apex_ui::format::utc_datetime(created.expires_at_unix_ms);
        let Some(mailer) = &self.mailer else {
            self.warnings.record(format!(
                "sign-in link for '{publisher}' was created but not sent: set {SINK_ENV}=email (with {FROM_ENV}) to deliver sign-in links (expires {expires})"
            ));
            return;
        };
        let text = format!(
            "Sign in to your Apex workspace:\n\n{link}\n\nThis link can be used once and expires on {expires}.\n{SIGNIN_EMAIL_WRONG_PERSON}"
        );
        let message = DeliveryMessage {
            to: email.to_string(),
            subject: SIGNIN_EMAIL_SUBJECT.to_string(),
            html: signin_email_html(&link, &expires),
            text,
            // The frozen template vocabulary has no sign-in class; the
            // console-access message family is the honest nearest fit (the
            // class labels delivery metrics and the composed Message-ID
            // identity only).
            template_class: TemplateClass::ReportingAccess,
            dedup_key: format!("signin_link:{}", created.token_hash),
            reference: None,
        };
        match mailer.send(&message).await {
            Ok(receipt) => tracing::info!(
                publisher,
                state = receipt.state.as_str(),
                "sign-in link sent"
            ),
            Err(error) => self.warnings.record(format!(
                "sign-in link for '{publisher}' could not be sent: {error}"
            )),
        }
    }
}

/// The `1..=720` hour window in milliseconds, clamped defensively.
fn ttl_ms_from_hours(hours: i64) -> i64 {
    hours
        .clamp(
            crate::config::MIN_SIGNIN_TTL_HOURS,
            crate::config::MAX_SIGNIN_TTL_HOURS,
        )
        .saturating_mul(3_600_000)
}

/// The minimal HTML twin of the plain-text sign-in email.
fn signin_email_html(link: &str, expires: &str) -> String {
    let href = crate::html_escape(link);
    format!(
        "<p>Sign in to your Apex workspace:</p>\
         <p><a href=\"{href}\">{href}</a></p>\
         <p>This link can be used once and expires on {expires}.</p>\
         <p>{}</p>",
        crate::html_escape(SIGNIN_EMAIL_WRONG_PERSON)
    )
}

/// Build the configured email sink from the environment when
/// `APEX_NOTIFY_SINK=email`; any other selection, or a malformed email
/// configuration, is `None` (with a logged warning for the latter) — the
/// audit flow never fails over mail.
pub fn email_sink_from_env(
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<Arc<dyn NotificationSink>> {
    let selected = lookup(SINK_ENV)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if selected.as_deref() != Some("email") {
        return None;
    }
    match EmailSink::from_env(lookup) {
        Ok(sink) => Some(Arc::new(sink)),
        Err(error) => {
            tracing::warn!(%error, "sign-in email sink is misconfigured; sign-in links stay undelivered");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "apex-signin-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    const HOUR_MS: i64 = 3_600_000;

    #[test]
    fn created_links_store_only_the_hash_and_consume_once() {
        let root = temp_root("store");
        let store = SigninStore::new(&root);
        let created = store
            .create("northstar-studio", 1_000, 72 * HOUR_MS)
            .expect("create");
        assert!(is_signin_token(&created.token), "{}", created.token);
        assert_ne!(
            created.token, created.token_hash,
            "the raw token is not stored"
        );
        assert_eq!(created.expires_at_unix_ms, 1_000 + 72 * HOUR_MS);

        let raw = std::fs::read_to_string(store.path()).expect("log");
        assert!(!raw.contains(&created.token), "raw token never on disk");
        assert!(raw.contains(&created.token_hash));
        let rows = store.read_rows().expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].at, 1_000);
        assert_eq!(rows[0].publisher, "northstar-studio");
        assert_eq!(rows[0].token_hash, token_hash(&created.token));
        assert_eq!(rows[0].used_at, None);

        let publisher = store.consume(&created.token, 2_000).expect("consume");
        assert_eq!(publisher.as_deref(), Some("northstar-studio"));
        let rows = store.read_rows().expect("rows");
        assert_eq!(rows.len(), 1, "the row is marked, not duplicated");
        assert_eq!(rows[0].used_at, Some(2_000));

        // Single use: the same token is refused afterwards and never mints
        // again, and the refusal does not rewrite the row.
        assert_eq!(store.consume(&created.token, 3_000).expect("again"), None);
        assert_eq!(
            store.read_rows().expect("rows")[0].used_at,
            Some(2_000),
            "a refused reuse does not move used_at"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn expiry_is_enforced_at_the_boundary_and_durability_survives_a_restart() {
        let root = temp_root("expiry");
        let store = SigninStore::new(&root);
        let created = store.create("pub", 1_000, 2 * HOUR_MS).expect("create");
        let expires = created.expires_at_unix_ms;

        // A fresh store over the same file sees the row (restart durability).
        let restarted = SigninStore::new(&root);
        assert_eq!(
            restarted.consume(&created.token, expires).expect("expired"),
            None,
            "the exact expiry instant is already expired"
        );
        assert_eq!(
            restarted
                .consume(&created.token, expires - 1)
                .expect("inside"),
            Some("pub".to_string())
        );
        assert_eq!(
            restarted
                .consume(&created.token, expires - 2)
                .expect("used"),
            None
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unknown_and_malformed_tokens_are_refused_without_touching_the_file() {
        let root = temp_root("unknown");
        let store = SigninStore::new(&root);
        for garbage in [
            "",
            "zzzz",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", // 64 hex, not ours
            "0123456789abcdef0123456789abcde",
            "0123456789ABCDEF0123456789ABCDEF",
            "0123456789abcdef0123456789abcdeg",
        ] {
            assert_eq!(
                store.consume(garbage, 1_000).expect("refused"),
                None,
                "{garbage}"
            );
        }
        assert!(
            !store.path().exists(),
            "a malformed or unknown token never creates or rewrites the log"
        );
        let created = store.create("pub", 1_000, 60 * HOUR_MS).expect("create");
        let before = std::fs::read_to_string(store.path()).expect("log");
        assert_eq!(
            store
                .consume(&"0".repeat(SIGNIN_TOKEN_HEX_LEN), 1_000)
                .expect("unknown"),
            None
        );
        assert_eq!(std::fs::read_to_string(store.path()).expect("log"), before);
        assert!(store
            .consume(&created.token, 2_000)
            .expect("valid")
            .is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unavailable_root_is_a_typed_error_never_a_panic() {
        let root = temp_root("unavailable");
        // The sign-in log path is a directory: reads and appends must fail
        // typed (the route turns this into the documented 503).
        let store_path = root.join(SIGNIN_FILE_NAME);
        std::fs::create_dir_all(&store_path).expect("squat the path");
        let store = SigninStore::new(&root);
        assert!(matches!(
            store.create("pub", 1_000, HOUR_MS),
            Err(ApiError::Internal(_))
        ));
        assert!(matches!(store.read_rows(), Err(ApiError::Internal(_))));
        assert!(matches!(
            store.consume(&"a".repeat(SIGNIN_TOKEN_HEX_LEN), 1_000),
            Err(ApiError::Internal(_))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tokens_are_unique_and_id_and_hash_shapes_are_exact() {
        let root = temp_root("unique");
        let store = SigninStore::new(&root);
        let mut tokens = std::collections::BTreeSet::new();
        for index in 0..32 {
            let created = store.create("pub", 1_000 + index, HOUR_MS).expect("create");
            assert!(is_signin_token(&created.token));
            assert_eq!(created.token_hash.len(), 64);
            assert!(created
                .token_hash
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()));
            assert!(tokens.insert(created.token), "duplicate token");
        }
        assert_eq!(store.read_rows().expect("rows").len(), 32);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn workspace_entries_are_created_extended_and_preserved() {
        let root = temp_root("workspace");
        let path = root.join("tokens.json");
        // An operator-managed entry that must survive every extend.
        std::fs::write(&path, "{\n  \"operator-pub\": \"operator-token\"\n}\n").expect("seed");
        let workspaces = WorkspaceTokens::open(&path);
        assert!(!workspaces.is_empty());
        assert_eq!(
            workspaces.token("operator-pub").as_deref(),
            Some("operator-token")
        );

        let first = workspaces.ensure("northstar-studio").expect("ensure");
        assert_eq!(first.len(), 64, "32 random bytes as hex");
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        let again = workspaces.ensure("northstar-studio").expect("idempotent");
        assert_eq!(first, again, "an existing entry is never replaced");
        workspaces.ensure("bright-arcade").expect("second");
        assert_ne!(first, workspaces.token("bright-arcade").expect("stored"));

        let raw = std::fs::read_to_string(&path).expect("tokens file");
        let file: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&raw).expect("json object");
        assert_eq!(
            file.get("operator-pub").map(String::as_str),
            Some("operator-token")
        );
        assert_eq!(
            file.get("northstar-studio").map(String::as_str),
            Some(first.as_str())
        );
        assert_eq!(file.len(), 3);
        // A fresh store over the same file sees the same entries.
        let fresh = WorkspaceTokens::open(&path);
        assert_eq!(
            fresh.token("bright-arcade"),
            workspaces.token("bright-arcade")
        );
        assert!(fresh.ensure("operator-pub").expect("keep") == "operator-token");

        // An invalid publisher never reaches the file.
        assert!(matches!(
            workspaces.ensure("../etc"),
            Err(ApiError::BadRequest("invalid_publisher"))
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_file_is_an_empty_workspace_and_an_invalid_one_fails_closed() {
        let root = temp_root("workspace-invalid");
        let missing = WorkspaceTokens::open(root.join("missing.json"));
        assert!(missing.is_empty());
        assert_eq!(missing.token("pub"), None);
        assert!(missing.ensure("pub").is_ok());

        let path = root.join("broken.json");
        std::fs::write(&path, "not json").expect("write");
        let broken = WorkspaceTokens::open(&path);
        assert!(broken.is_empty(), "an invalid file counts as empty");
        assert_eq!(broken.token("pub"), None);
        assert!(matches!(broken.ensure("pub"), Err(ApiError::Internal(_))));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn ttl_bounds_clamp_defensively() {
        let root = temp_root("ttl");
        let service = SigninService::open(&root).with_ttl_hours(0);
        assert_eq!(
            service.ttl_ms(),
            crate::config::MIN_SIGNIN_TTL_HOURS * HOUR_MS
        );
        let service = SigninService::open(&root).with_ttl_hours(99_999);
        assert_eq!(
            service.ttl_ms(),
            crate::config::MAX_SIGNIN_TTL_HOURS * HOUR_MS
        );
        let service = SigninService::open(&root).with_ttl_hours(72);
        assert_eq!(service.ttl_ms(), 72 * HOUR_MS);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn links_carry_the_configured_base_and_warn_without_a_mailer() {
        let root = temp_root("link");
        let service = SigninService::open(&root);
        assert_eq!(
            service.link_for("ab12"),
            "/console/signin?token=ab12".to_string()
        );
        let service = service.with_public_base("https://console.apexmediation.ee/");
        assert_eq!(
            service.link_for("ab12"),
            "https://console.apexmediation.ee/console/signin?token=ab12"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_email_sink_is_selected_only_for_the_email_kind() {
        assert!(email_sink_from_env(|_| None).is_none());
        assert!(email_sink_from_env(|name| match name {
            SINK_ENV => Some("webhook".to_string()),
            _ => None,
        })
        .is_none());
        assert!(email_sink_from_env(|name| match name {
            SINK_ENV => Some(" file ".to_string()),
            _ => None,
        })
        .is_none());
        // The email kind without a sender is a warned None, not a failure.
        assert!(email_sink_from_env(|name| match name {
            SINK_ENV => Some("email".to_string()),
            _ => None,
        })
        .is_none());
        let sink = email_sink_from_env(|name| match name {
            SINK_ENV => Some("email".to_string()),
            FROM_ENV => Some("Apex <hello@apex-transactional.example>".to_string()),
            _ => None,
        });
        assert!(sink.is_some(), "a configured email sink is built");
    }

    #[test]
    fn warning_log_is_bounded_and_never_carries_the_token() {
        let log = WarningLog::new();
        for index in 0..(MAX_WARNING_LINES + 5) {
            log.record(format!("warning {index}"));
        }
        let lines = log.lines();
        assert_eq!(lines.len(), MAX_WARNING_LINES);
        assert_eq!(lines.first().map(String::as_str), Some("warning 5"));
        let newest = format!("warning {}", MAX_WARNING_LINES + 4);
        assert_eq!(lines.last().map(String::as_str), Some(newest.as_str()));
    }

    #[tokio::test]
    async fn a_created_link_is_logged_as_a_warning_without_a_mailer() {
        let root = temp_root("warn");
        let warnings = Arc::new(WarningLog::new());
        let service = SigninService::open(&root).with_warnings(Arc::clone(&warnings));
        let created = service
            .issue_link("northstar-studio", "you@studio.example", 1_000)
            .await
            .expect("issue");
        let lines = warnings.lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("not sent"), "{}", lines[0]);
        assert!(lines[0].contains(SINK_ENV), "{}", lines[0]);
        assert!(lines[0].contains("northstar-studio"), "{}", lines[0]);
        assert!(
            !lines[0].contains(&created.token),
            "the raw token never enters a warning: {}",
            lines[0]
        );
        // The row still exists: the link is durable even without a mailer.
        assert_eq!(service.store().read_rows().expect("rows").len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
