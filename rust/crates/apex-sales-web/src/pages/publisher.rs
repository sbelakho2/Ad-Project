//! `/publishers/{domain}`: one publisher's contacts, evidence, typed event
//! timeline, score breakdown, suppression banner and the action forms.

use apex_sales::compose::best_evidence;
use apex_sales::model::{Evidence, Outcome, Publisher, ReplyClass, UnixSecs};
use apex_sales::policy::SuppressionReason;
use apex_sales::scoring::features;
use apex_sales::signals::SizeBand;
use apex_sales::store::{implied_suppression, Event};
use apex_ui::components::{
    csrf_hidden, data_table, field, notice, panel, select, textarea, Cell, EmptyState,
};
use apex_ui::format::{or_not_available, percent, utc_datetime};
use apex_ui::status::StatusKind;

use crate::data::RequestData;
use crate::pages::shared::{self, status_cell, PageMeta};
use crate::signing::random_hex_32;

/// The longest accepted outcome note, mirroring the form's `maxlength`.
pub const MAX_NOTE_CHARS: usize = 500;

/// Render the publisher page; the route validates the domain and 404s when
/// no publisher carries the key before calling this.
pub fn render(meta: &PageMeta, data: &RequestData, key: &str) -> String {
    let Some(publisher) = data.snap.publishers.get(key) else {
        return crate::pages::errors::not_found(meta.as_of_ms / 1000, &meta.data_location);
    };
    let banner = suppression_banner(data, key);
    let facts = publisher_facts(data, publisher);
    let contacts = contacts_table(data, key);
    let evidence = evidence_table(publisher);
    let scores = score_tables(data, key);
    let timeline = timeline_table(data, key);
    let forms = actions_panel(meta, data, key);
    let body = format!("{banner}{facts}\n{contacts}\n{evidence}\n{scores}\n{timeline}\n{forms}");
    shared::page(meta, &body)
}

fn suppression_banner(data: &RequestData, key: &str) -> String {
    let Some((reason, at, publisher_level)) = origin_or_listed(data, key) else {
        return String::new();
    };
    let since = match at {
        Some(at) => format!("since {}", utc_datetime(at * 1000)),
        None => "date Not available (origin event not found)".to_string(),
    };
    let (title, body) = if publisher_level {
        (
            "This publisher is suppressed",
            format!(
                "Reason: {} ({}). No planned touch may pass the gate while this holds; the CLI records the same suppression from the same engine state.",
                super::leads::suppression_label(reason),
                since
            ),
        )
    } else {
        (
            "Some addresses at this publisher are suppressed",
            format!(
                "Reason: {} ({}). Suppressed contacts are skipped by the plan; other contacts at this publisher are unaffected.",
                super::leads::suppression_label(reason),
                since
            ),
        )
    };
    format!(
        "{}\n",
        notice(
            StatusKind::Failure,
            title,
            &body,
            Some(("Review the contacts", "#contacts")),
        )
    )
}

/// The suppression for `key` as the learner resolved it (publisher-level
/// first), with the first event that implies it and whether it is
/// publisher-wide.
pub fn origin_or_listed(
    data: &RequestData,
    key: &str,
) -> Option<(SuppressionReason, Option<UnixSecs>, bool)> {
    if let Some((_, reason)) = data
        .learned
        .suppressions
        .publishers()
        .iter()
        .find(|(p, _)| p == key)
    {
        return Some((*reason, first_implying_event(data, key), true));
    }
    let suppressions = data.suppression_index();
    let reason = data
        .snap
        .contacts
        .values()
        .filter(|contact| contact.publisher_key == key)
        .find_map(|contact| suppressions.check(contact))?;
    Some((reason, first_implying_event(data, key), false))
}

