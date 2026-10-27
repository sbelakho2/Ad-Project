//! The append-only subscriber store.
//!
//! One JSONL file ([`SubscriberStore::subscribers_path`], `subscribers.jsonl`
//! under the watch root) holds every version of every subscriber row. The
//! newest row per `id` wins, so `subscribe_pending`, `confirm_email`,
//! `activate`, `cancel` and the daily or monthly field updates all append a
//! full row instead of editing in place.
//! A malformed line is a typed refusal: a store that cannot prove its rows
//! must not fabricate a subscriber.
//!
//! The folded set is cached and keyed by the log file's identity (length,
//! modification time, and on Unix ctime and inode), so bulk onboarding folds
//! the log once instead of once per signup. A successful append advances the
//! cached fold in place; an append from another process (a changed stamp)
//! refreshes it exactly once. The cached fold also carries hash indexes
//! (subscriber id, dedup identity, cancel-token hash, confirmation-token
//! hash) into the ordered rows, so deduplication, cancellation, confirmation
//! and in-place updates do not scan the growing vector.
//!
//! `subscribe_pending` deduplicates repeat submissions with the same
//! `(domain, email)` inside [`DEDUP_WINDOW_MS`], exactly like the free-audit
//! lead store: the duplicate keeps the reference and gets a freshly minted
//! confirmation token. The token is 32 lowercase hex characters (the same
//! shape as the console sign-in token), valid for
//! [`CONFIRMATION_TTL_MS`](crate::model::CONFIRMATION_TTL_MS), single-use, and only its
//! SHA-256 hash is stored. The legacy `subscribe` keeps the older immediate
//! activation used by tools and older tests and mints a cancel token.

use std::collections::HashMap;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::fetch::HttpValidators;
use crate::findings::StoredFinding;
use crate::model::{
    distinct_apps, publisher_slug_for_domain, tier_from_apps, validate_watch_signup,
    ActivateOutcome, BillingProfile, ConfirmOutcome, PendingOutcome, Status, SubscribeOutcome,
    SubscribeRequest, Subscriber, WatchFieldError, WatchSignup, CONFIRMATION_TOKEN_HEX_LEN,
    CONFIRMATION_TTL_MS,
};

/// Environment variable naming the watch root directory.
pub const WATCH_ROOT_ENV: &str = "APEX_WATCH_ROOT";
/// Default watch root, resolved against the working directory.
pub const DEFAULT_WATCH_ROOT: &str = ".apex/watch";
/// The subscriber file name under the root.
pub const SUBSCRIBERS_FILE: &str = "subscribers.jsonl";
/// The folded snapshot of the subscriber log, rewritten atomically by
/// [`SubscriberStore::checkpoint`].
pub const CHECKPOINT_FILE: &str = "subscribers.checkpoint.jsonl";
/// The small append log covering rows written after a checkpoint.
pub const DELTA_FILE: &str = "subscribers.delta.jsonl";
/// The persisted per-domain conditional-GET state (validators + findings).
pub const DOMAIN_CACHE_FILE: &str = "domain-cache.json";
/// The repeat-submission window: a submission with the same domain and email
/// inside this window is the same subscription.
pub const DEDUP_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
/// Format version of the checkpoint file.
const CHECKPOINT_VERSION: u32 = 1;
/// Format version of the delta file.
const DELTA_VERSION: u32 = 1;

/// Why a store operation failed.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum WatchError {
    /// The field-level sign-up validation refused the submission.
    #[error("the submission has {} field error(s)", .0.len())]
    Validation(Vec<WatchFieldError>),
    /// The subscriber file could not be read or is not subscriber rows.
    #[error("watch store is unusable: {detail}")]
    StoreCorrupt {
        /// What failed.
        detail: String,
    },
    /// A write failed.
    #[error("watch store write failed: {detail}")]
    Io {
        /// What failed.
        detail: String,
    },
    /// A malformed input (an invalid month, a corrupt JSON shape).
    #[error("malformed input: {0}")]
    Malformed(String),
    /// A billing operation failed.
    #[error("billing: {0}")]
    Billing(#[from] apex_billing::BillingError),
    /// No subscriber row carries the presented reference/id.
    #[error("no subscriber matches reference '{reference}'")]
    UnknownSubscriber {
        /// The reference that matched nothing.
        reference: String,
    },
    /// Activation was asked of a row that is not pending (canceled).
    #[error("subscriber '{reference}' is {status}, not pending")]
    NotPending {
        /// The row's id.
        reference: String,
        /// The row's current status.
        status: Status,
    },
    /// The confirmation token is unknown.
    #[error("the confirmation token is not recognised")]
    ConfirmationUnknown,
    /// The confirmation token has expired.
    #[error("the confirmation token has expired")]
    ConfirmationExpired,
    /// The confirmation token was already used (single use).
    #[error("the confirmation token was already used")]
    ConfirmationUsed,
    /// The email was already confirmed by an earlier token.
    #[error("the email address is already confirmed")]
    AlreadyConfirmed,
    /// The billing-profile token is unknown.
    #[error("the billing-profile token is not recognised")]
    ProfileTokenUnknown,
    /// The billing-profile token has expired.
    #[error("the billing-profile token has expired")]
    ProfileTokenExpired,
    /// The billing-profile token was already used (single use).
    #[error("the billing-profile token was already used")]
    ProfileTokenUsed,
    /// The subscriber already has a different billing profile. A recorded
    /// profile is never silently replaced.
    #[error("subscriber '{reference}' already has a different customer billing profile")]
    BillingProfileLocked {
        /// The row's id.
        reference: String,
    },
    /// The subscriber has no customer billing profile to invoice with.
    #[error("subscriber '{reference}' has no customer billing profile recorded")]
    BillingProfileMissing {
        /// The row's id.
        reference: String,
    },
    /// The row is canceled, so it cannot receive a profile or an invoice.
    #[error("subscriber '{reference}' is {status}; the action needs a pending or active row")]
    NotBillable {
        /// The row's id.
        reference: String,
        /// The row's current status.
        status: Status,
    },
}

/// The append-only subscriber store.
///
/// The folded subscriber set is cached in memory and keyed by the log file's
/// identity: every read and every write stats the file first, and the fold
/// is rebuilt only when the identity moved (a first read, or an append from
/// another process). A successful append updates the cached fold in place,
/// so onboarding `n` subscribers folds the log once instead of `n` times.
#[derive(Debug)]
pub struct SubscriberStore {
    root: PathBuf,
    lock: Mutex<()>,
    folded: Mutex<Option<Folded>>,
    folds: AtomicU64,
    row_probes: AtomicU64,
    rows_read: AtomicU64,
    /// The checkpoint generation this process believes the delta follows.
    /// Loaded lazily from the checkpoint header; `None` means no checkpoint
    /// exists, so appends write only the main log.
    delta_generation: Mutex<Option<String>>,
    /// Whether the delta was already found to be unusable (a format or
    /// generation mismatch). When set, appends stop growing it until the next
    /// checkpoint writes a fresh one.
    delta_broken: AtomicBool,
}

/// One file's cache identity: byte length, modification time (`None` when
/// the filesystem does not expose one) and, on Unix, the ctime and inode,
/// which an unprivileged writer cannot restore. A same-length rewrite that
/// puts the mtime back (`touch -t`) still changes the ctime, so it
/// invalidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    ctime: i64,
    #[cfg(unix)]
    ctime_nsec: i64,
    #[cfg(unix)]
    ino: u64,
}

impl FileStamp {
    #[cfg(unix)]
    fn covered_ino(&self) -> u64 {
        self.ino
    }

    #[cfg(not(unix))]
    fn covered_ino(&self) -> u64 {
        0
    }

    #[cfg(unix)]
    fn covered_ctime(&self) -> i64 {
        self.ctime
    }

    #[cfg(not(unix))]
    fn covered_ctime(&self) -> i64 {
        0
    }

    #[cfg(unix)]
    fn covered_ctime_nsec(&self) -> i64 {
        self.ctime_nsec
    }

    #[cfg(not(unix))]
    fn covered_ctime_nsec(&self) -> i64 {
        0
    }

    /// The stamp of `metadata`.
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

/// The dedup identity of a subscriber row: the exact domain plus the
/// case-insensitive email, mirroring the `(domain, email)` predicate the
/// repeat-submission scan used.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DedupKey {
    domain: String,
    email_lower: String,
}

impl DedupKey {
    fn of(subscriber: &Subscriber) -> Self {
        Self {
            domain: subscriber.domain.clone(),
            email_lower: subscriber.email.to_ascii_lowercase(),
        }
    }

    fn of_signup(domain: &str, email: &str) -> Self {
        Self {
            domain: domain.to_string(),
            email_lower: email.to_ascii_lowercase(),
        }
    }
}

/// The persisted conditional-GET state of one watched domain: the validators
/// the last 200 advertised and the findings that response produced. The
/// findings are what a 304 reuses; an entry without both is not used for a
/// conditional request (the next run fetches the file in full).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainCacheEntry {
    /// The validators of the last 200 response.
    #[serde(flatten)]
    pub validators: HttpValidators,
    /// The findings that response produced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub findings: Option<Vec<StoredFinding>>,
}

impl DomainCacheEntry {
    /// The validators to send only when the cached findings can also be
    /// reused for a 304.
    #[must_use]
    pub fn conditional_validators(&self) -> Option<&HttpValidators> {
        (self.findings.is_some() && !self.validators.is_empty()).then_some(&self.validators)
    }
}

/// The checkpoint header: the exact main-log prefix (byte length and, on
/// Unix, inode/ctime) the folded rows cover. A prefix that moved or shrank
/// invalidates the checkpoint and the caller falls back to the full log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckpointHeader {
    version: u32,
    generation: String,
    covered_bytes: u64,
    #[serde(default)]
    covered_ino: u64,
    #[serde(default)]
    covered_ctime: i64,
    #[serde(default)]
    covered_ctime_nsec: i64,
}

/// The delta header: which checkpoint generation the appended rows follow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeltaHeader {
    version: u32,
    generation: String,
}

/// The cached fold: the stamp it was built from (`None` is a missing file),
/// the newest row per id in first-seen order, and hash indexes into that
/// order so a signup, cancel or in-place update never scans the growing set.
///
/// Each index maps a key to every row position carrying it, in ascending
/// order. The repeat-submission and cancel lookups walk only the positions
/// for their key (normally one), which keeps the old first-match semantics
/// exactly even when an external writer appended several rows with the same
/// key. The ordered `subscribers` vector remains the only observable order.
#[derive(Debug)]
struct Folded {
    stamp: Option<FileStamp>,
    subscribers: Vec<Subscriber>,
    by_id: HashMap<String, usize>,
    by_dedup: HashMap<DedupKey, Vec<usize>>,
    by_cancel_token: HashMap<String, Vec<usize>>,
    by_confirmation_token: HashMap<String, Vec<usize>>,
    by_profile_token: HashMap<String, Vec<usize>>,
}

