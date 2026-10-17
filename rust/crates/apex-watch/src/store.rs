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
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::model::{
    publisher_slug_for_domain, tier_from_evidence, validate_watch_signup, ActivateOutcome,
    ConfirmOutcome, PendingOutcome, Status, SubscribeOutcome, SubscribeRequest, Subscriber,
    WatchFieldError, WatchSignup, CONFIRMATION_TOKEN_HEX_LEN, CONFIRMATION_TTL_MS,
};

/// Environment variable naming the watch root directory.
pub const WATCH_ROOT_ENV: &str = "APEX_WATCH_ROOT";
/// Default watch root, resolved against the working directory.
pub const DEFAULT_WATCH_ROOT: &str = ".apex/watch";
/// The subscriber file name under the root.
pub const SUBSCRIBERS_FILE: &str = "subscribers.jsonl";
/// The repeat-submission window: a submission with the same domain and email
/// inside this window is the same subscription.
pub const DEDUP_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

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
    #[cfg(test)]
    row_probes: AtomicU64,
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
}

impl Folded {
    /// Build the indexes for a freshly folded file.
    fn new(stamp: Option<FileStamp>, subscribers: Vec<Subscriber>) -> Self {
        let mut by_id = HashMap::with_capacity(subscribers.len());
        let mut by_dedup: HashMap<DedupKey, Vec<usize>> = HashMap::new();
        let mut by_cancel_token: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_confirmation_token: HashMap<String, Vec<usize>> = HashMap::new();
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
        }
        Self {
            stamp,
            subscribers,
            by_id,
            by_dedup,
            by_cancel_token,
            by_confirmation_token,
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
            if subscriber.is_active() && constant_time_eq(hash, &subscriber.cancel_token_hash) {
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
            #[cfg(test)]
            row_probes: AtomicU64::new(0),
        }
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
        let subscribers = self.read_fold(&path)?;
        self.folds.fetch_add(1, Ordering::Relaxed);
        *cached = Some(Folded::new(stamp, subscribers));
        Ok(())
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
            if !by_id.contains_key(&subscriber.id) {
                order.push(subscriber.id.clone());
            }
            by_id.insert(subscriber.id.clone(), subscriber);
        }
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

    /// Count the rows the indexed lookups had to examine. The scaling seam
    /// asserts this stays linear in the number of operations (one indexed
    /// position per lookup) instead of quadratic.
    #[inline]
    fn observe_row_probes(&self, _probes: u64) {
        #[cfg(test)]
        self.row_probes.fetch_add(_probes, Ordering::Relaxed);
    }

