//! Collection state at the webhook boundary.
//!
//! Verifying a delivery is not the same as recording it: an accepted webhook
//! that only logs marks nothing paid and protects against nothing. The store
//! persists every accepted Mollie payment, deduplicates on
//! `(payment id, status)` (Mollie resends until it sees a 2xx, and has no
//! signature; the fetch in `webhook` is the trust boundary), and answers
//! whether the event was new. The customer/mandate a first payment creates
//! travels on the event too, so the stored mandate can be charged later
//! ([`stored_mandate_for_publisher`]).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::errors::BillingError;
use crate::mollie::MolliePayment;

/// One recorded Mollie payment, flattened for storage and lookup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionEvent {
    pub at_unix_ms: i64,
    pub payment_id: String,
    pub reference: String,
    pub status: String,
    pub amount_minor_units: i64,
    pub amount_currency: String,
    /// The Mollie customer the payment belongs to, when reported.
    #[serde(default)]
    pub stored_customer_id: String,
    /// The Mollie mandate the payment created or refreshed, when reported.
    #[serde(default)]
    pub stored_mandate_id: String,
}

impl CollectionEvent {
    /// Flatten a fetched Mollie payment.
    pub fn from_payment(payment: &MolliePayment, at_unix_ms: i64) -> Self {
        Self {
            at_unix_ms,
            payment_id: payment.id.clone(),
            reference: payment.metadata_reference.clone().unwrap_or_default(),
            status: payment.status.clone(),
            amount_minor_units: payment.amount_minor_units,
            amount_currency: payment.currency.clone(),
            stored_customer_id: payment.customer_id.clone().unwrap_or_default(),
            stored_mandate_id: payment.mandate_id.clone().unwrap_or_default(),
        }
    }

    /// The replay key: a `(payment id, status)` pair Mollie will resend on
    /// retry until it sees a success. A status transition (open -> paid) is a
    /// new event; an identical retry is not.
    pub fn replay_key(&self) -> String {
        format!("{}:{}", self.payment_id, self.status)
    }

    /// The publisher segment of a deterministic reference
    /// `apex-fee:<publisher>:<experiment>:<period>`, when it has one.
    pub fn publisher_id(&self) -> Option<&str> {
        let mut segments = self.reference.split(':');
        if segments.next() != Some("apex-fee") {
            return None;
        }
        segments.next().filter(|publisher| !publisher.is_empty())
    }

    /// The encoded `customerId:mandateId` this event records, when both are
    /// present and non-empty.
    pub fn stored_mandate(&self) -> Option<String> {
        crate::mollie::StoredMandate::new(
            self.stored_customer_id.clone(),
            self.stored_mandate_id.clone(),
        )
        .map(|mandate| mandate.encode())
    }
}

/// The newest stored mandate the collection log records for `publisher`,
/// when any accepted event carried one. A missing log is "none yet"; a
/// malformed line is a typed refusal, never a silently skipped record.
pub fn stored_mandate_for_publisher(
    path: &Path,
    publisher: &str,
) -> Result<Option<String>, BillingError> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(BillingError::Malformed(format!(
                "collection log at {} could not be read: {error}",
                path.display()
            )))
        }
    };
    let mut newest: Option<(i64, String)> = None;
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event: CollectionEvent = serde_json::from_str(line).map_err(|error| {
            BillingError::Malformed(format!(
                "collection log at {} line {} is corrupt: {error}",
                path.display(),
                index + 1
            ))
        })?;
        if event.publisher_id() != Some(publisher) {
            continue;
        }
        let Some(mandate) = event.stored_mandate() else {
            continue;
        };
        if newest
            .as_ref()
            .is_none_or(|(at, _)| event.at_unix_ms >= *at)
        {
            newest = Some((event.at_unix_ms, mandate));
        }
    }
    Ok(newest.map(|(_, mandate)| mandate))
}

/// Where accepted collection events are recorded.
pub trait CollectionStore: Send + Sync + std::fmt::Debug {
    /// Record one accepted event. Returns `true` when the event is new and
    /// `false` when it is a replay of an already-recorded one.
    fn record(&self, event: &CollectionEvent) -> Result<bool, BillingError>;

    /// Whether a replay key has already been recorded.
    fn seen(&self, replay_key: &str) -> Result<bool, BillingError>;
}

/// Append-only JSONL store with an in-memory replay index. One line per
/// accepted event, so the collection history is inspectable beside the
/// ledger.
#[derive(Debug)]
pub struct FileCollectionStore {
    path: PathBuf,
    seen: Mutex<HashSet<String>>,
    /// Serializes the line append (not just the replay index). Two concurrent
    /// webhook deliveries used to write the JSON and the newline separately,
    /// which could concatenate two events on one line and corrupt the log.
    append: Mutex<()>,
}

