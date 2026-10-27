//! Self-serve sign-in: the workspace seam and the one-time link.
//!
//! The free-audit form is the front door: when `POST /free-audit` accepts a
//! submission, two things happen beside storing the lead.
//!
//! 1. **A workspace exists.** A new workspace's key is derived from the email
//!    domain the app-ads.txt proof accepted, never from the free-text studio
//!    name. A genuinely new workspace is filed under the collision-resistant
//!    [`hashed_workspace_key`] (`digitalturbine.com` becomes
//!    `digitalturbine-com-<8 hex>`), so two domains whose slugs collide
//!    (`a.b.com` and `a-b.com`) never share a key; the legacy slug key
//!    ([`workspace_key_for_email`]) is reused only for a workspace the
//!    deployment already recognizes ([`workspace_key_for_claim`]), and
//!    [`WorkspaceTokens::ensure`] creates or extends that publisher's entry
//!    in the deployment's ledger-token file (`APEX_LEDGER_TOKENS_FILE`, the
//!    operator's existing per-publisher token model). The studio name is
//!    carried as the lead's `displayName` label; the stored lead remains the
//!    contract. No second identity store exists.
//! 2. **A one-time sign-in link is issued.** When `APEX_CONSOLE_URL` names
//!    the console's public origin, [`SigninStore`] appends a row to
//!    `signin.jsonl` under the leads root with `at`, `publisher`,
//!    `token_hash` (SHA-256 hex of the opaque token), `expires_at` and
//!    `used_at`; the raw token exists only in the email and in memory. The
//!    email goes through the transactional sink (`APEX_NOTIFY_SINK=email`
//!    builds [`apex_notify::sink::EmailSink`]); without a mailer the link is
//!    logged as a warning and the audit success contract is kept — a saved
//!    lead is success even when the mail path fails. `APEX_CONSOLE_URL` is
//!    required to issue links: it must be an absolute `http(s)` URL with a
//!    host and no query/fragment (`https` unless the host is `localhost` or
//!    `127.0.0.1`), and an invalid configured value refuses startup. When it
//!    is unset, links are disabled: [`SigninService::issue_link`] returns
//!    [`ApiError::MissingBase`], records a warning and creates no row, while
//!    the audit still succeeds with its saved lead.
//!
//! `GET /console/signin?token=<opaque>` is inert: it shows a page whose
//! button POSTs the token back, so a mail scanner that follows the link
//! cannot burn it. `POST /console/signin` consumes a row: a valid, unused,
//! unexpired token mints the **existing** console session (the same
//! [`crate::session::LedgerSessions`] and cookie the ledger-token exchange
//! uses) and answers `303` to `/`. Single use is enforced while the store
//! lock is held, and it applies to minting only: sessions already minted
//! live out their own idle window. The token is never echoed.
//!
//! # The additive consume row
//!
//! The log is append-only. Consuming a link appends one marker row instead of
//! rewriting every earlier row:
//!
//! ```json
//! {"kind":"consumed","tokenHash":"<sha256 hex>","usedAtMs":1700000000000}
//! ```
//!
//! The marker carries the hash only, never the raw token. Readers fold each
//! marker onto the last create row carrying that hash, so [`SigninStore::read_rows`]
//! still reports one row per link with `used_at` filled in, and single use is
//! the same. A create row without a marker (every row written by an older
//! version) is live, so existing logs keep their meaning. One consume is one
//! append: durable work per consume is constant instead of proportional to
//! the log length.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

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
/// The route that consumes a setup-watch email confirmation token.
pub const WATCH_CONFIRM_PATH: &str = "/watch/confirm";
/// Subject of the setup-watch confirmation email.
pub const WATCH_CONFIRM_SUBJECT: &str = "Confirm your Apex setup watch";
/// The query parameter carrying the opaque link token.
pub const SIGNIN_TOKEN_PARAM: &str = "token";
/// Length of the opaque token: 32 lowercase hex characters (128 random bits),
/// the same shape as a console session id.
pub const SIGNIN_TOKEN_HEX_LEN: usize = 32;
/// The minimum interval between two sign-in links for the same publisher. A
/// second immediate request keeps the saved lead but creates no row and sends
/// no email, so an unauthenticated form cannot be used to bomb a mailbox.
pub const SIGNIN_ISSUE_COOLDOWN_MS: i64 = 60_000;
/// The environment variable naming the operator's per-publisher token file.
pub const LEDGER_TOKENS_FILE_ENV: &str = "APEX_LEDGER_TOKENS_FILE";
/// Largest sign-in log the store reads; a larger file is refused instead of
/// silently serving a truncated view of it.
pub const MAX_SIGNIN_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// Subject of the sign-in email.
pub const SIGNIN_EMAIL_SUBJECT: &str = "Your Apex sign-in link";
/// The "if this wasn't you" line of the email body.
pub const SIGNIN_EMAIL_WRONG_PERSON: &str = "If this wasn't you, you can ignore this email.";
/// The public console base URL environment variable. When unset, sign-in
/// links are disabled: no link row is created, no email is sent and
/// [`SigninService::issue_link`] returns [`ApiError::MissingBase`].
pub use crate::config::CONSOLE_URL_ENV;
/// The warning logged at startup and by [`SigninService::issue_link`] when
/// [`CONSOLE_URL_ENV`] is unset.
pub const SIGNIN_LINKS_DISABLED_WARNING: &str =
    "sign-in links are disabled: APEX_CONSOLE_URL is not configured";
/// Most recent warnings [`WarningLog`] keeps for diagnostics and tests.
const MAX_WARNING_LINES: usize = 64;

/// The `kind` value of an additive consume marker row.
const CONSUMED_KIND: &str = "consumed";

/// One appended sign-in row. The raw token never appears here: only its
/// SHA-256 hex digest. Rows written by an older version have no marker; a
/// later consume marker is folded into [`SigninRow::used_at`] when read.
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
    ///
    /// New writes leave this `null` and append a [`SigninConsumedRow`]
    /// marker; the marker is folded into this field when the log is read.
    #[serde(default)]
    pub used_at: Option<i64>,
}

/// An additive consume marker, appended by one link consumption.
///
/// The documented row shape is exactly
/// `{"kind":"consumed","tokenHash":"<sha256 hex>","usedAtMs":<epoch ms>}`.
/// Appending one marker is a constant-size durable write, so consuming `n`
/// links is O(n) instead of the old whole-log rewrite. The raw token is never
/// stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SigninConsumedRow {
    kind: String,
    token_hash: String,
    used_at_ms: i64,
}

/// One parsed log line: an original create row or an additive consume marker.
#[derive(Debug, Clone)]
enum SigninLogRow {
    Created(SigninRow),
    Consumed(SigninConsumedRow),
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

/// One file's cache identity: byte length, modification time (`None` when the
/// filesystem does not expose one) and, on Unix, the ctime and inode. A
/// same-length rewrite that puts the mtime back still changes the ctime, so
/// the cached parse is invalidated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SigninStamp {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    ctime: i64,
    #[cfg(unix)]
    ctime_nsec: i64,
    #[cfg(unix)]
    ino: u64,
}

impl SigninStamp {
    fn of(metadata: &std::fs::Metadata) -> Self {
        let len = metadata.len();
        let modified = metadata.modified().ok();
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            Self {
                len,
                modified,
                ctime: metadata.ctime(),
                ctime_nsec: metadata.ctime_nsec(),
                ino: metadata.ino(),
            }
        }
        #[cfg(not(unix))]
        {
            Self { len, modified }
        }
    }
}

/// The parsed log plus the hash index the GET and POST look through.
///
/// `by_hash` maps each token hash to the last create row carrying it, matching
/// the old `rposition` lookup exactly while turning every live/consume probe
/// into one hash-map access. The ordered `rows` vector stays the observable
/// shape [`SigninStore::read_rows`] returns: consume markers are folded into
/// the create rows instead of being exposed as rows of their own.
#[derive(Debug)]
struct SigninIndex {
    stamp: Option<SigninStamp>,
    rows: Vec<SigninRow>,
    by_hash: HashMap<String, usize>,
}

impl SigninIndex {
    fn new(stamp: Option<SigninStamp>, log: Vec<SigninLogRow>) -> Self {
        let mut rows = Vec::with_capacity(log.len());
        let mut by_hash = HashMap::with_capacity(log.len());
        let mut consumed = Vec::new();
        for entry in log {
            match entry {
                SigninLogRow::Created(row) => {
                    by_hash.insert(row.token_hash.clone(), rows.len());
                    rows.push(row);
                }
                SigninLogRow::Consumed(marker) => consumed.push(marker),
            }
        }
        for marker in consumed {
            // Fold onto the last create row with this hash. The first marker
            // wins, exactly like the old lock-protected first consume; a
            // marker with no create row (an external edit) is ignored.
            if let Some(&index) = by_hash.get(&marker.token_hash) {
                if rows[index].used_at.is_none() {
                    rows[index].used_at = Some(marker.used_at_ms);
                }
            }
        }
        Self {
            stamp,
            rows,
            by_hash,
        }
    }
}

