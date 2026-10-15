//! Access `/control/access` (page 71): the action history (including denied
//! rows), the roles reference and the live session list with revoke.

use crate::capabilities::{Capability, Role};
use crate::copy;
use crate::journal::ActionJournalEntry;
use crate::session::SessionListRow;

use apex_ui::components::{data_table, notice, Cell, EmptyState};
use apex_ui::escape::html_escape;
use apex_ui::format::utc_datetime;
use apex_ui::status::StatusKind;

use super::{action_form, document, header, ActionFormSpec, PageMeta};

/// The access page data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessData {
    /// The journal entries (newest first) or the reason they are unavailable.
    pub history: Result<Vec<ActionJournalEntry>, String>,
    /// The live sessions.
    pub sessions: Vec<SessionListRow>,
    /// Whether the actor holds `session_revoke` (shows revoke forms).
    pub can_revoke: bool,
    /// Revoke forms per session prefix.
    pub revoke_forms: Vec<ActionFormSpec>,
}

/// Render the access page.
pub fn render(meta: &PageMeta, data: &AccessData) -> String {
    let mut body = header(meta, "Access");
    body.push_str("<h2 class=\"section-heading\">Action history</h2>");
    match &data.history {
        Err(reason) => {
            body.push_str(&notice(
                StatusKind::Unavailable,
                copy::TELEMETRY_ABSENT,
                &format!("The action history is not readable: {reason}"),
                None,
            ));
        }
        Ok(entries) => {
            let cells: Vec<Vec<Cell>> = entries
                .iter()
                .rev()
                .take(200)
                .map(|entry| {
                    let outcome_kind = match entry.outcome.as_str() {
                        "denied" => StatusKind::Failure,
                        "confirmed" => StatusKind::Verified,
                        "failed" => StatusKind::Failure,
                        "requested" | "running" => StatusKind::Queued,
                        _ => StatusKind::Neutral,
                    };
                    vec![
                        Cell::Text(utc_datetime(entry.at_ms)),
                        Cell::Mono(entry.actor.clone()),
                        Cell::Text(format!("{} / {}", entry.capability, entry.target)),
                        Cell::Status(entry.outcome.clone(), outcome_kind),
                        Cell::Muted(entry.reason.clone()),
                    ]
                })
                .collect();
            body.push_str(&data_table(
                Some("Actions, newest first"),
                &["When (UTC)", "Actor", "Action / target", "Outcome", "Reason"],
                &cells,
                Some(EmptyState {
                    reason: "No operator action is recorded yet.".to_string(),
                    action: None,
                }),
                Some("Denied rows stay visible, with the outcome word Denied; values restricted before events enter the analytics store are redacted here too."),
            ));
        }
    }
    body.push_str("<h2 class=\"section-heading\">Sessions</h2>");
    let session_cells: Vec<Vec<Cell>> = data
        .sessions
        .iter()
        .map(|row| {
            vec![
                Cell::Mono(row.id_prefix.clone()),
                Cell::Text(format!("{} ({})", row.operator_name, row.role.label())),
                Cell::Text(row.device_label.clone()),
                Cell::Text(utc_datetime(row.issued_at_ms)),
                Cell::Text(utc_datetime(row.expires_at_ms)),
                Cell::Text(utc_datetime(row.last_seen_ms)),
            ]
        })
        .collect();
    body.push_str(&data_table(
        Some("Live sessions (the handle is shown as a prefix only)"),
        &[
            "Session",
            "Operator",
            "Device",
            "Issued",
            "Expires",
            "Last activity",
        ],
        &session_cells,
        Some(EmptyState {
            reason: "No session is live.".to_string(),
            action: None,
        }),
        Some("Revoking a session is scoped, journaled and counted."),
    ));
    if data.can_revoke {
        for form in &data.revoke_forms {
            body.push_str(&format!(
                "<section class=\"panel\"><h3 class=\"panel-heading\">Revoke session {}</h3>{}</section>",
                html_escape(&form.target),
                action_form(form)
            ));
        }
    } else {
        body.push_str(&format!(
            "<p class=\"table-note\">{}</p>",
            html_escape(
                "Revoking sessions needs the Security role; the request is denied here before anything is requested."
            )
        ));
    }
    body.push_str("<h2 class=\"section-heading\">Roles reference</h2>");
    let role_cells: Vec<Vec<Cell>> = Role::ALL
        .iter()
        .map(|role| {
            let capabilities: Vec<&str> = role
                .capabilities()
                .iter()
                .map(|capability| capability.as_str())
                .collect();
            vec![
                Cell::Text(role.label().to_string()),
                if capabilities.is_empty() {
                    Cell::Muted("Reads only".to_string())
                } else {
                    Cell::Text(capabilities.join(", "))
                },
                Cell::Muted(
                    if role.can_revoke_marker() {
                        "Sensitive session actions require a fresh sign-in."
                    } else {
                        "-"
                    }
                    .to_string(),
                ),
            ]
        })
        .collect();
    body.push_str(&data_table(
        Some("Five roles, deny by default"),
        &["Role", "Capabilities", "Note"],
        &role_cells,
        None,
        Some("Observer holds no mutation; every action is checked server-side."),
    ));
    body.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape(
            "Acceptance rules: a customer credential cannot open an operator route; an Observer cannot submit a mutation; an Operator cannot edit a historical result."
        )
    ));
    body.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape(&format!(
            "Capability count: {} across {} roles.",
            Capability::ALL.len(),
            Role::ALL.len()
        ))
    ));
    document(meta, "Access", &["Access"], &body)
}

impl Role {
    /// Whether this role holds any step-up-gated capability (for the roles
    /// reference note).
    fn can_revoke_marker(self) -> bool {
        self.capabilities()
            .iter()
            .any(|capability| capability.requires_step_up())
    }
}
