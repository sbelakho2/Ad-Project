//! The mutating actions: approve a planned touch, suppress a target, record
//! an outcome, reset the sending domain.
//!
//! Each action mirrors its CLI equivalent exactly — the same durable event
//! shapes in the same state directory — and carries a hidden idempotency
//! key: a replayed submission with the same key and the same payload answers
//! the remembered result page instead of writing a second event; the same
//! key with a different payload is a conflict, never a write.

use std::collections::BTreeMap;

use apex_sales::cli::Env;
use apex_sales::model::Outcome;
use apex_sales::policy::SuppressionReason;
use apex_sales::store::{Event, StateDir};

use crate::data::Target;
use crate::signing::{is_random_hex_32, operation_id};
use crate::state::{ActionResultRecord, Claim, WebState};

/// The longest accepted note, matching the form's `maxlength`.
pub const MAX_NOTE_CHARS: usize = 500;

/// The four mutating actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    /// Approve a planned touch (CLI `approve`).
    Approve,
    /// Suppress an address or publisher (CLI `suppress`, plus the implied
    /// outcome events for the other reasons).
    Suppress,
    /// Record an outcome (CLI `record`).
    Record,
    /// Reset the sending domain (CLI `reset-health`).
    ResetHealth,
}

impl ActionKind {
    /// Parse the route segment.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "approve" => Some(ActionKind::Approve),
            "suppress" => Some(ActionKind::Suppress),
            "record" => Some(ActionKind::Record),
            "reset-health" => Some(ActionKind::ResetHealth),
            _ => None,
        }
    }

    /// The stable string form.
    pub fn as_str(self) -> &'static str {
        match self {
            ActionKind::Approve => "approve",
            ActionKind::Suppress => "suppress",
            ActionKind::Record => "record",
            ActionKind::ResetHealth => "reset-health",
        }
    }
}

/// Why an action could not execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionError {
    /// The submission is invalid; nothing was written.
    BadRequest(String),
    /// The idempotency key was already used for a different payload.
    Conflict,
    /// The durable write failed.
    Io(String),
}

/// Parse an `application/x-www-form-urlencoded` body (the same parser the
/// control plane uses: first value wins, `+` is space, percent-decoded).
pub fn parse_form(body: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let text = std::str::from_utf8(body).map_err(|_| "form body is not UTF-8".to_string())?;
    let mut fields = BTreeMap::new();
    for pair in text.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        fields
            .entry(percent_decode(key)?)
            .or_insert(percent_decode(value)?);
    }
    Ok(fields)
}

fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' => {
                let high = bytes
                    .get(index + 1)
                    .and_then(|byte| hex_value(*byte))
                    .ok_or("bad percent escape")?;
                let low = bytes
                    .get(index + 2)
                    .and_then(|byte| hex_value(*byte))
                    .ok_or("bad percent escape")?;
                out.push(high << 4 | low);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| "form value is not UTF-8".to_string())
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Execute one action, or explain the refusal. Returns the result record
/// and whether it was answered from the idempotency memory.
pub fn execute(
    state: &WebState,
    kind: ActionKind,
    fields: &BTreeMap<String, String>,
) -> Result<(ActionResultRecord, bool), ActionError> {
    let key = fields.get("idempotency_key").cloned().unwrap_or_default();
    if !is_random_hex_32(&key) {
        return Err(ActionError::BadRequest(
            "The form's idempotency key is missing or malformed; nothing was written. Reload the page and submit again.".to_string(),
        ));
    }
    let fingerprint = fingerprint(kind, fields);
    // One atomic guard across claim -> execute -> remember: concurrent
    // requests with the same key serialize here, so the second one always
    // observes the completed record instead of claiming Fresh again. The
    // durable write happens under the guard; these actions are tiny,
    // owner-only and file-backed, so serializing them is the correct trade.
    state.with_actions(|memory| match memory.claim(&key, &fingerprint) {
        Claim::Duplicate(operation_id) => match memory.outcome(&operation_id).cloned() {
            Some(record) => Ok((record, true)),
            None => Err(ActionError::Conflict),
        },
        Claim::Conflict => Err(ActionError::Conflict),
        Claim::Fresh => {
            let record = run(state, kind, fields)?;
            memory.remember(&key, &fingerprint, record.clone());
            Ok((record, false))
        }
    })
}

/// The payload fingerprint: the kind plus the fields that define the write.
fn fingerprint(kind: ActionKind, fields: &BTreeMap<String, String>) -> String {
    let pick = |name: &str| fields.get(name).cloned().unwrap_or_default();
    match kind {
        ActionKind::Approve => format!("approve|{}", pick("touch_id")),
        ActionKind::Suppress => format!("suppress|{}|{}", pick("target"), pick("reason")),
        ActionKind::Record => format!(
            "record|{}|{}|{}",
            pick("target"),
            pick("outcome"),
            pick("note")
        ),
        ActionKind::ResetHealth => "reset-health|confirmed".to_string(),
    }
}