impl Folded {
    /// Build the indexes for a freshly folded file.
    fn new(stamp: Option<FileStamp>, subscribers: Vec<Subscriber>) -> Self {
        let mut by_id = HashMap::with_capacity(subscribers.len());
        let mut by_dedup: HashMap<DedupKey, Vec<usize>> = HashMap::new();
        let mut by_cancel_token: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_confirmation_token: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_profile_token: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, subscriber) in subscribers.iter().enumerate() {
            by_id.insert(subscriber.id.clone(), index);
            insert_sorted(by_dedup.entry(DedupKey::of(subscriber)).or_default(), index);
            insert_sorted(
                by_cancel_token
                    .entry(subscriber.cancel_token_hash.clone())
                    .or_default(),
                index,
            );
            if let Some(hash) = &subscriber.confirmation_token_hash {
                insert_sorted(
                    by_confirmation_token.entry(hash.clone()).or_default(),
                    index,
                );
            }
            if let Some(hash) = &subscriber.profile_token_hash {
                insert_sorted(by_profile_token.entry(hash.clone()).or_default(), index);
            }
        }
        Self {
            stamp,
            subscribers,
            by_id,
            by_dedup,
            by_cancel_token,
            by_confirmation_token,
            by_profile_token,
        }
    }

    /// The first row carrying the confirmation `hash`, and how many rows the
    /// indexed lookup examined.
    fn find_confirmation(&self, hash: &str) -> (Option<usize>, u64) {
        let Some(indexes) = self.by_confirmation_token.get(hash) else {
            return (None, 0);
        };
        let mut probes = 0;
        for &index in indexes {
            probes += 1;
            let subscriber = &self.subscribers[index];
            if subscriber.confirmation_token_hash.as_deref() == Some(hash) {
                return (Some(index), probes);
            }
        }
        (None, probes)
    }

    /// The first row carrying the billing-profile `hash`, and how many rows
    /// the indexed lookup examined.
    fn find_profile_token(&self, hash: &str) -> (Option<usize>, u64) {
        let Some(indexes) = self.by_profile_token.get(hash) else {
            return (None, 0);
        };
        let mut probes = 0;
        for &index in indexes {
            probes += 1;
            let subscriber = &self.subscribers[index];
            if subscriber.profile_token_hash.as_deref() == Some(hash) {
                return (Some(index), probes);
            }
        }
        (None, probes)
    }

    /// The row with `id`, and how many rows the indexed lookup examined.
    fn find_id(&self, id: &str) -> (Option<usize>, u64) {
        match self.by_id.get(id) {
            Some(&index) => (Some(index), 1),
            None => (None, 0),
        }
    }
    /// The first row for `key` inside the repeat-submission window, and how
    /// many rows the indexed lookup examined.
    fn find_dedup(&self, key: &DedupKey, now_ms: i64) -> (Option<usize>, u64) {
        let Some(indexes) = self.by_dedup.get(key) else {
            return (None, 0);
        };
        let mut probes = 0;
        for &index in indexes {
            probes += 1;
            let subscriber = &self.subscribers[index];
            if subscriber.domain == key.domain
                && subscriber.email.eq_ignore_ascii_case(&key.email_lower)
                && now_ms.saturating_sub(subscriber.created_ms) <= DEDUP_WINDOW_MS
            {
                return (Some(index), probes);
            }
        }
        (None, probes)
    }

    /// The first active row carrying `hash`, and how many rows the indexed
    /// lookup examined.
    fn find_cancel(&self, hash: &str) -> (Option<usize>, u64) {
        let Some(indexes) = self.by_cancel_token.get(hash) else {
            return (None, 0);
        };
        let mut probes = 0;
        for &index in indexes {
            probes += 1;
            let subscriber = &self.subscribers[index];
            // A pending sign-up is cancellable too: its owner must be able to
            // stop the watch before confirming or paying.
            if matches!(subscriber.status, Status::Active | Status::Pending)
                && constant_time_eq(hash, &subscriber.cancel_token_hash)
            {
                return (Some(index), probes);
            }
        }
        (None, probes)
    }

    /// Apply one appended row with the same newest-row-wins fold the file
    /// read uses: an existing id keeps its first-seen position, a new id is
    /// appended at the end. Returns how many rows the id lookup examined.
    fn apply(&mut self, subscriber: &Subscriber) -> u64 {
        if let Some(&index) = self.by_id.get(&subscriber.id) {
            let replaced = std::mem::replace(&mut self.subscribers[index], subscriber.clone());
            self.reindex(index, &replaced, subscriber);
            1
        } else {
            let index = self.subscribers.len();
            self.subscribers.push(subscriber.clone());
            self.by_id.insert(subscriber.id.clone(), index);
            insert_sorted(
                self.by_dedup.entry(DedupKey::of(subscriber)).or_default(),
                index,
            );
            insert_sorted(
                self.by_cancel_token
                    .entry(subscriber.cancel_token_hash.clone())
                    .or_default(),
                index,
            );
            if let Some(hash) = &subscriber.confirmation_token_hash {
                insert_sorted(
                    self.by_confirmation_token.entry(hash.clone()).or_default(),
                    index,
                );
            }
            if let Some(hash) = &subscriber.profile_token_hash {
                insert_sorted(
                    self.by_profile_token.entry(hash.clone()).or_default(),
                    index,
                );
            }
            0
        }
    }

    /// Move one first-seen position from the replaced row's index keys to the
    /// replacement's, leaving an unchanged key alone.
    fn reindex(&mut self, index: usize, replaced: &Subscriber, subscriber: &Subscriber) {
        let old_key = DedupKey::of(replaced);
        let new_key = DedupKey::of(subscriber);
        if old_key != new_key {
            remove_sorted(&mut self.by_dedup, &old_key, index);
            insert_sorted(self.by_dedup.entry(new_key).or_default(), index);
        }
        if replaced.cancel_token_hash != subscriber.cancel_token_hash {
            remove_sorted(
                &mut self.by_cancel_token,
                &replaced.cancel_token_hash,
                index,
            );
            insert_sorted(
                self.by_cancel_token
                    .entry(subscriber.cancel_token_hash.clone())
                    .or_default(),
                index,
            );
        }
        if replaced.confirmation_token_hash != subscriber.confirmation_token_hash {
            if let Some(old_hash) = &replaced.confirmation_token_hash {
                remove_sorted(&mut self.by_confirmation_token, old_hash, index);
            }
            if let Some(new_hash) = &subscriber.confirmation_token_hash {
                insert_sorted(
                    self.by_confirmation_token
                        .entry(new_hash.clone())
                        .or_default(),
                    index,
                );
            }
        }
        if replaced.profile_token_hash != subscriber.profile_token_hash {
            if let Some(old_hash) = &replaced.profile_token_hash {
                remove_sorted(&mut self.by_profile_token, old_hash, index);
            }
            if let Some(new_hash) = &subscriber.profile_token_hash {
                insert_sorted(
                    self.by_profile_token.entry(new_hash.clone()).or_default(),
                    index,
                );
            }
        }
    }
}

/// Insert `index` into an ascending position list, ignoring duplicates.
fn insert_sorted(indexes: &mut Vec<usize>, index: usize) {
    if let Err(position) = indexes.binary_search(&index) {
        indexes.insert(position, index);
    }
}

/// Remove `index` from an ascending position list, dropping an empty list.
fn remove_sorted<K: Eq + std::hash::Hash>(map: &mut HashMap<K, Vec<usize>>, key: &K, index: usize) {
    let Some(indexes) = map.get_mut(key) else {
        return;
    };
    if let Ok(position) = indexes.binary_search(&index) {
        indexes.remove(position);
    }
    if indexes.is_empty() {
        map.remove(key);
    }
}