impl FileCollectionStore {
    /// Open (or create) the store, loading the replay index from disk. Takes a
    /// concrete path so a dependent crate does not mint its own
    /// monomorphization of this constructor.
    pub fn open(path: &std::path::Path) -> Result<Self, BillingError> {
        let path = path.to_path_buf();
        let mut seen = HashSet::new();
        if let Ok(bytes) = std::fs::read(&path) {
            for line in String::from_utf8_lossy(&bytes).lines() {
                if line.trim().is_empty() {
                    continue;
                }
                let event: CollectionEvent = serde_json::from_str(line).map_err(|error| {
                    BillingError::Malformed(format!("collection log is corrupt: {error}"))
                })?;
                seen.insert(event.replay_key());
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(Self {
            path,
            seen: Mutex::new(seen),
            append: Mutex::new(()),
        })
    }

    fn append_line(&self, line: &str) -> Result<(), BillingError> {
        use std::io::Write;
        // One write of the whole record (including the newline), serialized
        // inside the process. O_APPEND keeps each write at EOF across
        // processes; a single write keeps a record from being interleaved.
        let _guard = self
            .append
            .lock()
            .map_err(|_| BillingError::Malformed("collection append lock poisoned".into()))?;
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&self.path)?;
        // The log carries provider references and stored payment identifiers:
        // never leave it readable by other local users, even when the file
        // pre-dates this writer.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        let mut record = String::with_capacity(line.len() + 1);
        record.push_str(line);
        record.push('\n');
        file.write_all(record.as_bytes())?;
        Ok(())
    }
}

impl CollectionStore for FileCollectionStore {
    fn record(&self, event: &CollectionEvent) -> Result<bool, BillingError> {
        let key = event.replay_key();
        let mut seen = self
            .seen
            .lock()
            .map_err(|_| BillingError::Malformed("collection index poisoned".into()))?;
        if !seen.insert(key.clone()) {
            return Ok(false);
        }
        drop(seen);
        let line = serde_json::to_string(event).map_err(|error| {
            BillingError::Malformed(format!("collection serialization: {error}"))
        })?;
        if let Err(error) = self.append_line(&line) {
            // The event was not persisted: release the dedup key so the
            // provider's retry can record it instead of being answered as a
            // replay of an event that never reached the log.
            if let Ok(mut seen) = self.seen.lock() {
                seen.remove(&key);
            }
            return Err(error);
        }
        Ok(true)
    }

    fn seen(&self, replay_key: &str) -> Result<bool, BillingError> {
        self.seen
            .lock()
            .map(|seen| seen.contains(replay_key))
            .map_err(|_| BillingError::Malformed("collection index poisoned".into()))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn event(payment_id: &str) -> CollectionEvent {
        CollectionEvent {
            at_unix_ms: 1,
            payment_id: payment_id.into(),
            reference: "apex-fee:pub:exp:2026-10".into(),
            status: "paid".into(),
            amount_minor_units: 1_500,
            amount_currency: "USD".into(),
            stored_customer_id: String::new(),
            stored_mandate_id: String::new(),
        }
    }

    #[test]
    fn records_new_events_and_refuses_replays() {
        let dir = std::env::temp_dir().join(format!("apex-coll-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = FileCollectionStore::open(&dir.join("coll.jsonl")).expect("open");
        assert!(store.record(&event("tr_psp-1")).unwrap(), "first is new");
        assert!(!store.record(&event("tr_psp-1")).unwrap(), "replay refused");
        assert!(store.record(&event("tr_psp-2")).unwrap());
        assert!(store.seen("tr_psp-1:paid").unwrap());
        assert!(!store.seen("tr_psp-9:paid").unwrap());

        // A reopened store keeps its replay index.
        let reopened = FileCollectionStore::open(&dir.join("coll.jsonl")).expect("reopen");
        assert!(!reopened.record(&event("tr_psp-1")).unwrap());
        let contents = std::fs::read_to_string(dir.join("coll.jsonl")).unwrap();
        assert_eq!(contents.lines().count(), 2, "one line per new event");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_appends_keep_one_valid_json_record_per_line() {
        use std::sync::Arc;
        let dir = std::env::temp_dir().join(format!("apex-coll-concurrent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("coll.jsonl");
        let store = Arc::new(FileCollectionStore::open(&path).expect("open"));
        let mut workers = Vec::new();
        for worker in 0..16 {
            let store = Arc::clone(&store);
            workers.push(std::thread::spawn(move || {
                for index in 0..25 {
                    store
                        .record(&event(&format!("psp-{worker}-{index}")))
                        .expect("record");
                }
            }));
        }
        for worker in workers {
            worker.join().expect("join");
        }
        let raw = std::fs::read_to_string(&path).expect("read");
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 16 * 25, "one line per event");
        for line in &lines {
            let parsed: CollectionEvent = serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("corrupt line {line:?}: {error}"));
            assert!(!parsed.payment_id.is_empty());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_append_releases_the_replay_key_for_the_providers_retry() {
        let dir = std::env::temp_dir().join(format!("apex-coll-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("coll.jsonl");
        let store = FileCollectionStore::open(&path).expect("open");
        // Make the log path an unwritable target (a directory) so the append
        // fails after the replay key was claimed.
        std::fs::create_dir_all(&path).expect("blocking directory");
        let error = store
            .record(&event("tr_psp-retry"))
            .expect_err("append fails");
        assert!(matches!(error, BillingError::Malformed(_)), "{error:?}");
        assert!(
            !store.seen("tr_psp-retry:paid").unwrap(),
            "the unpersisted event must not look like a replay"
        );

        // Remove the obstruction: the provider's retry is recorded exactly
        // once, and the next replay is a no-op.
        std::fs::remove_dir(&path).expect("remove blocking directory");
        assert!(
            store.record(&event("tr_psp-retry")).unwrap(),
            "retry records"
        );
        assert!(!store.record(&event("tr_psp-retry")).unwrap(), "replay");
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.lines().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