fn first_implying_event(data: &RequestData, key: &str) -> Option<UnixSecs> {
    let contact_at = |email: &str| {
        data.snap
            .contacts
            .get(email)
            .is_some_and(|contact| contact.publisher_key == key)
    };
    for event in &data.snap.events {
        match event {
            Event::Outcome {
                at,
                email,
                publisher,
                outcome,
                ..
            } => {
                if implied_suppression(*outcome).is_none() {
                    continue;
                }
                let applies =
                    publisher.as_deref() == Some(key) || email.as_deref().is_some_and(contact_at);
                if applies {
                    return Some(*at);
                }
            }
            Event::Suppress {
                at,
                email,
                publisher,
            } => {
                let applies =
                    publisher.as_deref() == Some(key) || email.as_deref().is_some_and(contact_at);
                if applies {
                    return Some(*at);
                }
            }
            _ => {}
        }
    }
    None
}

fn publisher_facts(data: &RequestData, publisher: &Publisher) -> String {
    let apps = publisher.apps.len();
    let max_ratings = publisher
        .max_rating_count()
        .map(|n| n.to_string())
        .unwrap_or_else(|| or_not_available("no store data"));
    let band = SizeBand::of(publisher.max_rating_count());
    let country = publisher
        .country
        .clone()
        .unwrap_or_else(|| or_not_available("no country"));
    let platform = super::leads::platform_of(publisher, &data.learned.mediation);
    let strongest = best_evidence(publisher)
        .map(|evidence| evidence.statement.clone())
        .unwrap_or_else(|| or_not_available("no evidence yet"));
    panel(
        "Publisher",
        &format!(
            "<p><strong>{}</strong> — {} / country {} / size band {} / platform {} / {} app(s), largest ratings count {} / inbound form: {}.</p>\
             <p>Strongest citable evidence: “{}”.</p>",
            apex_ui::escape::html_escape(&publisher.name),
            publisher
                .domain
                .clone()
                .unwrap_or_else(|| or_not_available("no domain")),
            apex_ui::escape::html_escape(&country),
            band.as_str(),
            apex_ui::escape::html_escape(&platform.1),
            apps,
            max_ratings,
            if publisher.inbound { "yes" } else { "no" },
            apex_ui::escape::html_escape(&strongest),
        ),
        None,
    )
}

fn contacts_table(data: &RequestData, key: &str) -> String {
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let suppressions = data.suppression_index();
    for (email, contact) in &data.snap.contacts {
        if contact.publisher_key != key {
            continue;
        }
        let status = match suppressions.check(contact) {
            Some(reason) => Cell::Status(
                format!("Suppressed ({})", super::leads::suppression_label(reason)),
                StatusKind::Failure,
            ),
            None => status_cell("Active", StatusKind::Verified),
        };
        rows.push(vec![
            Cell::Mono(email.clone()),
            match &contact.name {
                Some(name) => Cell::Text(name.clone()),
                None => Cell::Muted(or_not_available("no name")),
            },
            match &contact.role {
                Some(role) => Cell::Text(role.clone()),
                None => Cell::Muted(or_not_available("no role")),
            },
            Cell::Text(contact.kind.as_str().to_string()),
            status,
            shared::yes_no(contact.consent),
            Cell::Text(contact.source.clone()),
        ]);
    }
    let empty = EmptyState {
        reason: "No contacts are recorded at this publisher.".to_string(),
        action: Some(("Import a seed list".to_string(), "/leads".to_string())),
    };
    format!(
        "<div id=\"contacts\">{}</div>",
        data_table(
            Some("Contacts"),
            &["Email", "Name", "Role", "Kind", "Status", "Consent", "Source"],
            &rows,
            Some(empty),
            Some(&shared::count_note(rows.len(), rows.len(), "contacts")),
        )
    )
}