impl SubscriberStore {
    /// A store rooted at `root`; nothing is created until the first write.
    pub fn open(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            lock: Mutex::new(()),
            folded: Mutex::new(None),
            folds: AtomicU64::new(0),
            row_probes: AtomicU64::new(0),
            rows_read: AtomicU64::new(0),
            delta_generation: Mutex::new(None),
            delta_broken: AtomicBool::new(false),
        }
    }

    /// The checkpoint file path.
    pub fn checkpoint_path(&self) -> PathBuf {
        self.root.join(CHECKPOINT_FILE)
    }

    /// The delta file path.
    pub fn delta_path(&self) -> PathBuf {
        self.root.join(DELTA_FILE)
    }

    /// The per-domain validator cache path.
    pub fn domain_cache_path(&self) -> PathBuf {
        self.root.join(DOMAIN_CACHE_FILE)
    }

    /// How many subscriber rows the folds have parsed (the checkpoint seam).
    #[must_use]
    pub fn rows_read(&self) -> u64 {
        self.rows_read.load(Ordering::Relaxed)
    }

    /// Load the persisted per-domain validators. A missing or unreadable file
    /// is an empty cache, which makes every domain fetch unconditionally.
    #[must_use]
    pub fn load_domain_cache(&self) -> HashMap<String, DomainCacheEntry> {
        let raw = match std::fs::read_to_string(self.domain_cache_path()) {
            Ok(raw) => raw,
            Err(_) => return HashMap::new(),
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    /// Persist the per-domain validators atomically (temp+rename, 0600).
    pub fn save_domain_cache(
        &self,
        cache: &HashMap<String, DomainCacheEntry>,
    ) -> Result<(), WatchError> {
        let bytes = serde_json::to_vec(cache).map_err(|error| WatchError::Io {
            detail: format!("domain validator cache serialization: {error}"),
        })?;
        atomic_write(&self.domain_cache_path(), &bytes)
    }

    /// Write the folded snapshot of the log atomically and reset the delta to
    /// a fresh generation. A crash between the two writes leaves the old (or
    /// truncated) delta, which the reader detects and recovers from by
    /// folding the full log.
    pub fn checkpoint(&self) -> Result<(), WatchError> {
        let _guard = self.lock()?;
        let mut cached = self.fold_lock()?;
        self.refresh(&mut cached)?;
        let subscribers = cached
            .as_ref()
            .map(|state| state.subscribers.clone())
            .unwrap_or_default();
        restrict_mode(&self.subscribers_path())?;
        let stamp = Self::inspect(&self.subscribers_path())?;
        let (covered_bytes, covered_ino, covered_ctime, covered_ctime_nsec) = match stamp {
            Some(stamp) => (
                stamp.len,
                stamp.covered_ino(),
                stamp.covered_ctime(),
                stamp.covered_ctime_nsec(),
            ),
            None => (0, 0, 0, 0),
        };
        let generation = random_hex(8);
        let header = CheckpointHeader {
            version: CHECKPOINT_VERSION,
            generation: generation.clone(),
            covered_bytes,
            covered_ino,
            covered_ctime,
            covered_ctime_nsec,
        };
        let mut body = serde_json::to_vec(&header).map_err(|error| WatchError::Io {
            detail: format!("checkpoint header serialization: {error}"),
        })?;
        body.push(b'\n');
        for subscriber in &subscribers {
            let line = serde_json::to_vec(subscriber).map_err(|error| WatchError::Io {
                detail: format!("checkpoint row serialization: {error}"),
            })?;
            body.extend_from_slice(&line);
            body.push(b'\n');
        }
        atomic_write(&self.checkpoint_path(), &body)?;
        // Only after the checkpoint is durable: a crash before this leaves a
        // generation the old delta does not match, which is recoverable.
        let delta_header = DeltaHeader {
            version: DELTA_VERSION,
            generation: generation.clone(),
        };
        let mut delta = serde_json::to_vec(&delta_header).map_err(|error| WatchError::Io {
            detail: format!("delta header serialization: {error}"),
        })?;
        delta.push(b'\n');
        atomic_write(&self.delta_path(), &delta)?;
        if let Ok(mut guard) = self.delta_generation.lock() {
            *guard = Some(generation);
        }
        self.delta_broken.store(false, Ordering::Relaxed);
        Ok(())
    }

    /// Build from the environment: [`WATCH_ROOT_ENV`] when set, else
    /// [`DEFAULT_WATCH_ROOT`].
    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let root = lookup(WATCH_ROOT_ENV)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_WATCH_ROOT.to_string());
        Self::open(root)
    }

    /// The root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The subscriber file path.
    pub fn subscribers_path(&self) -> PathBuf {
        self.root.join(SUBSCRIBERS_FILE)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, WatchError> {
        self.lock.lock().map_err(|_| WatchError::StoreCorrupt {
            detail: "the subscriber lock is poisoned".to_string(),
        })
    }

    fn fold_lock(&self) -> Result<std::sync::MutexGuard<'_, Option<Folded>>, WatchError> {
        self.folded.lock().map_err(|_| WatchError::StoreCorrupt {
            detail: "the subscriber fold cache is poisoned".to_string(),
        })
    }

    /// The log's current identity: `None` when the file is missing. Any
    /// other stat failure is a typed refusal, never "an empty store".
    fn inspect(path: &Path) -> Result<Option<FileStamp>, WatchError> {
        match std::fs::metadata(path) {
            Ok(metadata) => Ok(Some(FileStamp::of(&metadata))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(WatchError::StoreCorrupt {
                detail: format!("{} could not be inspected: {error}", path.display()),
            }),
        }
    }

    /// Stat the log before use and rebuild the cached fold once when its
    /// identity moved (a first read, or an append from another process).
    /// The caller holds the fold cache guard, so one change is one fold.
    fn refresh(&self, cached: &mut Option<Folded>) -> Result<(), WatchError> {
        let path = self.subscribers_path();
        let stamp = Self::inspect(&path)?;
        if cached.as_ref().is_some_and(|state| state.stamp == stamp) {
            return Ok(());
        }
        // The checkpoint + delta is a pure read optimization: any stamp or
        // format problem falls back to folding the full append-only log.
        let subscribers = match self.read_checkpoint_and_delta(&path)? {
            Some(subscribers) => subscribers,
            None => self.read_fold(&path)?,
        };
        self.folds.fetch_add(1, Ordering::Relaxed);
        *cached = Some(Folded::new(stamp, subscribers));
        Ok(())
    }

    /// Read the checkpoint and the rows appended after it, validating that
    /// the delta is byte-for-byte the main log's uncovered tail. `None`
    /// means "fall back to the full log" (missing/corrupt/stale files).
    fn read_checkpoint_and_delta(
        &self,
        path: &Path,
    ) -> Result<Option<Vec<Subscriber>>, WatchError> {
        let checkpoint = match std::fs::read(self.checkpoint_path()) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Ok(None),
        };
        let Ok(text) = std::str::from_utf8(&checkpoint) else {
            return Ok(None);
        };
        let mut lines = text.lines();
        let Some(header_line) = lines.next() else {
            return Ok(None);
        };
        let Ok(header) = serde_json::from_str::<CheckpointHeader>(header_line) else {
            return Ok(None);
        };
        if header.version != CHECKPOINT_VERSION {
            return Ok(None);
        }
        let stamp = match Self::inspect(path)? {
            Some(stamp) => stamp,
            None if header.covered_bytes == 0
                && !text.lines().skip(1).any(|line| !line.trim().is_empty()) =>
            {
                // An empty store may legitimately have an empty checkpoint.
                return Ok(Some(Vec::new()));
            }
            None => return Ok(None),
        };
        if stamp.len < header.covered_bytes {
            return Ok(None);
        }
        #[cfg(unix)]
        if header.covered_bytes > 0 && stamp.ino != header.covered_ino {
            return Ok(None);
        }
        let mut folded = Folded::new(None, Vec::new());
        let mut parsed = 0u64;
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(subscriber) = serde_json::from_str::<Subscriber>(line) else {
                return Ok(None);
            };
            parsed += 1;
            folded.apply(&subscriber);
        }
        // The delta must be exactly the main log's uncovered tail.
        let delta = match std::fs::read(self.delta_path()) {
            Ok(bytes) => bytes,
            Err(_) => return Ok(None),
        };
        let Some(split) = delta.iter().position(|byte| *byte == b'\n') else {
            return Ok(None);
        };
        let Ok(delta_header) = serde_json::from_slice::<DeltaHeader>(&delta[..split]) else {
            return Ok(None);
        };
        if delta_header.version != DELTA_VERSION || delta_header.generation != header.generation {
            return Ok(None);
        }
        let delta_payload = &delta[split + 1..];
        let tail = match read_tail(path, header.covered_bytes, stamp.len) {
            Ok(tail) => tail,
            Err(_) => return Ok(None),
        };
        if tail != delta_payload {
            return Ok(None);
        }
        let Ok(delta_text) = std::str::from_utf8(delta_payload) else {
            return Ok(None);
        };
        for line in delta_text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(subscriber) = serde_json::from_str::<Subscriber>(line) else {
                return Ok(None);
            };
            parsed += 1;
            folded.apply(&subscriber);
        }
        self.rows_read.fetch_add(parsed, Ordering::Relaxed);
        Ok(Some(folded.subscribers))
    }

    /// Read and fold the file: the newest row per id wins, in first-seen
    /// order. A missing file is an empty store; a malformed line is a typed
    /// refusal.
    fn read_fold(&self, path: &Path) -> Result<Vec<Subscriber>, WatchError> {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(WatchError::StoreCorrupt {
                    detail: format!("{} could not be read: {error}", path.display()),
                })
            }
        };
        let mut order: Vec<String> = Vec::new();
        let mut by_id: HashMap<String, Subscriber> = HashMap::new();
        let mut rows = 0u64;
        for (index, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let subscriber: Subscriber =
                serde_json::from_str(line).map_err(|error| WatchError::StoreCorrupt {
                    detail: format!(
                        "{} line {} is not a subscriber: {error}",
                        path.display(),
                        index + 1
                    ),
                })?;
            rows += 1;
            if !by_id.contains_key(&subscriber.id) {
                order.push(subscriber.id.clone());
            }
            by_id.insert(subscriber.id.clone(), subscriber);
        }
        self.rows_read.fetch_add(rows, Ordering::Relaxed);
        Ok(order
            .into_iter()
            .filter_map(|id| by_id.remove(&id))
            .collect())
    }

    /// The current fold, rebuilt only when the log's identity moved.
    fn fold(&self) -> Result<Vec<Subscriber>, WatchError> {
        let mut cached = self.fold_lock()?;
        self.refresh(&mut cached)?;
        match cached.as_ref() {
            Some(state) => Ok(state.subscribers.clone()),
            None => Ok(Vec::new()),
        }
    }

    /// How many times the log was folded. The seam tests use to prove the
    /// cache serves every unchanged stamp instead of folding per operation.
    #[cfg(test)]
    fn fold_count(&self) -> u64 {
        self.folds.load(Ordering::Relaxed)
    }

    /// Count the rows the indexed lookups had to examine. The scaling seams
    /// assert this stays linear in the number of operations (one indexed
    /// position per lookup) instead of quadratic. The counter is a relaxed
    /// atomic add per lookup, cheap enough to leave on in production and the
    /// only way the HTTP-level tests can prove a request used the index.
    #[inline]
    fn observe_row_probes(&self, probes: u64) {
        self.row_probes.fetch_add(probes, Ordering::Relaxed);
    }

    /// The rows examined by indexed lookups so far (the diagnostics seam).
    #[must_use]
    pub fn indexed_row_probes(&self) -> u64 {
        self.row_probes.load(Ordering::Relaxed)
    }

    /// The rows examined by indexed lookups so far.
    #[cfg(test)]
    fn row_probe_count(&self) -> u64 {
        self.indexed_row_probes()
    }

    /// The current subscribers, oldest first.
    pub fn list(&self) -> Result<Vec<Subscriber>, WatchError> {
        self.fold()
    }

    /// The active subscribers, oldest first.
    pub fn list_active(&self) -> Result<Vec<Subscriber>, WatchError> {
        Ok(self
            .fold()?
            .into_iter()
            .filter(Subscriber::is_active)
            .collect())
    }

    /// Validate, deduplicate and record one sign-up.
    pub fn subscribe(
        &self,
        now_ms: i64,
        domain: &str,
        email: &str,
        accounts: &str,
        core: bool,
    ) -> Result<SubscribeOutcome, WatchError> {
        let signup: WatchSignup =
            validate_watch_signup(domain, email, accounts, core).map_err(WatchError::Validation)?;
        let _guard = self.lock()?;
        let mut cached = self.fold_lock()?;
        self.refresh(&mut cached)?;
        let dedup = DedupKey::of_signup(&signup.domain, &signup.email);
        let existing = cached.as_ref().and_then(|state| {
            let (index, probes) = state.find_dedup(&dedup, now_ms);
            self.observe_row_probes(probes);
            index.map(|index| state.subscribers[index].clone())
        });
        if let Some(existing) = existing {
            return Ok(SubscribeOutcome {
                subscriber: existing,
                created: false,
                cancel_token: None,
            });
        }
        let cancel_token = random_hex(16);
        let tier = signup.tier();
        let subscriber = Subscriber {
            id: format!("sub_{}", random_hex(16)),
            publisher: publisher_slug_for_domain(&signup.domain),
            domain: signup.domain,
            email: signup.email,
            apps: Vec::new(),
            tier,
            verified_app_count: None,
            apps_verified_at: None,
            apps_verification_error: None,
            tier_previous: None,
            tier_changed_at: None,
            account_ids: signup.account_ids,
            created_ms: now_ms,
            status: Status::Active,
            canceled_ms: None,
            confirmed: true,
            confirmation_token_hash: None,
            confirmation_expires_at: None,
            confirmation_used_at: None,
            email_confirmed_at: Some(now_ms),
            profile_token_hash: None,
            profile_token_expires_at: None,
            profile_token_used_at: None,
            paid_at: Some(now_ms),
            payment_link: None,
            payment_link_error: None,
            customer_reference: None,
            stored_mandate: None,
            last_check_at: None,
            last_findings_hash: None,
            last_findings: None,
            last_alert_at: None,
            cancel_token_hash: sha256_hex(&cancel_token),
            billing_profile: None,
            first_payment_amount_micros: None,
            first_payment_currency: None,
            first_invoice_number: None,
            first_invoice_month: None,
            first_invoice_net_micros: None,
            first_invoice_gross_micros: None,
            first_invoice_document: None,
            last_billed_month: None,
            last_paid_month: None,
            last_invoice_number: None,
            last_charge_reference: None,
            last_charge_amount_micros: None,
            last_charge_error: None,
        };
        self.append(&mut cached, &subscriber)?;
        Ok(SubscribeOutcome {
            subscriber,
            created: true,
            cancel_token: Some(cancel_token),
        })
    }

    /// Create one pending sign-up: no check runs until the creating email is
    /// confirmed and the billing profile is recorded. The tier is derived
    /// from the distinct named apps in `request.apps` (never from ad-system
    /// account ids and never a checkbox), the subscriber id is the durable
    /// reference, and the plaintext one-time confirmation token is returned
    /// exactly once.
    ///
    /// A repeat submission for the same `(domain, email)` inside
    /// [`DEDUP_WINDOW_MS`] is the same subscription: it keeps the reference
    /// and receives a freshly minted confirmation token instead of a second
    /// row.
    /// A recent pending sign-up for this domain/email, if one is still
    /// inside the per-recipient cooldown. The caller can redirect without
    /// repeating the store's external app lookups.
    pub fn recent_pending(
        &self,
        domain: &str,
        email: &str,
        now_ms: i64,
    ) -> Result<Option<String>, WatchError> {
        let _guard = self.lock()?;
        let mut cached = self.fold_lock()?;
        self.refresh(&mut cached)?;
        let dedup = DedupKey::of_signup(domain, email);
        let existing = cached.as_ref().and_then(|state| {
            let (index, probes) = state.find_dedup(&dedup, now_ms);
            self.observe_row_probes(probes);
            index.map(|index| state.subscribers[index].clone())
        });
        Ok(existing
            .filter(|subscriber| {
                subscriber.status == Status::Pending
                    && subscriber.created_ms.saturating_add(10 * 60_000) > now_ms
                    && subscriber
                        .confirmation_expires_at
                        .is_some_and(|expires| expires > now_ms)
            })
            .map(|subscriber| subscriber.id))
    }

    pub fn subscribe_pending(
        &self,
        now_ms: i64,
        request: &SubscribeRequest,
    ) -> Result<PendingOutcome, WatchError> {
        let signup: WatchSignup =
            validate_watch_signup(&request.domain, &request.email, &request.accounts, false)
                .map_err(WatchError::Validation)?;
        let _guard = self.lock()?;
        let mut cached = self.fold_lock()?;
        self.refresh(&mut cached)?;
        // Only the verified apps and developer catalogues decide the tier.
        // A supplied line the lookup could not confirm (a mistyped id, a
        // network failure) never buys the core price.
        let tier = tier_from_apps(request.verified_app_count.unwrap_or(0));
        let apps = distinct_apps(&request.apps);
        let token = random_hex(CONFIRMATION_TOKEN_HEX_LEN / 2);
        let token_hash = sha256_hex(&token);
        let expires_at = now_ms.saturating_add(CONFIRMATION_TTL_MS);
        let dedup = DedupKey::of_signup(&signup.domain, &signup.email);
        let existing = cached.as_ref().and_then(|state| {
            let (index, probes) = state.find_dedup(&dedup, now_ms);
            self.observe_row_probes(probes);
            index.map(|index| state.subscribers[index].clone())
        });
        if let Some(mut existing) = existing {
            let recent_pending = existing.status == Status::Pending
                && existing.created_ms.saturating_add(10 * 60_000) > now_ms
                && existing
                    .confirmation_expires_at
                    .is_some_and(|expires| expires > now_ms);
            existing.apps = apps.clone();
            existing.tier = tier;
            existing.verified_app_count = request.verified_app_count;
            existing.apps_verified_at = request.verification_error.is_none().then_some(now_ms);
            existing.apps_verification_error = request.verification_error.clone();
            existing.customer_reference = Some(existing.id.clone());
            if recent_pending {
                // Keep the already-mailed token valid; do not rotate it and do
                // not send a second message for the same recipient.
                self.append(&mut cached, &existing)?;
                return Ok(PendingOutcome {
                    reference: existing.id,
                    tier,
                    confirmation_token: String::new(),
                    send_email: false,
                });
            }
            existing.confirmation_token_hash = Some(token_hash);
            existing.confirmation_expires_at = Some(expires_at);
            existing.confirmation_used_at = None;
            self.append(&mut cached, &existing)?;
            return Ok(PendingOutcome {
                reference: existing.id,
                tier,
                confirmation_token: token,
                send_email: true,
            });
        }
        let id = format!("sub_{}", random_hex(16));
        let subscriber = Subscriber {
            id: id.clone(),
            publisher: publisher_slug_for_domain(&signup.domain),
            domain: signup.domain,
            email: signup.email,
            apps,
            tier,
            verified_app_count: request.verified_app_count,
            apps_verified_at: request.verification_error.is_none().then_some(now_ms),
            apps_verification_error: request.verification_error.clone(),
            tier_previous: None,
            tier_changed_at: None,
            account_ids: signup.account_ids,
            created_ms: now_ms,
            status: Status::Pending,
            canceled_ms: None,
            confirmed: false,
            confirmation_token_hash: Some(token_hash),
            confirmation_expires_at: Some(expires_at),
            confirmation_used_at: None,
            email_confirmed_at: None,
            profile_token_hash: None,
            profile_token_expires_at: None,
            profile_token_used_at: None,
            paid_at: None,
            payment_link: None,
            payment_link_error: None,
            customer_reference: Some(id.clone()),
            stored_mandate: None,
            last_check_at: None,
            last_findings_hash: None,
            last_findings: None,
            last_alert_at: None,
            cancel_token_hash: sha256_hex(&random_hex(16)),
            billing_profile: None,
            first_payment_amount_micros: None,
            first_payment_currency: None,
            first_invoice_number: None,
            first_invoice_month: None,
            first_invoice_net_micros: None,
            first_invoice_gross_micros: None,
            first_invoice_document: None,
            last_billed_month: None,
            last_paid_month: None,
            last_invoice_number: None,
            last_charge_reference: None,
            last_charge_amount_micros: None,
            last_charge_error: None,
        };
        self.append(&mut cached, &subscriber)?;
        Ok(PendingOutcome {
            reference: id,
            tier,
            confirmation_token: token,
            send_email: true,
        })
    }

    /// Consume the one-time confirmation token: the matching row is marked
    /// confirmed, the token is marked used, and the outcome names the
    /// subscriber. A second use, an expired token or an unknown token is a
    /// typed refusal; nothing else is written.
    pub fn confirm_email(&self, now_ms: i64, token: &str) -> Result<ConfirmOutcome, WatchError> {
        let token = token.trim();
        if !is_confirmation_token(token) {
            return Err(WatchError::ConfirmationUnknown);
        }
        let hash = sha256_hex(token);
        let _guard = self.lock()?;
        let mut cached = self.fold_lock()?;
        self.refresh(&mut cached)?;
        let found = cached.as_ref().and_then(|state| {
            let (index, probes) = state.find_confirmation(&hash);
            self.observe_row_probes(probes);
            index.map(|index| state.subscribers[index].clone())
        });
        let Some(mut subscriber) = found else {
            return Err(WatchError::ConfirmationUnknown);
        };
        if subscriber.confirmation_used_at.is_some() {
            return Err(WatchError::ConfirmationUsed);
        }
        if subscriber.confirmed {
            return Err(WatchError::AlreadyConfirmed);
        }
        if subscriber
            .confirmation_expires_at
            .is_none_or(|expires| now_ms >= expires)
        {
            return Err(WatchError::ConfirmationExpired);
        }
        subscriber.confirmed = true;
        subscriber.confirmation_used_at = Some(now_ms);
        subscriber.email_confirmed_at = Some(now_ms);
        self.append(&mut cached, &subscriber)?;
        Ok(ConfirmOutcome {
            reference: subscriber.id,
            email: subscriber.email,
            confirmed_at: now_ms,
        })
    }

    /// Switch the matching pending subscriber on after the email and billing
    /// profile were recorded.
    /// Only a pending row can be activated; an already-active row is a typed
    /// idempotent no-op (`already_active: true`) and appends nothing, and a
    /// canceled row or an unknown reference is a typed refusal.
    pub fn activate(&self, now_ms: i64, reference: &str) -> Result<ActivateOutcome, WatchError> {
        let reference = reference.trim();
        let _guard = self.lock()?;
        let mut cached = self.fold_lock()?;
        self.refresh(&mut cached)?;
        let found = cached.as_ref().and_then(|state| {
            let (index, probes) = state.find_id(reference);
            self.observe_row_probes(probes);
            index.map(|index| state.subscribers[index].clone())
        });
        let Some(mut subscriber) = found else {
            return Err(WatchError::UnknownSubscriber {
                reference: reference.to_string(),
            });
        };
        match subscriber.status {
            Status::Active => Ok(ActivateOutcome {
                reference: subscriber.id,
                paid_at: subscriber.paid_at.unwrap_or(now_ms),
                already_active: true,
            }),
            Status::Canceled => Err(WatchError::NotPending {
                reference: subscriber.id,
                status: Status::Canceled,
            }),
            Status::Pending => {
                subscriber.status = Status::Active;
                subscriber.paid_at = Some(now_ms);
                self.append(&mut cached, &subscriber)?;
                Ok(ActivateOutcome {
                    reference: subscriber.id,
                    paid_at: now_ms,
                    already_active: false,
                })
            }
        }
    }

    /// Legacy helper that records a payment link on an old pending row. New
    /// free sign-ups never call it; the field is read only for compatibility.
    pub fn record_payment_link(
        &self,
        reference: &str,
        payment_link: &str,
    ) -> Result<(), WatchError> {
        self.update_by_reference(reference, |subscriber| {
            subscriber.payment_link = Some(payment_link.to_string());
            subscriber.payment_link_error = None;
            subscriber.customer_reference = Some(subscriber.id.clone());
        })
    }

    /// Legacy helper that records a stored-card id on an old row. New free
    /// sign-ups never call it; the activation stays [`Self::activate`].
    pub fn record_stored_mandate(
        &self,
        reference: &str,
        stored_mandate: &str,
    ) -> Result<(), WatchError> {
        self.update_by_reference(reference, |subscriber| {
            subscriber.stored_mandate = Some(stored_mandate.to_string());
        })
    }

    /// The current row with `reference`, when one exists.
    pub fn get(&self, reference: &str) -> Result<Option<Subscriber>, WatchError> {
        let reference = reference.trim();
        let _guard = self.lock()?;
        let mut cached = self.fold_lock()?;
        self.refresh(&mut cached)?;
        Ok(cached.as_ref().and_then(|state| {
            let (index, probes) = state.find_id(reference);
            self.observe_row_probes(probes);
            index.map(|index| state.subscribers[index].clone())
        }))
    }

    /// Mint the one-time billing-profile token for `reference`.
    ///
    /// The token has the email-confirmation shape (32 lowercase hex
    /// characters), lives for [`CONFIRMATION_TTL_MS`] and only its SHA-256
    /// hash is stored. At most one token is outstanding per subscriber: a
    /// second call replaces the previous hash, so the older token stops
    /// matching. Unknown and canceled subscribers are typed refusals.
    pub fn create_profile_token(&self, reference: &str) -> Result<String, WatchError> {
        self.create_profile_token_at(now_unix_ms(), reference)
    }

    /// [`Self::create_profile_token`] with an explicit clock, for tests and
    /// deterministic callers.
    pub fn create_profile_token_at(
        &self,
        now_ms: i64,
        reference: &str,
    ) -> Result<String, WatchError> {
        let reference = reference.trim();
        let _guard = self.lock()?;
        let mut cached = self.fold_lock()?;
        self.refresh(&mut cached)?;
        let found = cached.as_ref().and_then(|state| {
            let (index, probes) = state.find_id(reference);
            self.observe_row_probes(probes);
            index.map(|index| state.subscribers[index].clone())
        });
        let Some(mut subscriber) = found else {
            return Err(WatchError::UnknownSubscriber {
                reference: reference.to_string(),
            });
        };
        if subscriber.status == Status::Canceled {
            return Err(WatchError::NotBillable {
                reference: subscriber.id,
                status: Status::Canceled,
            });
        }
        let token = random_hex(CONFIRMATION_TOKEN_HEX_LEN / 2);
        subscriber.profile_token_hash = Some(sha256_hex(&token));
        subscriber.profile_token_expires_at = Some(now_ms.saturating_add(CONFIRMATION_TTL_MS));
        subscriber.profile_token_used_at = None;
        self.append(&mut cached, &subscriber)?;
        Ok(token)
    }

    /// Consume one billing-profile token exactly once, returning the
    /// subscriber reference it was tied to. An expired, already-used,
    /// unknown or malformed token is a typed refusal; nothing else is
    /// written.
    pub fn consume_profile_token(&self, now_ms: i64, token: &str) -> Result<String, WatchError> {
        let token = token.trim();
        if !is_confirmation_token(token) {
            return Err(WatchError::ProfileTokenUnknown);
        }
        let hash = sha256_hex(token);
        let _guard = self.lock()?;
        let mut cached = self.fold_lock()?;
        self.refresh(&mut cached)?;
        let found = cached.as_ref().and_then(|state| {
            let (index, probes) = state.find_profile_token(&hash);
            self.observe_row_probes(probes);
            index.map(|index| state.subscribers[index].clone())
        });
        let Some(mut subscriber) = found else {
            return Err(WatchError::ProfileTokenUnknown);
        };
        if subscriber.profile_token_used_at.is_some() {
            return Err(WatchError::ProfileTokenUsed);
        }
        if subscriber.status == Status::Canceled {
            return Err(WatchError::NotBillable {
                reference: subscriber.id,
                status: Status::Canceled,
            });
        }
        if subscriber
            .profile_token_expires_at
            .is_none_or(|expires| now_ms >= expires)
        {
            return Err(WatchError::ProfileTokenExpired);
        }
        subscriber.profile_token_used_at = Some(now_ms);
        self.append(&mut cached, &subscriber)?;
        Ok(subscriber.id)
    }

    /// Validate and record the customer billing profile the first and monthly
    /// invoices render with.
    ///
    /// The profile is trimmed and the country is canonicalised before it is
    /// stored. A replay with the same profile returns the stored row without
    /// appending a duplicate; a different profile after one is set is a typed
    /// refusal. Unknown and canceled subscribers are refused.
    pub fn set_billing_profile(
        &self,
        reference: &str,
        profile: &BillingProfile,
        _now_ms: i64,
    ) -> Result<Subscriber, WatchError> {
        let reference = reference.trim();
        let profile = normalize_billing_profile(profile);
        profile.validate()?;
        let _guard = self.lock()?;
        let mut cached = self.fold_lock()?;
        self.refresh(&mut cached)?;
        let found = cached.as_ref().and_then(|state| {
            let (index, probes) = state.find_id(reference);
            self.observe_row_probes(probes);
            index.map(|index| state.subscribers[index].clone())
        });
        let Some(mut subscriber) = found else {
            return Err(WatchError::UnknownSubscriber {
                reference: reference.to_string(),
            });
        };
        if subscriber.status == Status::Canceled {
            return Err(WatchError::NotBillable {
                reference: subscriber.id,
                status: Status::Canceled,
            });
        }
        match &subscriber.billing_profile {
            Some(existing) if existing == &profile => return Ok(subscriber),
            Some(_) => {
                return Err(WatchError::BillingProfileLocked {
                    reference: subscriber.id,
                })
            }
            None => {}
        }
        subscriber.billing_profile = Some(profile);
        self.append(&mut cached, &subscriber)?;
        Ok(subscriber)
    }

    /// Consume a billing-profile token and record the profile it authorises
    /// in one call. The token is single-use, so a replay is a typed refusal
    /// even when the same profile is presented.
    pub fn set_billing_profile_with_token(
        &self,
        token: &str,
        profile: &BillingProfile,
        now_ms: i64,
    ) -> Result<Subscriber, WatchError> {
        let reference = self.consume_profile_token(now_ms, token)?;
        self.set_billing_profile(&reference, profile, now_ms)
    }

    /// Read-modify-append one row by reference. An unknown reference is a
    /// typed refusal, never a silent no-op.
    fn update_by_reference(
        &self,
        reference: &str,
        update: impl FnOnce(&mut Subscriber),
    ) -> Result<(), WatchError> {
        let reference = reference.trim();
        let _guard = self.lock()?;
        let mut cached = self.fold_lock()?;
        self.refresh(&mut cached)?;
        let found = cached.as_ref().and_then(|state| {
            let (index, probes) = state.find_id(reference);
            self.observe_row_probes(probes);
            index.map(|index| state.subscribers[index].clone())
        });
        let Some(mut subscriber) = found else {
            return Err(WatchError::UnknownSubscriber {
                reference: reference.to_string(),
            });
        };
        update(&mut subscriber);
        self.append(&mut cached, &subscriber)
    }

    /// Mint a fresh plaintext cancel token and its stored hash. The raw value
    /// is returned once (for the alert footer / unsubscribe header) and only
    /// the hash is ever written.
    pub fn mint_cancel_token() -> (String, String) {
        // The cancel token keeps the 32-hex (128-bit) capability shape the
        // CLI, the HTTP validator and the stored hash all agree on.
        let raw = random_hex(16);
        let hash = sha256_hex(&raw);
        (raw, hash)
    }

    /// The active subscriber a plaintext cancel token names, without changing
    /// anything (the cancel GET page is inert).
    pub fn find_by_cancel_token(&self, token: &str) -> Result<Option<Subscriber>, WatchError> {
        let token = token.trim();
        if token.is_empty() {
            return Ok(None);
        }
        let presented = sha256_hex(token);
        let _guard = self.lock()?;
        let mut cached = self.fold_lock()?;
        self.refresh(&mut cached)?;
        Ok(cached.as_ref().and_then(|state| {
            let (index, probes) = state.find_cancel(&presented);
            self.observe_row_probes(probes);
            index.map(|index| state.subscribers[index].clone())
        }))
    }

    /// Cancel with a presented plaintext token: the first active or pending
    /// subscriber whose stored hash matches is cancelled (a new row records the
    /// cancellation; nothing is deleted). `None` means no active subscriber
    /// matches the token.
    pub fn cancel(&self, token: &str, now_ms: i64) -> Result<Option<Subscriber>, WatchError> {
        let token = token.trim();
        if token.is_empty() {
            return Ok(None);
        }
        let presented = sha256_hex(token);
        let _guard = self.lock()?;
        let mut cached = self.fold_lock()?;
        self.refresh(&mut cached)?;
        let found = cached.as_ref().and_then(|state| {
            let (index, probes) = state.find_cancel(&presented);
            self.observe_row_probes(probes);
            index.map(|index| state.subscribers[index].clone())
        });
        let Some(mut found) = found else {
            return Ok(None);
        };
        found.status = Status::Canceled;
        found.canceled_ms = Some(now_ms);
        self.append(&mut cached, &found)?;
        Ok(Some(found))
    }

    /// Append the current state of `subscriber` (the newest row wins).
    ///
    /// An update row is self-contained, so this never reads the log: when
    /// the cached fold is current it is advanced in place; when it is stale
    /// (another process appended) the append invalidates it and the next
    /// read folds once.
    pub fn update(&self, subscriber: &Subscriber) -> Result<(), WatchError> {
        self.update_many(std::slice::from_ref(subscriber))
    }

    /// Append several rows with one open and one write (one durability
    /// window), then advance the cached fold in order. The daily loop uses
    /// this so recording n subscribers is one append instead of n.
    pub fn update_many(&self, subscribers: &[Subscriber]) -> Result<(), WatchError> {
        if subscribers.is_empty() {
            return Ok(());
        }
        let _guard = self.lock()?;
        let mut cached = self.fold_lock()?;
        self.append_many(&mut cached, subscribers)
    }

    /// Append rows and, after the write succeeded, advance the cached fold
    /// in place. The caller holds the write lock. When the file moved beyond
    /// our own bytes (a stale cache, a concurrent external append), the
    /// cache is dropped so the next use re-folds exactly once.
    fn append(
        &self,
        cached: &mut Option<Folded>,
        subscriber: &Subscriber,
    ) -> Result<(), WatchError> {
        self.append_many(cached, std::slice::from_ref(subscriber))
    }

    fn append_many(
        &self,
        cached: &mut Option<Folded>,
        subscribers: &[Subscriber],
    ) -> Result<(), WatchError> {
        let path = self.subscribers_path();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| WatchError::Io {
                    detail: format!("{}: parent directory: {error}", path.display()),
                })?;
            }
        }
        let mut lines = String::new();
        for subscriber in subscribers {
            lines.push_str(
                &serde_json::to_string(subscriber).map_err(|error| WatchError::Io {
                    detail: format!("subscriber serialization: {error}"),
                })?,
            );
            lines.push('\n');
        }
        // A stat before the write keeps a stale cache from absorbing the
        // new rows: only a cache whose stamp still matches the file may be
        // advanced in place.
        let observed = Self::inspect(&path).unwrap_or_default();
        let cache_current = cached.as_ref().is_some_and(|state| state.stamp == observed);
        let written = lines.len() as u64;
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(|error| WatchError::Io {
            detail: format!("{} could not be opened: {error}", path.display()),
        })?;
        // `mode(0o600)` only applies when the file is created. A restored or
        // externally-created log may be world-readable, and it carries
        // subscriber emails and token hashes; make the store's own log
        // owner-only every time it writes it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(
                |error| WatchError::Io {
                    detail: format!(
                        "{} could not be restricted to mode 0600: {error}",
                        path.display()
                    ),
                },
            )?;
        }
        file.write_all(lines.as_bytes())
            .map_err(|error| WatchError::Io {
                detail: format!("{} could not be appended to: {error}", path.display()),
            })?;
        drop(file);
        // The delta is a read optimization; if it cannot be written the full
        // log remains the source of truth and the next fold recovers.
        self.append_delta(lines.as_bytes());
        match Self::inspect(&path) {
            Ok(Some(stamp))
                if cache_current && appended_alone(observed.as_ref(), &stamp, written) =>
            {
                if let Some(state) = cached.as_mut() {
                    for subscriber in subscribers {
                        let probes = state.apply(subscriber);
                        self.observe_row_probes(probes);
                    }
                    state.stamp = Some(stamp);
                }
            }
            _ => *cached = None,
        }
        Ok(())
    }

    /// Append the same bytes to the delta log when a checkpoint generation is
    /// known and the delta header matches it. Any mismatch is left for the
    /// reader's validation, which falls back to the full log.
    fn append_delta(&self, bytes: &[u8]) {
        if self.delta_broken.load(Ordering::Relaxed) {
            return;
        }
        let generation = match self.delta_generation() {
            Some(generation) => generation,
            None => return,
        };
        let path = self.delta_path();
        let mut existing = std::fs::read(&path).unwrap_or_default();
        if existing.is_empty() {
            let header = DeltaHeader {
                version: DELTA_VERSION,
                generation: generation.clone(),
            };
            let Ok(mut line) = serde_json::to_vec(&header) else {
                self.delta_broken.store(true, Ordering::Relaxed);
                return;
            };
            line.push(b'\n');
            if atomic_write(&path, &line).is_err() {
                self.delta_broken.store(true, Ordering::Relaxed);
                return;
            }
            existing = line;
        }
        let matches = existing
            .split(|byte| *byte == b'\n')
            .next()
            .and_then(|line| serde_json::from_slice::<DeltaHeader>(line).ok())
            .is_some_and(|header| {
                header.version == DELTA_VERSION && header.generation == generation
            });
        if !matches {
            self.delta_broken.store(true, Ordering::Relaxed);
            return;
        }
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                if file.write_all(bytes).is_err() {
                    self.delta_broken.store(true, Ordering::Relaxed);
                }
            }
            Err(_) => self.delta_broken.store(true, Ordering::Relaxed),
        }
    }

    /// The checkpoint generation appends follow, loaded lazily from the
    /// checkpoint header. `None` means no checkpoint exists.
    fn delta_generation(&self) -> Option<String> {
        let mut guard = self.delta_generation.lock().ok()?;
        if guard.is_none() {
            let raw = std::fs::read_to_string(self.checkpoint_path()).ok()?;
            let header: CheckpointHeader = serde_json::from_str(raw.lines().next()?).ok()?;
            if header.version != CHECKPOINT_VERSION {
                return None;
            }
            *guard = Some(header.generation);
        }
        guard.clone()
    }

    /// Read the operator-provided billing sidecar for one publisher
    /// (`<root>/<publisher>.billing.json`, the runner's convention). A
    /// missing file is `Ok(None)`; unreadable or malformed content is a
    /// typed refusal.
    pub fn billing_sidecar(
        &self,
        publisher: &str,
    ) -> Result<Option<apex_billing::CustomerProfile>, WatchError> {
        self.read_billing_sidecar(publisher)
    }

    fn read_billing_sidecar(
        &self,
        publisher: &str,
    ) -> Result<Option<apex_billing::CustomerProfile>, WatchError> {
        let path = self.root.join(format!("{publisher}.billing.json"));
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(WatchError::Malformed(format!(
                    "customer billing profile at {} could not be read: {error}",
                    path.display()
                )))
            }
        };
        let profile: apex_billing::CustomerProfile =
            serde_json::from_str(&raw).map_err(|error| {
                WatchError::Malformed(format!(
                    "customer billing profile at {} is not valid JSON: {error}",
                    path.display()
                ))
            })?;
        Ok(Some(profile))
    }
}

