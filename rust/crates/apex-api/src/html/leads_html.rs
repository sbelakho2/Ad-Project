//! The free-audit request page and the confirmation page, rendered with the
//! marketing shell of the design system.
//!
//! Request page copy, field labels, field errors and the submission-failure
//! block are the finished copy of the design document. A validation failure
//! re-renders this page with the safe values preserved, the error summary
//! linked to the fields and the exact error text at each field. A storage
//! failure re-renders the same page with the submission-failure notice and
//! the retry action.

use apex_ui::components::{
    checkbox, field, form_error_summary, input, notice, panel, select, split, textarea,
};
use apex_ui::status::StatusKind;

use super::{html_escape, marketing_document, ConsoleCtx};
use crate::leads::{
    FreeAuditInvalid, FreeAuditRequest, FORM_SUMMARY, PLATFORM_CHOICES, REVENUE_BANDS,
    SUBMISSION_FAILURE_BODY, SUBMISSION_FAILURE_TITLE, TRY_AGAIN_LABEL,
};

/// The request form action; the POST handler validates and stores.
pub const FREE_AUDIT_ACTION: &str = "/free-audit";
/// Where the marketing site explains the audit.
pub const HOW_IT_WORKS_HREF: &str = "/how-it-works/";
/// The privacy notice of the marketing site.
pub const PRIVACY_HREF: &str = "/privacy/";

/// What the request page is rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestPageState {
    /// The untouched form.
    Fresh,
    /// A validation failure: preserved values plus field errors.
    Invalid(Box<FreeAuditInvalid>),
    /// The submit action itself failed (storage); preserved values and the
    /// submission-failure copy.
    SubmissionFailed(Box<FreeAuditRequest>),
}

/// Render the free-audit request page (`GET /free-audit/`).
pub fn render_request_page(ctx: &ConsoleCtx, state: &RequestPageState) -> String {
    let (request, errors, failed) = match state {
        RequestPageState::Fresh => (FreeAuditRequest::default(), Vec::new(), false),
        RequestPageState::Invalid(invalid) => {
            (invalid.request.clone(), invalid.errors.clone(), false)
        }
        RequestPageState::SubmissionFailed(request) => ((**request).clone(), Vec::new(), true),
    };
    let mut body = String::from("<h1>Start with a clear picture.</h1>");
    body.push_str(
        "<p>Tell us about your portfolio. We will confirm the scope of your free read-only audit \
         by email.</p>",
    );
    if failed {
        body.push_str(&notice(
            StatusKind::Failure,
            SUBMISSION_FAILURE_TITLE,
            SUBMISSION_FAILURE_BODY,
            None,
        ));
    }
    if !errors.is_empty() {
        // The frozen summary line of the design document heads the error
        // list; a summary-level error carries it itself.
        if !errors.iter().any(|error| error.field == "summary") {
            body.push_str(&format!(
                "<p class=\"form-errors-headline\">{}</p>",
                html_escape(FORM_SUMMARY)
            ));
        }
        let summary: Vec<(String, String)> = errors
            .iter()
            .map(|error| (error.field.to_string(), error.message.clone()))
            .collect();
        body.push_str(&form_error_summary(&summary));
    }
    let error_for = |field: &str| {
        errors
            .iter()
            .find(|error| error.field == field)
            .map(|error| error.message.as_str())
    };
    let platform_boxes = PLATFORM_CHOICES
        .iter()
        .map(|(name, label)| {
            checkbox(
                &format!("platform-{name}"),
                "platforms",
                label,
                request.platforms.iter().any(|chosen| chosen == name),
            )
        })
        .collect::<Vec<String>>()
        .join(" ");
    let revenue_options: Vec<(String, String)> =
        std::iter::once(("".to_string(), "Select a range".to_string()))
            .chain(
                REVENUE_BANDS
                    .iter()
                    .map(|band| ((*band).to_string(), (*band).to_string())),
            )
            .collect();
    let submit_label = if failed {
        TRY_AGAIN_LABEL
    } else {
        "Request the free audit"
    };
    let form = format!(
        "<form method=\"post\" action=\"{FREE_AUDIT_ACTION}\">{}{}{}{}{}\
         <div class=\"form-actions\">\
         <button class=\"action action-primary\" type=\"submit\">{submit_label}</button></div>\
         <p class=\"field-help\">We use these details to arrange your audit. \
         <a href=\"{PRIVACY_HREF}\">Read the privacy notice</a>.</p></form>",
        field(
            "email",
            "Work email",
            None,
            error_for("email"),
            &input(
                "email",
                "email",
                "email",
                Some(&request.email),
                false,
                Some("email")
            ),
        ),
        field(
            "studio",
            "Publisher or studio",
            None,
            error_for("studio"),
            &input(
                "studio",
                "studio",
                "text",
                Some(&request.studio),
                false,
                Some("organization")
            ),
        ),
        field(
            "platforms",
            "Platforms used",
            Some("Choose at least one platform."),
            error_for("platforms"),
            &platform_boxes,
        ),
        field(
            "revenue",
            "Monthly ad revenue (optional)",
            None,
            error_for("revenue"),
            &select(
                "revenue",
                "revenue",
                &revenue_options,
                request.revenue.as_deref().filter(|value| !value.is_empty()),
            ),
        ),
        field(
            "notes",
            "Anything we should know? (optional)",
            Some("Do not include platform credentials."),
            error_for("notes"),
            &textarea("notes", "notes", request.notes.as_deref(), 6, Some(2000)),
        ),
    );
    let aside = panel(
        "Before you send",
        "<p>Do not send platform credentials.</p><p>No contract is needed for the audit.</p>",
        None,
    );
    body.push_str(&split(&form, &aside));
    marketing_document(ctx, "Free audit", &body)
}