    /// The rows examined by indexed lookups so far.
    #[cfg(test)]
    fn row_probe_count(&self) -> u64 {
        self.row_probes.load(Ordering::Relaxed)
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
            id: format!("sub_{}", random_hex(5)),
            publisher: publisher_slug_for_domain(&signup.domain),
            domain: signup.domain,
            email: signup.email,
            tier,
            account_ids: signup.account_ids,
            created_ms: now_ms,
            status: Status::Active,
            canceled_ms: None,
            confirmed: true,
            confirmation_token_hash: None,
            confirmation_expires_at: None,
            confirmation_used_at: None,
            email_confirmed_at: Some(now_ms),
            paid_at: Some(now_ms),
            payment_link: None,
            shopper_reference: None,
            stored_payment_method_id: None,
            last_check_at: None,
            last_findings_hash: None,
            last_findings: None,
            last_alert_at: None,
            cancel_token_hash: sha256_hex(&cancel_token),
            billing: None,
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

    /// Create one pending sign-up: no check runs until the payment is
    /// accepted and the creating email is confirmed. The tier is derived
    /// from `request.app_count` (never a checkbox), the subscriber id is the
    /// durable reference, and the plaintext one-time confirmation token is
    /// returned exactly once.
    ///
    /// A repeat submission for the same `(domain, email)` inside
    /// [`DEDUP_WINDOW_MS`] is the same subscription: it keeps the reference
    /// and receives a freshly minted confirmation token instead of a second
    /// row.
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
        let tier = tier_from_evidence(request.app_count);
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
            existing.tier = tier;
            existing.confirmation_token_hash = Some(token_hash);
            existing.confirmation_expires_at = Some(expires_at);
            existing.confirmation_used_at = None;
            existing.shopper_reference = Some(existing.id.clone());
            self.append(&mut cached, &existing)?;
            return Ok(PendingOutcome {
                reference: existing.id,
                tier,
                confirmation_token: token,
            });
        }
        let id = format!("sub_{}", random_hex(5));
        let subscriber = Subscriber {
            id: id.clone(),
            publisher: publisher_slug_for_domain(&signup.domain),
            domain: signup.domain,
            email: signup.email,
            tier,
            account_ids: signup.account_ids,
            created_ms: now_ms,
            status: Status::Pending,
            canceled_ms: None,
            confirmed: false,
            confirmation_token_hash: Some(token_hash),
            confirmation_expires_at: Some(expires_at),
            confirmation_used_at: None,
            email_confirmed_at: None,
            paid_at: None,
            payment_link: None,
            shopper_reference: Some(id.clone()),
            stored_payment_method_id: None,
            last_check_at: None,
            last_findings_hash: None,
            last_findings: None,
            last_alert_at: None,
            cancel_token_hash: sha256_hex(&random_hex(16)),
            billing: None,
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

    /// Switch the matching pending subscriber on: the payment was accepted.
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

    /// Record the Adyen payment link created for a pending sign-up. The
    /// subscriber id is recorded as the `shopperReference`, the same identity
    /// the webhook settles against.
    pub fn record_payment_link(
        &self,
        reference: &str,
        payment_link: &str,
    ) -> Result<(), WatchError> {
        self.update_by_reference(reference, |subscriber| {
            subscriber.payment_link = Some(payment_link.to_string());
            subscriber.shopper_reference = Some(subscriber.id.clone());
        })
    }

    /// Record the stored card (Adyen stored payment method id) the first
    /// payment produced, so the monthly bill can charge it. The activation
    /// itself stays [`Self::activate`].
    pub fn record_stored_payment_method(
        &self,
        reference: &str,
        stored_payment_method_id: &str,
    ) -> Result<(), WatchError> {
        self.update_by_reference(reference, |subscriber| {
            subscriber.stored_payment_method_id = Some(stored_payment_method_id.to_string());
        })
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

    /// Cancel with a presented plaintext token: the first active subscriber
    /// whose stored hash matches is cancelled (a new row records the
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
        let _guard = self.lock()?;
        let mut cached = self.fold_lock()?;
        self.append(&mut cached, subscriber)
    }

    /// Append one row and, after the write succeeded, advance the cached
    /// fold in place. The caller holds the write lock. When the file moved
    /// beyond our own bytes (a stale cache, a concurrent external append),
    /// the cache is dropped so the next use re-folds exactly once.
    fn append(
        &self,
        cached: &mut Option<Folded>,
        subscriber: &Subscriber,
    ) -> Result<(), WatchError> {
        let path = self.subscribers_path();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| WatchError::Io {
                    detail: format!("{}: parent directory: {error}", path.display()),
                })?;
            }
        }
        let mut line = serde_json::to_string(subscriber).map_err(|error| WatchError::Io {
            detail: format!("subscriber serialization: {error}"),
        })?;
        line.push('\n');
        // A stat before the write keeps a stale cache from absorbing the
        // new row: only a cache whose stamp still matches the file may be
        // advanced in place.
        let observed = Self::inspect(&path).unwrap_or_default();
        let cache_current = cached.as_ref().is_some_and(|state| state.stamp == observed);
        let written = line.len() as u64;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| WatchError::Io {
                detail: format!("{} could not be opened: {error}", path.display()),
            })?;
        file.write_all(line.as_bytes())
            .map_err(|error| WatchError::Io {
                detail: format!("{} could not be appended to: {error}", path.display()),
            })?;
        drop(file);
        match Self::inspect(&path) {
            Ok(Some(stamp))
                if cache_current && appended_alone(observed.as_ref(), &stamp, written) =>
            {
                if let Some(state) = cached.as_mut() {
                    let probes = state.apply(subscriber);
                    self.observe_row_probes(probes);
                    state.stamp = Some(stamp);
                }
            }
            _ => *cached = None,
        }
        Ok(())
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
            tier: crate::model::Tier::Normal,
            account_ids: std::collections::BTreeMap::new(),
            created_ms,
            status: Status::Active,
            canceled_ms: None,
            confirmed: true,
            confirmation_token_hash: None,
            confirmation_expires_at: None,
            confirmation_used_at: None,
            email_confirmed_at: Some(created_ms),
            paid_at: Some(created_ms),
            payment_link: None,
            shopper_reference: None,
            stored_payment_method_id: None,
            last_check_at: None,
            last_findings_hash: None,
            last_findings: None,
            last_alert_at: None,
            cancel_token_hash: sha256_hex(id),
            billing: None,
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

    fn pending_request(domain: &str, email: &str, app_count: u32) -> SubscribeRequest {
        SubscribeRequest {
            domain: domain.to_string(),
            email: email.to_string(),
            accounts: "admob.com = pub-1".to_string(),
            app_count,
        }
    }

    #[test]
    fn pending_signup_records_no_check_and_derives_the_tier_from_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let normal = store
            .subscribe_pending(10, &pending_request("normal.example", "a@x.example", 9))
            .expect("pending");
        assert_eq!(normal.tier, crate::model::Tier::Normal);
        let core = store
            .subscribe_pending(11, &pending_request("core.example", "b@x.example", 10))
            .expect("pending");
        assert_eq!(core.tier, crate::model::Tier::Core);
        let rows = store.list().unwrap();
        assert_eq!(rows.len(), 2);
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
            .subscribe_pending(10, &pending_request("activate.example", "a@x.example", 1))
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
    fn confirmation_is_single_use_and_expires() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let pending = store
            .subscribe_pending(10, &pending_request("confirm.example", "a@x.example", 1))
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
            .subscribe_pending(20, &pending_request("late.example", "b@x.example", 1))
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
}
