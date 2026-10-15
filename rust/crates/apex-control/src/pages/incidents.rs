//! Incidents `/control/incidents` and `/control/incidents/{id}` (pages 63
//! and 64): impact queue, the assembled case, the communication draft, the
//! before-intervention preview and the scoped action forms.

use apex_analytics::projections::OperatorQueueRow;
use apex_ui::components::{data_table, notice, Cell, EmptyState};
use apex_ui::escape::html_escape;
use apex_ui::status::StatusKind;

use crate::copy;
use crate::fixtures::{IncidentCase, InterventionPreview};

use super::overview::age_label;
use super::{action_form, document, header, kpis, ActionFormSpec, PageMeta};

/// Render the incident queue.
pub fn render_list(meta: &PageMeta, incidents: Result<Vec<OperatorQueueRow>, String>) -> String {
    let mut body = header(meta, "Incidents");
    match incidents {
        Err(reason) => {
            body.push_str(&notice(
                StatusKind::Unavailable,
                copy::TELEMETRY_ABSENT,
                &format!("The incident queue is not readable: {reason}"),
                Some((copy::TELEMETRY_ABSENT_ACTION, "/control/system")),
            ));
        }
        Ok(rows) => {
            let critical = rows.iter().filter(|row| row.severity == "critical").count();
            let warning = rows.iter().filter(|row| row.severity == "warning").count();
            body.push_str(&kpis(&[
                (
                    "Open incidents",
                    &rows.len().to_string(),
                    &format!("{critical} critical / {warning} warning"),
                    None,
                ),
                (
                    "Oldest",
                    &rows
                        .iter()
                        .map(|row| row.age_seconds)
                        .max()
                        .map(age_label)
                        .unwrap_or_else(|| "0m".to_string()),
                    "Age of the oldest open item",
                    None,
                ),
                (
                    "Unassigned",
                    &rows
                        .iter()
                        .filter(|row| row.owner.is_none())
                        .count()
                        .to_string(),
                    "Acknowledging assigns responsibility",
                    None,
                ),
            ]));
            let cells: Vec<Vec<Cell>> = rows
                .iter()
                .map(|row| {
                    vec![
                        Cell::Status(
                            format!(
                                "{} / {}",
                                capitalize(&row.severity),
                                age_label(row.age_seconds)
                            ),
                            apex_ui::status::severity(&row.severity),
                        ),
                        Cell::Link(
                            row.object_id.clone(),
                            format!("/control/incidents/{}", row.object_id),
                        ),
                        Cell::Text(row.publisher_id.clone()),
                        Cell::Text(format!(
                            "{} / {}",
                            row.owner
                                .clone()
                                .unwrap_or_else(|| "Unassigned".to_string()),
                            super::overview::humanize(&row.next_action)
                        )),
                    ]
                })
                .collect();
            body.push_str(&data_table(
                Some("Open incidents by severity"),
                &[
                    "Severity / age",
                    "Incident",
                    "Customer impact",
                    "Owner / update",
                ],
                &cells,
                Some(EmptyState {
                    reason: copy::EMPTY_OPERATIONS_HEADLINE.to_string(),
                    action: Some((
                        copy::EMPTY_OPERATIONS_ACTION.to_string(),
                        "/control/ingestion".to_string(),
                    )),
                }),
                Some("Impact before uptime: each open incident names its customer impact."),
            ));
        }
    }
    document(meta, "Incidents", &["Incidents"], &body)
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// The incident detail page data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncidentDetail {
    /// The assembled case, or the reason it cannot be assembled.
    pub case: Result<IncidentCase, String>,
    /// The before-intervention preview, with its own availability.
    pub preview: Option<Result<InterventionPreview, String>>,
    /// The scoped action forms the actor may submit.
    pub forms: Vec<ActionFormSpec>,
    /// Whether any sensitive action needs a fresh sign-in.
    pub step_up_note: bool,
}