/// Read exactly `[start, end)` bytes from `path`.
fn read_tail(path: &Path, start: u64, end: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::with_capacity(usize::try_from(end.saturating_sub(start)).unwrap_or(0));
    file.take(end.saturating_sub(start))
        .read_to_end(&mut tail)?;
    Ok(tail)
}

/// Write `bytes` to `path` atomically: a same-directory temp file (0600),
/// fsync, then rename over the destination.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), WatchError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| WatchError::Io {
                detail: format!("{}: parent directory: {error}", path.display()),
            })?;
        }
    }
    let temp = path.with_file_name(format!(
        "{}.tmp.{}.{}",
        path.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "watch-state".to_string()),
        std::process::id(),
        random_hex(6)
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> std::io::Result<()> {
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(|error| WatchError::Io {
        detail: format!(
            "{} could not be written atomically: {error}",
            path.display()
        ),
    })
}

/// Restrict a state file that already exists to owner-only mode. A missing
/// file is left for the atomic writer to create with mode 0600.
#[cfg(unix)]
fn restrict_mode(path: &Path) -> Result<(), WatchError> {
    use std::os::unix::fs::PermissionsExt as _;
    match std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(WatchError::Io {
            detail: format!(
                "{} could not be restricted to mode 0600: {error}",
                path.display()
            ),
        }),
    }
}

