//! The durable, per-publisher deduplicated MAX S2S postback store.
//!
//! One SQLite database — `postbacks.sqlite` inside the spool directory
//! (`APEX_POSTBACK_SPOOL_DIR`; [`postback_store_path`]) — replaces the JSONL
//! spool as the source of truth for impression-level postbacks: one row per
//! `(publisher, event_id)`, inserted with `INSERT OR IGNORE` inside an
//! immediate transaction that is **committed before the caller answers**, so
//! deduplication is durable and per-publisher instead of a bounded
//! process-local set.
//!
//! [`PostbackStore::labels`] is the experiment-scoped read the MAX adapter
//! (`live_max`) uses: it loads labelled arm assignments only for one
//! `(publisher, experiment_instance, ad_unit)` scope.
//!
//! - A row stamped with another experiment instance or another ad unit is
//!   never read, so a label cannot leak between two runs of one ad unit or
//!   between two ad units.
//! - The receiver stamps an arriving row with the experiment instance whose
//!   epoch contained its receive time ([`PostbackStore::active_instance`]),
//!   so AppLovin never needs to know the pre-registered id and a historical
//!   assignment still cannot label a later experiment on the same ad unit.
//!   A row that arrives outside every epoch is not a label for any run.
//! - Two rows that assign one user differently under the same instance are a
//!   typed [`PostbackStoreError::Conflict`]: a later row never overwrites an
//!   earlier assignment, the read fails instead.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, TransactionBehavior};

/// The store's file name inside the spool directory.
pub const POSTBACK_DB_FILE: &str = "postbacks.sqlite";

/// The durable store path: `<spool dir>/postbacks.sqlite`.
pub fn postback_store_path(dir: &Path) -> PathBuf {
    dir.join(POSTBACK_DB_FILE)
}

/// The experiment epochs: the assignment window each pre-registered instance
/// owned on one ad unit. A postback is stamped with the instance whose epoch
/// contained its receive time, so a historical assignment can never label a
/// later experiment on the same ad unit. The window is half-open
/// `[started_at_ms, ended_at_ms)`; an open epoch ends when a later instance
/// starts on the same scope.
const EPOCH_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS experiment_epochs(
    publisher TEXT NOT NULL,
    ad_unit TEXT NOT NULL,
    experiment_instance TEXT NOT NULL,
    started_at_ms INTEGER NOT NULL,
    ended_at_ms INTEGER,
    PRIMARY KEY(publisher, ad_unit, experiment_instance)
)";

/// The events table: one row per `(publisher, event_id)`, the durable dedupe
/// key. The instance and ad unit columns scope which experiment may read a
/// row's label.
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS events(
    publisher TEXT NOT NULL,
    event_id TEXT NOT NULL,
    experiment_instance TEXT,
    user_id TEXT,
    ad_unit TEXT,
    ad_unit_test TEXT,
    revenue_micros INTEGER NOT NULL,
    received_at_ms INTEGER NOT NULL,
    PRIMARY KEY(publisher, event_id)
)";

/// Why the store refused a write or a label read.
#[derive(Debug, thiserror::Error)]
pub enum PostbackStoreError {
    /// The spool directory or database file could not be read or written.
    #[error("postback store io: {0}")]
    Io(#[from] std::io::Error),
    /// SQLite refused the statement (schema, lock, corrupt file, …).
    #[error("postback store sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// One user carries two different arm assignments under one experiment
    /// instance: the read fails typed rather than picking a winner.
    #[error(
        "conflicting postback labels for user {user_id} under one experiment instance: {first} vs {second}"
    )]
    Conflict {
        user_id: String,
        first: String,
        second: String,
    },
}

/// One impression-level postback as the receiver spools it. The receiver
/// cannot know the pre-registered experiment instance, so
/// `experiment_instance` is `None` for its rows.
#[derive(Debug, Clone)]
pub struct PostbackEvent<'a> {
    /// The publisher the event key authenticated.
    pub publisher: &'a str,
    /// The impression's event id (the per-publisher dedupe key).
    pub event_id: &'a str,
    /// The pre-registered experiment instance, when the writer knows it.
    pub experiment_instance: Option<&'a str>,
    /// The user the impression belongs to.
    pub user_id: &'a str,
    /// The ad unit the impression was served on.
    pub ad_unit: &'a str,
    /// The `max_ad_unit_test` label, when the impression carries one.
    pub ad_unit_test: Option<&'a str>,
    /// Revenue in micros.
    pub revenue_micros: i64,
    /// When the receiver accepted the impression (Unix ms).
    pub received_at_ms: i64,
}

