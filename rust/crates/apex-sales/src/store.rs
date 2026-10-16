//! Durable state: append-only JSON-lines files in one directory.
//!
//! | file | content | rule |
//! |---|---|---|
//! | `publishers.jsonl` | publisher records | last record per key wins |
//! | `contacts.jsonl` | contact records | last record per email wins |
//! | `events.jsonl` | touches, outcomes, suppressions, resets | append-only history |
//! | `decisions.jsonl` | every gate decision with reasons | audit log, never read back |
//! | `approvals.jsonl` | operator approvals of planned touches | append-only |
//!
//! Nothing is ever rewritten. Every model is rebuilt from these files on
//! each run ([`crate::engine::learn`]), so the engine's state is a pure
//! function of its history: replayable, auditable and immune to a crash
//! between writes. Each append is a single `write` followed by `sync_data`.
//! A malformed line is skipped and counted, never fatal.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::json::{self, Value};
use crate::model::{
    App, Contact, ContactKind, Evidence, EvidenceKind, Outcome, Publisher, Store as AppStoreKind,
    UnixSecs,
};
use crate::policy::SuppressionReason;

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
    /// Operator reset of the sending domain's health after an investigation.
    HealthReset {
        /// When.
        at: UnixSecs,
    },
}

impl Event {
    /// Event time.
    pub fn at(&self) -> UnixSecs {
        match self {
            Event::Touch { at, .. }
            | Event::Outcome { at, .. }
            | Event::Suppress { at, .. }
            | Event::HealthReset { at } => *at,
        }
    }

    /// JSON form.
    pub fn to_json(&self) -> Value {
        match self {
            Event::Touch {
                at,
                email,
                publisher,
                step,
                arm,
                message_id,
                local_hour,
                predicted,
            } => Value::object(vec![
                ("type", Value::str("touch")),
                ("at", Value::num(*at as f64)),
                ("email", Value::str(email.clone())),
                ("publisher", Value::str(publisher.clone())),
                ("step", Value::num(*step as f64)),
                ("arm", Value::str(arm.clone())),
                ("messageId", Value::str(message_id.clone())),
                ("localHour", Value::num(*local_hour)),
                ("predicted", predicted.map_or(Value::Null, Value::Number)),
            ]),
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
            Event::HealthReset { at } => Value::object(vec![
                ("type", Value::str("health_reset")),
                ("at", Value::num(*at as f64)),
            ]),
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
            "health_reset" => Event::HealthReset { at },
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
}

impl StateDir {
    /// Use (and create) `root`.
    pub fn open(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// The directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn append(&self, file: &str, v: &Value) -> std::io::Result<()> {
        let mut line = v.to_json();
        line.push('\n');
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join(file))?;
        f.write_all(line.as_bytes())?;
        f.sync_data()
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
            Event::HealthReset { at: 8 },
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
        store.put_event(&Event::HealthReset { at: 9 }).unwrap();
        store.put_event(&Event::HealthReset { at: 3 }).unwrap();
        store.put_approval("touch-1", 4).unwrap();
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
        assert_eq!(store.root(), dir.as_path());
        let _ = std::fs::remove_dir_all(&dir);
        let empty = StateDir::open(temp_dir("empty")).unwrap();
        assert_eq!(empty.load().unwrap(), Snapshot::default());
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