fn run(
    state: &WebState,
    kind: ActionKind,
    fields: &BTreeMap<String, String>,
) -> Result<ActionResultRecord, ActionError> {
    let env: &Env = &state.config.env;
    let dir_path = crate::data::RequestData::state_dir(env);
    let dir = StateDir::open(&dir_path).map_err(|error| ActionError::Io(error.to_string()))?;
    let now = state.now_secs();
    let at_ms = state.now_ms();
    let operation_id = operation_id();
    match kind {
        ActionKind::Approve => {
            let touch_id = fields.get("touch_id").cloned().unwrap_or_default();
            if !crate::data::valid_touch_id(&touch_id) {
                return Err(ActionError::BadRequest(
                    "The touch id is not a planned-touch id; nothing was written.".to_string(),
                ));
            }
            dir.put_approval(&touch_id, now)
                .map_err(|error| ActionError::Io(error.to_string()))?;
            Ok(ActionResultRecord {
                operation_id,
                action: kind.as_str().to_string(),
                target: touch_id.clone(),
                detail: format!(
                    "Approval recorded for touch {touch_id}; the runner sends it on its schedule when the mode requires approval"
                ),
                at_ms,
                confirmed_link: "/plan".to_string(),
            })
        }
        ActionKind::Suppress => {
            let target_raw = fields.get("target").cloned().unwrap_or_default();
            let reason_raw = fields.get("reason").cloned().unwrap_or_default();
            let target = crate::data::parse_target(&target_raw).ok_or_else(|| {
                ActionError::BadRequest(
                    "The suppress target is not an address or a publisher domain; nothing was written."
                        .to_string(),
                )
            })?;
            let event = suppression_event(&target, &reason_raw, now)?;
            dir.put_event(&event)
                .map_err(|error| ActionError::Io(error.to_string()))?;
            let link = crate::routes::confirming_link_for(&dir, &target);
            Ok(ActionResultRecord {
                operation_id,
                action: kind.as_str().to_string(),
                target: target_label(&target),
                detail: format!(
                    "Suppressed {} ({}); the engine rebuilds the suppression from this event on its next run",
                    target_label(&target),
                    suppression_reason_label(&reason_raw)
                ),
                at_ms,
                confirmed_link: link,
            })
        }
        ActionKind::Record => {
            let target_raw = fields.get("target").cloned().unwrap_or_default();
            let outcome_raw = fields.get("outcome").cloned().unwrap_or_default();
            let note = fields.get("note").cloned().unwrap_or_default();
            if note.chars().count() > MAX_NOTE_CHARS {
                return Err(ActionError::BadRequest(format!(
                    "The note is longer than {MAX_NOTE_CHARS} characters; nothing was written."
                )));
            }
            let target = crate::data::parse_target(&target_raw).ok_or_else(|| {
                ActionError::BadRequest(
                    "The record target is not an address or a publisher domain; nothing was written."
                        .to_string(),
                )
            })?;
            let outcome = Outcome::parse(&outcome_raw).ok_or_else(|| {
                ActionError::BadRequest(
                    "The outcome is not in the engine's outcome vocabulary; nothing was written."
                        .to_string(),
                )
            })?;
            let (email, publisher) = match &target {
                Target::Email(email) => (Some(email.clone()), None),
                Target::Publisher(key) => (None, Some(key.clone())),
            };
            let event = Event::Outcome {
                at: now,
                email,
                publisher,
                outcome,
                note: if note.trim().is_empty() {
                    None
                } else {
                    Some(note)
                },
            };
            dir.put_event(&event)
                .map_err(|error| ActionError::Io(error.to_string()))?;
            let link = crate::routes::confirming_link_for(&dir, &target);
            Ok(ActionResultRecord {
                operation_id,
                action: kind.as_str().to_string(),
                target: target_label(&target),
                detail: format!(
                    "Recorded outcome {} for {}",
                    outcome.as_string(),
                    target_label(&target)
                ),
                at_ms,
                confirmed_link: link,
            })
        }
        ActionKind::ResetHealth => {
            let confirmation = fields.get("confirmation").cloned().unwrap_or_default();
            if confirmation != crate::pages::domains::RESET_PHRASE {
                return Err(ActionError::BadRequest(format!(
                    "The confirmation must be exactly {}; nothing was written.",
                    crate::pages::domains::RESET_PHRASE
                )));
            }
            dir.put_event(&Event::HealthReset { at: now })
                .map_err(|error| ActionError::Io(error.to_string()))?;
            Ok(ActionResultRecord {
                operation_id,
                action: kind.as_str().to_string(),
                target: crate::pages::domains::RESET_PHRASE.to_string(),
                detail: "Sending domain reset to warming; the same durable event the CLI's reset-health writes".to_string(),
                at_ms,
                confirmed_link: "/domains".to_string(),
            })
        }
    }
}