// The test seam that proves a render does not re-parse the log, a lookup
// examines one indexed row, and a consume appends exactly one marker. The
// counters are thread-local so the scaling tests cannot observe another
// test's work.
#[cfg(test)]
thread_local! {
    static SIGNIN_PARSES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static SIGNIN_LOOKUP_PROBES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static SIGNIN_APPENDS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn note_signin_parse() {
    SIGNIN_PARSES.with(|count| count.set(count.get() + 1));
}

#[cfg(test)]
fn note_signin_lookup() {
    SIGNIN_LOOKUP_PROBES.with(|count| count.set(count.get() + 1));
}

#[cfg(test)]
fn note_signin_append() {
    SIGNIN_APPENDS.with(|count| count.set(count.get() + 1));
}

#[cfg(test)]
fn signin_parse_count() -> u64 {
    SIGNIN_PARSES.with(std::cell::Cell::get)
}

#[cfg(test)]
fn signin_lookup_count() -> u64 {
    SIGNIN_LOOKUP_PROBES.with(std::cell::Cell::get)
}

#[cfg(test)]
fn signin_append_count() -> u64 {
    SIGNIN_APPENDS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn reset_signin_counters() {
    SIGNIN_PARSES.with(|count| count.set(0));
    SIGNIN_LOOKUP_PROBES.with(|count| count.set(0));
    SIGNIN_APPENDS.with(|count| count.set(0));
}

/// The append-only `signin.jsonl` log under the leads root.
///
/// One lock guards appends and the fold that marks a row used, so a token can
/// be consumed exactly once even under concurrent requests. Consumption
/// appends one [`SigninConsumedRow`] marker; the log is never rewritten.
/// Reads never truncate: a malformed line is a typed error (fail closed) and
/// an oversized file is a typed error, never a silently short view.
///
/// The parsed rows and their hash index are cached against the file's
/// identity, so rendering the inert GET page after the first render costs a
/// stat and one hash lookup instead of a full re-parse. An external append
/// changes the identity and is folded exactly once; our own append or
/// consume marker advances the cache in place.
#[derive(Debug)]
pub struct SigninStore {
    path: PathBuf,
    lock: Mutex<()>,
    index: Mutex<Option<SigninIndex>>,
    /// The last issuance instant per publisher, for the anti-bombing
    /// cooldown. In memory only: a restart may issue one fresh link, which is
    /// harmless.
    cooldowns: Mutex<HashMap<String, i64>>,
}

impl SigninStore {
    /// The store at `<leads_root>/signin.jsonl`.
    pub fn new(leads_root: impl Into<PathBuf>) -> Self {
        Self {
            path: leads_root.into().join(SIGNIN_FILE_NAME),
            lock: Mutex::new(()),
            index: Mutex::new(None),
            cooldowns: Mutex::new(HashMap::new()),
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
        // Anti-bombing cooldown: the first request for a publisher claims the
        // window, and a repeat inside it creates no row and sends no email.
        // The saved lead is a separate contract and is unaffected.
        {
            let mut cooldowns = match self.cooldowns.lock() {
                Ok(cooldowns) => cooldowns,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(last) = cooldowns.get(publisher) {
                if now_unix_ms >= *last
                    && now_unix_ms.saturating_sub(*last) < SIGNIN_ISSUE_COOLDOWN_MS
                {
                    return Err(ApiError::BadRequest("signin_cooldown"));
                }
            }
            cooldowns.insert(publisher.to_string(), now_unix_ms);
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
        let mut cached = self.index_lock();
        self.append_row(&mut cached, &row)?;
        Ok(CreatedSignin {
            token,
            token_hash: hash,
            expires_at_unix_ms: expires_at,
        })
    }

    /// Consume `token`: on a valid, unused, unexpired row append one
    /// [`SigninConsumedRow`] marker under the same lock and return the
    /// publisher. Anything else — wrong shape, unknown, used or expired — is
    /// `None`, and the file is not touched. The row is found through the hash
    /// index (one probe), never a scan of the parsed log, and the durable
    /// write is one constant-size append.
    pub fn consume(&self, token: &str, now_unix_ms: i64) -> Result<Option<String>, ApiError> {
        if !is_signin_token(token) {
            return Ok(None);
        }
        let hash = token_hash(token);
        let _guard = self.lock();
        let mut cached = self.index_lock();
        self.refresh(&mut cached)?;
        let (publisher, index) = {
            let Some(state) = cached.as_ref() else {
                return Ok(None);
            };
            let Some(&index) = state.by_hash.get(&hash) else {
                return Ok(None);
            };
            #[cfg(test)]
            note_signin_lookup();
            let row = &state.rows[index];
            if row.used_at.is_some() || now_unix_ms >= row.expires_at {
                return Ok(None);
            }
            (row.publisher.clone(), index)
        };
        let mut line = serde_json::to_string(&SigninConsumedRow {
            kind: CONSUMED_KIND.to_string(),
            token_hash: hash,
            used_at_ms: now_unix_ms,
        })
        .map_err(|error| ApiError::Internal(format!("sign-in consume serialization: {error}")))?;
        line.push('\n');
        match self.append_line(&cached, &line)? {
            Some(stamp) => {
                if let Some(state) = cached.as_mut() {
                    state.rows[index].used_at = Some(now_unix_ms);
                    state.stamp = Some(stamp);
                }
            }
            // A concurrent external change made the cache stale; the marker
            // is durably appended, so the next lookup re-parses and folds it.
            None => *cached = None,
        }
        Ok(Some(publisher))
    }

    /// Whether `token` names a live (unused, unexpired) row, without
    /// consuming it. The GET sign-in page uses this so rendering the page
    /// cannot burn the link; only the POST consumes. The lookup is one
    /// indexed probe after the cached parse, so a mail scanner that follows
    /// the link repeatedly never re-reads the log.
    pub fn is_live(&self, token: &str, now_unix_ms: i64) -> Result<bool, ApiError> {
        if !is_signin_token(token) {
            return Ok(false);
        }
        let hash = token_hash(token);
        let _guard = self.lock();
        let mut cached = self.index_lock();
        self.refresh(&mut cached)?;
        let Some(cached) = cached.as_ref() else {
            return Ok(false);
        };
        let Some(&index) = cached.by_hash.get(&hash) else {
            return Ok(false);
        };
        #[cfg(test)]
        note_signin_lookup();
        let row = &cached.rows[index];
        Ok(row.used_at.is_none() && now_unix_ms < row.expires_at)
    }

    /// Every parsed row, in file order. Malformed lines are skipped.
    pub fn read_rows(&self) -> Result<Vec<SigninRow>, ApiError> {
        let _guard = self.lock();
        let mut cached = self.index_lock();
        self.refresh(&mut cached)?;
        Ok(cached
            .as_ref()
            .map(|cached| cached.rows.clone())
            .unwrap_or_default())
    }

    /// The cache guard. The caller always holds [`Self::lock`] first, so the
    /// lock order is fixed and a poisoned cache is recovered rather than
    /// turning a readable log into an outage.
    fn index_lock(&self) -> std::sync::MutexGuard<'_, Option<SigninIndex>> {
        match self.index.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// The log's current identity: `None` when the file is missing.
    fn inspect(&self) -> Result<Option<SigninStamp>, ApiError> {
        match std::fs::metadata(&self.path) {
            Ok(metadata) => Ok(Some(SigninStamp::of(&metadata))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ApiError::Internal(format!(
                "sign-in store {} could not be inspected: {error}",
                self.path.display()
            ))),
        }
    }

    /// Stat the log and re-parse it only when its identity moved. The caller
    /// holds the index guard, so one change is one parse.
    fn refresh(&self, cached: &mut Option<SigninIndex>) -> Result<(), ApiError> {
        let stamp = self.inspect()?;
        // A missing file is never trusted from the cache: a deletion after a
        // cached parse must read as the empty store it is now, not as the
        // rows that used to be there.
        if stamp.is_some() && cached.as_ref().is_some_and(|state| state.stamp == stamp) {
            return Ok(());
        }
        let rows = self.parse_rows()?;
        #[cfg(test)]
        note_signin_parse();
        *cached = Some(SigninIndex::new(stamp, rows));
        Ok(())
    }

    /// Read and parse the log. A malformed line fails closed; an oversized or
    /// non-UTF-8 file is a typed refusal. Both the
    /// original create rows and the additive consume markers are recognised;
    /// any other shape is malformed.
    fn parse_rows(&self) -> Result<Vec<SigninLogRow>, ApiError> {
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
        for (index, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let Some(row) = parse_log_line(line) else {
                // A one-time link journal must fail closed: skipping a
                // malformed row could resurrect a consumed token or hide a
                // forged issuance.
                return Err(ApiError::Internal(format!(
                    "sign-in store {} line {} is malformed",
                    self.path.display(),
                    index + 1
                )));
            };
            rows.push(row);
        }
        Ok(rows)
    }

    /// Append one serialized log line (mode 0600 on unix). Returns the new
    /// file identity when the cached index can advance in place. `None` means
    /// a concurrent external change (or a stat failure) makes the cache
    /// stale: the caller drops it so the next lookup folds exactly once.
    fn append_line(
        &self,
        cached: &Option<SigninIndex>,
        line: &str,
    ) -> Result<Option<SigninStamp>, ApiError> {
        let written = line.len() as u64;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| ApiError::Internal(format!("sign-in store directory: {error}")))?;
        }
        // A stat before the write marks a missing file as the empty cache; an
        // existing file that the cache has not parsed is left to fold later.
        let observed = self.inspect().ok().flatten();
        let cache_current = match cached.as_ref() {
            Some(state) => state.stamp == observed,
            None => observed.is_none(),
        };
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
        })?;
        drop(file);
        #[cfg(test)]
        note_signin_append();
        match self.inspect() {
            Ok(Some(stamp))
                if cache_current && appended_alone(observed.as_ref(), &stamp, written) =>
            {
                Ok(Some(stamp))
            }
            _ => Ok(None),
        }
    }

    /// Append one serialized create row and advance the cached index in place
    /// when it was current. A cache that had not seen the file yet (or a
    /// concurrent external append between the stat and our write) is dropped
    /// so the next lookup folds exactly once.
    fn append_row(
        &self,
        cached: &mut Option<SigninIndex>,
        row: &SigninRow,
    ) -> Result<(), ApiError> {
        let mut line = serde_json::to_string(row)
            .map_err(|error| ApiError::Internal(format!("sign-in row serialization: {error}")))?;
        line.push('\n');
        match self.append_line(cached, &line)? {
            Some(stamp) => {
                if cached.is_none() {
                    *cached = Some(SigninIndex::new(None, Vec::new()));
                }
                if let Some(state) = cached.as_mut() {
                    let index = state.rows.len();
                    state.rows.push(row.clone());
                    state.by_hash.insert(row.token_hash.clone(), index);
                    state.stamp = Some(stamp);
                }
            }
            None => *cached = None,
        }
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        match self.lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// Parse one log line as a create row or a consume marker. A line of any
/// other shape is malformed and skipped, exactly as before.
fn parse_log_line(line: &str) -> Option<SigninLogRow> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("kind").and_then(serde_json::Value::as_str) == Some(CONSUMED_KIND) {
        serde_json::from_value(value)
            .ok()
            .map(SigninLogRow::Consumed)
    } else {
        serde_json::from_value(value)
            .ok()
            .map(SigninLogRow::Created)
    }
}

/// Whether the file after our append is exactly the file before it plus the
/// bytes we wrote. A concurrent external append between the stamp and our
/// write makes the length (or the inode) disagree, so the cached index is
/// dropped and rebuilt once instead of hiding the external row.
fn appended_alone(before: Option<&SigninStamp>, after: &SigninStamp, written: u64) -> bool {
    match before {
        Some(before) => after.len == before.len.saturating_add(written) && same_file(before, after),
        None => after.len == written,
    }
}

#[cfg(unix)]
fn same_file(before: &SigninStamp, after: &SigninStamp) -> bool {
    before.ino == after.ino
}

#[cfg(not(unix))]
fn same_file(_before: &SigninStamp, _after: &SigninStamp) -> bool {
    true
}

/// Suffix of the workspace-ownership sidecar written beside the token file.
/// The token file itself keeps the operator's plain `publisher -> token`
/// shape; the sidecar records who created each workspace and the contacts a
/// sign-in link may ever be sent to.
pub const WORKSPACES_FILE_SUFFIX: &str = ".workspaces.json";

/// The recorded-contacts rule, stated verbatim wherever a workspace refuses
/// an unrecorded address.
pub const RECORDED_CONTACTS_RULE: &str =
    "Sign-in links go only to the email addresses recorded when the workspace was created.";
/// Why an existing workspace refused a different address.
pub const UNKNOWN_CONTACT_REASON: &str = "This email is not a recorded contact for this workspace.";
/// Why a new workspace was refused: its email domain could not prove
/// ownership through the public app-ads.txt file.
pub const UNPROVEN_DOMAIN_REASON: &str =
    "The email domain's app-ads.txt does not record this address as OWNERDOMAIN= or CONTACT=.";

/// The creating email and the contacts of one workspace. The first email is
/// the owner; every later accepted address must already be in `contacts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRecord {
    /// The address that created the workspace (the first recorded contact).
    pub owner_email: String,
    /// Every address a sign-in link may be delivered to, canonicalized.
    #[serde(default)]
    pub contacts: BTreeSet<String>,
    /// The email domain that proved ownership when this workspace was
    /// claimed. A record without it predates the binding (legacy) and is
    /// still accepted for its recorded contacts only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

impl WorkspaceRecord {
    /// Whether `email` is one of the recorded contacts (case-insensitive).
    pub fn has_contact(&self, email: &str) -> bool {
        let email = canonical_email(email);
        self.contacts.contains(&email)
    }
}

/// A typed workspace refusal, carrying the recorded reason.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkspaceRefusal {
    /// The publisher segment cannot be a file name.
    #[error("invalid publisher")]
    InvalidPublisher,
    /// The supplied email is not a single usable address.
    #[error("invalid email")]
    InvalidEmail,
    /// The workspace already exists and the address is not one of its
    /// recorded contacts. The workspace never grows its contact list.
    #[error("{UNKNOWN_CONTACT_REASON} {RECORDED_CONTACTS_RULE}")]
    UnknownContact {
        /// The workspace that refused.
        publisher: String,
    },
    /// An operator-provisioned workspace already holds runner data, so
    /// self-serve cannot claim it without an operator-declared domain.
    #[error("the workspace holds runner data and needs operator provisioning")]
    OperatorProvisioningRequired {
        /// The workspace that refused.
        publisher: String,
    },
    /// A new workspace needs a proof (the email domain's app-ads.txt names
    /// the domain as owner or the address as a contact).
    #[error("{UNPROVEN_DOMAIN_REASON} {reason}")]
    UnprovenDomain {
        /// The email domain that was checked.
        domain: String,
        /// The bounded, operator-facing detail.
        reason: String,
    },
    /// The workspace store could not be read or written.
    #[error("workspace store unavailable: {detail}")]
    Store {
        /// What failed.
        detail: String,
    },
}

impl WorkspaceRefusal {
    /// The stable, operator-facing reason line (never a raw email).
    pub fn reason(&self) -> &'static str {
        match self {
            Self::InvalidPublisher => "invalid publisher",
            Self::InvalidEmail => "invalid email",
            Self::UnknownContact { .. } => UNKNOWN_CONTACT_REASON,
            Self::OperatorProvisioningRequired { .. } => {
                "This workspace already holds runner data. An operator must record its domain before self-serve can claim it."
            }
            Self::UnprovenDomain { .. } => UNPROVEN_DOMAIN_REASON,
            Self::Store { .. } => "workspace store unavailable",
        }
    }
}

