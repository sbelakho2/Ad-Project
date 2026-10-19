//! The control-plane error page family: one h1, a factual headline, the
//! useful body, an optional detail line and the recovery actions. The 409
//! conflict page and the session-expired page carry the copy the design
//! document prescribes; the rest use short sentences in the same voice.

use apex_ui::components::action;
use apex_ui::escape::html_escape;

use super::{document, fixtures_banner, PageMeta};

/// One error page.
pub fn render(
    meta: &PageMeta,
    headline: &str,
    body: &str,
    detail: Option<&str>,
    actions: &[(&str, &str)],
) -> String {
    let mut panel_body = format!("<p class=\"panel-body\">{}</p>", html_escape(body));
    if let Some(detail) = detail {
        panel_body.push_str(&format!(
            "<p class=\"panel-next\">{}</p>",
            html_escape(detail)
        ));
    }
    let mut links: Vec<String> = actions
        .iter()
        .map(|(label, href)| action(label, href, false))
        .collect();
    links.push(action("Back to the overview", "/control/", false));
    panel_body.push_str(&format!("<p class=\"panel-body\">{}</p>", links.join(" ")));
    let document_body = format!(
        "{}<section class=\"panel\"><h1 class=\"page-title\">{}</h1>{panel_body}</section>",
        fixtures_banner(meta),
        html_escape(headline)
    );
    document(meta, headline, &["Error"], &document_body)
}

/// 400: a request field was missing or malformed.
pub fn bad_request(meta: &PageMeta, detail: Option<&str>) -> String {
    render(
        meta,
        "Check the request.",
        "A field or form value could not be accepted.",
        detail,
        &[],
    )
}

/// 404: the named object does not exist in this snapshot.
pub fn not_found(meta: &PageMeta) -> String {
    render(
        meta,
        "Not found.",
        "The address does not match any object in this control plane.",
        None,
        &[],
    )
}

/// 405: a known path over the wrong method.
pub fn method_not_allowed(meta: &PageMeta) -> String {
    render(
        meta,
        "Method not allowed.",
        "This page is available over GET. Open the address again from the navigation.",
        None,
        &[("Open the overview", "/control/")],
    )
}

/// 409: the stale-state check failed (page 64: the check runs on the
/// server; no operation was requested).
pub fn conflict(meta: &PageMeta, reason: &str, return_path: &str) -> String {
    render(
        meta,
        crate::copy::CONFLICT_HEADLINE,
        crate::copy::CONFLICT_BODY,
        Some(reason),
        &[("Return to the object", return_path)],
    )
}

/// 500: a view could not be opened.
pub fn internal(meta: &PageMeta, reference: &str) -> String {
    render(
        meta,
        "We could not open this view.",
        "Try again or quote this reference when contacting support.",
        Some(reference),
        &[("Contact support", apex_ui::shell::DEFAULT_SUPPORT_URL)],
    )
}

/// 503: the view is not configured in this deployment.
pub fn not_configured(meta: &PageMeta, reason: &str) -> String {
    render(
        meta,
        "This view is not available in this deployment.",
        reason,
        None,
        &[("Contact support", apex_ui::shell::DEFAULT_SUPPORT_URL)],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::Role;

    fn meta() -> PageMeta {
        PageMeta {
            environment: "PRODUCTION".to_string(),
            as_of_ms: crate::fixtures::DEMO_NOW_MS,
            operator: Some("Maya".to_string()),
            role: Some(Role::Operator),
            fixtures: false,
            csrf: String::new(),
        }
    }

    fn assert_page(html: &str, headline: &str) {
        assert!(html.contains(&format!("<h1 class=\"page-title\">{headline}</h1>")));
        assert_eq!(html.matches("<h1").count(), 1);
        assert!(!html.contains("<script"));
        assert!(html.contains("Back to the overview"));
    }

    #[test]
    fn every_error_page_has_one_h1_and_a_recovery_link() {
        assert_page(
            &bad_request(&meta(), Some("the request body did not parse")),
            "Check the request.",
        );
        assert_page(&not_found(&meta()), "Not found.");
        assert_page(&method_not_allowed(&meta()), "Method not allowed.");
        assert_page(
            &internal(&meta(), "/control/ at 15 Oct 2026, 09:10 UTC"),
            "We could not open this view.",
        );
        assert_page(
            &not_configured(&meta(), "No operator configuration is present."),
            "This view is not available in this deployment.",
        );
    }

    #[test]
    fn conflict_carries_the_stale_state_copy_and_a_return_link() {
        let html = conflict(
            &meta(),
            "the object moved to a newer version",
            "/control/incidents/inc_018",
        );
        assert_page(&html, "This object changed while the form was open.");
        assert!(html.contains("no operation was requested"));
        assert!(html.contains("the object moved to a newer version"));
        assert!(html.contains("href=\"/control/incidents/inc_018\""));
    }

    #[test]
    fn hostile_details_stay_escaped() {
        let html = internal(&meta(), "<script>alert(1)</script>");
        assert!(!html.contains("<script>alert(1)"));
        assert!(html.contains("&lt;script&gt;"));
    }
}