/// The durable event a suppression writes: the reason maps onto the engine's
/// own vocabulary exactly as [`apex_sales::store::implied_suppression`]
/// reads it back.
pub fn suppression_event(target: &Target, reason: &str, now: i64) -> Result<Event, ActionError> {
    let (email, publisher) = match target {
        Target::Email(email) => (Some(email.clone()), None),
        Target::Publisher(key) => (None, Some(key.clone())),
    };
    match reason {
        "operator" => Ok(Event::Suppress {
            at: now,
            email,
            publisher,
        }),
        "opt_out" => match email {
            Some(email) => Ok(Event::Outcome {
                at: now,
                email: Some(email),
                publisher: None,
                outcome: Outcome::Unsubscribe,
                note: Some("operator-recorded opt out".to_string()),
            }),
            None => Err(ActionError::BadRequest(
                "An opt-out suppression needs an address target; a publisher key cannot opt out. Nothing was written.".to_string(),
            )),
        },
        "hard_bounce" => match email {
            Some(email) => Ok(Event::Outcome {
                at: now,
                email: Some(email),
                publisher: None,
                outcome: Outcome::HardBounce,
                note: Some("operator-recorded hard bounce".to_string()),
            }),
            None => Err(ActionError::BadRequest(
                "A hard-bounce suppression needs an address target. Nothing was written."
                    .to_string(),
            )),
        },
        "complaint" => Ok(Event::Outcome {
            at: now,
            email,
            publisher,
            outcome: Outcome::Complaint,
            note: Some("operator-recorded complaint".to_string()),
        }),
        other => Err(ActionError::BadRequest(format!(
            "Unknown suppression reason {other}; the vocabulary is opt_out, complaint, hard_bounce, operator. Nothing was written."
        ))),
    }
}

fn target_label(target: &Target) -> String {
    match target {
        Target::Email(email) => email.clone(),
        Target::Publisher(key) => key.clone(),
    }
}

fn suppression_reason_label(reason: &str) -> &'static str {
    match reason {
        "opt_out" => "opt out",
        "complaint" => "spam complaint",
        "hard_bounce" => "hard bounce",
        _ => "operator block",
    }
}