fn evidence_table(publisher: &Publisher) -> String {
    let rows: Vec<Vec<Cell>> = publisher
        .evidence
        .iter()
        .map(|evidence: &Evidence| {
            vec![
                Cell::Text(evidence.kind.as_str().to_string()),
                Cell::Text(evidence.statement.clone()),
                Cell::Text(evidence.source.clone()),
                Cell::Text(utc_datetime(evidence.observed_at.saturating_mul(1000))),
            ]
        })
        .collect();
    let empty = EmptyState {
        reason: "No evidence is recorded yet; emails may only assert evidence, so nothing can cite this publisher until discovery finds something.".to_string(),
        action: Some(("See the CLI discovery step".to_string(), "/plan".to_string())),
    };
    data_table(
        Some("Evidence"),
        &["Kind", "Statement", "Source", "Observed at"],
        &rows,
        Some(empty),
        Some(&shared::count_note(
            publisher.evidence.len(),
            publisher.evidence.len(),
            "evidence entries",
        )),
    )
}

fn score_tables(data: &RequestData, key: &str) -> String {
    let publisher = match data.snap.publishers.get(key) {
        Some(publisher) => publisher,
        None => return String::new(),
    };
    let mut summary_rows: Vec<Vec<Cell>> = Vec::new();
    let mut contribution_rows: Vec<Vec<Cell>> = Vec::new();
    for (email, contact) in &data.snap.contacts {
        if contact.publisher_key != key {
            continue;
        }
        let fv = features(publisher, contact, &data.learned.mediation);
        let score = data.learned.scoring.predict(&fv.x);
        let logit = data.learned.scoring.logit_mean(&fv.x);
        let mut reasons = data.learned.scoring.reasons(&fv);
        reasons.sort_by(|a, b| {
            b.log_odds
                .abs()
                .total_cmp(&a.log_odds.abs())
                .then_with(|| a.feature.cmp(b.feature))
        });
        summary_rows.push(vec![
            Cell::Mono(email.clone()),
            Cell::Num(percent(score)),
            Cell::Num(format!("{logit:+.2}")),
            Cell::Text(
                reasons
                    .iter()
                    .take(3)
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; "),
            ),
        ]);
        let mut sorted = reasons;
        sorted.sort_by(|a, b| a.feature.cmp(b.feature));
        for reason in sorted {
            contribution_rows.push(vec![
                Cell::Mono(email.clone()),
                Cell::Text(reason.feature.to_string()),
                Cell::Num(format!("{:+.3}", reason.log_odds)),
                Cell::Text(reason.note),
            ]);
        }
    }
    let summary_empty = EmptyState {
        reason: "No contacts are recorded at this publisher, so there is no score to break down."
            .to_string(),
        action: None,
    };
    let contributions_empty = EmptyState {
        reason: "No feature contributions exist without a contact.".to_string(),
        action: None,
    };
    format!(
        "{}\n{}\n<p class=\"table-note\">The score is the lead model's predicted interest probability; each contribution is the posterior-mean weight times the feature value, in log-odds. Contributions sum to the lean log-odds shown.</p>",
        data_table(
            Some("Scores"),
            &["Contact", "Predicted interest", "Log-odds", "Top reasons"],
            &summary_rows,
            Some(summary_empty),
            Some(&shared::count_note(summary_rows.len(), summary_rows.len(), "contacts")),
        ),
        data_table(
            Some("Score breakdown by feature contribution"),
            &["Contact", "Feature", "Log-odds", "Basis"],
            &contribution_rows,
            Some(contributions_empty),
            None,
        )
    )
}

/// A human label for a recorded outcome.
pub fn outcome_label(outcome: Outcome) -> String {
    match outcome {
        Outcome::Sent => "Sent".to_string(),
        Outcome::HardBounce => "Hard bounce".to_string(),
        Outcome::SoftBounce => "Soft bounce".to_string(),
        Outcome::Complaint => "Spam complaint".to_string(),
        Outcome::Unsubscribe => "Unsubscribed".to_string(),
        Outcome::AutoReply => "Automatic reply".to_string(),
        Outcome::Reply(class) => format!("Reply: {}", reply_label(class)),
        Outcome::AuditRequested => "Audit requested".to_string(),
        Outcome::Connected => "Platform connected".to_string(),
        Outcome::Paid => "Paid".to_string(),
        Outcome::WatchSignup => "Setup Watch signup".to_string(),
    }
}

