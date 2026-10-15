//! Collection state at the webhook boundary.
//!
//! Verifying a delivery is not the same as recording it: an accepted webhook
//! that only logs marks nothing paid and protects against nothing. The store
//! persists every accepted notification item, deduplicates on
//! `eventCode` + `pspReference` (Adyen signs no timestamp, so replay
//! protection lives here), and answers whether the event was new.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::adyen::NotificationItem;
use crate::errors::BillingError;

/// One recorded notification item, flattened for storage and lookup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionEvent {
    pub at_unix_ms: i64,
    pub event_code: String,
    pub psp_reference: String,
    pub merchant_reference: String,
    /// `"true"` / `"false"` as Adyen sends it.
    pub success: String,
    pub amount_value: i64,
    pub amount_currency: String,
}

impl CollectionEvent {
    /// Flatten a verified notification item.
    pub fn from_item(item: &NotificationItem, at_unix_ms: i64) -> Self {
        Self {
            at_unix_ms,
            event_code: item.event_code.clone(),
            psp_reference: item.psp_reference.clone(),
            merchant_reference: item.merchant_reference.clone(),
            success: item.success.clone(),
            amount_value: item.amount_value,
            amount_currency: item.amount_currency.clone(),
        }
    }

    /// The replay key: an `(eventCode, pspReference)` pair Adyen will resend
    /// on retry until it sees a success.
    pub fn replay_key(&self) -> String {
        format!("{}:{}", self.event_code, self.psp_reference)
    }
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
        })
    }

    fn append_line(&self, line: &str) -> Result<(), BillingError> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
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
        if !seen.insert(key) {
            return Ok(false);
        }
        drop(seen);
        let line = serde_json::to_string(event).map_err(|error| {
            BillingError::Malformed(format!("collection serialization: {error}"))
        })?;
        self.append_line(&line)?;
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

    fn event(psp: &str) -> CollectionEvent {
        CollectionEvent {
            at_unix_ms: 1,
            event_code: "AUTHORISATION".into(),
            psp_reference: psp.into(),
            merchant_reference: "apex-fee:pub:exp:2026-10".into(),
            success: "true".into(),
            amount_value: 1_500,
            amount_currency: "USD".into(),
        }
    }

    #[test]
    fn records_new_events_and_refuses_replays() {
        let dir = std::env::temp_dir().join(format!("apex-coll-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = FileCollectionStore::open(&dir.join("coll.jsonl")).expect("open");
        assert!(store.record(&event("psp-1")).unwrap(), "first is new");
        assert!(!store.record(&event("psp-1")).unwrap(), "replay refused");
        assert!(store.record(&event("psp-2")).unwrap());
        assert!(store.seen("AUTHORISATION:psp-1").unwrap());
        assert!(!store.seen("AUTHORISATION:psp-9").unwrap());

        // A reopened store keeps its replay index.
        let reopened = FileCollectionStore::open(&dir.join("coll.jsonl")).expect("reopen");
        assert!(!reopened.record(&event("psp-1")).unwrap());
        let contents = std::fs::read_to_string(dir.join("coll.jsonl")).unwrap();
        assert_eq!(contents.lines().count(), 2, "one line per new event");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
