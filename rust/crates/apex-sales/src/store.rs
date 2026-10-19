//! Durable state: append-only JSON-lines files in one directory.
//!
//! | file | content | rule |
//! |---|---|---|
//! | `publishers.jsonl` | publisher records | last record per key wins |
//! | `contacts.jsonl` | contact records | last record per email wins |
//! | `events.jsonl` | touches, outcomes, suppressions, resets | append-only history |
//! | `decisions.jsonl` | every gate decision with reasons | audit log, never read back |
//! | `approvals.jsonl` | operator approvals of planned touches | append-only |
//! | `ingest-seen.jsonl` | maildir files the reply ingest already processed | seen set |
//!
//! Nothing is ever rewritten. Every model is rebuilt from these files on
//! each run ([`crate::engine::learn`]), so the engine's state is a pure
//! function of its history: replayable, auditable and immune to a crash
//! between writes. A single-record append is one `write` followed by one
//! `sync_data`. Batch appends (imports and outcome imports) write every record
//! of the batch and then sync once: the durability point is the end of the
//! batch, so a crash leaves a valid prefix of whole records. The send path
//! (one touch event per delivered message) uses [`GroupCommit`]: each record is
//! a plain `write` and a barrier runs every [`GROUP_COMMIT_SYNC_EVERY`] records
//! and once at the end of the pass, so a pass of N sends costs
//! `ceil(N / GROUP_COMMIT_SYNC_EVERY)` barriers instead of N. If a crash tore
//! the last record, the next batch starts on a fresh line and the torn line is
//! skipped and counted on load, never glued to a later record.
//! A malformed line is skipped and counted, never fatal.

use std::collections::BTreeMap;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::json::{self, Value};
use crate::model::{
    App, Contact, ContactKind, Evidence, EvidenceKind, Outcome, Publisher, Store as AppStoreKind,
    UnixSecs,
};
use crate::policy::SuppressionReason;

/// How many records a [`GroupCommit`] writes before it forces a durability
/// barrier.
pub const GROUP_COMMIT_SYNC_EVERY: u64 = 50;

/// Make a state file owner-only. The history holds contact addresses,
/// replies and decisions, so it is never group- or world-readable.
fn restrict_file(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Make the state directory owner-only.
fn restrict_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// A group-commit appender for one append-only JSON-lines file.
///
/// Each record is written with a plain `write` and no `fsync`. A durability
/// barrier (`sync_data`) runs every `sync_every` records and once more in
/// [`GroupCommit::finish`], so a completed pass of N records costs
/// `ceil(N / sync_every)` barriers instead of N.
///
/// Durability: a process crash (a panic, a SIGKILL, a dropped writer) cannot
/// lose a record the process already wrote, because the bytes are in the
/// kernel page cache and a read after reopen sees them. Only power loss or a
/// kernel crash can drop the records written since the last barrier, so the
/// exposure window is at most `sync_every - 1` trailing records (at most 49
/// sends, the accepted bound) and [`GroupCommit::finish`] closes it at the end
/// of every pass.
#[derive(Debug)]
pub struct GroupCommit {
    path: PathBuf,
    file: Option<std::fs::File>,
    pending: u64,
    sync_every: u64,
    syncs: Arc<AtomicU64>,
}

impl GroupCommit {
    /// Append one JSON record. The target file is opened on first use, so an
    /// empty pass never creates it.
    pub fn append(&mut self, value: &Value) -> std::io::Result<()> {
        let line = value.to_json();
        self.write_line(&line)
    }

    /// Append one event record.
    pub fn append_event(&mut self, event: &Event) -> std::io::Result<()> {
        self.append(&event.to_json())
    }

    fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        if self.file.is_none() {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .append(true)
                .open(&self.path)?;
            restrict_file(&self.path)?;
            // Heal a torn tail left by an earlier crash so this record is not
            // glued to the previous one.
            let mut needs_newline = false;
            if file.metadata()?.len() > 0 {
                file.seek(SeekFrom::End(-1))?;
                let mut last = [0u8; 1];
                file.read_exact(&mut last)?;
                needs_newline = last[0] != b'\n';
            }
            if needs_newline {
                file.write_all(b"\n")?;
            }
            self.file = Some(file);
        }
        if let Some(file) = self.file.as_mut() {
            file.write_all(line.as_bytes())?;
            file.write_all(b"\n")?;
            self.pending += 1;
            if self.pending >= self.sync_every {
                file.sync_data()?;
                self.syncs.fetch_add(1, Ordering::SeqCst);
                self.pending = 0;
            }
        }
        Ok(())
    }

    /// Flush the trailing unsynced records and close. A no-op when nothing was
    /// written. Always call it at the end of a pass.
    pub fn finish(mut self) -> std::io::Result<()> {
        if self.pending > 0 {
            if let Some(file) = self.file.as_mut() {
                file.sync_data()?;
                self.syncs.fetch_add(1, Ordering::SeqCst);
                self.pending = 0;
            }
        }
        Ok(())
    }
}