/// What [`WorkspaceTokens::ensure`] found or created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceEnsure {
    /// The publisher's ledger token (unchanged for an existing workspace).
    pub token: String,
    /// The recorded ownership.
    pub record: WorkspaceRecord,
    /// Whether this call created the workspace.
    pub created: bool,
}

/// Canonical contact form: trimmed and ASCII-lowercased (email domains are
/// case-insensitive; the local part is compared the same way so one address
/// has one recorded identity).
pub fn canonical_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

/// Canonical workspace-domain form: trimmed, trailing dots removed and ASCII
/// lowercased. The legacy slug key and the hashed key share it so both agree
/// about what "the same domain" means.
fn normalize_workspace_domain(domain: &str) -> String {
    domain.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// The legacy (pre-collision-hardening) workspace key a domain folds to: the
/// email domain lowercased and folded to the console's publisher-key charset
/// (`digitalturbine.com` becomes `digitalturbine-com`). The form is a slug
/// rather than the exact domain because publisher keys may not contain dots.
/// Kept for operator-provisioned and already-claimed workspaces; a genuinely
/// new workspace uses the collision-resistant [`hashed_workspace_key`].
pub fn workspace_key_for_email(email: &str) -> Option<String> {
    let (_, domain) = email.trim().split_once('@')?;
    let domain = normalize_workspace_domain(domain);
    if domain.is_empty() {
        return None;
    }
    let key = apex_watch::model::publisher_slug_for_domain(&domain);
    crate::is_valid_publisher(&key).then_some(key)
}

/// The collision-resistant workspace key for `domain`:
/// `<slug>-<first 8 hex of SHA-256(normalized domain)>`, where `slug` is the
/// legacy [`publisher_slug_for_domain`](apex_watch::model::publisher_slug_for_domain)
/// form. Two domains whose slugs collide (`a.b.com` and `a-b.com`) therefore
/// get distinct keys, so a first claimant can never read data that later lands
/// under the shared legacy slug.
pub fn hashed_workspace_key(domain: &str) -> String {
    let domain = normalize_workspace_domain(domain);
    let slug = apex_watch::model::publisher_slug_for_domain(&domain);
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("{slug}-{}", &digest[..8])
}

/// The workspace key a self-serve claim is filed under.
///
/// The legacy slug key is reused only when the deployment already recognizes
/// the slug as this workspace: a recorded ownership whose domain matches (or,
/// for a record that predates the domain binding, an address already recorded
/// as one of its contacts), or an operator-provisioned ledger token under it.
/// Every genuinely new workspace is filed under the collision-resistant
/// [`hashed_workspace_key`], so two domains whose slugs collide get distinct
/// keys unless the legacy slug is genuinely theirs. `None` is an email without
/// a usable domain.
pub fn workspace_key_for_claim(
    workspaces: Option<&WorkspaceTokens>,
    email: &str,
) -> Option<String> {
    let (_, domain) = email.trim().split_once('@')?;
    let domain = normalize_workspace_domain(domain);
    if domain.is_empty() {
        return None;
    }
    let legacy = apex_watch::model::publisher_slug_for_domain(&domain);
    if !crate::is_valid_publisher(&legacy) {
        return None;
    }
    if let Some(workspaces) = workspaces {
        if let Some(record) = workspaces.record(&legacy) {
            let belongs = match record.domain.as_deref() {
                Some(bound) => bound.eq_ignore_ascii_case(&domain),
                // A record without a domain predates the binding: its
                // recorded contacts are the only claim, exactly as before.
                None => record.has_contact(email),
            };
            if belongs {
                return Some(legacy);
            }
        } else if workspaces.token(&legacy).is_some() {
            // Operator-provisioned (a ledger token, no ownership yet): keep
            // the legacy key so the operator's runner provisioning still
            // matches; the proof and binding rules still decide the claim.
            return Some(legacy);
        }
    }
    Some(hashed_workspace_key(&domain))
}

/// Whether `email` is a single usable address (the same minimal shape the
/// lead form accepts, plus a dotted domain).
///
/// This is the *syntactic* check. It deliberately does not require the email
/// domain to be a public internet domain: an operator may provision a
/// workspace for a local mailbox (`name@buildbox.local`) and record that
/// address as a contact. A new workspace still needs the public app-ads.txt
/// proof, whose own domain normalizer refuses reserved suffixes, so accepting
/// a local domain here cannot create a workspace; it only lets an
/// already-recorded contact receive its one-time link.
pub fn is_recordable_email(email: &str) -> bool {
    let email = email.trim();
    if email.len() < 6 || email.len() > 320 || email.contains(char::is_whitespace) {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !local.contains('@')
        && domain.contains('.')
        && domain.len() <= 253
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
        && !domain.contains('@')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

/// A new-workspace ownership proof source: fetch the creating email domain's
/// public app-ads.txt and accept only an `OWNERDOMAIN=` equal to the domain
/// or a `CONTACT=` equal to the address. Failure is a typed refusal; a link
/// is never issued without it.
#[async_trait::async_trait]
pub trait WorkspaceProofSource: Send + Sync + std::fmt::Debug {
    /// Prove that `email`'s domain owns the workspace.
    async fn prove_owner(&self, email: &str) -> Result<(), WorkspaceRefusal>;
    /// The account-id histogram of `domain`'s public app-ads.txt, when a
    /// caller wants that evidence. The watch tier deliberately does not use
    /// it: it counts the distinct apps the subscriber names.
    async fn app_count(&self, domain: &str) -> Result<u32, WorkspaceRefusal>;
}

// The number of times a WorkspaceTokens instance parsed its token file and
// ownership sidecar. The scaling seam asserts an existing workspace's submit
// path serves the cache instead of re-reading either file.
#[cfg(test)]
thread_local! {
    static WORKSPACE_LOADS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn note_workspace_load() {
    WORKSPACE_LOADS.with(|count| count.set(count.get() + 1));
}

#[cfg(test)]
fn workspace_load_count() -> u64 {
    WORKSPACE_LOADS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn reset_workspace_counters() {
    WORKSPACE_LOADS.with(|count| count.set(0));
}

/// The per-publisher ledger-token file (`APEX_LEDGER_TOKENS_FILE`) behind the
/// self-serve workspace: a JSON object of publisher to token, exactly the
/// operator's existing format. [`ensure`](Self::ensure) creates or extends an
/// entry without touching any other one.
///
/// Ownership lives in the [`WORKSPACES_FILE_SUFFIX`] sidecar: an existing
/// workspace never adds a contact and never delivers to an unrecorded
/// address; a new workspace records its creating email as owner only after
/// [`WorkspaceProofSource::prove_owner`] accepted the domain.
fn workspace_file_fingerprint(path: &Path) -> (u128, u64) {
    std::fs::metadata(path)
        .ok()
        .and_then(|metadata| {
            let modified = metadata.modified().ok()?;
            let nanos = modified
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_nanos();
            Some((nanos, metadata.len()))
        })
        .unwrap_or((0, 0))
}

#[derive(Debug)]
pub struct WorkspaceTokens {
    path: PathBuf,
    records_path: PathBuf,
    inner: Mutex<WorkspaceState>,
}

#[derive(Debug, Default)]
struct WorkspaceState {
    tokens: BTreeMap<String, String>,
    records: BTreeMap<String, WorkspaceRecord>,
    /// `(modified_ns, len)` for the token file and ownership sidecar. A
    /// change by another process invalidates the cache on the next lookup.
    fingerprint: Option<(u128, u64, u128, u64)>,
}

impl WorkspaceTokens {
    /// A workspace over the token file at `path` (loaded lazily). The
    /// ownership sidecar is `<path>.workspaces.json`.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut records_name = path.clone().into_os_string();
        records_name.push(WORKSPACES_FILE_SUFFIX);
        Self {
            path,
            records_path: PathBuf::from(records_name),
            inner: Mutex::new(WorkspaceState::default()),
        }
    }

    /// The token file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The ownership sidecar path.
    pub fn records_path(&self) -> &Path {
        &self.records_path
    }

    /// Ensure `publisher` has a ledger token and a recorded owner.
    ///
    /// An existing workspace is returned unchanged, but **only** when
    /// `email` is already one of its recorded contacts: the contact list is
    /// never extended by a later submission. A new workspace is created only
    /// when `new_workspace_proven` is true (the caller has verified the
    /// creating email domain's app-ads.txt through
    /// [`WorkspaceProofSource`]); it records `email` as owner and its only
    /// contact.
    pub fn ensure(
        &self,
        publisher: &str,
        email: &str,
        new_workspace_proven: bool,
    ) -> Result<WorkspaceEnsure, WorkspaceRefusal> {
        if !crate::is_valid_publisher(publisher) {
            return Err(WorkspaceRefusal::InvalidPublisher);
        }
        if !is_recordable_email(email) {
            return Err(WorkspaceRefusal::InvalidEmail);
        }
        let email = canonical_email(email);
        let domain = email
            .split_once('@')
            .map(|(_, domain)| domain.to_string())
            .unwrap_or_default();
        let mut state = self.lock();
        self.load(&mut state)
            .map_err(|error| WorkspaceRefusal::Store {
                detail: error.to_string(),
            })?;

        let existing_token = state.tokens.get(publisher).cloned();
        let existing_record = state.records.get(publisher).cloned();
        match (existing_token, existing_record) {
            (Some(token), Some(record)) => {
                if record
                    .domain
                    .as_deref()
                    .is_some_and(|bound| !bound.eq_ignore_ascii_case(&domain))
                {
                    return Err(WorkspaceRefusal::UnknownContact {
                        publisher: publisher.to_string(),
                    });
                }
                if !record.has_contact(&email) {
                    return Err(WorkspaceRefusal::UnknownContact {
                        publisher: publisher.to_string(),
                    });
                }
                Ok(WorkspaceEnsure {
                    token,
                    record,
                    created: false,
                })
            }
            // A token without ownership is an operator-provisioned entry
            // that has never been claimed. A publisher whose domain proof
            // passes may claim it exactly once; an unproven submission is
            // refused. Existing ownership is never extended this way.
            (Some(token), None) => {
                if !new_workspace_proven {
                    let domain = email
                        .split_once('@')
                        .map(|(_, domain)| domain.to_string())
                        .unwrap_or_default();
                    return Err(WorkspaceRefusal::UnprovenDomain {
                        domain,
                        reason: "no app-ads.txt proof was recorded".to_string(),
                    });
                }
                let record = WorkspaceRecord {
                    owner_email: email.clone(),
                    contacts: BTreeSet::from([email.clone()]),
                    domain: Some(domain.clone()),
                };
                let previous = state.records.clone();
                state.records.insert(publisher.to_string(), record.clone());
                if let Err(error) = self.write_records(&state.records) {
                    state.records = previous;
                    return Err(WorkspaceRefusal::Store {
                        detail: error.to_string(),
                    });
                }
                Ok(WorkspaceEnsure {
                    token,
                    record,
                    created: false,
                })
            }
            // Ownership without a token is a partially written creation: the
            // same recorded owner may finish it, nobody else may.
            (None, Some(record)) => {
                if record
                    .domain
                    .as_deref()
                    .is_some_and(|bound| !bound.eq_ignore_ascii_case(&domain))
                {
                    return Err(WorkspaceRefusal::UnknownContact {
                        publisher: publisher.to_string(),
                    });
                }
                if !record.has_contact(&email) {
                    return Err(WorkspaceRefusal::UnknownContact {
                        publisher: publisher.to_string(),
                    });
                }
                let token = random_workspace_token();
                let previous = state.tokens.clone();
                state.tokens.insert(publisher.to_string(), token.clone());
                if let Err(error) = self.write_tokens(&state.tokens) {
                    state.tokens = previous;
                    return Err(WorkspaceRefusal::Store {
                        detail: error.to_string(),
                    });
                }
                Ok(WorkspaceEnsure {
                    token,
                    record,
                    created: false,
                })
            }
            (None, None) => {
                if !new_workspace_proven {
                    let domain = email
                        .split_once('@')
                        .map(|(_, domain)| domain.to_string())
                        .unwrap_or_default();
                    return Err(WorkspaceRefusal::UnprovenDomain {
                        domain,
                        reason: "no app-ads.txt proof was recorded".to_string(),
                    });
                }
                let token = random_workspace_token();
                let record = WorkspaceRecord {
                    owner_email: email.clone(),
                    contacts: BTreeSet::from([email.clone()]),
                    domain: Some(domain.clone()),
                };
                // Write the ownership first: a crash between the two writes
                // leaves a record whose recorded owner can finish creation,
                // never a token with no recorded contacts.
                let previous_records = state.records.clone();
                state.records.insert(publisher.to_string(), record.clone());
                if let Err(error) = self.write_records(&state.records) {
                    state.records = previous_records;
                    return Err(WorkspaceRefusal::Store {
                        detail: error.to_string(),
                    });
                }
                let previous_tokens = state.tokens.clone();
                state.tokens.insert(publisher.to_string(), token.clone());
                if let Err(error) = self.write_tokens(&state.tokens) {
                    state.tokens = previous_tokens;
                    return Err(WorkspaceRefusal::Store {
                        detail: error.to_string(),
                    });
                }
                Ok(WorkspaceEnsure {
                    token,
                    record,
                    created: true,
                })
            }
        }
    }

    /// The stored token for `publisher`, when the file holds one. An
    /// unreadable file yields `None` (fail closed).
    pub fn token(&self, publisher: &str) -> Option<String> {
        let mut state = self.lock();
        self.load(&mut state).ok()?;
        state.tokens.get(publisher).cloned()
    }

    /// The recorded ownership for `publisher`, when the sidecar holds one.
    /// An unreadable sidecar yields `None` (fail closed).
    pub fn record(&self, publisher: &str) -> Option<WorkspaceRecord> {
        let mut state = self.lock();
        self.load(&mut state).ok()?;
        state.records.get(publisher).cloned()
    }

    /// Whether `publisher` has recorded ownership (a claimed workspace).
    /// A token alone is an operator-provisioned, unclaimed entry.
    pub fn has_record(&self, publisher: &str) -> bool {
        self.record(publisher).is_some()
    }

    /// Whether `publisher` has a workspace entry or recorded ownership.
    pub fn has_workspace(&self, publisher: &str) -> bool {
        let mut state = self.lock();
        if self.load(&mut state).is_err() {
            return false;
        }
        state.tokens.contains_key(publisher) || state.records.contains_key(publisher)
    }

    /// Whether a sign-in link for `publisher` may be delivered to `email`:
    /// true only for an address recorded with the workspace.
    pub fn contact_recorded(&self, publisher: &str, email: &str) -> bool {
        self.record(publisher)
            .is_some_and(|record| record.has_contact(email))
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

    /// Read the token object and the ownership sidecar once per store
    /// instance. A missing sidecar is an empty ownership map: operator
    /// entries then keep working as tokens but can never receive a self-serve
    /// link (they have no recorded contacts).
    fn load(&self, state: &mut WorkspaceState) -> Result<(), ApiError> {
        let fingerprint = (
            workspace_file_fingerprint(&self.path),
            workspace_file_fingerprint(&self.records_path),
        );
        let fingerprint = (
            fingerprint.0 .0,
            fingerprint.0 .1,
            fingerprint.1 .0,
            fingerprint.1 .1,
        );
        if state.fingerprint == Some(fingerprint) {
            return Ok(());
        }
        let raw = match std::fs::read_to_string(&self.path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(ApiError::Internal(format!(
                    "ledger tokens read {}: {error}",
                    self.path.display()
                )))
            }
        };
        let tokens: BTreeMap<String, String> = if raw.trim().is_empty() {
            BTreeMap::new()
        } else {
            serde_json::from_str(&raw).map_err(|error| {
                ApiError::Internal(format!(
                    "ledger tokens {} are not a JSON object: {error}",
                    self.path.display()
                ))
            })?
        };
        let records_raw = match std::fs::read_to_string(&self.records_path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(ApiError::Internal(format!(
                    "workspace ownership read {}: {error}",
                    self.records_path.display()
                )))
            }
        };
        let records: BTreeMap<String, WorkspaceRecord> = if records_raw.trim().is_empty() {
            BTreeMap::new()
        } else {
            serde_json::from_str(&records_raw).map_err(|error| {
                ApiError::Internal(format!(
                    "workspace ownership {} is not a JSON object: {error}",
                    self.records_path.display()
                ))
            })?
        };
        state.tokens = tokens;
        state.records = records;
        state.fingerprint = Some(fingerprint);
        #[cfg(test)]
        note_workspace_load();
        Ok(())
    }

    /// Write the token object atomically (temp file plus rename, mode 0600).
    fn write_tokens(&self, tokens: &BTreeMap<String, String>) -> Result<(), ApiError> {
        // The next reader must re-read the file it just changed.
        // (The caller holds the state lock; this is a hint for the next
        // lookup, not a second write.)
        let body = serde_json::to_string_pretty(tokens)
            .map_err(|error| ApiError::Internal(format!("ledger tokens serialization: {error}")))?;
        self.write_atomic(&self.path, body.as_bytes(), "ledger tokens")
    }

    /// Write the ownership sidecar atomically (temp file plus rename, mode
    /// 0600).
    fn write_records(&self, records: &BTreeMap<String, WorkspaceRecord>) -> Result<(), ApiError> {
        let body = serde_json::to_string_pretty(records).map_err(|error| {
            ApiError::Internal(format!("workspace ownership serialization: {error}"))
        })?;
        self.write_atomic(&self.records_path, body.as_bytes(), "workspace ownership")
    }

    /// Replace `path` atomically with `body` (a unique sibling temp file
    /// created with `create_new` and mode 0600 on Unix, then renamed).
    ///
    /// The temp name is random for every write and `create_new` refuses to
    /// open an existing path (including a planted symlink), so a fixed
    /// `.tmp` name can never be used to redirect the write at another file.
    fn write_atomic(&self, path: &Path, body: &[u8], label: &str) -> Result<(), ApiError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| ApiError::Internal(format!("{label} directory: {error}")))?;
            }
        }
        use std::io::Write;
        for _ in 0..8 {
            let mut tmp_name = path.to_path_buf().into_os_string();
            tmp_name.push(format!(".{}.tmp", random_token()));
            let tmp = PathBuf::from(tmp_name);
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = match options.open(&tmp) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(ApiError::Internal(format!(
                        "{label} temp {}: {error}",
                        tmp.display()
                    )))
                }
            };
            let written = file
                .write_all(body)
                .and_then(|()| file.flush())
                .and_then(|()| file.sync_all());
            drop(file);
            if let Err(error) = written {
                let _ = std::fs::remove_file(&tmp);
                return Err(ApiError::Internal(format!("{label} temp write: {error}")));
            }
            return std::fs::rename(&tmp, path).map_err(|error| {
                let _ = std::fs::remove_file(&tmp);
                ApiError::Internal(format!("{label} rename {}: {error}", path.display()))
            });
        }
        Err(ApiError::Internal(format!(
            "{label} could not allocate a unique temp file"
        )))
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