/// A human label for a reply classification.
pub fn reply_label(class: ReplyClass) -> &'static str {
    match class {
        ReplyClass::Positive => "interested",
        ReplyClass::Referral => "referral",
        ReplyClass::Question => "question",
        ReplyClass::NotNow => "not now",
        ReplyClass::NotInterested => "not interested",
        ReplyClass::OptOut => "opt out",
        ReplyClass::Unclear => "unclear (needs a person)",
    }
}

fn timeline_table(data: &RequestData, key: &str) -> String {
    let contact_at = |email: &str| {
        data.snap
            .contacts
            .get(email)
            .is_some_and(|contact| contact.publisher_key == key)
    };
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    for event in &data.snap.events {
        match event {
            Event::Touch {
                at,
                email,
                publisher,
                step,
                arm,
                message_id,
                local_hour,
                ..
            } => {
                if publisher != key && !contact_at(email) {
                    continue;
                }
                rows.push(vec![
                    Cell::Text(utc_datetime(at * 1000)),
                    Cell::Text(format!("Touch sent (step {})", step + 1)),
                    Cell::Mono(format!(
                        "to {email}; arm {arm}; message {message_id}; local hour {local_hour}"
                    )),
                ]);
            }
            Event::Outcome {
                at,
                email,
                publisher,
                outcome,
                note,
            } => {
                let applies =
                    publisher.as_deref() == Some(key) || email.as_deref().is_some_and(contact_at);
                if !applies {
                    continue;
                }
                rows.push(vec![
                    Cell::Text(utc_datetime(at * 1000)),
                    Cell::Text(outcome_label(*outcome)),
                    Cell::Text(if email.is_none() {
                        note.clone()
                            .unwrap_or_else(|| "publisher-level record".to_string())
                    } else {
                        format!(
                            "{} / {}",
                            email.clone().unwrap_or_default(),
                            note.clone().unwrap_or_else(|| "no note".to_string())
                        )
                    }),
                ]);
            }
            Event::Suppress {
                at,
                email,
                publisher,
            } => {
                let applies =
                    publisher.as_deref() == Some(key) || email.as_deref().is_some_and(contact_at);
                if !applies {
                    continue;
                }
                rows.push(vec![
                    Cell::Text(utc_datetime(at * 1000)),
                    Cell::Text("Suppressed by operator".to_string()),
                    Cell::Text(
                        email
                            .clone()
                            .or_else(|| publisher.clone())
                            .unwrap_or_else(|| "target not recorded".to_string()),
                    ),
                ]);
            }
            Event::HealthReset { .. } => {}
        }
    }
    let empty = EmptyState {
        reason: "No events touch this publisher yet; the timeline fills as the engine sends, records and suppresses.".to_string(),
        action: None,
    };
    data_table(
        Some("Event timeline"),
        &["When (UTC)", "Event", "Detail"],
        &rows,
        Some(empty),
        Some(&shared::count_note(rows.len(), rows.len(), "events")),
    )
}

