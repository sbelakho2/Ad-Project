//! `/replies`: ingested responses (human replies, bounces, complaints,
//! unsubscribe requests) with the classification exactly as recorded, and
//! per-row record/suppress actions that mirror the engine's implied rules.

use apex_sales::model::{Outcome, ReplyClass};
use apex_sales::store::Event;
use apex_ui::components::{
    csrf_hidden, data_table, details, field, notice, panel, select, Cell, EmptyState,
};
use apex_ui::format::{or_not_available, utc_datetime};
use apex_ui::status::StatusKind;

use crate::data::RequestData;
use crate::pages::shared::{self, PageMeta};
use crate::signing::random_hex_32;

/// The most response rows shown at once.
pub const REPLIES_LIMIT: usize = 200;
/// The most per-response action blocks rendered at once.
pub const ACTIONS_LIMIT: usize = 50;

/// Render `/replies`.
pub fn render(meta: &PageMeta, data: &RequestData) -> String {
    let rows = rows(data);
    let total = rows.len();
    let shown: Vec<&ResponseRow> = rows.iter().take(REPLIES_LIMIT).collect();
    let table_rows: Vec<Vec<Cell>> = shown.iter().map(|row| table_row(row)).collect();
    let empty = EmptyState {
        reason: "No responses are recorded yet. Ingesting stays in the CLI: apex-sales ingest reply.eml classifies a message and records it (apex-sales ingest is fed from the mailbox; this page only reads the result).".to_string(),
        action: Some(("Read the plan".to_string(), "/plan".to_string())),
    };
    let table = data_table(
        Some("Ingested responses"),
        &[
            "Received (UTC)",
            "From",
            "Classification (as recorded)",
            "Rationale",
            "Publisher",
            "Stage",
        ],
        &table_rows,
        Some(empty),
        Some(&shared::count_note(shown.len(), total, "responses")),
    );
    let body = format!(
        "{}\n{}\n{}\n{}\n{}",
        notice(
            StatusKind::Neutral,
            "Ingesting stays in the CLI",
            "apex-sales ingest <file.eml> classifies a reply or bounce and records it. The actions here record what a person concluded, or suppress a target, without re-classifying anything.",
            None,
        ),
        table,
        actions_section(meta, data, &shown),
        "<p class=\"table-note\">The classification column shows the outcome value exactly as the ingest recorded it (reply:* values are the classifier's classes); the rationale is the classifier's own sentence.</p>",
        "<p class=\"table-note\">Suppressing mirrors the engine's implied suppression rules: opt out records an unsubscribe outcome (implied opt-out), hard bounce records a hard bounce, complaint records a complaint (publisher-wide), and operator block writes the CLI's suppress event.</p>"
    );
    shared::page(meta, &body)
}

/// One details block per response with a matched address: the record-outcome
/// and suppress forms for that response. Details keep the page compact while
/// every form stays a real form (a table cell never carries one).
fn actions_section(meta: &PageMeta, _data: &RequestData, shown: &[&ResponseRow]) -> String {
    let actionable: Vec<&&ResponseRow> = shown
        .iter()
        .filter(|row| row.email.is_some())
        .take(ACTIONS_LIMIT)
        .collect();
    if actionable.is_empty() {
        return String::new();
    }
    let mut blocks = String::new();
    for row in &actionable {
        let email = row.email.clone().unwrap_or_default();
        let classification = row.outcome.as_string();
        blocks.push_str(&details(
            &format!(
                "Act on {} — received {} ({classification})",
                email,
                utc_datetime(row.at * 1000)
            ),
            &format!(
                "{}\n{}",
                suppress_form(meta, &email, row.outcome),
                record_form(meta, &email)
            ),
            false,
        ));
        blocks.push('\n');
    }
    panel(
        "Record outcomes and suppressions",
        &format!(
            "{blocks}<p class=\"table-note\">{} of {} responses have a matched address; the newest {} carry the forms here. Each submission reuses the engine's own events and an idempotency key.</p>",
            shown.iter().filter(|row| row.email.is_some()).count(),
            shown.len(),
            actionable.len(),
        ),
        None,
    )
}