/// The suppression reason the engine will rebuild for a target, used by the
/// confirming link helper.
pub fn engine_reason(reason: &str) -> SuppressionReason {
    match reason {
        "opt_out" => SuppressionReason::OptOut,
        "complaint" => SuppressionReason::Complaint,
        "hard_bounce" => SuppressionReason::HardBounce,
        _ => SuppressionReason::Operator,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WebConfig;
    use crate::state::FixedClock;
    use std::sync::Arc;

    fn state(dir: &std::path::Path) -> WebState {
        let mut env = Env::new();
        env.insert("APEX_SALES_DIR".into(), dir.display().to_string());
        env.insert("APEX_SALES_WEB_TOKEN".into(), "t".into());
        WebState::new(WebConfig::from_env(&env).unwrap())
            .with_clock(Arc::new(FixedClock::new(1_792_000_000_000)))
    }

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn the_form_parser_decodes_and_first_wins() {
        let parsed = parse_form(b"a=1&b=x%20y&a=2&c=%C3%A9&b=").unwrap();
        assert_eq!(parsed.get("a").map(String::as_str), Some("1"));
        assert_eq!(parsed.get("b").map(String::as_str), Some("x y"));
        assert_eq!(parsed.get("c").map(String::as_str), Some("é"));
        assert!(parse_form(b"%GG").is_err());
    }

    #[test]
    fn approve_writes_one_approval_and_replays_idempotently() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-act-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = state(&dir);
        let first = fields(&[
            ("touch_id", "t0123456789abcdef"),
            ("idempotency_key", &"a".repeat(32)),
        ]);
        let (record, duplicate) = execute(&state, ActionKind::Approve, &first).unwrap();
        assert!(!duplicate);
        assert_eq!(record.confirmed_link, "/plan");
        let line = std::fs::read_to_string(dir.join("approvals.jsonl")).unwrap();
        assert_eq!(line.lines().count(), 1, "exactly one approval line");
        let (again, duplicate) = execute(&state, ActionKind::Approve, &first).unwrap();
        assert!(duplicate);
        assert_eq!(again.operation_id, record.operation_id);
        assert_eq!(
            std::fs::read_to_string(dir.join("approvals.jsonl"))
                .unwrap()
                .lines()
                .count(),
            1,
            "a replay writes nothing"
        );
        // The same key with a different touch id is a conflict.
        let conflicting = fields(&[
            ("touch_id", "tffffffffffffffff"),
            ("idempotency_key", &"a".repeat(32)),
        ]);
        assert_eq!(
            execute(&state, ActionKind::Approve, &conflicting),
            Err(ActionError::Conflict)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn suppress_writes_the_events_the_implied_rules_read_back() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-act2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = state(&dir);
        let (record, _) = execute(
            &state,
            ActionKind::Suppress,
            &fields(&[
                ("target", "Spam@Else.Example"),
                ("reason", "opt_out"),
                ("idempotency_key", &"b".repeat(32)),
            ]),
        )
        .unwrap();
        assert!(record.detail.contains("opt out"));
        let events = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
        assert!(events.contains("\"outcome\":\"unsubscribe\""), "{events}");
        assert!(events.contains("spam@else.example"));
        // A publisher-only opt out is refused and writes nothing new.
        let before = events.lines().count();
        let refused = execute(
            &state,
            ActionKind::Suppress,
            &fields(&[
                ("target", "studio.example"),
                ("reason", "opt_out"),
                ("idempotency_key", &"c".repeat(32)),
            ]),
        );
        assert!(matches!(refused, Err(ActionError::BadRequest(_))));
        assert_eq!(
            std::fs::read_to_string(dir.join("events.jsonl"))
                .unwrap()
                .lines()
                .count(),
            before
        );
        let (record, _) = execute(
            &state,
            ActionKind::Suppress,
            &fields(&[
                ("target", "studio.example"),
                ("reason", "operator"),
                ("idempotency_key", &"d".repeat(32)),
            ]),
        )
        .unwrap();
        assert_eq!(record.confirmed_link, "/publishers/studio.example");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn record_is_bounded_and_mirrors_the_cli() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-act3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = state(&dir);
        let long_note = "x".repeat(MAX_NOTE_CHARS + 1);
        let refused = execute(
            &state,
            ActionKind::Record,
            &fields(&[
                ("target", "a@b.io"),
                ("outcome", "soft_bounce"),
                ("note", &long_note),
                ("idempotency_key", &"e".repeat(32)),
            ]),
        );
        assert!(matches!(refused, Err(ActionError::BadRequest(_))));
        assert!(!dir.join("events.jsonl").exists());
        let (record, _) = execute(
            &state,
            ActionKind::Record,
            &fields(&[
                ("target", "a@b.io"),
                ("outcome", "reply:question"),
                ("note", "asked about pricing"),
                ("idempotency_key", &"f".repeat(32)),
            ]),
        )
        .unwrap();
        assert_eq!(record.action, "record");
        let events = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
        assert!(events.contains("\"outcome\":\"reply:question\""));
        assert!(events.contains("asked about pricing"));
        assert!(execute(
            &state,
            ActionKind::Record,
            &fields(&[
                ("target", "a@b.io"),
                ("outcome", "nonsense"),
                ("idempotency_key", &"2".repeat(32)),
            ]),
        )
        .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reset_health_requires_the_exact_phrase() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-act4-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = state(&dir);
        let refused = execute(
            &state,
            ActionKind::ResetHealth,
            &fields(&[
                ("confirmation", "reset health"),
                ("idempotency_key", &"0".repeat(32)),
            ]),
        );
        assert!(matches!(refused, Err(ActionError::BadRequest(_))));
        assert!(!dir.join("events.jsonl").exists());
        let (record, _) = execute(
            &state,
            ActionKind::ResetHealth,
            &fields(&[
                ("confirmation", "RESET HEALTH"),
                ("idempotency_key", &"1".repeat(32)),
            ]),
        )
        .unwrap();
        assert_eq!(record.confirmed_link, "/domains");
        let events = std::fs::read_to_string(dir.join("events.jsonl")).unwrap();
        assert!(events.contains("\"type\":\"health_reset\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_idempotency_key_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-act5-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = state(&dir);
        let refused = execute(
            &state,
            ActionKind::ResetHealth,
            &fields(&[("confirmation", "RESET HEALTH")]),
        );
        assert!(matches!(refused, Err(ActionError::BadRequest(_))));
        assert!(!dir.join("events.jsonl").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kinds_round_trip_and_engine_reasons_map() {
        for kind in [
            ActionKind::Approve,
            ActionKind::Suppress,
            ActionKind::Record,
            ActionKind::ResetHealth,
        ] {
            assert_eq!(ActionKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ActionKind::parse("nope"), None);
        assert_eq!(engine_reason("opt_out"), SuppressionReason::OptOut);
        assert_eq!(engine_reason("complaint"), SuppressionReason::Complaint);
        assert_eq!(engine_reason("hard_bounce"), SuppressionReason::HardBounce);
        assert_eq!(engine_reason("operator"), SuppressionReason::Operator);
    }
}