fn actions_panel(meta: &PageMeta, data: &RequestData, key: &str) -> String {
    let idem_suppress = random_hex_32();
    let idem_record = random_hex_32();
    let mut targets = vec![("publisher".to_string(), format!("Publisher: {key}"))];
    for contact in data.snap.contacts.values() {
        if contact.publisher_key == key {
            targets.push((contact.email.clone(), format!("Contact: {}", contact.email)));
        }
    }
    let suppress = format!(
        "<form method=\"post\" action=\"/actions/suppress\">\n{}{}\n{}\n{}\n<p class=\"form-actions\"><button class=\"action action-primary\" type=\"submit\">Suppress this target</button></p></form>",
        csrf_hidden(&meta.csrf),
        hidden("idempotency_key", &idem_suppress),
        field(
            "suppress-target",
            "Target",
            Some("A mailbox suppresses that address; a publisher key suppresses everyone at it (operator block). Complaint suppression always covers the publisher."),
            None,
            &select("suppress-target", "target", &targets, None),
        ),
        field(
            "suppress-reason",
            "Reason",
            Some("Mapped onto the engine's own events: opt out and hard bounce record the matching outcome; complaint records a complaint (publisher-wide); operator block writes the CLI's suppress event."),
            None,
            &select(
                "suppress-reason",
                "reason",
                &[
                    ("opt_out".to_string(), "Opt out (records an unsubscribe)".to_string()),
                    ("complaint".to_string(), "Spam complaint (suppresses the publisher)".to_string()),
                    ("hard_bounce".to_string(), "Hard bounce (address-level)".to_string()),
                    ("operator".to_string(), "Operator block".to_string()),
                ],
                None,
            ),
        )
    );
    let outcome_options: Vec<(String, String)> = [
        Outcome::Sent,
        Outcome::HardBounce,
        Outcome::SoftBounce,
        Outcome::Complaint,
        Outcome::Unsubscribe,
        Outcome::AutoReply,
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
        Outcome::WatchSignup,
    ]
    .into_iter()
    .map(|outcome| {
        let value = outcome.as_string();
        let label = outcome_label(outcome);
        (value, label)
    })
    .collect();
    let record = format!(
        "<form method=\"post\" action=\"/actions/record\">\n{}{}\n{}\n{}\n{}\n<p class=\"form-actions\"><button class=\"action action-primary\" type=\"submit\">Record the outcome</button></p></form>",
        csrf_hidden(&meta.csrf),
        hidden("idempotency_key", &idem_record),
        field(
            "record-target",
            "Target",
            None,
            None,
            &select("record-target", "target", &targets, None),
        ),
        field(
            "record-outcome",
            "Outcome",
            Some("The engine's full outcome vocabulary; reply:* values are the classified human replies."),
            None,
            &select("record-outcome", "outcome", &outcome_options, None),
        ),
        field(
            "record-note",
            "Note",
            Some(&format!("At most {MAX_NOTE_CHARS} characters; stored verbatim on the event.")),
            None,
            &textarea("record-note", "note", None, 3, Some(MAX_NOTE_CHARS)),
        )
    );
    let note = panel(
        "Record outcomes by hand",
        "<p>Reply ingestion stays in the CLI: <code>apex-sales ingest reply.eml</code> classifies a message and records it. Use this form when a person read the reply or an outcome arrived outside the classification.</p>",
        None,
    );
    format!(
        "{}\n{}\n{}\n{}",
        panel("Suppress a target", &suppress, None),
        panel("Record an outcome", &record, None),
        note,
        "<p class=\"table-note\">Both forms write onto the engine's durable event history with an idempotency key; a replayed submission never duplicates the event.</p>"
    )
}