/// One ingested response.
#[derive(Debug, Clone, PartialEq)]
pub struct ResponseRow {
    /// When it was recorded (Unix seconds).
    pub at: i64,
    /// The event's outcome, exactly as recorded.
    pub outcome: Outcome,
    /// The recorded note (the classifier's rationale for replies).
    pub note: Option<String>,
    /// The matched contact, when any.
    pub email: Option<String>,
    /// The publisher key resolved through the contact, when any.
    pub publisher: Option<String>,
    /// The publisher's furthest reached stage label, when any.
    pub stage: Option<&'static str>,
}

/// Whether an outcome belongs on this page.
pub fn is_response(outcome: Outcome) -> bool {
    matches!(
        outcome,
        Outcome::Reply(_)
            | Outcome::HardBounce
            | Outcome::SoftBounce
            | Outcome::Complaint
            | Outcome::Unsubscribe
            | Outcome::AutoReply
    )
}

/// Collect the response rows, newest first.
pub fn rows(data: &RequestData) -> Vec<ResponseRow> {
    let mut rows: Vec<ResponseRow> = data
        .snap
        .events
        .iter()
        .filter_map(|event| {
            let Event::Outcome {
                at,
                email,
                publisher,
                outcome,
                note,
            } = event
            else {
                return None;
            };
            if !is_response(*outcome) {
                return None;
            }
            let key = publisher.clone().or_else(|| {
                email
                    .as_deref()
                    .and_then(|email| data.snap.contacts.get(email))
                    .map(|contact| contact.publisher_key.clone())
            });
            let stage = key.as_deref().and_then(|key| {
                data.learned
                    .stages
                    .get(key)
                    .map(|stages| super::overview::stage_label(stages.reached()))
            });
            Some(ResponseRow {
                at: *at,
                outcome: *outcome,
                note: note.clone(),
                email: email.clone(),
                publisher: key,
                stage,
            })
        })
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.at));
    rows
}

/// The suppress reason implied by an outcome, mirroring
/// [`apex_sales::store::implied_suppression`].
pub fn implied_reason(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Reply(ReplyClass::OptOut) | Outcome::Unsubscribe => "opt_out",
        Outcome::HardBounce => "hard_bounce",
        Outcome::Complaint => "complaint",
        _ => "operator",
    }
}

fn table_row(row: &ResponseRow) -> Vec<Cell> {
    let classification = format!(
        "{} — {}",
        row.outcome.as_string(),
        super::publisher::outcome_label(row.outcome)
    );
    let publisher_cell = match &row.publisher {
        Some(key) => Cell::Link(key.clone(), format!("/publishers/{key}")),
        None => Cell::Muted(or_not_available("no matching contact")),
    };
    let stage_cell = match row.stage {
        Some(stage) => Cell::Text(stage.to_string()),
        None => Cell::Muted(or_not_available("no publisher record")),
    };
    let from = row
        .email
        .clone()
        .map(Cell::Mono)
        .unwrap_or_else(|| Cell::Muted(or_not_available("sender not matched")));
    vec![
        Cell::Text(utc_datetime(row.at * 1000)),
        from,
        Cell::Text(classification),
        match &row.note {
            Some(note) => Cell::Text(note.clone()),
            None => Cell::Muted(or_not_available("no note recorded")),
        },
        publisher_cell,
        stage_cell,
    ]
}

fn suppress_form(meta: &PageMeta, email: &str, outcome: Outcome) -> String {
    let options = [
        ("opt_out", "Opt out (unsubscribe)"),
        ("complaint", "Spam complaint (publisher-wide)"),
        ("hard_bounce", "Hard bounce"),
        ("operator", "Operator block"),
    ];
    let selected = implied_reason(outcome);
    let scoped: String = email
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    let reason_id = format!("suppress-reason-{scoped}");
    format!(
        "<form method=\"post\" action=\"/actions/suppress\">\n{}{}{}{}<button class=\"action action-secondary\" type=\"submit\">Suppress</button></form>",
        csrf_hidden(&meta.csrf),
        hidden("idempotency_key", &random_hex_32()),
        hidden("target", email),
        field(
            &reason_id,
            "Reason",
            None,
            None,
            &select(
                &reason_id,
                "reason",
                &options
                    .iter()
                    .map(|(value, label)| ((*value).to_string(), (*label).to_string()))
                    .collect::<Vec<_>>(),
                Some(selected),
            ),
        )
    )
}