/// Render the confirmation page (`GET /audit-requested/`).
pub fn render_confirmation(ctx: &ConsoleCtx, request_id: Option<&str>) -> String {
    let mut body = String::from("<h1>Your request is with us.</h1>");
    body.push_str(
        "<p>We will email you to confirm the platform, portfolio scope and access needed for the \
         audit. There is no platform access to grant on this page.</p>",
    );
    body.push_str(
        "<p>If your email address was incorrect, submit a new request. Otherwise, no further \
         action is needed.</p>",
    );
    if let Some(request_id) = request_id {
        body.push_str(&format!(
            "<p class=\"table-note\">Confirmation ID: <code>{}</code></p>",
            html_escape(request_id)
        ));
    }
    body.push_str(&format!(
        "<p><a class=\"action action-secondary\" href=\"{HOW_IT_WORKS_HREF}\">Read how the audit \
         works</a></p>"
    ));
    marketing_document(ctx, "Request confirmed", &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leads::{FieldError, FORM_SUMMARY};

    fn ctx() -> ConsoleCtx {
        ConsoleCtx::default()
    }

    #[test]
    fn request_page_has_the_finished_labels_and_trust_lines() {
        let html = render_request_page(&ctx(), &RequestPageState::Fresh);
        assert!(html.contains("<h1>Start with a clear picture.</h1>"));
        assert!(html.contains("We will confirm the scope of your free read-only audit"));
        assert!(html.contains(">Work email</label>"));
        assert!(html.contains(">Publisher or studio</label>"));
        assert!(html.contains(">Platforms used</label>"));
        assert!(html.contains(">Monthly ad revenue (optional)</label>"));
        assert!(html.contains(">Anything we should know? (optional)</label>"));
        assert!(html.contains("Do not include platform credentials."));
        assert!(html.contains("Do not send platform credentials."));
        assert!(html.contains("No contract is needed for the audit."));
        assert!(html.contains(">Request the free audit</button>"));
        assert!(html.contains("Read the privacy notice"));
        assert!(html.contains("<form method=\"post\" action=\"/free-audit\">"));
        for band in REVENUE_BANDS {
            assert!(html.contains(band), "missing band {band}");
        }
        for (name, label) in PLATFORM_CHOICES {
            assert!(html.contains(&format!("id=\"platform-{name}\"")));
            assert!(html.contains(&format!("> {label}</label>")));
        }
        assert!(html.contains("<header class=\"marketing-header\">"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn invalid_state_preserves_safe_values_and_links_errors() {
        let invalid = FreeAuditInvalid {
            request: FreeAuditRequest {
                email: "you@studio.example".into(),
                studio: "\" onmouseover=\"alert(1)".into(),
                platforms: vec!["admob".into()],
                revenue: Some("$10,000 to $50,000".into()),
                notes: Some("hello".into()),
            },
            errors: vec![
                FieldError {
                    field: "email",
                    message: "Enter an email address we can reply to.".into(),
                },
                FieldError {
                    field: "platforms",
                    message: "Choose at least one platform.".into(),
                },
            ],
        };
        let html = render_request_page(&ctx(), &RequestPageState::Invalid(Box::new(invalid)));
        assert!(html.contains("role=\"alert\""));
        assert!(html.contains("<a href=\"#email\">Enter an email address we can reply to.</a>"));
        assert!(html.contains("value=\"you@studio.example\""));
        assert!(html.contains("value=\"&quot; onmouseover=&quot;alert(1)\""));
        assert!(!html.contains("onmouseover=\"alert(1)\""));
        assert!(html.contains("<option value=\"$10,000 to $50,000\" selected>"));
        assert!(html.contains(">hello</textarea>"));
        assert!(html.contains(
            "<input type=\"checkbox\" id=\"platform-admob\" name=\"platforms\" checked>"
        ));
        assert!(!html.contains("Your request was not confirmed."));
        assert!(html.contains(">Request the free audit</button>"));
    }

    #[test]
    fn summary_only_error_renders_without_field_links() {
        let invalid = FreeAuditInvalid {
            request: FreeAuditRequest::default(),
            errors: vec![FieldError {
                field: "summary",
                message: FORM_SUMMARY.into(),
            }],
        };
        let html = render_request_page(&ctx(), &RequestPageState::Invalid(Box::new(invalid)));
        assert!(html.contains(FORM_SUMMARY));
        assert!(html.contains("href=\"#summary\""));
    }

    #[test]
    fn submission_failure_switches_the_action_to_try_again() {
        let request = FreeAuditRequest {
            email: "you@studio.example".into(),
            studio: "Northstar Studio".into(),
            platforms: vec!["admob".into()],
            revenue: None,
            notes: None,
        };
        let html = render_request_page(
            &ctx(),
            &RequestPageState::SubmissionFailed(Box::new(request)),
        );
        assert!(html.contains(SUBMISSION_FAILURE_TITLE));
        assert!(html.contains("We could not confirm that the request was saved."));
        assert!(html.contains("You can also email hello@apexmediation.ee."));
        assert!(html.contains(">Try again</button>"));
        assert!(!html.contains(">Request the free audit</button>"));
        assert!(html.contains("value=\"you@studio.example\""));
    }

    #[test]
    fn confirmation_shows_the_copy_and_the_stable_id() {
        let html = render_confirmation(&ctx(), Some("req_0123456789"));
        assert!(html.contains("<h1>Your request is with us.</h1>"));
        assert!(html.contains("We will email you to confirm the platform, portfolio scope and access needed for the audit. There is no platform access to grant on this page."));
        assert!(html.contains("If your email address was incorrect, submit a new request. Otherwise, no further action is needed."));
        assert!(html.contains("Confirmation ID: <code>req_0123456789</code>"));
        assert!(html.contains("href=\"/how-it-works/\""));
        assert!(html.contains("Read how the audit works"));
        assert!(!html.contains("<script"));
        let without = render_confirmation(&ctx(), None);
        assert!(!without.contains("Confirmation ID"));
    }

    #[test]
    fn hostile_request_id_is_escaped() {
        let html = render_confirmation(&ctx(), Some("req_<script>alert(1)</script>"));
        assert!(!html.contains("<script>alert(1)"));
        assert!(html.contains("&lt;script&gt;alert(1)"));
    }
}