pub(crate) fn hidden(name: &str, value: &str) -> String {
    format!(
        "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
        apex_ui::escape::attr(name),
        apex_ui::escape::attr(value)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use apex_sales::model::{App, Contact, ContactKind, DAY};
    use apex_sales::store::StateDir;

    const NOW: i64 = 20_745 * DAY + 10 * 3600;

    pub(crate) fn seeded(dir: &std::path::Path) -> RequestData {
        let store = StateDir::open(dir).unwrap();
        let mut publisher = Publisher::new("studio.example", "Studio \"Q\"");
        publisher.domain = Some("studio.example".into());
        publisher.country = Some("FI".into());
        publisher.apps.push(App {
            store: apex_sales::model::Store::AppStore,
            id: "42".into(),
            name: "Tile Quest".into(),
            genre: Some("Games".into()),
            rating_count: Some(20_000),
            free: Some(true),
            days_since_update: Some(3),
        });
        publisher.ads_txt_text = None;
        publisher.ads_txt_missing = true;
        publisher.evidence.push(Evidence {
            id: "ads-ads_txt_missing".into(),
            kind: apex_sales::model::EvidenceKind::AppAdsTxt,
            source: "https://studio.example/app-ads.txt".into(),
            statement: "https://studio.example/app-ads.txt was not found".into(),
            observed_at: NOW - 2 * DAY,
        });
        store.put_publisher(&publisher).unwrap();
        store
            .put_contact(&Contact {
                email: "maria@studio.example".into(),
                name: Some("Maria".into()),
                role: Some("CEO".into()),
                publisher_key: "studio.example".into(),
                kind: ContactKind::Named,
                source: "seed list".into(),
                consent: false,
            })
            .unwrap();
        store
            .put_event(&Event::Touch {
                at: NOW - DAY,
                email: "maria@studio.example".into(),
                publisher: "studio.example".into(),
                from: None,
                step: 0,
                arm: "t0-finding".into(),
                message_id: "m1".into(),
                local_hour: 10,
                predicted: Some(0.05),
            })
            .unwrap();
        store
            .put_event(&Event::Outcome {
                at: NOW - DAY + 3600,
                email: Some("maria@studio.example".into()),
                publisher: None,
                outcome: Outcome::Reply(ReplyClass::Positive),
                note: Some("yes, run it".into()),
            })
            .unwrap();
        let mut env = apex_sales::cli::Env::new();
        env.insert("APEX_SALES_DIR".to_string(), dir.display().to_string());
        crate::data::load_from(&env, NOW).unwrap()
    }

    #[test]
    fn the_page_shows_contacts_evidence_timeline_and_forms() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-pub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let data = seeded(&dir);
        let meta = PageMeta::new(
            "studio.example",
            "Leads",
            vec!["studio.example".to_string()],
            data.now,
            dir.display().to_string(),
            "a".repeat(32),
        );
        let html = render(&meta, &data, "studio.example");
        assert!(html.contains("maria@studio.example"));
        assert!(html.contains("was not found"));
        assert!(html.contains("Touch sent (step 1)"));
        assert!(html.contains("Reply: interested"));
        assert!(html.contains("Score breakdown by feature contribution"));
        assert!(html.contains("action=\"/actions/suppress\""));
        assert!(html.contains("action=\"/actions/record\""));
        assert!(html.contains("name=\"csrf_token\" value=\"aaaa"));
        assert_eq!(html.matches("<h1").count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_suppressed_publisher_gets_the_banner_with_reason_and_date() {
        let dir = std::env::temp_dir().join(format!("apex-sales-web-pub2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = seeded(&dir);
        let store = StateDir::open(&dir).unwrap();
        store
            .put_event(&Event::Suppress {
                at: NOW - 3600,
                email: None,
                publisher: Some("studio.example".into()),
            })
            .unwrap();
        let mut env = apex_sales::cli::Env::new();
        env.insert("APEX_SALES_DIR".to_string(), dir.display().to_string());
        let data = crate::data::load_from(&env, NOW).unwrap();
        let meta = PageMeta::new(
            "studio.example",
            "Leads",
            Vec::new(),
            data.now,
            ".apex".to_string(),
            String::new(),
        );
        let html = render(&meta, &data, "studio.example");
        assert!(
            html.contains("This publisher is suppressed")
                || html.contains("Some addresses at this publisher are suppressed")
        );
        assert!(html.contains("operator block"));
        assert!(
            html.contains("19 Oct 2026, 09:00 UTC"),
            "banner carries the date"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn outcome_labels_cover_the_vocabulary() {
        assert_eq!(
            outcome_label(Outcome::Reply(ReplyClass::OptOut)),
            "Reply: opt out"
        );
        assert_eq!(outcome_label(Outcome::HardBounce), "Hard bounce");
        assert_eq!(outcome_label(Outcome::AuditRequested), "Audit requested");
    }
}