#[cfg(not(unix))]
fn restrict_mode(_path: &Path) -> Result<(), WatchError> {
    Ok(())
}

/// Whether the file after our append is exactly the file before it plus the
/// bytes we wrote. A concurrent external append between the stamp and our
/// write makes the length (or the inode) disagree, so the cached fold is
/// dropped and rebuilt once instead of hiding the external row.
fn appended_alone(before: Option<&FileStamp>, after: &FileStamp, written: u64) -> bool {
    match before {
        Some(before) => after.len == before.len.saturating_add(written) && same_file(before, after),
        None => after.len == written,
    }
}

#[cfg(unix)]
fn same_file(before: &FileStamp, after: &FileStamp) -> bool {
    before.ino == after.ino
}

#[cfg(not(unix))]
fn same_file(_before: &FileStamp, _after: &FileStamp) -> bool {
    true
}

/// Trim the profile's text and canonicalise the country to uppercase, so an
/// idempotent replay with the same values compares equal. Validation happens
/// after this normalization.
fn normalize_billing_profile(profile: &BillingProfile) -> BillingProfile {
    BillingProfile {
        legal_name: profile.legal_name.trim().to_string(),
        address_lines: profile
            .address_lines
            .iter()
            .map(|line| line.trim().to_string())
            .collect(),
        country: profile.country.trim().to_ascii_uppercase(),
        vat_number: profile
            .vat_number
            .as_ref()
            .map(|vat| vat.trim().to_string()),
    }
}

/// Hex-encode `bytes`.
fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// `bytes` random bytes, lowercase hex.
pub fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buffer);
    hex_encode(&buffer)
}

/// Lowercase hex SHA-256 of `value`.
pub fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex_encode(&hasher.finalize())
}

/// The exact shape of a confirmation token: 32 lowercase hex characters,
/// the same shape as the console sign-in token.
pub fn is_confirmation_token(value: &str) -> bool {
    value.len() == CONFIRMATION_TOKEN_HEX_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Constant-time equality over byte slices of equal intent; differing
/// lengths still compare in constant time over the shorter prefix.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in a.iter().zip(b.iter()) {
        difference |= left ^ right;
    }
    difference == 0
}

