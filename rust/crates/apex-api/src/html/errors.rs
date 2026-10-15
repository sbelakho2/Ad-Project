//! The shared error-page family of the console.
//!
//! One renderer frames every error state as a neutral panel with exactly one
//! `<h1>`, the useful body copy, an optional support reference and recovery
//! actions. The headline/body pairs are the frozen copy of the design
//! document where it defines them (400, 404 no-snapshot, 422, 500, 503); the
//! remaining statuses (405 hint, 409, 429) have no document copy and use
//! short, factual sentences in the same voice.
//!
//! A reference is never invented: callers pass a real, quotable value (the
//! route plus the UTC moment it failed, or the ledger verification line).

use apex_ui::components::action;

use super::{console_document, html_escape, ConsoleCtx};

/// One arm of the error family. The kind picks the status phrasing; callers
/// still choose the HTTP status code they answer with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// 400: a refused request or field.
    BadRequest,
    /// 404: the named view does not exist here.
    NotFound,
    /// 405: a known path, wrong method.
    MethodNotAllowed,
    /// 409: the request conflicts with the current state.
    Conflict,
    /// 422: a view that exists but cannot be verified.
    Unprocessable,
    /// 429: too many requests.
    TooManyRequests,
    /// 500: the view could not be opened.
    Internal,
    /// 503: the view is not configured in this deployment.
    NotConfigured,
}

/// Render one error page. `reference` is shown as a quotable support
/// reference; `actions` are `(label, href)` recovery links.
pub fn render(
    ctx: &ConsoleCtx,
    kind: ErrorKind,
    headline: &str,
    body: &str,
    detail: Option<&str>,
    reference: Option<&str>,
    actions: &[(&str, &str)],
) -> String {
    let _ = kind;
    let mut panel_body = format!("<p class=\"panel-body\">{}</p>", html_escape(body));
    if let Some(detail) = detail {
        panel_body.push_str(&format!(
            "<p class=\"panel-next\">{}</p>",
            html_escape(detail)
        ));
    }
    if let Some(reference) = reference {
        panel_body.push_str(&format!(
            "<p class=\"panel-next\">Reference for support: <code>{}</code></p>",
            html_escape(reference)
        ));
    }
    let mut links: Vec<String> = actions
        .iter()
        .map(|(label, href)| action(label, href, false))
        .collect();
    links.push(action("Back to the overview", "/", false));
    panel_body.push_str(&format!("<p class=\"panel-body\">{}</p>", links.join(" ")));
    let doc_body = format!(
        "<section class=\"panel\"><h1 class=\"page-title\">{}</h1>{panel_body}</section>",
        html_escape(headline)
    );
    console_document(ctx, headline, area_for(kind), &doc_body)
}

/// The breadcrumb area an error page belongs to, derived from the reference
/// route the caller passes (defaults to `Overview`).
fn area_for(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Unprocessable => "Change log",
        _ => "Overview",
    }
}

/// 400: the frozen request copy, with an optional precise detail line.
pub fn bad_request(ctx: &ConsoleCtx, detail: Option<&str>) -> String {
    render(
        ctx,
        ErrorKind::BadRequest,
        "Check the request.",
        "A field or upload could not be accepted.",
        detail,
        None,
        &[],
    )
}

/// 400: the frozen request copy plus a precise detail line. Same page as
/// [`bad_request`]; kept as one constructor so every caller shows the same
/// headline and body.
pub fn bad_request_with_detail(ctx: &ConsoleCtx, detail: &str) -> String {
    bad_request(ctx, Some(detail))
}

/// 413: the oversized-body shape of the same request copy.
pub fn body_too_large(ctx: &ConsoleCtx, limit_kib: usize) -> String {
    render(
        ctx,
        ErrorKind::BadRequest,
        "Check the request.",
        "A field or upload could not be accepted.",
        Some(&format!(
            "The request body exceeds the deployment's {limit_kib} KiB limit."
        )),
        None,
        &[],
    )
}