fn record_form(meta: &PageMeta, email: &str) -> String {
    let outcome_options: Vec<(String, String)> = [
        Outcome::SoftBounce,
        Outcome::Reply(ReplyClass::Positive),
        Outcome::Reply(ReplyClass::Referral),
        Outcome::Reply(ReplyClass::Question),
        Outcome::Reply(ReplyClass::NotNow),
        Outcome::Reply(ReplyClass::NotInterested),
        Outcome::Reply(ReplyClass::OptOut),
        Outcome::Reply(ReplyClass::Unclear),
        Outcome::AuditRequested,
        Outcome::Connected,
        Outcome::Paid,
    ]
    .into_iter()
    .map(|outcome| {
        let value = outcome.as_string();
        (value, super::publisher::outcome_label(outcome))
    })
    .collect();
    let scoped: String = email
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect();
    let outcome_id = format!("record-outcome-{scoped}");
    format!(
        "<form method=\"post\" action=\"/actions/record\">\n{}{}{}{}<button class=\"action action-secondary\" type=\"submit\">Record outcome</button></form>",
        csrf_hidden(&meta.csrf),
        hidden("idempotency_key", &random_hex_32()),
        hidden("target", email),
        field(
            &outcome_id,
            "Outcome",
            None,
            None,
            &select(&outcome_id, "outcome", &outcome_options, None),
        )
    )
}

fn hidden(name: &str, value: &str) -> String {
    super::publisher::hidden(name, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_sales::model::DAY;

    const NOW: i64 = 20_745 * DAY + 10 * 3600;

    #[test]
    fn implied_reasons_mirror_the_engine_rules() {
        assert_eq!(
            implied_reason(Outcome::Reply(ReplyClass::OptOut)),
            "opt_out"
        );
        assert_eq!(implied_reason(Outcome::Unsubscribe), "opt_out");
        assert_eq!(implied_reason(Outcome::HardBounce), "hard_bounce");
        assert_eq!(implied_reason(Outcome::Complaint), "complaint");
        assert_eq!(
            implied_reason(Outcome::Reply(ReplyClass::Positive)),
            "operator"
        );
        assert!(is_response(Outcome::Reply(ReplyClass::Unclear)));
        assert!(is_response(Outcome::AutoReply));
        assert!(!is_response(Outcome::Paid));
    }

    #[test]
    fn rows_render_with_actions_and_pre_selected_reasons() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-rep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = apex_sales::store::StateDir::open(&dir).unwrap();
        let mut publisher = apex_sales::model::Publisher::new("studio.example", "Studio");
        publisher.domain = Some("studio.example".into());
        store.put_publisher(&publisher).unwrap();
        store
            .put_contact(&apex_sales::model::Contact {
                email: "maria@studio.example".into(),
                name: None,
                role: None,
                publisher_key: "studio.example".into(),
                kind: apex_sales::model::ContactKind::Named,
                source: "seed list".into(),
                consent: false,
            })
            .unwrap();
        store
            .put_event(&Event::Outcome {
                at: NOW - 3600,
                email: Some("maria@studio.example".into()),
                publisher: None,
                outcome: Outcome::Reply(ReplyClass::OptOut),
                note: Some("please remove me".into()),
            })
            .unwrap();
        let mut env = apex_sales::cli::Env::new();
        env.insert("APEX_SALES_DIR".to_string(), dir.display().to_string());
        let data = crate::data::load_from(&env, NOW).unwrap();
        let meta = PageMeta::new(
            "Replies",
            "Replies",
            Vec::new(),
            NOW,
            dir.display().to_string(),
            "a".repeat(32),
        );
        let html = render(&meta, &data);
        assert!(html.contains("reply:opt_out"));
        assert!(html.contains("please remove me"));
        assert!(html.contains("<option value=\"opt_out\" selected>"));
        assert!(html.contains("action=\"/actions/suppress\""));
        assert!(html.contains("action=\"/actions/record\""));
        assert!(html.contains("apex-sales ingest"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