/// The current unix time in milliseconds.
pub fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// A fold helper shared by tests: the distinct ids seen in a file.
#[cfg(test)]
fn ids_in_file(path: &Path) -> std::collections::HashSet<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<Subscriber>(line).ok())
        .map(|subscriber| subscriber.id)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn store(dir: &tempfile::TempDir) -> SubscriberStore {
        SubscriberStore::open(dir.path())
    }

    #[cfg(unix)]
    #[test]
    fn the_subscriber_log_is_created_mode_0600() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        store
            .subscribe(1_000, "northstar.example", "a@northstar.example", "", false)
            .expect("subscribe");
        let mode = std::fs::metadata(store.subscribers_path())
            .expect("stat")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "subscribers.jsonl");
    }

    #[cfg(unix)]
    #[test]
    fn a_restored_world_readable_log_is_restricted_on_the_next_write() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let row = row("sub_restore", "restore.example", "a@x.example", 10);
        write_rows(&store.subscribers_path(), std::slice::from_ref(&row));
        std::fs::set_permissions(
            store.subscribers_path(),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let mut updated = row;
        updated.last_check_at = Some(20);
        store.update(&updated).expect("update");
        let mode = std::fs::metadata(store.subscribers_path())
            .expect("stat")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the log carries subscriber data");
    }

    #[test]
    fn subscribe_records_one_row_and_returns_the_token_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let outcome = store
            .subscribe(
                1_000,
                "Northstar.Example",
                " founder@northstar.example ",
                "",
                false,
            )
            .expect("subscribe");
        assert!(outcome.created);
        assert_eq!(outcome.subscriber.domain, "northstar.example");
        assert_eq!(outcome.subscriber.publisher, "northstar-example");
        assert_eq!(outcome.subscriber.tier, crate::model::Tier::Normal);
        assert!(outcome.subscriber.is_active());
        let token = outcome.cancel_token.clone().expect("token");
        assert_eq!(token.len(), 32);
        let raw = std::fs::read_to_string(store.subscribers_path()).unwrap();
        assert_eq!(raw.lines().count(), 1);
        assert!(raw.contains(&outcome.subscriber.cancel_token_hash));
        assert!(!raw.contains(&token), "the plaintext token is never stored");
    }

    #[test]
    fn repeat_submissions_inside_the_window_are_the_same_subscription() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let first = store
            .subscribe(1_000, "example.com", "a@x.example", "", false)
            .expect("first");
        let repeat = store
            .subscribe(1_500, "example.com", "A@x.example", "", true)
            .expect("repeat");
        assert!(!repeat.created);
        assert_eq!(repeat.subscriber.id, first.subscriber.id);
        assert!(repeat.cancel_token.is_none());
        assert_eq!(
            std::fs::read_to_string(store.subscribers_path())
                .unwrap()
                .lines()
                .count(),
            1,
            "a duplicate appends nothing"
        );

        // Outside the window the same pair is a new subscription.
        let later = store
            .subscribe(
                1_000 + DEDUP_WINDOW_MS + 1,
                "example.com",
                "a@x.example",
                "",
                false,
            )
            .expect("later");
        assert!(later.created);
        assert_ne!(later.subscriber.id, first.subscriber.id);
        assert_eq!(
            std::fs::read_to_string(store.subscribers_path())
                .unwrap()
                .lines()
                .count(),
            2
        );
    }

    #[test]
    fn cancel_matches_the_token_and_appends_a_canceled_row() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let outcome = store
            .subscribe(10, "example.com", "a@x.example", "", false)
            .expect("subscribe");
        let token = outcome.cancel_token.clone().expect("token");

        assert!(
            store.cancel("not-the-token", 20).expect("wrong").is_none(),
            "a wrong token matches nothing"
        );
        let canceled = store.cancel(&token, 30).expect("cancel").expect("match");
        assert_eq!(canceled.status, Status::Canceled);
        assert_eq!(canceled.canceled_ms, Some(30));

        // The folded state is canceled; a second cancel with the same token
        // matches nothing because the row is no longer active.
        let folded = store.list().expect("list");
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].status, Status::Canceled);
        assert!(store.cancel(&token, 40).expect("again").is_none());
        assert!(store.list_active().expect("active").is_empty());
    }

    #[test]
    fn updates_append_rows_and_the_newest_row_wins() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let outcome = store
            .subscribe(10, "example.com", "a@x.example", "", false)
            .expect("subscribe");
        let mut subscriber = outcome.subscriber;
        subscriber.last_check_at = Some(99);
        subscriber.last_findings_hash = Some("ab".repeat(32));
        store.update(&subscriber).expect("update");
        let folded = store.list().expect("list");
        assert_eq!(folded.len(), 1);
        assert_eq!(folded[0].last_check_at, Some(99));
        assert_eq!(folded[0].last_findings_hash, subscriber.last_findings_hash);
        assert_eq!(
            ids_in_file(&store.subscribers_path()).len(),
            1,
            "still one subscriber id"
        );
    }

    #[test]
    fn invalid_submissions_are_refused_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let error = store
            .subscribe(10, "bad domain", "nope", "", false)
            .expect_err("refused");
        match error {
            WatchError::Validation(errors) => assert_eq!(errors.len(), 2),
            other => panic!("unexpected {other:?}"),
        }
        assert!(!store.subscribers_path().exists());
    }

    #[test]
    fn malformed_files_fail_closed_and_missing_files_are_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        assert!(store.list().expect("missing").is_empty());

        std::fs::write(store.subscribers_path(), "{ not a subscriber").unwrap();
        assert!(matches!(store.list(), Err(WatchError::StoreCorrupt { .. })));
        // A corrupt file also refuses new subscriptions: rows could be lost.
        assert!(matches!(
            store.subscribe(1, "example.com", "a@x.example", "", false),
            Err(WatchError::StoreCorrupt { .. })
        ));
    }

    #[test]
    fn billing_sidecar_reads_the_runner_shape() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        assert!(store.billing_sidecar("pub").expect("missing").is_none());
        std::fs::write(
            dir.path().join("pub.billing.json"),
            r#"{"legalName":"Buyer OÜ","addressLines":["Telliskivi 1"],"country":"EE","vatNumber":"EE111111111"}"#,
        )
        .unwrap();
        let profile = store.billing_sidecar("pub").expect("read").expect("some");
        assert_eq!(profile.legal_name, "Buyer OÜ");
        std::fs::write(dir.path().join("pub.billing.json"), "{").unwrap();
        assert!(matches!(
            store.billing_sidecar("pub"),
            Err(WatchError::Malformed(_))
        ));
    }

    #[test]
    fn from_env_reads_the_documented_default() {
        assert_eq!(
            SubscriberStore::from_env(|_| None).root(),
            Path::new(DEFAULT_WATCH_ROOT)
        );
        let custom = SubscriberStore::from_env(|name| {
            (name == WATCH_ROOT_ENV).then(|| " /tmp/watch ".to_string())
        });
        assert_eq!(custom.root(), Path::new("/tmp/watch"));
        let blank = SubscriberStore::from_env(|name| (name == WATCH_ROOT_ENV).then(String::new));
        assert_eq!(blank.root(), Path::new(DEFAULT_WATCH_ROOT));
    }

    /// Append one row the way another process would: a plain append to the
    /// same log, bypassing the store and its cache.
    fn external_append(path: &Path, subscriber: &Subscriber) {
        use std::io::Write as _;
        let mut line = serde_json::to_string(subscriber).expect("serialize");
        line.push('\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open external append");
        file.write_all(line.as_bytes()).expect("external append");
        file.sync_all().expect("sync external append");
    }

    /// A verbatim copy of the pre-cache fold: read every line, the newest
    /// row per id wins, first-seen order.
    fn naive_fold(path: &Path) -> Vec<Subscriber> {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(error) => panic!("{} could not be read: {error}", path.display()),
        };
        let mut order: Vec<String> = Vec::new();
        let mut by_id: HashMap<String, Subscriber> = HashMap::new();
        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let subscriber: Subscriber = serde_json::from_str(line).expect("subscriber row");
            if !by_id.contains_key(&subscriber.id) {
                order.push(subscriber.id.clone());
            }
            by_id.insert(subscriber.id.clone(), subscriber);
        }
        order
            .into_iter()
            .filter_map(|id| by_id.remove(&id))
            .collect()
    }

    /// The same verbatim fold over the rows a reference run appended, so the
    /// fuzz can check outcomes without re-reading the log on every step.
    fn reference_fold(rows: &[Subscriber]) -> Vec<Subscriber> {
        let mut order: Vec<String> = Vec::new();
        let mut by_id: HashMap<String, Subscriber> = HashMap::new();
        for subscriber in rows {
            if !by_id.contains_key(&subscriber.id) {
                order.push(subscriber.id.clone());
            }
            by_id.insert(subscriber.id.clone(), subscriber.clone());
        }
        order
            .into_iter()
            .filter_map(|id| by_id.remove(&id))
            .collect()
    }

    fn row(id: &str, domain: &str, email: &str, created_ms: i64) -> Subscriber {
        Subscriber {
            id: id.to_string(),
            publisher: publisher_slug_for_domain(domain),
            domain: domain.to_string(),
            email: email.to_string(),
            apps: Vec::new(),
            tier: crate::model::Tier::Normal,
            verified_app_count: None,
            apps_verified_at: None,
            apps_verification_error: None,
            tier_previous: None,
            tier_changed_at: None,
            account_ids: std::collections::BTreeMap::new(),
            created_ms,
            status: Status::Active,
            canceled_ms: None,
            confirmed: true,
            confirmation_token_hash: None,
            confirmation_expires_at: None,
            confirmation_used_at: None,
            email_confirmed_at: Some(created_ms),
            profile_token_hash: None,
            profile_token_expires_at: None,
            profile_token_used_at: None,
            paid_at: Some(created_ms),
            payment_link: None,
            payment_link_error: None,
            customer_reference: None,
            stored_mandate: None,
            last_check_at: None,
            last_findings_hash: None,
            last_findings: None,
            last_alert_at: None,
            cancel_token_hash: sha256_hex(id),
            billing_profile: None,
            first_payment_amount_micros: None,
            first_payment_currency: None,
            first_invoice_number: None,
            first_invoice_month: None,
            first_invoice_net_micros: None,
            first_invoice_gross_micros: None,
            first_invoice_document: None,
            last_billed_month: None,
            last_paid_month: None,
            last_invoice_number: None,
            last_charge_reference: None,
            last_charge_amount_micros: None,
            last_charge_error: None,
        }
    }

    #[test]
    fn two_thousand_signups_fold_the_log_once_per_stamp_change() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        for index in 0..2_000 {
            let outcome = store
                .subscribe(
                    10 + index as i64,
                    &format!("signup{index}.example"),
                    &format!("ops{index}@x.example"),
                    "",
                    false,
                )
                .expect("subscribe");
            assert!(outcome.created, "each signup is new");
        }
        assert_eq!(
            store.fold_count(),
            1,
            "2,000 signups must fold once, not once per insert"
        );
        assert_eq!(store.list().expect("list").len(), 2_000);
        assert_eq!(
            store.fold_count(),
            1,
            "an unchanged stamp serves the cached fold"
        );
    }

    #[test]
    fn fold_counts_stay_constant_across_onboarding_sizes() {
        for signups in [100usize, 500, 1_000, 2_000] {
            let dir = tempfile::tempdir().unwrap();
            let store = store(&dir);
            for index in 0..signups {
                store
                    .subscribe(
                        10 + index as i64,
                        &format!("bulk{index}.example"),
                        &format!("bulk{index}@x.example"),
                        "",
                        false,
                    )
                    .expect("subscribe");
            }
            println!("signups={signups} folds={}", store.fold_count());
            assert_eq!(store.fold_count(), 1, "signups={signups}");
        }
    }

    #[test]
    fn thirty_two_thousand_signups_do_constant_index_work() {
        for signups in [8_000usize, 16_000, 32_000] {
            let dir = tempfile::tempdir().unwrap();
            let store = store(&dir);
            let mut first_token = None;
            let started = std::time::Instant::now();
            for index in 0..signups {
                let outcome = store
                    .subscribe(
                        10 + index as i64,
                        &format!("seam{index}.example"),
                        &format!("seam{index}@x.example"),
                        "",
                        false,
                    )
                    .expect("subscribe");
                assert!(outcome.created, "each signup is new");
                if first_token.is_none() {
                    first_token = outcome.cancel_token;
                }
            }
            let elapsed = started.elapsed();
            let probes = store.row_probe_count();
            println!(
                "signups={signups} folds={} row_probes={probes} elapsed_ms={} probes_per_op={:.4}",
                store.fold_count(),
                elapsed.as_millis(),
                probes as f64 / signups as f64
            );
            assert_eq!(store.fold_count(), 1, "signups={signups}");
            assert!(
                probes <= signups as u64,
                "signups={signups}: indexed work must be linear, got {probes} row probes"
            );

            let before = store.row_probe_count();
            let repeat = store
                .subscribe(10, "seam0.example", "SEAM0@x.example", "", false)
                .expect("repeat");
            assert!(!repeat.created);
            assert_eq!(
                store.row_probe_count() - before,
                1,
                "dedup examines exactly its indexed position"
            );

            let before = store.row_probe_count();
            let canceled = store
                .cancel(first_token.as_deref().expect("token"), 20)
                .expect("cancel")
                .expect("match");
            assert_eq!(canceled.status, Status::Canceled);
            assert_eq!(
                store.row_probe_count() - before,
                2,
                "cancel examines its indexed position plus the in-place row"
            );
            assert_eq!(store.list().expect("list").len(), signups);
        }
    }

    #[test]
    fn randomized_five_thousand_operations_match_the_naive_reference() {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};

        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let path = store.subscribers_path();
        let mut rng = StdRng::seed_from_u64(0x0a9e_5f0a_d0c5_1017);
        let identities: Vec<(String, String)> = (0..32)
            .map(|index| {
                (
                    format!("fuzz{index}.example"),
                    format!("ops{index}@x.example"),
                )
            })
            .collect();
        let mut reference_rows: Vec<Subscriber> = Vec::new();
        let mut tokens: Vec<String> = Vec::new();
        let mut external_count = 0usize;
        let mut expected_folds = 0u64;
        let mut now = 1_000i64;
        let (mut created, mut duplicates, mut cancels) = (0usize, 0usize, 0usize);

        for step in 0..5_000 {
            now += rng.gen_range(1..20);
            if rng.gen_bool(0.003) {
                now += DEDUP_WINDOW_MS + rng.gen_range(1..1_000);
            }
            let op = rng.gen_range(0..100);
            if op < 55 {
                let (domain, email) = &identities[rng.gen_range(0..identities.len())];
                let submitted = if rng.gen_bool(0.5) {
                    email.to_ascii_uppercase()
                } else {
                    email.clone()
                };
                let reference = reference_fold(&reference_rows);
                let expected = reference
                    .iter()
                    .find(|row| {
                        row.domain == domain.as_str()
                            && row.email.eq_ignore_ascii_case(&submitted)
                            && now.saturating_sub(row.created_ms) <= DEDUP_WINDOW_MS
                    })
                    .cloned();
                let outcome = store
                    .subscribe(now, domain, &submitted, "", rng.gen_bool(0.2))
                    .expect("subscribe");
                if let Some(expected) = expected {
                    assert!(!outcome.created, "step {step}: duplicate must not append");
                    assert_eq!(outcome.subscriber.id, expected.id, "step {step}");
                    assert!(outcome.cancel_token.is_none(), "step {step}");
                    duplicates += 1;
                } else {
                    assert!(outcome.created, "step {step}: new signup");
                    let token = outcome.cancel_token.clone().expect("token");
                    assert_eq!(token.len(), 32);
                    assert_eq!(
                        outcome.subscriber.cancel_token_hash,
                        sha256_hex(&token),
                        "step {step}"
                    );
                    reference_rows.push(outcome.subscriber);
                    tokens.push(token);
                    created += 1;
                }
                if expected_folds == 0 {
                    expected_folds = 1;
                }
            } else if op < 80 {
                let token = if !tokens.is_empty() && rng.gen_bool(0.8) {
                    tokens[rng.gen_range(0..tokens.len())].clone()
                } else {
                    format!("not-a-token-{}", rng.gen_range(0..1_000))
                };
                let hash = sha256_hex(&token);
                let reference = reference_fold(&reference_rows);
                let expected = reference
                    .iter()
                    .find(|row| row.is_active() && constant_time_eq(&hash, &row.cancel_token_hash))
                    .cloned();
                let canceled = store.cancel(&token, now).expect("cancel");
                match expected {
                    Some(mut expected) => {
                        let got = canceled.expect("token must match");
                        expected.status = Status::Canceled;
                        expected.canceled_ms = Some(now);
                        assert_eq!(got, expected, "step {step}");
                        reference_rows.push(got);
                        cancels += 1;
                    }
                    None => assert!(canceled.is_none(), "step {step}"),
                }
                if expected_folds == 0 {
                    expected_folds = 1;
                }
            } else {
                external_count += 1;
                let token = format!("external-token-{external_count}");
                let mut external = row(
                    &format!("sub_external{external_count:05}"),
                    &format!("external{external_count}.example"),
                    &format!("external{external_count}@x.example"),
                    now,
                );
                external.cancel_token_hash = sha256_hex(&token);
                external_append(&path, &external);
                reference_rows.push(external);
                tokens.push(token);
                expected_folds += 1;
                assert_eq!(
                    store.list().expect("list"),
                    reference_fold(&reference_rows),
                    "step {step}: external row must fold in"
                );
                assert_eq!(
                    store.list().expect("list"),
                    naive_fold(&path),
                    "step {step}: folded state is the file"
                );
            }
            assert_eq!(
                store.list().expect("list"),
                reference_fold(&reference_rows),
                "step {step}: indexed fold diverged"
            );
            if step % 250 == 0 {
                assert_eq!(
                    naive_fold(&path),
                    reference_fold(&reference_rows),
                    "step {step}: reference rows are the file"
                );
            }
        }

        assert_eq!(naive_fold(&path), reference_fold(&reference_rows));
        assert_eq!(store.list().expect("list"), naive_fold(&path), "final fold");
        assert_eq!(
            store.fold_count(),
            expected_folds,
            "the cache must fold once per stamp change"
        );
        println!(
            "fuzz steps=5000 created={created} duplicates={duplicates} cancels={cancels} external_appends={external_count} folds={}",
            store.fold_count()
        );
    }

    #[test]
    fn a_cancel_after_an_external_append_matches_the_external_row() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        store
            .subscribe(10, "inside.example", "a@x.example", "", false)
            .expect("subscribe");
        assert_eq!(store.fold_count(), 1);

        let token = "external-cancel-token";
        let mut external = row("sub_external09", "outside.example", "ops@x.example", 20);
        external.cancel_token_hash = sha256_hex(token);
        external_append(&store.subscribers_path(), &external);

        let canceled = store.cancel(token, 30).expect("cancel").expect("match");
        assert_eq!(canceled.id, external.id);
        assert_eq!(canceled.status, Status::Canceled);
        assert_eq!(canceled.canceled_ms, Some(30));
        assert_eq!(
            store.fold_count(),
            2,
            "the changed stamp folds exactly once"
        );
        assert_eq!(
            store.list().expect("list"),
            naive_fold(&store.subscribers_path())
        );
        assert!(
            store.cancel(token, 40).expect("again").is_none(),
            "the canceled external row no longer matches"
        );
    }

    #[test]
    fn an_external_append_is_folded_before_the_next_signup() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        store
            .subscribe(10, "inside.example", "a@x.example", "", false)
            .expect("subscribe");
        assert_eq!(store.fold_count(), 1);

        let external = row("sub_external01", "external.example", "ext@x.example", 100);
        external_append(&store.subscribers_path(), &external);

        let outcome = store
            .subscribe(200, "external.example", "EXT@x.example", "", false)
            .expect("subscribe");
        assert!(!outcome.created, "the external row must be visible");
        assert_eq!(outcome.subscriber.id, external.id);
        assert_eq!(
            store.fold_count(),
            2,
            "exactly one refresh for the changed stamp"
        );
        assert_eq!(
            std::fs::read_to_string(store.subscribers_path())
                .unwrap()
                .lines()
                .count(),
            2,
            "the duplicate appends nothing"
        );
        assert_eq!(store.list().unwrap(), naive_fold(&store.subscribers_path()));
    }

    #[test]
    fn an_update_after_an_external_append_never_hides_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let alpha = store
            .subscribe(10, "alpha.example", "a@x.example", "", false)
            .expect("alpha");
        let external = row("sub_external03", "external.example", "ext@x.example", 20);
        external_append(&store.subscribers_path(), &external);

        let mut updated = alpha.subscriber.clone();
        updated.last_check_at = Some(30);
        store.update(&updated).expect("update");

        // The stale cache must be dropped instead of absorbing the update
        // row and hiding the external subscriber forever.
        let folded = store.list().expect("list");
        assert!(folded.iter().any(|row| row.id == external.id), "{folded:?}");
        assert_eq!(folded, naive_fold(&store.subscribers_path()));
        assert_eq!(
            store.fold_count(),
            2,
            "the stale cache folds once on the next read"
        );
    }

    #[test]
    fn folded_state_matches_a_verbatim_naive_fold_through_the_replay() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let path = store.subscribers_path();

        let alpha = store
            .subscribe(100, "alpha.example", "a@x.example", "", false)
            .expect("alpha");
        assert_eq!(store.list().unwrap(), naive_fold(&path));
        let beta = store
            .subscribe(110, "beta.example", "b@x.example", "", true)
            .expect("beta");
        assert_eq!(store.list().unwrap(), naive_fold(&path));

        // A duplicate submission folds to the original row and appends
        // nothing.
        let duplicate = store
            .subscribe(120, "alpha.example", "A@x.example", "", false)
            .expect("duplicate");
        assert!(!duplicate.created);
        assert_eq!(duplicate.subscriber.id, alpha.subscriber.id);
        assert_eq!(store.list().unwrap(), naive_fold(&path));

        // An update replaces the newest row for beta without moving its
        // first-seen position.
        let mut updated = beta.subscriber.clone();
        updated.last_check_at = Some(123);
        updated.last_findings_hash = Some("ab".repeat(32));
        store.update(&updated).expect("update");
        assert_eq!(store.list().unwrap(), naive_fold(&path));

        // An external append competes with the cached fold.
        let mut external = row("sub_external02", "gamma.example", "c@x.example", 130);
        external.last_alert_at = Some(140);
        external_append(&path, &external);
        assert_eq!(store.list().unwrap(), naive_fold(&path));

        // A cancel appends the newest row and folds as canceled.
        let token = alpha.cancel_token.clone().expect("token");
        let canceled = store.cancel(&token, 150).expect("cancel").expect("match");
        assert_eq!(canceled.status, Status::Canceled);
        assert_eq!(store.list().unwrap(), naive_fold(&path));
        assert_eq!(
            store.list_active().unwrap(),
            naive_fold(&path)
                .into_iter()
                .filter(Subscriber::is_active)
                .collect::<Vec<_>>()
        );

        // A signup after the external row keeps first-seen order.
        let delta = store
            .subscribe(160, "delta.example", "d@x.example", "", false)
            .expect("delta");
        assert!(delta.created);
        assert_eq!(store.list().unwrap(), naive_fold(&path));
    }

    fn pending_request(domain: &str, email: &str, apps: &[&str]) -> SubscribeRequest {
        SubscribeRequest {
            domain: domain.to_string(),
            email: email.to_string(),
            accounts: "admob.com = pub-1".to_string(),
            apps: apps.iter().map(|app| (*app).to_string()).collect(),
            verified_app_count: Some(u32::try_from(apps.len()).unwrap_or(0)),
            verification_error: None,
        }
    }

    #[test]
    fn pending_signup_records_no_check_and_derives_the_tier_from_named_apps() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let nine: Vec<String> = (0..9).map(|index| format!("app-{index}")).collect();
        let ten: Vec<String> = (0..10).map(|index| format!("app-{index}")).collect();
        let nine_refs: Vec<&str> = nine.iter().map(String::as_str).collect();
        let ten_refs: Vec<&str> = ten.iter().map(String::as_str).collect();
        let normal = store
            .subscribe_pending(
                10,
                &pending_request("normal.example", "a@x.example", &nine_refs),
            )
            .expect("pending");
        assert_eq!(normal.tier, crate::model::Tier::Normal);
        let core = store
            .subscribe_pending(
                11,
                &pending_request("core.example", "b@x.example", &ten_refs),
            )
            .expect("pending");
        assert_eq!(core.tier, crate::model::Tier::Core);
        // Account ids never affect the tier: the same nine-app signup lists
        // one account-id textarea row, and a ten-row account list with one
        // app still stays Normal.
        let one_app_many_accounts = store
            .subscribe_pending(
                12,
                &SubscribeRequest {
                    domain: "accounts.example".to_string(),
                    email: "c@x.example".to_string(),
                    accounts: (0..10)
                        .map(|index| format!("net{index}.example = pub-{index}\n"))
                        .collect(),
                    apps: vec!["OnlyApp".to_string()],
                    verified_app_count: Some(1),
                    verification_error: None,
                },
            )
            .expect("pending");
        assert_eq!(one_app_many_accounts.tier, crate::model::Tier::Normal);
        let rows = store.list().unwrap();
        assert_eq!(rows.len(), 3);
        for row in rows {
            assert_eq!(row.status, Status::Pending);
            assert!(!row.confirmed);
            assert_eq!(row.paid_at, None);
            assert_eq!(row.last_check_at, None);
            assert!(row.confirmation_token_hash.is_some());
            assert!(row.confirmation_expires_at.is_some());
            // Only the hash is stored, never the token.
            assert!(!row
                .confirmation_token_hash
                .as_deref()
                .unwrap()
                .contains(&normal.confirmation_token));
        }
        assert_eq!(normal.confirmation_token.len(), CONFIRMATION_TOKEN_HEX_LEN);
        assert!(is_confirmation_token(&normal.confirmation_token));
        assert!(!is_confirmation_token("NOT-A-TOKEN"));
        assert!(!is_confirmation_token("ABCDEF0123456789ABCDEF0123456789"));
    }

    #[test]
    fn activation_needs_the_matching_reference_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let pending = store
            .subscribe_pending(
                10,
                &pending_request("activate.example", "a@x.example", &["app"]),
            )
            .expect("pending");
        assert!(matches!(
            store.activate(11, "sub_nope"),
            Err(WatchError::UnknownSubscriber { .. })
        ));
        let activated = store.activate(12, &pending.reference).expect("activate");
        assert_eq!(activated.reference, pending.reference);
        assert_eq!(activated.paid_at, 12);
        assert!(!activated.already_active);
        let row = store.list().unwrap().remove(0);
        assert_eq!(row.status, Status::Active);
        assert_eq!(row.paid_at, Some(12));
        let rows_before = std::fs::read_to_string(store.subscribers_path())
            .unwrap()
            .lines()
            .count();

        // A replay is a typed no-op and appends no second row.
        let replay = store.activate(13, &pending.reference).expect("replay");
        assert!(replay.already_active);
        assert_eq!(replay.paid_at, 12);
        assert_eq!(
            std::fs::read_to_string(store.subscribers_path())
                .unwrap()
                .lines()
                .count(),
            rows_before,
            "a replay appends nothing"
        );
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn a_recent_pending_signup_is_reused_without_a_second_email() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let first = store
            .subscribe_pending(
                10_000,
                &pending_request("cooldown.example", "owner@cooldown.example", &["app"]),
            )
            .expect("first");
        assert!(first.send_email);
        assert_eq!(
            store
                .recent_pending("cooldown.example", "owner@cooldown.example", 20_000)
                .expect("recent"),
            Some(first.reference.clone())
        );
        let second = store
            .subscribe_pending(
                20_000,
                &pending_request("cooldown.example", "owner@cooldown.example", &["app"]),
            )
            .expect("second");
        assert_eq!(second.reference, first.reference);
        assert!(!second.send_email);
        assert!(second.confirmation_token.is_empty());
        let third = store
            .subscribe_pending(
                10_000 + 11 * 60_000,
                &pending_request("cooldown.example", "owner@cooldown.example", &["app"]),
            )
            .expect("third");
        assert!(third.send_email);
        assert!(!third.confirmation_token.is_empty());
        assert_eq!(
            store
                .recent_pending(
                    "cooldown.example",
                    "owner@cooldown.example",
                    10_000 + 11 * 60_000
                )
                .expect("expired"),
            None
        );
    }

    #[test]
    fn confirmation_is_single_use_and_expires() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let pending = store
            .subscribe_pending(
                10,
                &pending_request("confirm.example", "a@x.example", &["app"]),
            )
            .expect("pending");
        let confirmed = store
            .confirm_email(11, &pending.confirmation_token)
            .expect("confirm");
        assert_eq!(confirmed.reference, pending.reference);
        assert_eq!(confirmed.confirmed_at, 11);
        let row = store.list().unwrap().remove(0);
        assert!(row.confirmed);
        assert_eq!(row.email_confirmed_at, Some(11));
        assert_eq!(row.confirmation_used_at, Some(11));

        // The same token is refused typed and never confirms a second row.
        assert!(matches!(
            store.confirm_email(12, &pending.confirmation_token),
            Err(WatchError::ConfirmationUsed)
        ));
        assert!(matches!(
            store.confirm_email(12, "ffffffffffffffffffffffffffffffff"),
            Err(WatchError::ConfirmationUnknown)
        ));

        // Expiry is refused too, and a malformed token never looks up.
        let expiring = store
            .subscribe_pending(
                20,
                &pending_request("late.example", "b@x.example", &["app"]),
            )
            .expect("pending");
        assert!(matches!(
            store.confirm_email(20 + CONFIRMATION_TTL_MS, &expiring.confirmation_token),
            Err(WatchError::ConfirmationExpired)
        ));
        assert!(matches!(
            store.confirm_email(21, "short"),
            Err(WatchError::ConfirmationUnknown)
        ));
        assert!(!store
            .list()
            .unwrap()
            .iter()
            .any(|row| row.domain == "late.example" && row.confirmed));
    }

    fn buyer() -> BillingProfile {
        BillingProfile {
            legal_name: "Buyer OÜ".to_string(),
            address_lines: vec!["Telliskivi 1".to_string()],
            country: "ee".to_string(),
            vat_number: Some("EE111111111".to_string()),
        }
    }

    #[test]
    fn profile_tokens_are_single_outstanding_and_single_use() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let pending = store
            .subscribe_pending(
                10,
                &pending_request("profile.example", "a@x.example", &["app"]),
            )
            .expect("pending");

        let first = store
            .create_profile_token_at(20, &pending.reference)
            .expect("token");
        assert_eq!(first.len(), CONFIRMATION_TOKEN_HEX_LEN);
        assert!(is_confirmation_token(&first));
        let raw = std::fs::read_to_string(store.subscribers_path()).unwrap();
        assert!(raw.contains(&sha256_hex(&first)), "{raw}");
        assert!(!raw.contains(&first), "only the hash is stored");

        // Single-outstanding: the second token replaces the first.
        let second = store
            .create_profile_token_at(21, &pending.reference)
            .expect("second token");
        assert_ne!(first, second);
        assert!(matches!(
            store.consume_profile_token(22, &first),
            Err(WatchError::ProfileTokenUnknown)
        ));
        assert_eq!(
            store.consume_profile_token(22, &second).expect("consume"),
            pending.reference
        );
        assert!(matches!(
            store.consume_profile_token(23, &second),
            Err(WatchError::ProfileTokenUsed)
        ));

        // Expiry and malformed input are typed refusals.
        let expiring = store
            .create_profile_token_at(30, &pending.reference)
            .expect("expiring token");
        assert!(matches!(
            store.consume_profile_token(30 + CONFIRMATION_TTL_MS, &expiring),
            Err(WatchError::ProfileTokenExpired)
        ));
        assert!(matches!(
            store.consume_profile_token(31, "short"),
            Err(WatchError::ProfileTokenUnknown)
        ));
    }

    #[test]
    fn setting_a_profile_validates_and_is_idempotent_but_never_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let pending = store
            .subscribe_pending(
                10,
                &pending_request("buyer.example", "a@x.example", &["app"]),
            )
            .expect("pending");

        assert!(matches!(
            store.set_billing_profile("sub_nope", &buyer(), 11),
            Err(WatchError::UnknownSubscriber { .. })
        ));

        let stored = store
            .set_billing_profile(&pending.reference, &buyer(), 12)
            .expect("set");
        assert_eq!(
            stored.billing_profile.as_ref().expect("profile").country,
            "EE",
            "the stored country is canonical"
        );
        let rows_after_first = std::fs::read_to_string(store.subscribers_path())
            .unwrap()
            .lines()
            .count();

        // The same profile (modulo trimming and country case) is a replay:
        // it returns the stored row and appends nothing.
        let mut replay = buyer();
        replay.legal_name = "  Buyer OÜ  ".to_string();
        replay.country = " EE ".to_string();
        let replay = store
            .set_billing_profile(&pending.reference, &replay, 13)
            .expect("replay");
        assert_eq!(replay.billing_profile, stored.billing_profile);
        assert_eq!(
            std::fs::read_to_string(store.subscribers_path())
                .unwrap()
                .lines()
                .count(),
            rows_after_first,
            "an identical replay appends no row"
        );

        // A different profile after one is set is refused typed.
        let mut different = buyer();
        different.legal_name = "Someone Else OÜ".to_string();
        assert!(matches!(
            store.set_billing_profile(&pending.reference, &different, 14),
            Err(WatchError::BillingProfileLocked { .. })
        ));

        // An unusable profile is refused before any write.
        let mut broken = buyer();
        broken.legal_name = "  ".to_string();
        assert!(matches!(
            store.set_billing_profile(&pending.reference, &broken, 15),
            Err(WatchError::Billing(_))
        ));
    }

    #[test]
    fn canceled_subscribers_are_refused_a_profile_and_a_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let signup = store
            .subscribe(10, "gone.example", "a@x.example", "", false)
            .expect("subscribe");
        store
            .cancel(signup.cancel_token.as_deref().expect("token"), 11)
            .expect("cancel")
            .expect("match");
        assert!(matches!(
            store.create_profile_token(&signup.subscriber.id),
            Err(WatchError::NotBillable { .. })
        ));
        assert!(matches!(
            store.set_billing_profile(&signup.subscriber.id, &buyer(), 12),
            Err(WatchError::NotBillable { .. })
        ));
    }

    #[test]
    fn a_profile_token_authorises_recording_the_profile_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let pending = store
            .subscribe_pending(
                10,
                &pending_request("token.example", "a@x.example", &["app"]),
            )
            .expect("pending");
        let token = store
            .create_profile_token_at(11, &pending.reference)
            .expect("token");
        let stored = store
            .set_billing_profile_with_token(&token, &buyer(), 12)
            .expect("profile");
        assert_eq!(stored.id, pending.reference);
        assert!(stored.billing_profile.is_some());
        assert!(matches!(
            store.set_billing_profile_with_token(&token, &buyer(), 13),
            Err(WatchError::ProfileTokenUsed)
        ));
    }

    /// Write rows straight to the main log (the external-writer shape).
    fn write_rows(path: &Path, rows: &[Subscriber]) {
        let mut raw = String::new();
        for row in rows {
            raw.push_str(&serde_json::to_string(row).expect("serialize"));
            raw.push('\n');
        }
        std::fs::write(path, raw).expect("write rows");
    }

    #[cfg(unix)]
    #[test]
    fn the_checkpoint_and_delta_are_created_mode_0600() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        store
            .subscribe(10, "mode.example", "a@x.example", "", false)
            .expect("subscribe");
        store.checkpoint().expect("checkpoint");
        for path in [store.checkpoint_path(), store.delta_path()] {
            assert!(path.exists(), "{}", path.display());
            let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{}", path.display());
        }
    }

    #[test]
    fn checkpoint_and_delta_match_the_full_fold_over_randomized_histories() {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};

        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let path = store.subscribers_path();
        let mut rows = Vec::new();
        for index in 0..40 {
            let outcome = store
                .subscribe(
                    10 + index,
                    &format!("hist{index}.example"),
                    &format!("ops{index}@x.example"),
                    "",
                    false,
                )
                .expect("subscribe");
            rows.push(outcome.subscriber);
        }
        let mut rng = StdRng::seed_from_u64(0x5eed_c0de_1234);
        let mut now = 1_000i64;
        for step in 0..400 {
            now += rng.gen_range(1..100);
            let index = rng.gen_range(0..rows.len());
            rows[index].last_check_at = Some(now);
            rows[index].last_alert_at = Some(now + rng.gen_range(0..5));
            store.update(&rows[index]).expect("update");
            if step % 40 == 39 {
                store.checkpoint().expect("checkpoint");
            }
            assert_eq!(
                store.list().expect("list"),
                naive_fold(&path),
                "step {step}: checkpoint+delta diverged from the full log"
            );
        }
        // A fresh process reads the checkpoint and the small delta.
        let reopened = SubscriberStore::open(dir.path());
        assert_eq!(reopened.list().expect("reopen"), naive_fold(&path));
    }

    #[test]
    fn a_truncated_delta_recovers_by_folding_the_full_log() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let first = store
            .subscribe(10, "recover.example", "a@x.example", "", false)
            .expect("subscribe");
        store.checkpoint().expect("checkpoint");
        let mut updated = first.subscriber;
        updated.last_check_at = Some(20);
        store.update(&updated).expect("first delta row");
        updated.last_check_at = Some(30);
        store.update(&updated).expect("second delta row");
        assert_eq!(
            SubscriberStore::open(dir.path())
                .list()
                .expect("valid delta"),
            naive_fold(&store.subscribers_path())
        );

        // A crash while the delta is being written leaves it truncated. The
        // main log is still complete, so the reader must recover the same
        // fold rather than fail or lose a row.
        let delta = std::fs::read(store.delta_path()).expect("delta");
        assert!(delta.len() > 20);
        std::fs::write(store.delta_path(), &delta[..delta.len() - 20]).expect("truncate");
        let recovered = SubscriberStore::open(dir.path()).list().expect("recover");
        assert_eq!(recovered, naive_fold(&store.subscribers_path()));
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].last_check_at, Some(30));
    }

    #[test]
    fn duplicate_rows_keep_last_write_wins_through_the_checkpoint_and_delta() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let path = store.subscribers_path();
        let mut first = row("sub_dupe", "dupe.example", "a@x.example", 10);
        first.last_check_at = Some(1);
        let mut second = first.clone();
        second.last_check_at = Some(2);
        let mut third = first.clone();
        third.last_check_at = Some(3);
        write_rows(&path, &[first, second, third.clone()]);
        store.checkpoint().expect("checkpoint");
        assert_eq!(
            SubscriberStore::open(dir.path())
                .list()
                .expect("checkpoint fold"),
            vec![third.clone()]
        );

        // Two more versions arrive through the delta log: the newest wins.
        let mut fourth = third.clone();
        fourth.last_check_at = Some(4);
        let mut fifth = third.clone();
        fifth.last_check_at = Some(5);
        store
            .update_many(&[fourth, fifth.clone()])
            .expect("delta rows");
        assert_eq!(
            SubscriberStore::open(dir.path())
                .list()
                .expect("delta fold"),
            vec![fifth]
        );
    }

    /// 100,000 historical rows collapse to 1,000 checkpoint rows: a later
    /// fold must read the checkpoint plus the (empty) delta, not the whole
    /// append-only history.
    #[test]
    fn one_hundred_thousand_history_rows_fold_through_the_checkpoint() {
        const IDS: usize = 1_000;
        const VERSIONS: usize = 100;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SUBSCRIBERS_FILE);
        let mut raw = String::with_capacity(IDS * VERSIONS * 512);
        for version in 0..VERSIONS {
            for id in 0..IDS {
                let mut subscriber = row(
                    &format!("sub_{id:05}"),
                    &format!("history{id}.example"),
                    &format!("ops{id}@x.example"),
                    10,
                );
                subscriber.last_check_at = Some(version as i64);
                raw.push_str(&serde_json::to_string(&subscriber).expect("serialize"));
                raw.push('\n');
            }
        }
        std::fs::write(&path, raw).expect("history");

        // The before number: the same full fold the pre-checkpoint reader did
        // on every run.
        let full_started = std::time::Instant::now();
        let full = naive_fold(&path);
        let full_elapsed = full_started.elapsed();
        assert_eq!(full.len(), IDS);

        let store = SubscriberStore::open(dir.path());
        let checkpoint_started = std::time::Instant::now();
        store.checkpoint().expect("checkpoint");
        let checkpoint_elapsed = checkpoint_started.elapsed();
        assert_eq!(
            store.rows_read(),
            (IDS * VERSIONS) as u64,
            "the first fold reads the history once"
        );

        let reopened = SubscriberStore::open(dir.path());
        let fold_started = std::time::Instant::now();
        let folded = reopened.list().expect("checkpoint fold");
        let fold_elapsed = fold_started.elapsed();
        let rows_read = reopened.rows_read();
        println!(
            "history_rows={} checkpoint_rows={IDS} full_fold_ms={} checkpoint_ms={} checkpointed_fold_ms={} fold_rows_read={rows_read}",
            IDS * VERSIONS,
            full_elapsed.as_millis(),
            checkpoint_elapsed.as_millis(),
            fold_elapsed.as_millis()
        );
        assert_eq!(folded.len(), IDS);
        assert_eq!(folded[0].last_check_at, Some((VERSIONS - 1) as i64));
        assert!(
            rows_read <= (IDS + 8) as u64,
            "the fold must read the checkpoint rows, not the 100k history; read {rows_read}"
        );
        assert_eq!(reopened.fold_count(), 1);
    }
}