/// One historical fact.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// A message was handed to the transport.
    Touch {
        /// When.
        at: UnixSecs,
        /// Recipient.
        email: String,
        /// Publisher key.
        publisher: String,
        /// The from-domain that sent it. `None` on history recorded before
        /// domains existed or sent by a single-domain configuration; such
        /// touches belong to the first configured sender.
        from: Option<String>,
        /// Sequence step (0 = first touch).
        step: usize,
        /// Template / arm id.
        arm: String,
        /// Message-ID.
        message_id: String,
        /// Local hour of the recipient at send time.
        local_hour: u32,
        /// The lead score's sequence-level interest probability at send
        /// time, kept for out-of-sample calibration.
        predicted: Option<f64>,
    },
    /// Something happened after a send, or at the publisher.
    Outcome {
        /// When.
        at: UnixSecs,
        /// Contact, when known.
        email: Option<String>,
        /// Publisher, when known.
        publisher: Option<String>,
        /// Outcome.
        outcome: Outcome,
        /// Free text (reply body excerpt, invoice id, ...).
        note: Option<String>,
    },
    /// Operator suppression.
    Suppress {
        /// When.
        at: UnixSecs,
        /// Address, if address-level.
        email: Option<String>,
        /// Publisher, if publisher-level.
        publisher: Option<String>,
    },
    /// Operator reset of sending-domain health after an investigation. The
    /// domain is named when one domain is reset; `None` resets every domain.
    HealthReset {
        /// When.
        at: UnixSecs,
        /// The domain, when only one was reset.
        domain: Option<String>,
    },
}

impl Event {
    /// Event time.
    pub fn at(&self) -> UnixSecs {
        match self {
            Event::Touch { at, .. }
            | Event::Outcome { at, .. }
            | Event::Suppress { at, .. }
            | Event::HealthReset { at, .. } => *at,
        }
    }

    /// JSON form.
    pub fn to_json(&self) -> Value {
        match self {
            Event::Touch {
                at,
                email,
                publisher,
                from,
                step,
                arm,
                message_id,
                local_hour,
                predicted,
            } => {
                let mut pairs = vec![
                    ("type".to_string(), Value::str("touch")),
                    ("at".to_string(), Value::num(*at as f64)),
                    ("email".to_string(), Value::str(email.clone())),
                    ("publisher".to_string(), Value::str(publisher.clone())),
                ];
                // The sending domain is recorded only when one was assigned
                // (multi-domain configurations); single-domain history stays
                // byte-for-byte as it was.
                if let Some(domain) = from {
                    pairs.push(("from".to_string(), Value::str(domain.clone())));
                }
                pairs.extend([
                    ("step".to_string(), Value::num(*step as f64)),
                    ("arm".to_string(), Value::str(arm.clone())),
                    ("messageId".to_string(), Value::str(message_id.clone())),
                    ("localHour".to_string(), Value::num(*local_hour)),
                    (
                        "predicted".to_string(),
                        predicted.map_or(Value::Null, Value::Number),
                    ),
                ]);
                Value::Object(pairs)
            }
            Event::Outcome {
                at,
                email,
                publisher,
                outcome,
                note,
            } => Value::object(vec![
                ("type", Value::str("outcome")),
                ("at", Value::num(*at as f64)),
                ("email", Value::opt_str(email.as_deref())),
                ("publisher", Value::opt_str(publisher.as_deref())),
                ("outcome", Value::str(outcome.as_string())),
                ("note", Value::opt_str(note.as_deref())),
            ]),
            Event::Suppress {
                at,
                email,
                publisher,
            } => Value::object(vec![
                ("type", Value::str("suppress")),
                ("at", Value::num(*at as f64)),
                ("email", Value::opt_str(email.as_deref())),
                ("publisher", Value::opt_str(publisher.as_deref())),
            ]),
            Event::HealthReset { at, domain } => {
                let mut pairs = vec![
                    ("type".to_string(), Value::str("health_reset")),
                    ("at".to_string(), Value::num(*at as f64)),
                ];
                if let Some(d) = domain {
                    pairs.push(("domain".to_string(), Value::str(d.clone())));
                }
                Value::Object(pairs)
            }
        }
    }