/// One stored row as [`PostbackStore::stored_events`] returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPostback {
    /// The publisher the event key authenticated.
    pub publisher: String,
    /// The impression's event id.
    pub event_id: String,
    /// The pre-registered experiment instance, when the writer knew it.
    pub experiment_instance: Option<String>,
    /// The user the impression belongs to.
    pub user_id: Option<String>,
    /// The ad unit the impression was served on.
    pub ad_unit: Option<String>,
    /// The `max_ad_unit_test` label, when the impression carried one.
    pub ad_unit_test: Option<String>,
    /// Revenue in micros.
    pub revenue_micros: i64,
    /// When the receiver accepted the impression (Unix ms).
    pub received_at_ms: i64,
}

/// The labelled arm assignments one experiment scope carries: `user id ->
/// is test arm`, plus every distinct test-arm label the scope named.
#[derive(Debug, Default, Clone)]
pub struct AssignedLabels {
    /// `user id -> is test arm`.
    pub arms: BTreeMap<String, bool>,
    /// Every distinct test-arm label the scope named.
    pub test_labels: BTreeSet<String>,
}

/// One user's assignment under one experiment instance: the control arm is
/// no label (or the literal `Control`), the test arm is exactly one other
/// label. Two rows for one user are the same assignment only when both
/// fields match.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Assignment {
    is_test: bool,
    label: Option<String>,
}

impl Assignment {
    /// The arm one postback row labels: empty and `Control` spellings are
    /// the control arm, any other label is the test arm.
    fn of(test_name: Option<&str>) -> Self {
        match test_name.map(str::trim).filter(|name| !name.is_empty()) {
            None => Self {
                is_test: false,
                label: None,
            },
            Some(name) if name.eq_ignore_ascii_case("control") => Self {
                is_test: false,
                label: None,
            },
            Some(name) => Self {
                is_test: true,
                label: Some(name.to_string()),
            },
        }
    }

    /// The assignment as a conflict message names it.
    fn describe(&self) -> String {
        self.label.clone().unwrap_or_else(|| "Control".to_string())
    }
}

/// The durable spool: one SQLite database per spool directory.
#[derive(Debug)]
pub struct PostbackStore {
    connection: Connection,
}

impl PostbackStore {
    /// Open the store inside `dir`, creating the directory and the database
    /// when they do not exist yet, and ensure the schema.
    pub fn open(dir: &Path) -> Result<Self, PostbackStoreError> {
        std::fs::create_dir_all(dir)?;
        Self::open_at(&postback_store_path(dir))
    }

    /// Open one store file, creating it when missing.
    pub fn open_at(path: &Path) -> Result<Self, PostbackStoreError> {
        let connection = Connection::open(path)?;
        // Durability first: the receiver answers only after the commit, and a
        // concurrent delivery must wait for this one's write lock.
        connection.execute_batch(&format!(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA busy_timeout = 5000;
             {SCHEMA};
             {EPOCH_SCHEMA};"
        ))?;
        Ok(Self { connection })
    }

    /// Open an existing store for a read. `Ok(None)` — never a created file —
    /// when no store has been written yet: a missing spool simply means no
    /// labelled rows.
    pub fn open_existing(dir: &Path) -> Result<Option<Self>, PostbackStoreError> {
        let path = postback_store_path(dir);
        if path.exists() {
            Self::open_at(&path).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Record one impression durably: `INSERT OR IGNORE` on `(publisher,
    /// event_id)` inside an immediate transaction, **committed before
    /// success**. Returns `true` for a newly stored event and `false` when
    /// this `(publisher, event_id)` pair is already durable — the duplicate
    /// is answered without a second row.
    pub fn record(&mut self, event: &PostbackEvent<'_>) -> Result<bool, PostbackStoreError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO events (
                publisher, event_id, experiment_instance, user_id, ad_unit,
                ad_unit_test, revenue_micros, received_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                event.publisher,
                event.event_id,
                event.experiment_instance,
                event.user_id,
                event.ad_unit,
                event.ad_unit_test,
                event.revenue_micros,
                event.received_at_ms,
            ],
        )?;
        transaction.commit()?;
        Ok(inserted > 0)
    }