/// Whether `path` is readable by every local user (Unix "other read" bit).
/// Non-Unix platforms have no such bit and answer `false`.
pub fn is_world_readable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o004 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// Warn on stderr when `path` is world-readable. The file is never chmodded:
/// an operator may not own it, and silently changing another owner's file is
/// worse than a loud warning.
pub fn warn_if_world_readable(path: &Path, label: &str) {
    if is_world_readable(path) {
        eprintln!(
            "warning: {label} {} is world-readable; restrict it to the service user (mode 0600)",
            path.display()
        );
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
    workspaces: Option<Arc<WorkspaceTokens>>,
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
            workspaces: None,
        }
    }

    /// Bind the workspace ownership records: a link is delivered only to an
    /// address recorded with the workspace. Without this binding the service
    /// creates the durable link row but never sends it.
    #[must_use]
    pub fn with_workspaces(mut self, workspaces: Arc<WorkspaceTokens>) -> Self {
        self.workspaces = Some(workspaces);
        self
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
    /// `https://console.apexmediation.ee`). Callers pass a value already
    /// validated by [`crate::config::console_url`]. Unset disables links:
    /// [`Self::issue_link`] returns [`ApiError::MissingBase`].
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

    /// Whether a link token is still live, without consuming it. The GET
    /// landing page uses this; the POST is the only consumer.
    pub fn is_live(&self, token: &str, now_unix_ms: i64) -> Result<bool, ApiError> {
        self.store.is_live(token, now_unix_ms)
    }

    /// The link a token travels in. A missing public base is the typed
    /// [`ApiError::MissingBase`] refusal: a relative emailed link cannot work.
    pub fn link_for(&self, token: &str) -> Result<String, ApiError> {
        self.link_at(SIGNIN_PATH, token)
    }

    /// The setup-watch confirmation link a token travels in.
    pub fn watch_confirm_link(&self, token: &str) -> Result<String, ApiError> {
        self.link_at(WATCH_CONFIRM_PATH, token)
    }

    /// The absolute one-click cancel link for a pending or active watch.
    pub fn watch_cancel_link(&self, token: &str) -> Result<String, ApiError> {
        self.link_at("/watch/cancel", token)
    }

    fn link_at(&self, path: &str, token: &str) -> Result<String, ApiError> {
        let Some(base) = &self.public_base else {
            return Err(ApiError::MissingBase);
        };
        Ok(format!("{base}{path}?{SIGNIN_TOKEN_PARAM}={token}"))
    }

    /// Send the setup-watch confirmation link through the transactional
    /// sink. Best effort by contract:
    /// a missing mailer or a send failure records a warning and never fails
    /// the accepted pending sign-up.
    pub async fn send_watch_confirmation(
        &self,
        email: &str,
        reference: &str,
        token: &str,
        cancel_token: Option<&str>,
        expires_at_unix_ms: i64,
    ) {
        let link = match self.watch_confirm_link(token) {
            Ok(link) => link,
            Err(_) => {
                self.warnings.record(format!(
                    "setup-watch confirmation for '{reference}' was created but not sent: {CONSOLE_URL_ENV} is not configured"
                ));
                return;
            }
        };
        let expires = apex_ui::format::utc_datetime(expires_at_unix_ms);
        let free_line = "Confirm to start the free watch. There is no payment.";
        let cancel_url = cancel_token.and_then(|token| self.watch_cancel_link(token).ok());
        let Some(mailer) = &self.mailer else {
            self.warnings.record(format!(
                "setup-watch confirmation for '{reference}' was created but not sent: set {SINK_ENV}=email (with {FROM_ENV}) to deliver it (expires {expires})"
            ));
            return;
        };
        let cancel_line = cancel_url
            .as_deref()
            .map(|url| format!("\nTo cancel this sign-up: {url}\n"))
            .unwrap_or_default();
        let text = format!(
            "Confirm your Apex setup watch:\n\n{link}\n\nThis link can be used once and expires on {expires}.\n{free_line}\n{cancel_line}\n{SIGNIN_EMAIL_WRONG_PERSON}"
        );
        let message = DeliveryMessage {
            to: email.to_string(),
            subject: WATCH_CONFIRM_SUBJECT.to_string(),
            html: watch_confirm_email_html(&link, &expires, free_line, cancel_url.as_deref()),
            text,
            template_class: TemplateClass::ReportingAccess,
            dedup_key: format!("watch_confirm:{reference}"),
            reference: None,
            list_unsubscribe: cancel_url.clone(),
        };
        match mailer.send(&message).await {
            Ok(receipt) => tracing::info!(
                reference,
                state = receipt.state.as_str(),
                "setup-watch confirmation sent"
            ),
            Err(error) => self.warnings.record(format!(
                "setup-watch confirmation for '{reference}' could not be sent: {error}"
            )),
        }
    }

    /// Create the durable single-use row for `publisher` and send the link to
    /// `email`.
    ///
    /// Delivery is restricted to the workspace's recorded contacts: when a
    /// workspace binding is present and `email` is not one of them, no row is
    /// created and the typed refusal is returned. Without a workspace binding
    /// the row may be created for a direct caller, but nothing is ever sent
    /// (a warning is recorded). An absent mailer or a delivery failure is a
    /// warning, never a failure: a saved lead stays a success even when the
    /// mail path fails.
    pub async fn issue_link(
        &self,
        publisher: &str,
        email: &str,
        now_unix_ms: i64,
    ) -> Result<CreatedSignin, ApiError> {
        if self.public_base.is_none() {
            self.warnings.record(SIGNIN_LINKS_DISABLED_WARNING);
            return Err(ApiError::MissingBase);
        }
        if let Some(workspaces) = &self.workspaces {
            if !workspaces.contact_recorded(publisher, email) {
                return Err(ApiError::BadRequest("workspace_contact_not_recorded"));
            }
        }
        let created = self.store.create(publisher, now_unix_ms, self.ttl_ms)?;
        if self.workspaces.is_none() {
            let expires = apex_ui::format::utc_datetime(created.expires_at_unix_ms);
            let reason = if self.mailer.is_some() {
                format!("no workspace contact records are configured (expires {expires})")
            } else {
                format!(
                    "set {SINK_ENV}=email (with {FROM_ENV}) to deliver sign-in links (expires {expires})"
                )
            };
            self.warnings.record(format!(
                "sign-in link for '{publisher}' was created but not sent: {reason}"
            ));
            return Ok(created);
        }
        self.deliver(publisher, email, &created).await;
        Ok(created)
    }

    /// Record one operator-facing refusal reason (also emitted as a
    /// `tracing::warn!`). The message never carries a raw email address.
    pub fn record_warning(&self, message: impl Into<String>) {
        self.warnings.record(message);
    }

    /// The recorded warning lines (tests and diagnostics).
    pub fn warnings(&self) -> Vec<String> {
        self.warnings.lines()
    }

    /// Best-effort delivery of one created link.
    async fn deliver(&self, publisher: &str, email: &str, created: &CreatedSignin) {
        let link = match self.link_for(&created.token) {
            Ok(link) => link,
            Err(_) => {
                self.warnings.record(SIGNIN_LINKS_DISABLED_WARNING);
                return;
            }
        };
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
            list_unsubscribe: None,
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

/// The minimal HTML twin of the plain-text setup-watch confirmation email.
fn watch_confirm_email_html(
    link: &str,
    expires: &str,
    free_line: &str,
    cancel_url: Option<&str>,
) -> String {
    let href = crate::html_escape(link);
    let cancel = cancel_url
        .map(|url| {
            let url = crate::html_escape(url);
            format!("<p><a href=\"{url}\">Cancel this sign-up</a></p>")
        })
        .unwrap_or_default();
    format!(
        "<p>Confirm your Apex setup watch:</p>\
         <p><a href=\"{href}\">{href}</a></p>\
         <p>This link can be used once and expires on {expires}.</p>\
         <p>{}</p>\
         <p>{}</p>{cancel}",
        crate::html_escape(free_line),
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
    fn repeated_issuance_for_one_publisher_is_cooled_down() {
        let root = temp_root("cooldown");
        let store = SigninStore::new(&root);
        let first = store
            .create("northstar-studio", 1_000, HOUR_MS)
            .expect("first link");
        let refused = store
            .create(
                "northstar-studio",
                1_000 + SIGNIN_ISSUE_COOLDOWN_MS - 1,
                HOUR_MS,
            )
            .expect_err("inside the cooldown");
        assert!(
            matches!(refused, ApiError::BadRequest("signin_cooldown")),
            "{refused}"
        );
        assert_eq!(
            store.read_rows().expect("rows").len(),
            1,
            "a cooled-down request creates no row"
        );

        // Exactly at the boundary a new link is allowed, and another
        // publisher is never affected by this publisher's cooldown.
        let second = store
            .create(
                "northstar-studio",
                1_000 + SIGNIN_ISSUE_COOLDOWN_MS,
                HOUR_MS,
            )
            .expect("after the cooldown");
        assert_ne!(first.token, second.token);
        store
            .create("bright-arcade", 1_000, HOUR_MS)
            .expect("independent publisher");
        assert_eq!(store.read_rows().expect("rows").len(), 3);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn created_secret_files_are_mode_0600() {
        use std::os::unix::fs::PermissionsExt as _;

        fn mode(path: &Path) -> u32 {
            std::fs::metadata(path).expect("stat").permissions().mode() & 0o777
        }

        let root = temp_root("permissions");
        let store = SigninStore::new(&root);
        store
            .create("northstar-studio", 1_000, HOUR_MS)
            .expect("create");
        assert_eq!(mode(store.path()), 0o600, "signin.jsonl");

        let workspaces = WorkspaceTokens::open(root.join("tokens.json"));
        workspaces
            .ensure("northstar-studio", "founder@studio.example", true)
            .expect("workspace");
        assert_eq!(mode(workspaces.path()), 0o600, "ledger token file");
        assert_eq!(
            mode(workspaces.records_path()),
            0o600,
            "workspace ownership sidecar"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_fixed_temp_symlink_is_never_followed() {
        let root = temp_root("temp-symlink");
        let path = root.join("tokens.json");
        let victim = root.join("victim.txt");
        std::fs::write(&victim, "do not overwrite").expect("victim");
        // The earlier writer opened `<token file>.tmp` with truncate; a
        // planted symlink there redirected the token body into any file the
        // service could write. The writer now allocates a random create_new
        // sibling, so this name is never opened.
        std::os::unix::fs::symlink(&victim, root.join("tokens.json.tmp")).expect("symlink");

        let workspaces = WorkspaceTokens::open(&path);
        workspaces
            .ensure("northstar-studio", "founder@studio.example", true)
            .expect("workspace");
        assert_eq!(
            std::fs::read_to_string(&victim).expect("victim"),
            "do not overwrite",
            "the symlink target was overwritten"
        );
        assert!(path.exists(), "the token file is still written");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn the_world_readable_warning_helper_detects_the_other_read_bit() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = temp_root("world-readable");
        let path = root.join("operators.json");
        std::fs::write(&path, "{}").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("mode");
        assert!(is_world_readable(&path), "0644 has the other-read bit");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("mode");
        assert!(!is_world_readable(&path), "0600 is not world-readable");
        let _ = std::fs::remove_dir_all(&root);
    }

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
    fn consume_appends_one_marker_and_folds_it_without_rewriting() {
        let root = temp_root("consume-marker");
        let store = SigninStore::new(&root);
        let created = store
            .create("northstar-studio", 1_000, 72 * HOUR_MS)
            .expect("create");
        let before = std::fs::read_to_string(store.path()).expect("log");
        assert!(!before.contains(&created.token), "raw token never on disk");
        assert!(!before.contains("\"kind\":\"consumed\""));

        assert_eq!(
            store.consume(&created.token, 2_000).expect("consume"),
            Some("northstar-studio".to_string())
        );
        let raw = std::fs::read_to_string(store.path()).expect("log");
        assert!(raw.starts_with(&before), "the create row is not rewritten");
        assert!(
            !raw.contains(&created.token),
            "the marker stores the hash only"
        );
        let marker = raw.lines().last().expect("marker row");
        assert!(marker.contains("\"kind\":\"consumed\""), "{marker}");
        assert!(
            marker.contains(&format!("\"tokenHash\":\"{}\"", created.token_hash)),
            "{marker}"
        );
        assert!(marker.contains("\"usedAtMs\":2000"), "{marker}");

        // The observable row shape is unchanged: one create row, marked used.
        let rows = store.read_rows().expect("rows");
        assert_eq!(rows.len(), 1, "a marker is not exposed as a link row");
        assert_eq!(rows[0].used_at, Some(2_000));

        // Single use: a second call appends nothing and a restart still sees
        // the folded used marker.
        assert_eq!(store.consume(&created.token, 3_000).expect("again"), None);
        assert_eq!(
            std::fs::read_to_string(store.path()).expect("log"),
            raw,
            "a refused reuse performs no write"
        );
        let restarted = SigninStore::new(&root);
        assert!(!restarted.is_live(&created.token, 3_001).expect("spent"));
        assert_eq!(
            restarted.consume(&created.token, 3_002).expect("used"),
            None
        );
        assert_eq!(
            restarted.read_rows().expect("rows")[0].used_at,
            Some(2_000),
            "the restart folds the marker"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn older_rows_without_a_marker_still_read_as_live() {
        let root = temp_root("legacy-rows");
        let store = SigninStore::new(&root);
        let token = "a".repeat(SIGNIN_TOKEN_HEX_LEN);
        let hash = token_hash(&token);
        let spent = "b".repeat(SIGNIN_TOKEN_HEX_LEN);
        // The pre-marker row shape: no `kind`, and `used_at` may be absent or
        // present. Older writers are still readable.
        std::fs::write(
            store.path(),
            format!(
                "{{\"at\":1000,\"publisher\":\"legacy\",\"token_hash\":\"{hash}\",\"expires_at\":1000000}}\n\
                 {{\"at\":1100,\"publisher\":\"spent\",\"token_hash\":\"{}\",\"expires_at\":1000000,\"used_at\":1500}}\n",
                token_hash(&spent)
            ),
        )
        .expect("seed legacy log");

        let rows = store.read_rows().expect("rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].used_at, None, "a row without a marker is live");
        assert!(store.is_live(&token, 2_000).expect("live"));
        assert_eq!(
            store.consume(&token, 2_000).expect("consume"),
            Some("legacy".to_string())
        );
        let restarted = SigninStore::new(&root);
        assert!(!restarted.is_live(&token, 2_001).expect("spent"));
        assert!(!restarted.is_live(&spent, 2_001).expect("legacy used_at"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The finding's exact scale: `n` durable links followed by `n / 2`
    /// consumes. Each consume is one fixed-size append (the marker row), the
    /// log is parsed once (the first lookup), and total durable work is
    /// linear in the number of links; the old shape rewrote and parsed the
    /// whole log per consume (quadratic).
    #[test]
    fn consume_is_constant_work_per_link_at_every_scale() {
        for links in [2_000usize, 4_000, 8_000] {
            let consumes = links / 2;
            let root = temp_root(&format!("consume-scale-{links}"));
            let writer = SigninStore::new(&root);
            let mut tokens = Vec::with_capacity(links);
            for index in 0..links {
                tokens.push(
                    writer
                        .create(
                            "northstar-studio",
                            1_000 + index as i64 * (SIGNIN_ISSUE_COOLDOWN_MS + 1),
                            72 * HOUR_MS,
                        )
                        .expect("create")
                        .token,
                );
            }
            let initial = std::fs::metadata(writer.path()).expect("stat").len();
            let reader = SigninStore::new(&root);
            reset_signin_counters();
            let started = std::time::Instant::now();
            let mut deltas = Vec::with_capacity(consumes);
            let mut previous = initial;
            for token in tokens.iter().take(consumes) {
                assert_eq!(
                    reader.consume(token, 2_000).expect("consume").as_deref(),
                    Some("northstar-studio")
                );
                let current = std::fs::metadata(reader.path()).expect("stat").len();
                deltas.push(current - previous);
                previous = current;
            }
            let elapsed = started.elapsed();
            let parses = signin_parse_count();
            let appends = signin_append_count();
            let min_delta = deltas.iter().copied().min().expect("one consume");
            let max_delta = deltas.iter().copied().max().expect("one consume");
            println!(
                "signin_consume_scale links={links} consumes={consumes} appends={appends} parses={parses} bytes_per_consume={min_delta}..{max_delta} total_appended={} elapsed_ms={}",
                previous - initial,
                elapsed.as_millis()
            );
            assert_eq!(
                appends, consumes as u64,
                "links={links}: one append per consume"
            );
            assert_eq!(
                parses, 1,
                "links={links}: one fold for the whole consume run, never one per consume"
            );
            assert_eq!(
                min_delta, max_delta,
                "links={links}: every consume appends the same fixed marker size"
            );
            assert!(
                min_delta < 512,
                "links={links}: a consume appends a marker, not the whole log"
            );
            assert_eq!(
                previous - initial,
                min_delta * consumes as u64,
                "links={links}: total durable bytes are linear in consumes"
            );
            assert_eq!(
                reader.read_rows().expect("rows").len(),
                links,
                "links={links}: markers never become link rows"
            );
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    /// A same-length external rewrite with the mtime put back still changes
    /// the ctime, so the cached parse is not served for the new bytes.
    #[cfg(unix)]
    #[test]
    fn an_external_same_length_rewrite_with_a_restored_mtime_is_detected() {
        let root = temp_root("consume-rewrite");
        let store = SigninStore::new(&root);
        let created = store
            .create("northstar-studio", 1_000, 72 * HOUR_MS)
            .expect("create");
        let path = store.path().to_path_buf();
        assert!(store.is_live(&created.token, 2_000).expect("live"));

        let original = std::fs::read_to_string(&path).expect("log");
        let metadata = std::fs::metadata(&path).expect("stat");
        let modified = metadata.modified().expect("mtime");
        let expires = created.expires_at_unix_ms;
        let expired = 259_199_000i64;
        assert_eq!(expires.to_string().len(), expired.to_string().len());
        let rewritten = original.replace(
            &format!("\"expires_at\":{expires}"),
            &format!("\"expires_at\":{expired}"),
        );
        assert_eq!(
            rewritten.len(),
            original.len(),
            "the rewrite keeps the length"
        );
        std::fs::write(&path, &rewritten).expect("rewrite");
        std::fs::File::options()
            .write(true)
            .open(&path)
            .expect("open")
            .set_modified(modified)
            .expect("restore mtime");
        assert_eq!(
            std::fs::metadata(&path).expect("stat").len(),
            metadata.len(),
            "length and mtime are restored"
        );

        reset_signin_counters();
        assert!(
            !store.is_live(&created.token, 500_000_000).expect("expired"),
            "the rewritten expiry is folded"
        );
        assert_eq!(
            signin_parse_count(),
            1,
            "the ctime change re-parses the log exactly once"
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
    fn a_malformed_one_time_journal_line_fails_closed() {
        let root = temp_root("malformed-journal");
        let store = SigninStore::new(&root);
        let created = store.create("pub", 1_000, HOUR_MS).expect("create");
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(store.path())
            .expect("open log");
        writeln!(file, "{{not a sign-in row").expect("append garbage");
        drop(file);

        let error = store.read_rows().expect_err("corrupt journal fails closed");
        assert!(matches!(error, ApiError::Internal(_)));
        assert!(error.to_string().contains("line 2"), "{error}");
        assert!(
            store.consume(&created.token, 2_000).is_err(),
            "a valid token cannot be consumed through a corrupt journal"
        );
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
            let created = store
                .create(
                    "pub",
                    1_000 + index as i64 * (SIGNIN_ISSUE_COOLDOWN_MS + 1),
                    HOUR_MS,
                )
                .expect("create");
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

        let first = workspaces
            .ensure("northstar-studio", "Founder@Studio.Example", true)
            .expect("ensure");
        assert_eq!(first.token.len(), 64, "32 random bytes as hex");
        assert!(first.token.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(first.created);
        assert_eq!(first.record.owner_email, "founder@studio.example");
        assert!(first.record.has_contact("FOUNDER@studio.example"));
        let again = workspaces
            .ensure("northstar-studio", "founder@studio.example", false)
            .expect("idempotent");
        assert_eq!(
            first.token, again.token,
            "an existing entry is never replaced"
        );
        assert!(!again.created);
        workspaces
            .ensure("bright-arcade", "b@arcade.example", true)
            .expect("second");
        assert_ne!(
            first.token,
            workspaces.token("bright-arcade").expect("stored")
        );

        let raw = std::fs::read_to_string(&path).expect("tokens file");
        let file: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&raw).expect("json object");
        assert_eq!(
            file.get("operator-pub").map(String::as_str),
            Some("operator-token")
        );
        assert_eq!(
            file.get("northstar-studio").map(String::as_str),
            Some(first.token.as_str())
        );
        assert_eq!(file.len(), 3);
        let records_raw = std::fs::read_to_string(workspaces.records_path()).expect("sidecar");
        let records: std::collections::BTreeMap<String, WorkspaceRecord> =
            serde_json::from_str(&records_raw).expect("ownership json");
        assert_eq!(records.len(), 2);
        assert_eq!(
            records
                .get("northstar-studio")
                .map(|record| record.owner_email.as_str()),
            Some("founder@studio.example")
        );

        // A fresh store over the same files sees the same entries.
        let fresh = WorkspaceTokens::open(&path);
        assert_eq!(
            fresh.token("bright-arcade"),
            workspaces.token("bright-arcade")
        );
        assert!(fresh.contact_recorded("northstar-studio", "founder@studio.example"));
        assert!(!fresh.contact_recorded("northstar-studio", "attacker@evil.example"));

        // The recorded domain binds the workspace: a colliding slug from a
        // different domain can never read or extend it.
        let colliding = WorkspaceTokens::open(&path);
        let dot = colliding
            .ensure("a-b-example", "x@a.b.example", true)
            .expect("first claim");
        assert_eq!(dot.record.domain.as_deref(), Some("a.b.example"));
        let refusal = colliding
            .ensure("a-b-example", "x@a-b.example", true)
            .expect_err("a colliding domain is not the owner");
        assert!(matches!(refusal, WorkspaceRefusal::UnknownContact { .. }));
        assert!(!colliding.contact_recorded("a-b-example", "x@a-b.example"));
        assert!(colliding.contact_recorded("a-b-example", "x@a.b.example"));

        // A legacy record without a domain still serves its recorded
        // contacts (the binding is additive, not a forced migration).
        let legacy = WorkspaceRecord {
            owner_email: "founder@legacy.example".to_string(),
            contacts: BTreeSet::from(["founder@legacy.example".to_string()]),
            domain: None,
        };
        let legacy_json = serde_json::to_string(&legacy).unwrap();
        let parsed: WorkspaceRecord = serde_json::from_str(&legacy_json).unwrap();
        assert_eq!(parsed, legacy);
        let parsed_old: WorkspaceRecord = serde_json::from_str(
            r#"{"ownerEmail":"founder@legacy.example","contacts":["founder@legacy.example"]}"#,
        )
        .unwrap();
        assert!(parsed_old.domain.is_none());

        // An existing workspace never adds a contact, even with a "proof".
        let before = std::fs::read_to_string(workspaces.records_path()).expect("sidecar");
        let refusal = workspaces
            .ensure("northstar-studio", "attacker@evil.example", true)
            .expect_err("unrecorded contact");
        assert!(matches!(refusal, WorkspaceRefusal::UnknownContact { .. }));
        assert!(refusal.to_string().contains(RECORDED_CONTACTS_RULE));
        assert_eq!(
            std::fs::read_to_string(workspaces.records_path()).expect("sidecar"),
            before,
            "a refusal writes nothing"
        );
        // An operator-provisioned token is claimable once, and only with a
        // recorded domain proof; the claim records the owner.
        assert!(matches!(
            workspaces.ensure("operator-pub", "ops@studio.example", false),
            Err(WorkspaceRefusal::UnprovenDomain { .. })
        ));
        assert!(!workspaces.contact_recorded("operator-pub", "ops@studio.example"));
        let claimed = workspaces
            .ensure("operator-pub", "ops@studio.example", true)
            .expect("a proven claim");
        assert!(!claimed.created, "the token already existed");
        assert!(claimed.record.has_contact("ops@studio.example"));
        assert!(workspaces.contact_recorded("operator-pub", "ops@studio.example"));
        // Once claimed, a different address is refused.
        assert!(matches!(
            workspaces.ensure("operator-pub", "other@studio.example", true),
            Err(WorkspaceRefusal::UnknownContact { .. })
        ));

        // Invalid names and addresses are refused before any write.
        assert!(matches!(
            workspaces.ensure("../etc", "a@studio.example", true),
            Err(WorkspaceRefusal::InvalidPublisher)
        ));
        assert!(matches!(
            workspaces.ensure("valid-pub", "not-an-email", true),
            Err(WorkspaceRefusal::InvalidEmail)
        ));
        assert!(!workspaces.has_workspace("valid-pub"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_seeded_local_mailbox_contact_claims_without_a_public_proof() {
        // Regression: a local/self-hosted mailbox (`buildbox.local`) could
        // never sign in because the recordable-email check required the
        // public-domain normalizer, which reserves `.local`. The syntactic
        // check accepts it; a new workspace still needs the public proof.
        assert!(is_recordable_email("owner@BuildBox.local"));
        assert!(is_recordable_email("owner@studio.example"));
        assert!(!is_recordable_email("owner@-bad.example"));
        assert!(!is_recordable_email("owner@bad..example"));
        assert!(!is_recordable_email("owner@localhost"));
        assert!(!is_recordable_email("not-an-email"));

        let root = temp_root("workspace-local-mailbox");
        let path = root.join("tokens.json");
        std::fs::write(&path, r#"{"buildbox-local":"local-token"}"#).expect("token file");
        let records = format!(
            "{}{}",
            path.display(),
            crate::signin::WORKSPACES_FILE_SUFFIX
        );
        std::fs::write(
            &records,
            r#"{"buildbox-local":{"ownerEmail":"owner@buildbox.local","contacts":["owner@buildbox.local"],"domain":"buildbox.local"}}"#,
        )
        .expect("ownership sidecar");
        let workspaces = WorkspaceTokens::open(&path);
        let ensured = workspaces
            .ensure("buildbox-local", "Owner@BuildBox.local", false)
            .expect("a recorded local contact claims without a proof");
        assert!(!ensured.created);
        assert_eq!(ensured.token, "local-token");
        assert!(workspaces.contact_recorded("buildbox-local", "owner@buildbox.local"));
        // A different address is still refused, even on the same local domain.
        assert!(matches!(
            workspaces.ensure("buildbox-local", "other@buildbox.local", true),
            Err(WorkspaceRefusal::UnknownContact { .. })
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_new_workspace_needs_a_proof_and_a_missing_file_is_empty() {
        let root = temp_root("workspace-invalid");
        let missing = WorkspaceTokens::open(root.join("missing.json"));
        assert!(missing.is_empty());
        assert_eq!(missing.token("pub"), None);
        assert!(missing.record("pub").is_none());
        assert!(matches!(
            missing.ensure("pub", "a@studio.example", false),
            Err(WorkspaceRefusal::UnprovenDomain { .. })
        ));
        assert!(!missing.has_workspace("pub"));
        let created = missing
            .ensure("pub", "a@studio.example", true)
            .expect("proven");
        assert!(created.created);
        assert!(missing.contact_recorded("pub", "a@studio.example"));

        let path = root.join("broken.json");
        std::fs::write(&path, "not json").expect("write");
        let broken = WorkspaceTokens::open(&path);
        assert!(broken.is_empty(), "an invalid file counts as empty");
        assert_eq!(broken.token("pub"), None);
        assert!(matches!(
            broken.ensure("pub", "a@studio.example", true),
            Err(WorkspaceRefusal::Store { .. })
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn existing_workspace_lookups_parse_the_files_once_at_every_size() {
        for workspaces in [2_000usize, 4_000, 8_000] {
            let root = temp_root(&format!("workspace-index-{workspaces}"));
            let path = root.join("tokens.json");
            let tokens: BTreeMap<String, String> = (0..workspaces)
                .map(|index| (format!("pub-{index}"), format!("{index:064x}")))
                .collect();
            std::fs::write(&path, serde_json::to_string(&tokens).expect("tokens")).expect("write");
            let store = WorkspaceTokens::open(path.clone());
            let records: BTreeMap<String, WorkspaceRecord> = (0..workspaces)
                .map(|index| {
                    let email = format!("owner{index}@x.example");
                    (
                        format!("pub-{index}"),
                        WorkspaceRecord {
                            owner_email: email.clone(),
                            contacts: BTreeSet::from([email]),
                            domain: None,
                        },
                    )
                })
                .collect();
            std::fs::write(
                store.records_path(),
                serde_json::to_string(&records).expect("records"),
            )
            .expect("write");

            reset_workspace_counters();
            let target = format!("pub-{}", workspaces / 2);
            let email = format!("owner{}@x.example", workspaces / 2);
            let lookups = 4 * workspaces;
            for _ in 0..lookups {
                assert!(store.has_workspace(&target));
                assert!(store.contact_recorded(&target, &email));
                assert!(
                    !store
                        .ensure(&target, &email, false)
                        .expect("existing workspace")
                        .created
                );
            }
            println!(
                "workspaces={workspaces} lookups={lookups} file_parses={}",
                workspace_load_count()
            );
            assert_eq!(
                workspace_load_count(),
                1,
                "workspaces={workspaces}: every existing-workspace lookup serves one parse"
            );
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[tokio::test]
    async fn issue_link_refuses_unrecorded_contacts_and_delivers_recorded_ones() {
        let root = temp_root("workspace-delivery");
        let workspaces = Arc::new(WorkspaceTokens::open(root.join("tokens.json")));
        workspaces
            .ensure("northstar-studio", "founder@studio.example", true)
            .expect("workspace");
        let service = SigninService::open(&root)
            .with_public_base("https://console.example")
            .with_workspaces(Arc::clone(&workspaces));

        let refused = service
            .issue_link("northstar-studio", "attacker@evil.example", 1_000)
            .await
            .expect_err("unrecorded contact");
        assert!(matches!(
            refused,
            ApiError::BadRequest("workspace_contact_not_recorded")
        ));
        assert!(
            service.store().read_rows().expect("rows").is_empty(),
            "a refused delivery creates no link row"
        );

        let created = service
            .issue_link("northstar-studio", "founder@studio.example", 1_000)
            .await
            .expect("recorded contact");
        assert_eq!(service.store().read_rows().expect("rows").len(), 1);
        assert!(!created.token.is_empty());
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
    fn the_workspace_key_is_the_email_domain_not_the_studio_name() {
        assert_eq!(
            workspace_key_for_email("Founder@DigitalTurbine.com").as_deref(),
            Some("digitalturbine-com")
        );
        assert_eq!(
            workspace_key_for_email("you@studio.example").as_deref(),
            Some("studio-example")
        );
        assert_eq!(
            workspace_key_for_email("a@b.example").as_deref(),
            Some("b-example")
        );
        assert_eq!(workspace_key_for_email("not-an-email"), None);
    }

    #[test]
    fn hashed_workspace_key_is_stable_and_collision_free() {
        // The collision the finding names: `a.b.com` and `a-b.com` share the
        // legacy slug `a-b-com` but must never share a key.
        let dotted = hashed_workspace_key("a.b.com");
        let dashed = hashed_workspace_key("a-b.com");
        assert_ne!(dotted, dashed, "colliding slugs must get distinct keys");
        assert_eq!(dotted, hashed_workspace_key("a.b.com"), "stable");
        assert_eq!(
            dotted,
            hashed_workspace_key("  A.B.CoM.  "),
            "normalization is stable"
        );
        // `<slug>-<first 8 hex of sha256(normalized domain)>`.
        let mut hasher = Sha256::new();
        hasher.update(b"a.b.com");
        let digest = hex::encode(hasher.finalize());
        assert_eq!(dotted, format!("a-b-com-{}", &digest[..8]));
        assert_eq!(
            dashed,
            "a-b-com-".to_string()
                + &{
                    let mut hasher = Sha256::new();
                    hasher.update(b"a-b.com");
                    hex::encode(hasher.finalize())
                }[..8]
        );
        assert!(crate::is_valid_publisher(&dotted), "{dotted}");
        assert!(crate::is_valid_publisher(&dashed), "{dashed}");
        assert!(dotted.len() <= 64, "{}", dotted.len());
    }

    #[test]
    fn a_new_claim_uses_the_hashed_key_unless_the_legacy_slug_is_genuinely_owned() {
        let root = temp_root("claim-key");
        let path = root.join("tokens.json");
        let workspaces = WorkspaceTokens::open(&path);

        // Nothing exists yet: every fresh domain gets the hashed key, so two
        // colliding domains are distinct.
        let dotted =
            workspace_key_for_claim(Some(&workspaces), "x@a.b.example").expect("dotted domain");
        let dashed =
            workspace_key_for_claim(Some(&workspaces), "x@a-b.example").expect("dashed domain");
        assert_eq!(dotted, hashed_workspace_key("a.b.example"));
        assert_eq!(dashed, hashed_workspace_key("a-b.example"));
        assert_ne!(dotted, dashed);
        assert!(workspace_key_for_claim(None, "x@a.b.example")
            .is_some_and(|key| key == hashed_workspace_key("a.b.example")));
        assert_eq!(workspace_key_for_claim(None, "not-an-email"), None);

        // An operator-provisioned token keeps the legacy slug so the runner's
        // provisioning still matches.
        std::fs::write(&path, "{\"a-b-example\": \"operator-token\"}\n").expect("seed token");
        let provisioned = WorkspaceTokens::open(&path);
        assert_eq!(
            workspace_key_for_claim(Some(&provisioned), "x@a-b.example").as_deref(),
            Some("a-b-example")
        );

        // A legacy slug claimed by a different domain is not genuinely theirs:
        // the colliding domain gets the hashed key instead.
        let claimed = WorkspaceTokens::open(&path);
        claimed
            .ensure("a-b-example", "owner@a-b.example", true)
            .expect("claim the legacy slug");
        assert_eq!(
            workspace_key_for_claim(Some(&claimed), "owner@a-b.example").as_deref(),
            Some("a-b-example"),
            "the recorded domain keeps the legacy slug"
        );
        assert_eq!(
            workspace_key_for_claim(Some(&claimed), "x@a.b.example"),
            Some(hashed_workspace_key("a.b.example")),
            "a colliding domain must not reuse the claimed slug"
        );
        assert!(!claimed.contact_recorded("a-b-example", "x@a.b.example"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn links_require_the_configured_base() {
        let root = temp_root("link");
        let service = SigninService::open(&root);
        assert!(matches!(
            service.link_for("ab12"),
            Err(ApiError::MissingBase)
        ));
        assert!(matches!(
            service.watch_confirm_link("ab12"),
            Err(ApiError::MissingBase)
        ));
        let service = service.with_public_base("https://console.apexmediation.ee/");
        assert_eq!(
            service.link_for("ab12").expect("configured base"),
            "https://console.apexmediation.ee/console/signin?token=ab12"
        );
        assert_eq!(
            service.watch_confirm_link("ab12").expect("configured base"),
            "https://console.apexmediation.ee/watch/confirm?token=ab12"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_missing_base_is_a_typed_refusal_with_no_row() {
        let root = temp_root("missing-base");
        let warnings = Arc::new(WarningLog::new());
        let service = SigninService::open(&root).with_warnings(Arc::clone(&warnings));
        let error = service
            .issue_link("northstar-studio", "you@studio.example", 1_000)
            .await
            .expect_err("no console base");
        assert!(matches!(error, ApiError::MissingBase), "{error}");
        assert!(
            service.store().read_rows().expect("rows").is_empty(),
            "a disabled link creates no row"
        );
        let lines = warnings.lines();
        assert_eq!(lines, vec![SIGNIN_LINKS_DISABLED_WARNING.to_string()]);
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

    #[test]
    fn inert_gets_parse_once_and_probe_one_indexed_row_at_every_size() {
        for rows in [250usize, 500, 1_000] {
            let root = temp_root(&format!("index-{rows}"));
            let writer = SigninStore::new(&root);
            let mut token = String::new();
            for index in 0..rows {
                let created = writer
                    .create(
                        "northstar-studio",
                        1_000 + index as i64 * (SIGNIN_ISSUE_COOLDOWN_MS + 1),
                        72 * HOUR_MS,
                    )
                    .expect("create");
                if index + 1 == rows {
                    token = created.token;
                }
            }

            // A fresh store (a restart, or another process) folds the log on
            // its first lookup; every later lookup serves that one parse.
            let store = SigninStore::new(&root);
            reset_signin_counters();
            let gets = 4 * rows;
            for _ in 0..gets {
                assert!(store.is_live(&token, 2_000).expect("live"));
            }
            println!(
                "signin_rows={rows} gets={gets} parses={} lookups={}",
                signin_parse_count(),
                signin_lookup_count()
            );
            assert_eq!(
                signin_parse_count(),
                1,
                "rows={rows}: every unchanged GET serves the cached parse"
            );
            assert_eq!(
                signin_lookup_count(),
                gets as u64,
                "rows={rows}: each GET examines exactly one indexed row"
            );

            // The POST consumes through the same index: one more probe, no
            // re-parse, and the row really is marked used on disk.
            assert_eq!(
                store.consume(&token, 3_000).expect("consume").as_deref(),
                Some("northstar-studio")
            );
            assert_eq!(signin_parse_count(), 1, "rows={rows}");
            assert_eq!(signin_lookup_count(), gets as u64 + 1, "rows={rows}");
            assert!(!store.is_live(&token, 3_001).expect("spent"));
            assert_eq!(signin_parse_count(), 1, "rows={rows}");

            // An append from another handle changes the identity: exactly one
            // re-parse, then the cached path resumes.
            let other = SigninStore::new(&root);
            let extra = other
                .create("northstar-studio", 4_000, 72 * HOUR_MS)
                .expect("external create");
            assert!(store.is_live(&extra.token, 5_000).expect("live"));
            assert_eq!(
                signin_parse_count(),
                2,
                "rows={rows}: one external change is one fold"
            );

            // A deletion is the empty store, never the stale cached rows.
            std::fs::remove_file(store.path()).expect("remove log");
            assert!(!store.is_live(&extra.token, 6_000).expect("deleted"));
            assert_eq!(
                signin_parse_count(),
                3,
                "rows={rows}: the missing file is not served from the cache"
            );
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    /// Every create advances the cached index in place, so a deployment that
    /// interleaves link issuance with renders does not re-parse the growing
    /// log after each append (the old append-then-invalidate shape). The
    /// earlier code reparsed once per render in this loop.
    #[test]
    fn interleaved_creates_and_renders_update_the_index_without_a_reparse() {
        const LINKS: usize = 1_000;
        let root = temp_root("index-interleaved");
        let store = SigninStore::new(&root);
        reset_signin_counters();
        for index in 0..LINKS {
            let created = store
                .create(
                    "northstar-studio",
                    1_000 + index as i64 * (SIGNIN_ISSUE_COOLDOWN_MS + 1),
                    72 * HOUR_MS,
                )
                .expect("create");
            assert!(store.is_live(&created.token, 2_000).expect("live"));
        }
        let parses = signin_parse_count();
        let lookups = signin_lookup_count();
        println!(
            "signin_interleaved links={LINKS} renders={LINKS} parses={parses} lookups={lookups}"
        );
        assert_eq!(parses, 0, "each append advances the index in place");
        assert_eq!(lookups, LINKS as u64, "one indexed row per render");
        assert_eq!(store.read_rows().expect("rows").len(), LINKS);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The finding's exact shape: 10k durable links, then 1k GET/POST checks
    /// through a fresh store. Earlier every check read and parsed the whole
    /// log; the cached index serves all 1k checks from one parse. An external
    /// append between requests is still discovered with one further parse.
    #[test]
    fn ten_thousand_links_then_a_thousand_renders_parse_the_log_once() {
        const LINKS: usize = 10_000;
        const CHECKS: usize = 1_000;
        let root = temp_root("index-10k");
        let writer = SigninStore::new(&root);
        let mut tokens = Vec::with_capacity(LINKS);
        for index in 0..LINKS {
            let created = writer
                .create(
                    "northstar-studio",
                    1_000 + index as i64 * (SIGNIN_ISSUE_COOLDOWN_MS + 1),
                    72 * HOUR_MS,
                )
                .expect("create");
            tokens.push(created.token);
        }
        // A fresh store (a restart) starts with no index: its first check
        // folds the whole log exactly once.
        let reader = SigninStore::new(&root);
        reset_signin_counters();
        for (index, token) in tokens.iter().enumerate().take(CHECKS) {
            if index % 2 == 0 {
                assert!(
                    reader.is_live(token, 2_000).expect("live"),
                    "GET {index} must not consume"
                );
            } else {
                let now = 2_000 + index as i64;
                assert_eq!(
                    reader.consume(token, now).expect("consume").as_deref(),
                    Some("northstar-studio")
                );
            }
        }
        let parses = signin_parse_count();
        let lookups = signin_lookup_count();
        let appends = signin_append_count();
        println!(
            "signin_scale links={LINKS} checks={CHECKS} gets={} posts={} appends={appends} parses={parses} lookups={lookups}",
            CHECKS / 2,
            CHECKS / 2
        );
        assert_eq!(parses, 1, "1k GET/POST checks share one parse");
        assert_eq!(
            lookups, CHECKS as u64,
            "one indexed lookup per GET/POST check"
        );
        assert_eq!(
            appends,
            (CHECKS / 2) as u64,
            "each successful POST appends exactly one marker"
        );

        // A replay of a spent token is a single-use refusal: one lookup, no
        // append and no re-parse.
        assert_eq!(reader.consume(&tokens[1], 9_000).expect("used"), None);
        assert_eq!(signin_lookup_count(), CHECKS as u64 + 1);
        assert_eq!(signin_append_count(), (CHECKS / 2) as u64);
        assert_eq!(signin_parse_count(), 1);
        assert_eq!(reader.read_rows().expect("rows").len(), LINKS);

        // An append from another handle between requests changes the stamp:
        // exactly one re-parse, then the cached index resumes.
        let extra = writer
            .create("northstar-studio", 5_000, 72 * HOUR_MS)
            .expect("external append");
        assert!(reader.is_live(&extra.token, 6_000).expect("live"));
        assert_eq!(signin_parse_count(), 2, "one external change is one fold");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn a_created_link_is_logged_as_a_warning_without_a_mailer() {
        let root = temp_root("warn");
        let warnings = Arc::new(WarningLog::new());
        let service = SigninService::open(&root)
            .with_public_base("https://console.example")
            .with_warnings(Arc::clone(&warnings));
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