    /// Parse the JSON form.
    pub fn from_json(v: &Value) -> Option<Self> {
        let at = v.get("at").and_then(Value::as_i64)?;
        let s = |k: &str| v.str_at(k).map(str::to_string);
        Some(match v.str_at("type")? {
            "touch" => Event::Touch {
                at,
                email: s("email")?,
                publisher: s("publisher")?,
                from: s("from"),
                step: usize::try_from(v.get("step").and_then(Value::as_i64)?).ok()?,
                arm: s("arm")?,
                message_id: s("messageId")?,
                local_hour: u32::try_from(v.get("localHour").and_then(Value::as_i64)?).ok()?,
                predicted: v.f64_at("predicted"),
            },
            "outcome" => Event::Outcome {
                at,
                email: s("email"),
                publisher: s("publisher"),
                outcome: Outcome::parse(v.str_at("outcome")?)?,
                note: s("note"),
            },
            "suppress" => Event::Suppress {
                at,
                email: s("email"),
                publisher: s("publisher"),
            },
            "health_reset" => Event::HealthReset {
                at,
                domain: s("domain"),
            },
            _ => return None,
        })
    }
}

/// Map an outcome to the suppression it implies, if any.
pub fn implied_suppression(outcome: Outcome) -> Option<SuppressionReason> {
    match outcome {
        Outcome::HardBounce => Some(SuppressionReason::HardBounce),
        Outcome::Unsubscribe | Outcome::Reply(crate::model::ReplyClass::OptOut) => {
            Some(SuppressionReason::OptOut)
        }
        Outcome::Complaint => Some(SuppressionReason::Complaint),
        _ => None,
    }
}