/// Render the incident detail.
pub fn render_detail(meta: &PageMeta, incident_id: &str, detail: &IncidentDetail) -> String {
    let title = format!("Incident {incident_id}");
    let mut body = header(meta, &title);
    match &detail.case {
        Err(reason) => {
            body.push_str(&notice(
                StatusKind::Unavailable,
                copy::TELEMETRY_ABSENT,
                &format!("The incident record is not readable: {reason}"),
                None,
            ));
        }
        Ok(case) => {
            body.push_str(&format!(
                "<p class=\"panel-body\">{} {}</p>",
                apex_ui::status::chip(
                    &format!(
                        "{} / {}",
                        capitalize(&case.severity),
                        age_label(case.age_seconds)
                    ),
                    apex_ui::status::severity(&case.severity),
                ),
                html_escape(&case.headline)
            ));
            body.push_str(&kpis(&[
                (
                    "Last confirmed state",
                    case.last_confirmed_state
                        .as_deref()
                        .unwrap_or("Not available"),
                    "The last platform read that succeeded",
                    None,
                ),
                (
                    "Failed operation",
                    case.failed_operation.as_deref().unwrap_or("Not available"),
                    "What did not complete",
                    None,
                ),
                (
                    "Owner",
                    case.owner.as_deref().unwrap_or("Unassigned"),
                    &case
                        .next_update
                        .clone()
                        .map(|update| format!("Next update {update}"))
                        .unwrap_or_else(|| "No update committed".to_string()),
                    None,
                ),
            ]));
            body.push_str(&format!(
                "<p class=\"panel-body\">Affected publisher <code>{}</code>{} {}</p>",
                html_escape(&case.publisher_id),
                case.experiment
                    .as_deref()
                    .map(|experiment| format!(
                        " / experiment <code>{}</code>",
                        html_escape(experiment)
                    ))
                    .unwrap_or_default(),
                case.scope
                    .as_deref()
                    .map(|scope| format!(" / scope {scope}"))
                    .unwrap_or_default()
            ));
            body.push_str(&format!(
                "<p class=\"table-note\">Occurrence: {}. The platform did not confirm the stop; the test is not marked restored.</p>",
                html_escape(case.occurred_display.as_deref().unwrap_or("not recorded"))
            ));
            body.push_str(&section_draft(&case.communication_draft));
            if let Some(traffic) = case.affects_traffic {
                body.push_str(&format!(
                    "<p class=\"table-note\">{}</p>",
                    html_escape(if traffic {
                        "This operation can affect active traffic."
                    } else {
                        "This operation does not affect active traffic."
                    })
                ));
            }
        }
    }
    if detail.step_up_note {
        body.push_str(&notice(
            StatusKind::Attention,
            copy::STEP_UP_HEADLINE,
            "Sensitive actions on this incident need a fresh sign-in (password and authenticator code) before they run.",
            None,
        ));
    }
    if let Some(preview) = &detail.preview {
        body.push_str(&render_preview(preview));
    }
    for form in &detail.forms {
        body.push_str(&format!(
            "<section class=\"panel\"><h3 class=\"panel-heading\">{}</h3>{}</section>",
            html_escape(&form.label),
            action_form(form)
        ));
    }
    body.push_str(&render_timeline(&detail.case));
    document(
        meta,
        &format!("Incident {incident_id}"),
        &["Incidents", incident_id],
        &body,
    )
}

fn section_draft(draft: &str) -> String {
    format!(
        "<section class=\"panel\"><h2 class=\"panel-heading\">Customer communication draft</h2>\
         <p class=\"panel-body\">{}</p>\
         <p class=\"table-note\">Send through the account team with the incident reference; this draft is not sent automatically.</p></section>",
        html_escape(draft)
    )
}

fn render_preview(preview: &Result<InterventionPreview, String>) -> String {
    match preview {
        Err(reason) => format!(
            "<section class=\"panel\"><h2 class=\"panel-heading\">Before intervention</h2>{}</section>",
            notice(
                StatusKind::Unavailable,
                "The platform state read for this action is not available.",
                &format!("Nothing is changed from this page. {reason}"),
                None,
            )
        ),
        Ok(preview) => {
            let rows = data_table(
                Some("What the action would change"),
                &["Field", "Value"],
                &[
                    vec![Cell::Muted("Publisher".to_string()), Cell::Text(preview.publisher.clone())],
                    vec![Cell::Muted("Operation".to_string()), Cell::Text(preview.operation.clone())],
                    vec![Cell::Muted("Prior state".to_string()), Cell::Text(preview.prior_state.clone())],
                    vec![Cell::Muted("Target state".to_string()), Cell::Text(preview.target_state.clone())],
                    vec![Cell::Muted("Policy".to_string()), Cell::Text(preview.policy.clone())],
                    vec![Cell::Muted("Permissions".to_string()), Cell::Text(preview.permissions.clone())],
                    vec![
                        Cell::Muted("Idempotency key".to_string()),
                        Cell::Muted("fresh per form render (hidden)".to_string()),
                    ],
                    vec![
                        Cell::Muted("Traffic effect".to_string()),
                        Cell::Text(if preview.affects_traffic {
                            "Can affect active traffic: the live ad unit configuration changes.".to_string()
                        } else {
                            "No traffic change: reads and reporting are unaffected.".to_string()
                        }),
                    ],
                ],
                None,
                Some("Read the current platform state and compare it with the intended action."),
            );
            format!(
                "<section class=\"panel\"><h2 class=\"panel-heading\">Before intervention</h2>{rows}</section>"
            )
        }
    }
}

fn render_timeline(case: &Result<IncidentCase, String>) -> String {
    let Ok(case) = case else {
        return String::new();
    };
    let cells: Vec<Vec<Cell>> = case
        .timeline
        .iter()
        .map(|entry| {
            vec![
                Cell::Text(entry.occurred_at_utc.clone()),
                Cell::Text(entry.event_type.as_str().replace('_', " ")),
                Cell::Text(
                    entry
                        .outcome
                        .clone()
                        .unwrap_or_else(|| "recorded".to_string()),
                ),
                Cell::Text(entry.reason_code.clone().unwrap_or_else(|| "-".to_string())),
                Cell::Muted(entry.received_at_utc.clone()),
            ]
        })
        .collect();
    data_table(
        Some("Timeline (occurrence order, received time available)"),
        &[
            "Occurred (UTC)",
            "Event",
            "Outcome",
            "Reason",
            "Received (UTC)",
        ],
        &cells,
        Some(EmptyState {
            reason: "No timeline entries are recorded for this incident yet.".to_string(),
            action: None,
        }),
        None,
    )
}