/// 404: no snapshot for the selected platform (frozen copy).
pub fn snapshot_not_found(ctx: &ConsoleCtx) -> String {
    render(
        ctx,
        ErrorKind::NotFound,
        "No audit snapshot is available.",
        "This platform has no report in the current workspace.",
        None,
        None,
        &[("Contact support", "/support/")],
    )
}

/// 404: the requested finding is in no current snapshot.
pub fn finding_not_found(ctx: &ConsoleCtx) -> String {
    render(
        ctx,
        ErrorKind::NotFound,
        "No finding matches this address.",
        "The finding is not in any snapshot available to this workspace. Open the audit and choose \
         a platform.",
        None,
        None,
        &[("Open audit", "/console/audit")],
    )
}

/// 404: the publisher has no ledger in this deployment.
pub fn ledger_not_found(ctx: &ConsoleCtx) -> String {
    render(
        ctx,
        ErrorKind::NotFound,
        "No ledger is available.",
        "This publisher has no verified history in this deployment.",
        None,
        None,
        &[("Open the change log", "/console/ledger")],
    )
}

/// 404: any other missing page.
pub fn not_found(ctx: &ConsoleCtx) -> String {
    render(
        ctx,
        ErrorKind::NotFound,
        "Not found.",
        "The address does not match any page in this workspace.",
        None,
        None,
        &[],
    )
}

/// 405: a known console path with the wrong method.
pub fn method_not_allowed(ctx: &ConsoleCtx) -> String {
    render(
        ctx,
        ErrorKind::MethodNotAllowed,
        "Method not allowed.",
        "This page is available over GET. Open the address again from the navigation.",
        None,
        None,
        &[("Open the overview", "/")],
    )
}

/// 409: the request conflicts with the current state (no console mutation
/// reaches this state in this phase; the page is unit tested and rendered as
/// a fixture).
pub fn conflict(ctx: &ConsoleCtx) -> String {
    render(
        ctx,
        ErrorKind::Conflict,
        "Conflict.",
        "The request conflicts with the current state of this workspace.",
        None,
        None,
        &[],
    )
}

/// 422: the ledger could not be verified (frozen copy, with the offending
/// line as the incident reference and the support action).
pub fn ledger_unverified(ctx: &ConsoleCtx, line: usize) -> String {
    render(
        ctx,
        ErrorKind::Unprocessable,
        "This history could not be verified.",
        &format!("Verification failed at line {line}. No records are displayed."),
        None,
        Some(&format!("ledger verification failed at line {line}")),
        &[("Contact support", "/support/")],
    )
}

/// 429: too many requests (no document copy exists for this status).
pub fn too_many_requests(ctx: &ConsoleCtx) -> String {
    render(
        ctx,
        ErrorKind::TooManyRequests,
        "Too many requests.",
        "Wait a moment, then try again.",
        None,
        None,
        &[],
    )
}

/// 500: the audit snapshot could not be read (frozen copy).
pub fn audit_read_failure(ctx: &ConsoleCtx, reference: &str) -> String {
    render(
        ctx,
        ErrorKind::Internal,
        "We could not open this snapshot.",
        "Try again or quote this reference when contacting support.",
        None,
        Some(reference),
        &[("Contact support", "/support/")],
    )
}

/// 500: any other view failed to open.
pub fn internal(ctx: &ConsoleCtx, reference: &str) -> String {
    render(
        ctx,
        ErrorKind::Internal,
        "We could not open this view.",
        "Try again or quote this reference when contacting support.",
        None,
        Some(reference),
        &[("Contact support", "/support/")],
    )
}