/// Publisher to JSON.
pub fn publisher_to_json(p: &Publisher) -> Value {
    Value::object(vec![
        ("key", Value::str(p.key.clone())),
        ("name", Value::str(p.name.clone())),
        ("domain", Value::opt_str(p.domain.as_deref())),
        ("country", Value::opt_str(p.country.as_deref())),
        (
            "apps",
            Value::Array(
                p.apps
                    .iter()
                    .map(|a| {
                        Value::object(vec![
                            ("store", Value::str(a.store.as_str())),
                            ("id", Value::str(a.id.clone())),
                            ("name", Value::str(a.name.clone())),
                            ("genre", Value::opt_str(a.genre.as_deref())),
                            (
                                "ratingCount",
                                a.rating_count.map_or(Value::Null, |n| Value::num(n as f64)),
                            ),
                            ("free", a.free.map_or(Value::Null, Value::Bool)),
                            (
                                "daysSinceUpdate",
                                a.days_since_update.map_or(Value::Null, Value::num),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("adsTxt", Value::opt_str(p.ads_txt_text.as_deref())),
        ("adsTxtMissing", Value::Bool(p.ads_txt_missing)),
        (
            "evidence",
            Value::Array(
                p.evidence
                    .iter()
                    .map(|e| {
                        Value::object(vec![
                            ("id", Value::str(e.id.clone())),
                            ("kind", Value::str(e.kind.as_str())),
                            ("source", Value::str(e.source.clone())),
                            ("statement", Value::str(e.statement.clone())),
                            ("observedAt", Value::num(e.observed_at as f64)),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("inbound", Value::Bool(p.inbound)),
        (
            "declaredPlatforms",
            Value::Array(
                p.declared_platforms
                    .iter()
                    .map(|s| Value::str(s.clone()))
                    .collect(),
            ),
        ),
    ])
}

/// Publisher from JSON.
pub fn publisher_from_json(v: &Value) -> Option<Publisher> {
    let mut p = Publisher::new(v.str_at("key")?, v.str_at("name").unwrap_or_default());
    p.domain = v.str_at("domain").map(str::to_string);
    p.country = v.str_at("country").map(str::to_string);
    for a in v.get("apps").and_then(Value::as_array).unwrap_or_default() {
        p.apps.push(App {
            store: AppStoreKind::parse(a.str_at("store")?)?,
            id: a.str_at("id")?.to_string(),
            name: a.str_at("name")?.to_string(),
            genre: a.str_at("genre").map(str::to_string),
            rating_count: a
                .get("ratingCount")
                .and_then(Value::as_i64)
                .and_then(|n| u64::try_from(n).ok()),
            free: a.get("free").and_then(Value::as_bool),
            days_since_update: a
                .get("daysSinceUpdate")
                .and_then(Value::as_i64)
                .and_then(|n| u32::try_from(n).ok()),
        });
    }
    p.ads_txt_text = v.str_at("adsTxt").map(str::to_string);
    p.ads_txt_missing = v
        .get("adsTxtMissing")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    for e in v
        .get("evidence")
        .and_then(Value::as_array)
        .unwrap_or_default()
    {
        p.evidence.push(Evidence {
            id: e.str_at("id")?.to_string(),
            kind: EvidenceKind::parse(e.str_at("kind")?)?,
            source: e.str_at("source").unwrap_or_default().to_string(),
            statement: e.str_at("statement")?.to_string(),
            observed_at: e.get("observedAt").and_then(Value::as_i64).unwrap_or(0),
        });
    }
    p.inbound = v.get("inbound").and_then(Value::as_bool).unwrap_or(false);
    p.declared_platforms = v
        .get("declaredPlatforms")
        .and_then(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|s| s.as_str().map(str::to_string))
        .collect();
    Some(p)
}

/// Contact to JSON.
pub fn contact_to_json(c: &Contact) -> Value {
    Value::object(vec![
        ("email", Value::str(c.email.clone())),
        ("name", Value::opt_str(c.name.as_deref())),
        ("role", Value::opt_str(c.role.as_deref())),
        ("publisher", Value::str(c.publisher_key.clone())),
        ("kind", Value::str(c.kind.as_str())),
        ("source", Value::str(c.source.clone())),
        ("consent", Value::Bool(c.consent)),
    ])
}

/// Contact from JSON.
pub fn contact_from_json(v: &Value) -> Option<Contact> {
    let email = crate::model::normalize_email(v.str_at("email")?)?;
    Some(Contact {
        kind: match v.str_at("kind") {
            Some("named") => ContactKind::Named,
            Some("role") => ContactKind::Role,
            Some("generic") => ContactKind::Generic,
            _ => ContactKind::of_email(&email),
        },
        email,
        name: v.str_at("name").map(str::to_string),
        role: v.str_at("role").map(str::to_string),
        publisher_key: v.str_at("publisher")?.to_string(),
        source: v.str_at("source").unwrap_or_default().to_string(),
        consent: v.get("consent").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// Everything loaded from the directory.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    /// Publishers by key.
    pub publishers: BTreeMap<String, Publisher>,
    /// Contacts by email.
    pub contacts: BTreeMap<String, Contact>,
    /// Events in time order (stable for equal times).
    pub events: Vec<Event>,
    /// Approved touch ids.
    pub approvals: Vec<String>,
    /// Lines that failed to parse.
    pub skipped_lines: usize,
}

/// The state directory.
#[derive(Debug, Clone)]
pub struct StateDir {
    root: PathBuf,
    syncs: Arc<AtomicU64>,
}

impl StateDir {
    /// Use (and create) `root`.
    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        restrict_dir(&root)?;
        Ok(Self {
            root,
            syncs: Arc::new(AtomicU64::new(0)),
        })
    }

    /// The directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The number of durability barriers (`sync_data`) this directory has
    /// performed. Test seam for the fsync counters; a batch of any size costs
    /// exactly one.
    #[cfg(test)]
    pub(crate) fn sync_count(&self) -> u64 {
        self.syncs.load(Ordering::SeqCst)
    }

    fn append(&self, file: &str, v: &Value) -> std::io::Result<()> {
        self.append_batch(file, std::slice::from_ref(v))
    }

    /// Append `values` as JSON lines and sync once at the end.
    ///
    /// The durability point is the returned `Ok`: every record of the batch
    /// is on stable storage then, and a crash before it leaves a prefix of
    /// whole lines. A pre-existing torn tail (a crash during an earlier
    /// write) cannot swallow the next record because a newline is inserted
    /// first.
    fn append_batch(&self, file: &str, values: &[Value]) -> std::io::Result<()> {
        if values.is_empty() {
            return Ok(());
        }
        let mut body = String::new();
        for value in values {
            body.push_str(&value.to_json());
            body.push('\n');
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(self.root.join(file))?;
        restrict_file(&self.root.join(file))?;
        let mut needs_newline = false;
        if f.metadata()?.len() > 0 {
            f.seek(SeekFrom::End(-1))?;
            let mut last = [0u8; 1];
            f.read_exact(&mut last)?;
            needs_newline = last[0] != b'\n';
        }
        if needs_newline {
            f.write_all(b"\n")?;
        }
        f.write_all(body.as_bytes())?;
        f.sync_data()?;
        self.syncs.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn append_batch_of(
        &self,
        file: &str,
        values: impl IntoIterator<Item = Value>,
    ) -> std::io::Result<()> {
        let values: Vec<Value> = values.into_iter().collect();
        self.append_batch(file, &values)
    }

    /// Append a publisher record.
    pub fn put_publisher(&self, p: &Publisher) -> std::io::Result<()> {
        self.append("publishers.jsonl", &publisher_to_json(p))
    }

    /// Append a contact record.
    pub fn put_contact(&self, c: &Contact) -> std::io::Result<()> {
        self.append("contacts.jsonl", &contact_to_json(c))
    }

    /// Append an event.
    pub fn put_event(&self, e: &Event) -> std::io::Result<()> {
        self.append("events.jsonl", &e.to_json())
    }

    /// Append a decision record.
    pub fn put_decision(&self, v: &Value) -> std::io::Result<()> {
        self.append("decisions.jsonl", v)
    }

    /// Append a batch of publisher records with one durability barrier.
    pub fn put_publishers(&self, publishers: &[Publisher]) -> std::io::Result<()> {
        self.append_batch_of("publishers.jsonl", publishers.iter().map(publisher_to_json))
    }

    /// Append a batch of contact records with one durability barrier.
    pub fn put_contacts(&self, contacts: &[Contact]) -> std::io::Result<()> {
        self.append_batch_of("contacts.jsonl", contacts.iter().map(contact_to_json))
    }

    /// Append a batch of events with one durability barrier.
    pub fn put_events(&self, events: &[Event]) -> std::io::Result<()> {
        self.append_batch_of("events.jsonl", events.iter().map(Event::to_json))
    }

    /// Append a batch of decision records with one durability barrier.
    pub fn put_decisions(&self, decisions: &[Value]) -> std::io::Result<()> {
        self.append_batch_of("decisions.jsonl", decisions.iter().cloned())
    }

    /// Record an operator approval of a planned touch id.
    pub fn put_approval(&self, touch_id: &str, at: UnixSecs) -> std::io::Result<()> {
        self.append(
            "approvals.jsonl",
            &Value::object(vec![
                ("touchId", Value::str(touch_id)),
                ("at", Value::num(at as f64)),
            ]),
        )
    }

    /// Record one maildir file as processed by the reply ingest.
    pub fn put_ingest_seen(&self, v: &Value) -> std::io::Result<()> {
        self.append("ingest-seen.jsonl", v)
    }

    /// Record a batch of processed maildir files with one barrier.
    pub fn put_ingest_seens(&self, rows: &[Value]) -> std::io::Result<()> {
        self.append_batch_of("ingest-seen.jsonl", rows.iter().cloned())
    }

    /// Open a group-commit writer over `file`: each record is a plain write,
    /// with one barrier every `sync_every` records and one at
    /// [`GroupCommit::finish`]. The file is created on the first record, so an
    /// empty pass writes nothing.
    pub fn group_commit(&self, file: &str, sync_every: u64) -> GroupCommit {
        GroupCommit {
            path: self.root.join(file),
            file: None,
            pending: 0,
            sync_every: sync_every.max(1),
            syncs: Arc::clone(&self.syncs),
        }
    }

    fn lines(&self, file: &str) -> std::io::Result<Vec<String>> {
        match std::fs::read_to_string(self.root.join(file)) {
            Ok(text) => Ok(text.lines().map(str::to_string).collect()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    }

    /// Load everything.
    pub fn load(&self) -> std::io::Result<Snapshot> {
        let mut snap = Snapshot::default();
        for line in self.lines("publishers.jsonl")? {
            match json::parse(&line)
                .ok()
                .as_ref()
                .and_then(publisher_from_json)
            {
                Some(p) => {
                    snap.publishers.insert(p.key.clone(), p);
                }
                None => snap.skipped_lines += 1,
            }
        }
        for line in self.lines("contacts.jsonl")? {
            match json::parse(&line).ok().as_ref().and_then(contact_from_json) {
                Some(c) => {
                    snap.contacts.insert(c.email.clone(), c);
                }
                None => snap.skipped_lines += 1,
            }
        }
        for line in self.lines("events.jsonl")? {
            match json::parse(&line).ok().as_ref().and_then(Event::from_json) {
                Some(e) => snap.events.push(e),
                None => snap.skipped_lines += 1,
            }
        }
        snap.events.sort_by_key(Event::at);
        for line in self.lines("approvals.jsonl")? {
            match json::parse(&line)
                .ok()
                .as_ref()
                .and_then(|v| v.str_at("touchId").map(str::to_string))
            {
                Some(id) => snap.approvals.push(id),
                None => snap.skipped_lines += 1,
            }
        }
        Ok(snap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ReplyClass;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("apex-sales-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn sample_publisher() -> Publisher {
        let mut p = Publisher::new("studio.example", "Studio \"Q\"");
        p.domain = Some("studio.example".into());
        p.country = Some("FI".into());
        p.apps.push(App {
            store: AppStoreKind::AppStore,
            id: "42".into(),
            name: "Tile Quest".into(),
            genre: Some("Games".into()),
            rating_count: Some(12_000),
            free: Some(true),
            days_since_update: Some(3),
        });
        p.ads_txt_text = Some("applovin.com, 1, DIRECT\n".into());
        p.evidence.push(Evidence {
            id: "ev1".into(),
            kind: EvidenceKind::AppAdsTxt,
            source: "https://studio.example/app-ads.txt".into(),
            statement: "fact".into(),
            observed_at: 7,
        });
        p.inbound = true;
        p.declared_platforms = vec!["max".into()];
        p
    }

    #[test]
    fn records_round_trip_through_json() {
        let p = sample_publisher();
        assert_eq!(publisher_from_json(&publisher_to_json(&p)), Some(p));
        let c = Contact {
            email: "maria@studio.example".into(),
            name: Some("Maria".into()),
            role: Some("CEO".into()),
            publisher_key: "studio.example".into(),
            kind: ContactKind::Named,
            source: "seed".into(),
            consent: true,
        };
        assert_eq!(contact_from_json(&contact_to_json(&c)), Some(c));
        let events = vec![
            Event::Touch {
                at: 5,
                email: "a@b.io".into(),
                publisher: "b.io".into(),
                from: Some("one.example".into()),
                step: 1,
                arm: "t1-bump".into(),
                message_id: "m@x".into(),
                local_hour: 10,
                predicted: Some(0.04),
            },
            Event::Outcome {
                at: 6,
                email: Some("a@b.io".into()),
                publisher: None,
                outcome: Outcome::Reply(ReplyClass::Positive),
                note: Some("yes".into()),
            },
            Event::Suppress {
                at: 7,
                email: None,
                publisher: Some("b.io".into()),
            },
            Event::HealthReset {
                at: 8,
                domain: Some("one.example".into()),
            },
            Event::HealthReset {
                at: 9,
                domain: None,
            },
        ];
        for e in events {
            assert_eq!(Event::from_json(&e.to_json()), Some(e));
        }
        assert_eq!(
            Event::from_json(&Value::object(vec![
                ("type", Value::str("x")),
                ("at", Value::num(1))
            ])),
            None
        );
    }

    #[test]
    fn directory_appends_and_reloads_with_last_record_winning() {
        let dir = temp_dir("roundtrip");
        let store = StateDir::open(&dir).unwrap();
        let mut p = sample_publisher();
        store.put_publisher(&p).unwrap();
        p.name = "Renamed".into();
        store.put_publisher(&p).unwrap();
        store
            .put_contact(&Contact {
                email: "x@studio.example".into(),
                name: None,
                role: None,
                publisher_key: p.key.clone(),
                kind: ContactKind::Named,
                source: "s".into(),
                consent: false,
            })
            .unwrap();
        store
            .put_event(&Event::HealthReset {
                at: 9,
                domain: None,
            })
            .unwrap();
        store
            .put_event(&Event::HealthReset {
                at: 3,
                domain: Some("one.example".into()),
            })
            .unwrap();
        store.put_approval("touch-1", 4).unwrap();
        store
            .put_ingest_seen(&Value::object(vec![("file", Value::str("m1.eml"))]))
            .unwrap();
        store
            .put_decision(&Value::object(vec![("v", Value::num(1))]))
            .unwrap();
        std::fs::write(dir.join("events.jsonl"), {
            let mut t = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
            t.push_str("{not json\n");
            t
        })
        .unwrap();
        let snap = store.load().unwrap();
        assert_eq!(snap.publishers["studio.example"].name, "Renamed");
        assert_eq!(snap.contacts.len(), 1);
        assert_eq!(
            snap.events.iter().map(Event::at).collect::<Vec<_>>(),
            vec![3, 9]
        );
        assert_eq!(snap.approvals, vec!["touch-1".to_string()]);
        assert_eq!(snap.skipped_lines, 1);
        let seen = std::fs::read_to_string(dir.join("ingest-seen.jsonl")).unwrap();
        assert!(seen.contains("\"file\":\"m1.eml\""), "{seen}");
        assert_eq!(store.root(), dir.as_path());
        let _ = std::fs::remove_dir_all(&dir);
        let empty = StateDir::open(temp_dir("empty")).unwrap();
        assert_eq!(empty.load().unwrap(), Snapshot::default());
    }

    fn health_reset(at: i64) -> Event {
        Event::HealthReset { at, domain: None }
    }

    #[test]
    fn group_commit_syncs_ceil_n_over_fifty_times_for_n_sends() {
        for count in [0usize, 1, 49, 50, 51, 100, 123] {
            let dir = temp_dir(&format!("group-sync-{count}"));
            let store = StateDir::open(&dir).unwrap();
            let mut writer = store.group_commit("events.jsonl", GROUP_COMMIT_SYNC_EVERY);
            let events: Vec<Event> = (0..count as i64).map(health_reset).collect();
            for event in &events {
                writer.append_event(event).unwrap();
            }
            writer.finish().unwrap();
            let expected = (count as u64).div_ceil(GROUP_COMMIT_SYNC_EVERY);
            assert_eq!(store.sync_count(), expected, "sends={count} barriers");
            println!("group_commit sends={count} barriers={expected}");
            let snapshot = store.load().unwrap();
            assert_eq!(snapshot.events.len(), count);
            assert_eq!(snapshot.skipped_lines, 0);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[cfg(unix)]
    #[test]
    fn state_directory_and_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = temp_dir("state-mode");
        let store = StateDir::open(&dir).unwrap();
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700, "the state directory is owner-only");
        store
            .put_publisher(&sample_publisher())
            .expect("publisher write");
        store.put_event(&health_reset(1)).expect("event write");
        let mut grouped = store.group_commit("approvals.jsonl", GROUP_COMMIT_SYNC_EVERY);
        grouped
            .append(&Value::object(vec![(
                "touchId",
                Value::str("t0123456789abcdef"),
            )]))
            .expect("group-commit write");
        grouped.finish().expect("group-commit finish");
        for name in ["publishers.jsonl", "events.jsonl", "approvals.jsonl"] {
            assert_eq!(mode(&dir.join(name)), 0o600, "{name} is owner-only");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn group_committed_events_are_byte_identical_to_a_batch_append() {
        let batch_dir = temp_dir("group-bytes-batch");
        let group_dir = temp_dir("group-bytes-group");
        let batch = StateDir::open(&batch_dir).unwrap();
        let grouped = StateDir::open(&group_dir).unwrap();
        let events: Vec<Event> = (0..123).map(health_reset).collect();
        batch.put_events(&events).unwrap();
        let mut writer = grouped.group_commit("events.jsonl", GROUP_COMMIT_SYNC_EVERY);
        for event in &events {
            writer.append_event(event).unwrap();
        }
        writer.finish().unwrap();
        assert_eq!(
            std::fs::read(batch_dir.join("events.jsonl")).unwrap(),
            std::fs::read(group_dir.join("events.jsonl")).unwrap(),
            "the group-commit bytes equal the batch-append bytes"
        );
        assert_eq!(grouped.load().unwrap().events.len(), events.len());
        let _ = std::fs::remove_dir_all(&batch_dir);
        let _ = std::fs::remove_dir_all(&group_dir);
    }

    #[test]
    fn a_plain_write_survives_a_process_crash_and_is_seen_on_reopen() {
        let dir = temp_dir("group-crash");
        let store = StateDir::open(&dir).unwrap();
        {
            let mut writer = store.group_commit("events.jsonl", GROUP_COMMIT_SYNC_EVERY);
            writer.append_event(&health_reset(11)).unwrap();
            writer.append_event(&health_reset(12)).unwrap();
            // Simulate a crash: the writer is dropped WITHOUT `finish`, so the
            // records are in the page cache but were never fsynced (a process
            // crash preserves them; only power loss would not).
            drop(writer);
        }
        let reopened = StateDir::open(&dir).unwrap();
        let snapshot = reopened.load().unwrap();
        let times: Vec<i64> = snapshot.events.iter().map(Event::at).collect();
        assert_eq!(
            times,
            vec![11, 12],
            "records written before the crash are visible on reopen"
        );
        assert_eq!(snapshot.skipped_lines, 0);
        assert_eq!(store.sync_count(), 0, "no barrier ran before the crash");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_ten_and_fifty_thousand_event_batch_syncs_once_and_reloads_every_record() {
        for count in [10_000u64, 50_000] {
            let dir = temp_dir(&format!("batch-{count}"));
            let store = StateDir::open(&dir).unwrap();
            let events: Vec<Event> = (0..count as i64).map(health_reset).collect();
            let before = store.sync_count();
            let started = std::time::Instant::now();
            store.put_events(&events).unwrap();
            let elapsed = started.elapsed();
            assert_eq!(store.sync_count() - before, 1, "one barrier per batch");
            let snapshot = store.load().unwrap();
            assert_eq!(snapshot.events.len() as u64, count);
            assert_eq!(snapshot.skipped_lines, 0);
            println!(
                "batch_events count={count} wall_ms={:.2} fsyncs=1",
                elapsed.as_secs_f64() * 1_000.0
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn an_interrupted_batch_leaves_a_valid_prefix_and_the_next_batch_appends_cleanly() {
        let dir = temp_dir("batch-prefix");
        let store = StateDir::open(&dir).unwrap();
        let events: Vec<Event> = (0..100).map(health_reset).collect();
        store.put_events(&events).unwrap();
        let path = dir.join("events.jsonl");
        let text = std::fs::read_to_string(&path).unwrap();
        let whole: Vec<&str> = text.lines().collect();
        // Kill after 40 whole records: a valid prefix with no torn line.
        let prefix: String = whole[..40].iter().map(|line| format!("{line}\n")).collect();
        std::fs::write(&path, prefix).unwrap();
        let snapshot = store.load().unwrap();
        assert_eq!(snapshot.events.len(), 40);
        assert_eq!(snapshot.skipped_lines, 0);
        // The next batch starts a new line and is not glued to a torn tail.
        store.put_event(&health_reset(500)).unwrap();
        let snapshot = store.load().unwrap();
        assert_eq!(snapshot.events.len(), 41);
        assert_eq!(snapshot.events.last().map(Event::at), Some(500));
        assert_eq!(snapshot.skipped_lines, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_torn_tail_is_skipped_and_the_next_batch_still_parses() {
        let dir = temp_dir("batch-torn");
        let store = StateDir::open(&dir).unwrap();
        let events: Vec<Event> = (0..100).map(health_reset).collect();
        store.put_events(&events).unwrap();
        let path = dir.join("events.jsonl");
        let text = std::fs::read_to_string(&path).unwrap();
        let whole: Vec<&str> = text.lines().collect();
        let mut prefix: String = whole[..40].iter().map(|line| format!("{line}\n")).collect();
        // A crash mid-write leaves half of record 41.
        prefix.push_str(&whole[40][..weird_cut(whole[40])]);
        std::fs::write(&path, prefix).unwrap();
        let snapshot = store.load().unwrap();
        assert_eq!(snapshot.events.len(), 40);
        assert_eq!(
            snapshot.skipped_lines, 1,
            "the torn line is counted, not fatal"
        );
        store.put_event(&health_reset(600)).unwrap();
        let snapshot = store.load().unwrap();
        assert_eq!(snapshot.events.len(), 41);
        assert_eq!(
            snapshot.skipped_lines, 1,
            "only the old torn line is skipped"
        );
        assert!(snapshot.events.iter().any(|event| event.at() == 600));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn weird_cut(line: &str) -> usize {
        line.len() / 2
    }

    #[test]
    fn outcomes_imply_suppressions() {
        assert_eq!(
            implied_suppression(Outcome::HardBounce),
            Some(SuppressionReason::HardBounce)
        );
        assert_eq!(
            implied_suppression(Outcome::Unsubscribe),
            Some(SuppressionReason::OptOut)
        );
        assert_eq!(
            implied_suppression(Outcome::Reply(ReplyClass::OptOut)),
            Some(SuppressionReason::OptOut)
        );
        assert_eq!(
            implied_suppression(Outcome::Complaint),
            Some(SuppressionReason::Complaint)
        );
        assert_eq!(implied_suppression(Outcome::SoftBounce), None);
    }
}