    /// Every stored row in acceptance order (diagnostics and tests).
    pub fn stored_events(&self) -> Result<Vec<StoredPostback>, PostbackStoreError> {
        let mut statement = self.connection.prepare(
            "SELECT publisher, event_id, experiment_instance, user_id, ad_unit,
                    ad_unit_test, revenue_micros, received_at_ms
             FROM events ORDER BY received_at_ms, rowid",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(StoredPostback {
                publisher: row.get(0)?,
                event_id: row.get(1)?,
                experiment_instance: row.get(2)?,
                user_id: row.get(3)?,
                ad_unit: row.get(4)?,
                ad_unit_test: row.get(5)?,
                revenue_micros: row.get(6)?,
                received_at_ms: row.get(7)?,
            })
        })?;
        let mut stored = Vec::new();
        for row in rows {
            stored.push(row?);
        }
        Ok(stored)
    }

    /// Record the start of one experiment's assignment epoch on an ad unit.
    /// Re-recording an instance keeps the earliest start: an epoch never
    /// grows backwards. The epoch stays open until a later instance starts on
    /// the same scope, which makes the windows cover the timeline without
    /// gaps.
    pub fn record_epoch(
        &self,
        publisher: &str,
        ad_unit: &str,
        experiment_instance: &str,
        started_at_ms: i64,
    ) -> Result<(), PostbackStoreError> {
        self.connection.execute(
            "INSERT INTO experiment_epochs(publisher, ad_unit, experiment_instance, started_at_ms)
             VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(publisher, ad_unit, experiment_instance)
             DO UPDATE SET started_at_ms = MIN(started_at_ms, excluded.started_at_ms)",
            params![publisher, ad_unit, experiment_instance, started_at_ms],
        )?;
        Ok(())
    }

    /// Close one experiment's assignment epoch: a postback received at or
    /// after `ended_at_ms` names no experiment unless a later epoch covers
    /// it. Closing twice keeps the earliest end, like an epoch start never
    /// grows backwards.
    pub fn close_epoch(
        &self,
        publisher: &str,
        ad_unit: &str,
        experiment_instance: &str,
        ended_at_ms: i64,
    ) -> Result<(), PostbackStoreError> {
        self.connection.execute(
            "UPDATE experiment_epochs
             SET ended_at_ms = MIN(COALESCE(ended_at_ms, ?4), ?4)
             WHERE publisher = ?1 AND ad_unit = ?2 AND experiment_instance = ?3",
            params![publisher, ad_unit, experiment_instance, ended_at_ms],
        )?;
        Ok(())
    }

    /// The experiment instance whose assignment epoch contained `at_ms` on
    /// this publisher and ad unit. `None` when no epoch covers the instant:
    /// the postback belongs to no experiment and is never read as a label.
    pub fn active_instance(
        &self,
        publisher: &str,
        ad_unit: &str,
        at_ms: i64,
    ) -> Result<Option<String>, PostbackStoreError> {
        // The greatest start at or before the instant is the only candidate:
        // a later epoch supersedes an earlier open one, so a closed epoch
        // leaves a gap instead of letting an older epoch resume.
        let mut statement = self.connection.prepare(
            "SELECT experiment_instance, ended_at_ms FROM experiment_epochs
             WHERE publisher = ?1
               AND ad_unit = ?2
               AND started_at_ms <= ?3
             ORDER BY started_at_ms DESC
             LIMIT 1",
        )?;
        let mut rows = statement.query(params![publisher, ad_unit, at_ms])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let instance: String = row.get(0)?;
        let ended_at_ms: Option<i64> = row.get(1)?;
        if ended_at_ms.is_some_and(|end| at_ms >= end) {
            return Ok(None);
        }
        Ok(Some(instance))
    }

    /// The labelled arm assignments for exactly one experiment scope:
    /// `(publisher, experiment_instance, ad_unit)`. `publisher` is the
    /// adapter's own publisher when it is known (`None` reads every
    /// publisher, the scope is still instance- and ad-unit-bound). Rows of
    /// another instance or ad unit are ignored; two rows assigning one user
    /// differently are a typed [`PostbackStoreError::Conflict`] — the read
    /// never lets a later row overwrite an earlier assignment.
    pub fn labels(
        &self,
        publisher: Option<&str>,
        experiment_instance: &str,
        ad_unit: &str,
    ) -> Result<AssignedLabels, PostbackStoreError> {
        let mut statement = self.connection.prepare(
            "SELECT user_id, ad_unit_test FROM events
             WHERE ad_unit = ?1
               AND experiment_instance = ?2
               AND (?3 IS NULL OR publisher = ?3)
             ORDER BY received_at_ms, rowid",
        )?;
        let rows =
            statement.query_map(params![ad_unit, experiment_instance, publisher], |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            })?;
        let mut assigned: BTreeMap<String, Assignment> = BTreeMap::new();
        for row in rows {
            let (user_id, test_name) = row?;
            let Some(user_id) = user_id
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
            else {
                continue;
            };
            let assignment = Assignment::of(test_name.as_deref());
            if let Some(earlier) = assigned.get(&user_id) {
                if *earlier != assignment {
                    return Err(PostbackStoreError::Conflict {
                        user_id,
                        first: earlier.describe(),
                        second: assignment.describe(),
                    });
                }
                continue;
            }
            assigned.insert(user_id, assignment);
        }
        let mut arms = BTreeMap::new();
        let mut test_labels = BTreeSet::new();
        for (user_id, assignment) in assigned {
            if let Some(label) = assignment.label {
                test_labels.insert(label);
            }
            arms.insert(user_id, assignment.is_test);
        }
        Ok(AssignedLabels { arms, test_labels })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// A fresh spool directory per test, named so a crashed run never
    /// collides with the next one.
    fn temp_spool(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "apex-postback-store-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("spool dir");
        dir
    }

    fn event<'a>(
        publisher: &'a str,
        event_id: &'a str,
        experiment_instance: Option<&'a str>,
        user_id: &'a str,
        ad_unit: &'a str,
        ad_unit_test: Option<&'a str>,
    ) -> PostbackEvent<'a> {
        PostbackEvent {
            publisher,
            event_id,
            experiment_instance,
            user_id,
            ad_unit,
            ad_unit_test,
            revenue_micros: 250_000,
            received_at_ms: 1_792_055_400_000,
        }
    }

    #[test]
    fn durable_dedupe_survives_reopening_the_store() {
        let dir = temp_spool("reopen");
        {
            let mut store = PostbackStore::open(&dir).expect("open");
            assert!(
                store
                    .record(&event("northstar", "ev-1", None, "u1", "au1", None))
                    .expect("record"),
                "the first delivery is new"
            );
            assert!(
                !store
                    .record(&event("northstar", "ev-1", None, "u1", "au1", None))
                    .expect("record"),
                "the replay in the same process is a duplicate"
            );
        }
        {
            // A second open is a fresh process: the dedupe must come from
            // the committed row, not from any in-process state.
            let mut store = PostbackStore::open(&dir).expect("reopen");
            assert!(
                !store
                    .record(&event("northstar", "ev-1", None, "u1", "au1", None))
                    .expect("record"),
                "the durable dedupe survives reopening"
            );
            assert_eq!(store.stored_events().expect("events").len(), 1);
        }
        std::fs::remove_dir_all(&dir).expect("clean");
    }

    #[test]
    fn the_same_event_id_from_two_publishers_is_stored_twice() {
        let dir = temp_spool("publishers");
        let mut store = PostbackStore::open(&dir).expect("open");
        assert!(store
            .record(&event(
                "northstar",
                "ev-1",
                Some("i1"),
                "northstar-user",
                "au1",
                Some("holdout-a")
            ))
            .expect("record"));
        assert!(
            store
                .record(&event(
                    "second",
                    "ev-1",
                    Some("i1"),
                    "second-user",
                    "au1",
                    None
                ))
                .expect("record"),
            "the dedupe key is per publisher, so this is a new row"
        );
        assert_eq!(store.stored_events().expect("events").len(), 2);

        // A publisher-scoped read keeps the two apart, too.
        let northstar = store
            .labels(Some("northstar"), "i1", "au1")
            .expect("labels");
        assert_eq!(northstar.arms.len(), 1);
        assert_eq!(northstar.arms.get("northstar-user"), Some(&true));
        let second = store.labels(Some("second"), "i1", "au1").expect("labels");
        assert_eq!(second.arms.len(), 1);
        assert_eq!(second.arms.get("second-user"), Some(&false));
        // Without a publisher the read still sees both rows.
        let all = store.labels(None, "i1", "au1").expect("labels");
        assert_eq!(all.arms.len(), 2);
        std::fs::remove_dir_all(&dir).expect("clean");
    }

    #[test]
    fn conflicting_labels_under_one_instance_are_a_typed_error() {
        let dir = temp_spool("conflict");
        let mut store = PostbackStore::open(&dir).expect("open");
        store
            .record(&event(
                "northstar",
                "ev-1",
                Some("i1"),
                "u1",
                "au1",
                Some("holdout-a"),
            ))
            .expect("record");
        store
            .record(&event(
                "northstar",
                "ev-2",
                Some("i1"),
                "u1",
                "au1",
                Some("Control"),
            ))
            .expect("record");
        let error = store
            .labels(Some("northstar"), "i1", "au1")
            .expect_err("two arms for one user");
        match &error {
            PostbackStoreError::Conflict {
                user_id,
                first,
                second,
            } => {
                assert_eq!(user_id, "u1");
                assert_eq!(first, "holdout-a", "the earlier assignment is named");
                assert_eq!(second, "Control");
            }
            other => panic!("expected a typed conflict, got {other:?}"),
        }
        assert!(
            error.to_string().contains("u1"),
            "the conflict names the user: {error}"
        );

        // Two rows that agree are one assignment, not a conflict. They live
        // under another ad unit so the conflicting rows above never enter
        // this read.
        store
            .record(&event("northstar", "ev-3", Some("i1"), "u2", "au2", None))
            .expect("record");
        store
            .record(&event(
                "northstar",
                "ev-4",
                Some("i1"),
                "u2",
                "au2",
                Some("control"),
            ))
            .expect("record");
        let labels = store
            .labels(Some("northstar"), "i1", "au2")
            .expect("agreeing rows");
        assert_eq!(labels.arms.get("u2"), Some(&false));
        std::fs::remove_dir_all(&dir).expect("clean");
    }

    #[test]
    fn labels_from_a_different_experiment_instance_are_ignored() {
        let dir = temp_spool("instance");
        let mut store = PostbackStore::open(&dir).expect("open");
        store
            .record(&event(
                "northstar",
                "ev-1",
                Some("i1"),
                "run-one-user",
                "au1",
                Some("holdout-a"),
            ))
            .expect("record");
        store
            .record(&event(
                "northstar",
                "ev-2",
                Some("i2"),
                "run-two-user",
                "au1",
                Some("holdout-b"),
            ))
            .expect("record");

        let first = store
            .labels(Some("northstar"), "i1", "au1")
            .expect("labels");
        assert_eq!(first.arms.len(), 1, "only the instance's own row");
        assert_eq!(first.arms.get("run-one-user"), Some(&true));
        assert!(
            !first.arms.contains_key("run-two-user"),
            "a row of another experiment instance is never read"
        );
        assert_eq!(
            first.test_labels.iter().cloned().collect::<Vec<_>>(),
            vec!["holdout-a".to_string()],
            "the other instance's label never enters this scope"
        );

        let second = store
            .labels(Some("northstar"), "i2", "au1")
            .expect("labels");
        assert!(!second.arms.contains_key("run-one-user"));
        assert_eq!(second.arms.get("run-two-user"), Some(&true));
        std::fs::remove_dir_all(&dir).expect("clean");
    }

    #[test]
    fn rows_for_another_ad_unit_are_ignored() {
        let dir = temp_spool("ad-unit");
        let mut store = PostbackStore::open(&dir).expect("open");
        store
            .record(&event(
                "northstar",
                "ev-1",
                Some("i1"),
                "this-unit-user",
                "au1",
                Some("holdout-a"),
            ))
            .expect("record");
        store
            .record(&event(
                "northstar",
                "ev-2",
                Some("i1"),
                "other-unit-user",
                "au2",
                Some("holdout-b"),
            ))
            .expect("record");

        let labels = store
            .labels(Some("northstar"), "i1", "au1")
            .expect("labels");
        assert_eq!(labels.arms.len(), 1);
        assert_eq!(labels.arms.get("this-unit-user"), Some(&true));
        assert!(
            !labels.arms.contains_key("other-unit-user"),
            "another ad unit's rows never reach this experiment"
        );
        std::fs::remove_dir_all(&dir).expect("clean");
    }

    #[test]
    fn epochs_cover_their_half_open_window_only() {
        let dir = temp_spool("epoch-window");
        let store = PostbackStore::open(&dir).expect("open");
        store
            .record_epoch("northstar", "au1", "instance-a", 1_000)
            .expect("epoch a");
        store
            .record_epoch("northstar", "au1", "instance-b", 9_000)
            .expect("epoch b");
        // Half-open [start, next start): both boundaries are pinned.
        assert_eq!(
            store.active_instance("northstar", "au1", 999).unwrap(),
            None,
            "before the first epoch nothing names an experiment"
        );
        assert_eq!(
            store.active_instance("northstar", "au1", 1_000).unwrap(),
            Some("instance-a".to_string()),
            "the start instant belongs to its epoch"
        );
        assert_eq!(
            store.active_instance("northstar", "au1", 8_999).unwrap(),
            Some("instance-a".to_string())
        );
        assert_eq!(
            store.active_instance("northstar", "au1", 9_000).unwrap(),
            Some("instance-b".to_string()),
            "the next start ends the earlier window"
        );
        // An explicitly closed epoch leaves a gap: a postback there names no
        // experiment instead of the last stopped one.
        store
            .close_epoch("northstar", "au1", "instance-b", 12_000)
            .expect("close b");
        assert_eq!(
            store.active_instance("northstar", "au1", 11_999).unwrap(),
            Some("instance-b".to_string()),
            "the closed window still covers instants before its end"
        );
        assert_eq!(
            store.active_instance("northstar", "au1", 12_000).unwrap(),
            None,
            "the close instant is outside the epoch"
        );
        std::fs::remove_dir_all(&dir).expect("clean");
    }

    #[test]
    fn a_row_outside_every_epoch_never_labels_a_run() {
        // The receiver stamps the active instance at ingestion. A row that
        // carries no instance (written before any epoch, or by an older
        // receiver) names no experiment, so no instance-scoped read may use
        // it: fail closed instead of letting it label every later run on the
        // ad unit.
        let dir = temp_spool("unversioned");
        let mut store = PostbackStore::open(&dir).expect("open");
        store
            .record(&event(
                "northstar",
                "ev-1",
                None,
                "orphan-user",
                "au1",
                Some("holdout-a"),
            ))
            .expect("record");
        for instance in ["instance-1", "instance-2"] {
            let labels = store
                .labels(Some("northstar"), instance, "au1")
                .expect("labels");
            assert!(labels.arms.is_empty(), "{instance} read an unowned row");
        }
        // A stamped row is read by exactly its own instance.
        store
            .record_epoch("northstar", "au1", "instance-2", 1_000)
            .expect("epoch");
        store
            .record(&event(
                "northstar",
                "ev-2",
                Some("instance-2"),
                "stamped-user",
                "au1",
                Some("holdout-a"),
            ))
            .expect("record");
        let own = store
            .labels(Some("northstar"), "instance-2", "au1")
            .expect("labels");
        assert_eq!(own.arms.get("stamped-user"), Some(&true));
        let other = store
            .labels(Some("northstar"), "instance-1", "au1")
            .expect("labels");
        assert!(other.arms.is_empty());
        std::fs::remove_dir_all(&dir).expect("clean");
    }
}