/// 503: the view is not configured in this deployment (frozen copy).
pub fn not_configured(ctx: &ConsoleCtx) -> String {
    render(
        ctx,
        ErrorKind::NotConfigured,
        "This view is not available in this deployment.",
        "Contact the account team for access.",
        None,
        None,
        &[("Contact support", "/support/")],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ConsoleCtx {
        ConsoleCtx::new("Northstar Studio", "USD", true)
    }

    fn assert_common(html: &str, headline: &str) {
        assert!(html.contains(&format!("<h1 class=\"page-title\">{headline}</h1>")));
        assert!(html.contains("Back to the overview"));
        assert_eq!(html.matches("<h1").count(), 1, "exactly one h1");
        assert!(!html.contains("<script"));
        assert!(html.contains("Read-only mode. No platform settings are changed here."));
    }

    #[test]
    fn every_kind_renders_a_single_h1_neutral_panel() {
        let pages = [
            (ErrorKind::BadRequest, "Check the request."),
            (ErrorKind::NotFound, "Not found."),
            (ErrorKind::MethodNotAllowed, "Method not allowed."),
            (ErrorKind::Conflict, "Conflict."),
            (
                ErrorKind::Unprocessable,
                "This history could not be verified.",
            ),
            (ErrorKind::TooManyRequests, "Too many requests."),
            (ErrorKind::Internal, "We could not open this view."),
            (
                ErrorKind::NotConfigured,
                "This view is not available in this deployment.",
            ),
        ];
        for (kind, headline) in pages {
            let html = render(&ctx(), kind, headline, "body copy", None, None, &[]);
            assert_common(&html, headline);
            assert!(html.contains("class=\"panel\""));
            assert!(html.contains("body copy"));
        }
    }

    #[test]
    fn bad_request_pages_use_the_frozen_copy_and_optional_detail() {
        let html = bad_request(&ctx(), None);
        assert_common(&html, "Check the request.");
        assert!(html.contains("A field or upload could not be accepted."));
        let detailed =
            bad_request_with_detail(&ctx(), "the request parameters could not be interpreted");
        assert!(detailed.contains("the request parameters could not be interpreted"));
        let large = body_too_large(&ctx(), 256);
        assert!(large.contains("The request body exceeds the deployment&#39;s 256 KiB limit."));
    }

    #[test]
    fn snapshot_not_found_carries_the_frozen_404_copy() {
        let html = snapshot_not_found(&ctx());
        assert_common(&html, "No audit snapshot is available.");
        assert!(html.contains("This platform has no report in the current workspace."));
        assert!(html.contains("Contact support"));
    }

    #[test]
    fn ledger_unverified_names_the_line_and_offers_support() {
        let html = ledger_unverified(&ctx(), 18);
        assert_common(&html, "This history could not be verified.");
        assert!(html.contains("Verification failed at line 18. No records are displayed."));
        assert!(html.contains("ledger verification failed at line 18"));
        assert!(html.contains("Contact support"));
    }

    #[test]
    fn read_failures_quote_the_reference_and_keep_exception_text_out() {
        let html = audit_read_failure(
            &ctx(),
            "/console/audit?platform=admob at 14 Oct 2026, 08:10 UTC",
        );
        assert_common(&html, "We could not open this snapshot.");
        assert!(html.contains("Try again or quote this reference when contacting support."));
        assert!(html.contains("Reference for support: <code>/console/audit?platform=admob at 14 Oct 2026, 08:10 UTC</code>"));
        let generic = internal(&ctx(), "route /console/ledger at 0");
        assert!(generic.contains("We could not open this view."));
        // Hostile references stay escaped.
        let hostile = internal(&ctx(), "<script>alert(1)</script>");
        assert!(!hostile.contains("<script>alert(1)"));
        assert!(hostile.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[test]
    fn not_configured_is_the_frozen_503_copy() {
        let html = not_configured(&ctx());
        assert_common(&html, "This view is not available in this deployment.");
        assert!(html.contains("Contact the account team for access."));
    }

    #[test]
    fn method_hint_lists_the_get_route() {
        let html = method_not_allowed(&ctx());
        assert_common(&html, "Method not allowed.");
        assert!(html.contains("This page is available over GET."));
    }

    #[test]
    fn finding_and_ledger_not_found_offer_recovery() {
        let finding = finding_not_found(&ctx());
        assert!(finding.contains("No finding matches this address."));
        assert!(finding.contains("href=\"/console/audit\""));
        let ledger = ledger_not_found(&ctx());
        assert!(ledger.contains("No ledger is available."));
        assert!(ledger.contains("href=\"/console/ledger\""));
    }
}
