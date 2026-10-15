//! The action result page, the page-97 denied-action page and the result
//! state vocabulary (Requested, Running, Confirmed, Failed, State unknown).

use apex_ui::components::{action, notice};
use apex_ui::escape::html_escape;
use apex_ui::status::{chip, StatusKind};

use crate::actions::ActionState;
use crate::capabilities::Capability;
use crate::copy;

use super::{document, fixtures_banner, PageMeta};

/// Map an action state to its status chip.
pub fn state_chip(state: ActionState) -> String {
    let kind = match state {
        ActionState::Requested => StatusKind::Queued,
        ActionState::Running => StatusKind::InProgress,
        ActionState::Confirmed => StatusKind::Completed,
        ActionState::Failed => StatusKind::Failure,
        ActionState::StateUnknown => StatusKind::Unavailable,
    };
    chip(state.label(), kind)
}

/// The action result page.
#[allow(clippy::too_many_arguments)]
pub fn result(
    meta: &PageMeta,
    capability: Capability,
    result_id: &str,
    state: ActionState,
    operation_id: Option<&str>,
    confirming_read: &str,
    note: &str,
    job_state: Option<&str>,
) -> String {
    let mut body = String::new();
    body.push_str(&format!(
        "<p class=\"panel-body\">{} {}</p>",
        state_chip(state),
        html_escape(&format!("{}: {}", capability.label(), result_id))
    ));
    if let Some(operation_id) = operation_id {
        body.push_str(&format!(
            "<p class=\"panel-body\">Operation id: <code>{}</code></p>",
            html_escape(operation_id)
        ));
    }
    if let Some(job_state) = job_state {
        body.push_str(&format!(
            "<p class=\"panel-body\">Durable job state: {}</p>",
            html_escape(job_state)
        ));
    }
    if !note.is_empty() {
        body.push_str(&format!(
            "<p class=\"panel-body\">{}</p>",
            html_escape(note)
        ));
    }
    body.push_str(&format!(
        "<p class=\"panel-body\">{}</p>",
        action("Confirming read", confirming_read, false)
    ));
    body.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape(copy::RESULT_RECONCILE_NOTE)
    ));
    let document_body = format!(
        "{}<h1 class=\"page-title\">Action result</h1>{body}",
        fixtures_banner(meta)
    );
    document(meta, "Action result", &["Action result"], &document_body)
}

/// The page-97 denied-action page: the frozen copy, the journal note and the
/// return link to the object.
pub fn denied(meta: &PageMeta, capability: Capability, target: &str, return_path: &str) -> String {
    let notice_html = notice(
        StatusKind::Failure,
        copy::DENIED_HEADLINE,
        copy::DENIED_BODY,
        Some((copy::DENIED_ACTION, return_path)),
    );
    let body = format!(
        "{}<h1 class=\"page-title\">{}</h1>{notice_html}\
         <p class=\"table-note\">Attempted {} against {}; the attempt is journaled and counted, and no operation was requested from any platform.</p>",
        fixtures_banner(meta),
        html_escape(copy::DENIED_HEADLINE),
        html_escape(capability.as_str()),
        html_escape(target)
    );
    document(meta, copy::DENIED_HEADLINE, &["Access", "Denied"], &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::Role;

    fn meta() -> PageMeta {
        PageMeta {
            environment: "TEST".to_string(),
            as_of_ms: crate::fixtures::DEMO_NOW_MS,
            operator: Some("Maya".to_string()),
            role: Some(Role::Support),
            fixtures: false,
        }
    }

    #[test]
    fn denied_page_is_the_frozen_copy() {
        let html = denied(
            &meta(),
            Capability::IncidentAcknowledge,
            "inc_018",
            "/control/incidents/inc_018",
        );
        assert!(html.contains("Your role does not permit this action."));
        assert!(html.contains("No operation was requested."));
        assert!(html.contains("Return to the object."));
        assert!(html.contains("href=\"/control/incidents/inc_018\""));
        assert_eq!(html.matches("<h1").count(), 1);
    }

    #[test]
    fn every_result_state_renders_its_label() {
        for (state, label) in [
            (ActionState::Requested, "Requested"),
            (ActionState::Running, "Running"),
            (ActionState::Confirmed, "Confirmed"),
            (ActionState::Failed, "Failed"),
            (ActionState::StateUnknown, "State unknown"),
        ] {
            let html = result(
                &meta(),
                Capability::RetryRead,
                "res_0123456789ab",
                state,
                Some("opj_0123456789ab"),
                "/control/ingestion",
                "note text",
                Some("Queued"),
            );
            assert!(html.contains(label), "{label}");
            assert!(html.contains("opj_0123456789ab"));
            assert!(html.contains("Confirming read"));
            assert!(html.contains("State unknown") || state != ActionState::StateUnknown);
            assert_eq!(html.matches("<h1").count(), 1);
        }
    }

    #[test]
    fn result_page_escapes_and_carries_the_reconcile_note() {
        let html = result(
            &meta(),
            Capability::RetryRead,
            "res_x",
            ActionState::Requested,
            None,
            "/control/ingestion",
            "<script>alert(1)</script>",
            None,
        );
        assert!(!html.contains("<script>alert(1)"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("An interrupted request never implies the platform did nothing."));
    }
}
