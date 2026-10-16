//! The sign-in surface and the not-configured explanation.
//!
//! The owner gate is a single token (`APEX_SALES_WEB_TOKEN`) compared in
//! constant time. With the variable unset the deployment renders the
//! not-configured state instead of the form and no session can be minted;
//! a wrong token answers the same form with an error, never a session.

use apex_ui::components::{action, field, form_error_summary, input, panel};
use apex_ui::status::StatusKind;

use crate::pages::shared::{self, PageMeta};

/// The form's error copy for a rejected token.
pub const LOGIN_FAILED: &str =
    "That token was not accepted. Nothing was recorded; try the APEX_SALES_WEB_TOKEN value again.";

/// The unconfigured state: shown for every page until the token is set.
pub fn unconfigured(as_of_secs: i64, reason: &str) -> String {
    let meta = PageMeta::new("Sign in", "Overview", Vec::new(), as_of_secs, reason, "");
    let body = format!(
        "{}\n{}",
        apex_ui::components::notice(
            StatusKind::Failure,
            "This deployment is not configured",
            &format!(
                "{reason}. The sales workspace fails closed until the owner token is set: no session can be minted and no page renders data."
            ),
            None,
        ),
        panel(
            "Set the owner token",
            "<p>Start the service with <code>APEX_SALES_WEB_TOKEN</code> set to a long random value, e.g. <code>export APEX_SALES_WEB_TOKEN=\"$(openssl rand -hex 32)\"</code>. Restarts pick the value up from the environment.</p>",
            Some("then reload this page"),
        )
    );
    shared::page(&meta, &body)
}

/// The sign-in form; `problem` renders the error summary when a token was
/// rejected.
pub fn form(as_of_secs: i64, data_location: &str, problem: Option<&str>) -> String {
    let meta = PageMeta::new(
        "Sign in",
        "Overview",
        Vec::new(),
        as_of_secs,
        data_location,
        "",
    );
    let error_html = problem
        .map(|message| form_error_summary(&[("token".to_string(), message.to_string())]))
        .unwrap_or_default();
    let form_html = format!(
        "<form method=\"post\" action=\"/login\">\n{}\n{}</form>",
        field(
            "token",
            "Owner token",
            Some("The value of APEX_SALES_WEB_TOKEN on this deployment."),
            problem,
            &input("token", "token", "password", None, true, Some("off")),
        ),
        "<p class=\"form-actions\"><button class=\"action action-primary\" type=\"submit\">Sign in</button></p>"
    );
    let body = format!(
        "{error_html}\n{}\n{}",
        panel(
            "Owner sign-in",
            &form_html,
            Some("wrong or missing tokens never mint a session"),
        ),
        "<p class=\"table-note\">This page is the only public surface. Every other route requires the session cookie minted here; it is host-only, HttpOnly, SameSite=Lax and expires after 8 hours at the latest.</p>"
    );
    shared::page(&meta, &body)
}

/// The signed-out state after a successful logout.
pub fn signed_out(as_of_secs: i64, data_location: &str) -> String {
    let meta = PageMeta::new(
        "Signed out",
        "Overview",
        Vec::new(),
        as_of_secs,
        data_location,
        "",
    );
    let body = format!(
        "{}\n{}",
        apex_ui::components::notice(
            StatusKind::Neutral,
            "Signed out",
            "The session cookie is cleared. Sign in again to open the workspace.",
            None,
        ),
        action("Sign in", "/login", true)
    );
    shared::page(&meta, &body)
}

/// The session-required state for a protected page.
pub fn session_required(as_of_secs: i64, data_location: &str) -> String {
    let meta = PageMeta::new(
        "Sign in",
        "Overview",
        Vec::new(),
        as_of_secs,
        data_location,
        "",
    );
    let body = format!(
        "{}\n{}",
        apex_ui::components::notice(
            StatusKind::Attention,
            "Sign-in required",
            "This workspace requires the owner session. Sign in with the APEX_SALES_WEB_TOKEN value.",
            None,
        ),
        action("Sign in", "/login", true)
    );
    // The form itself, so the sign-in can happen from any protected page.
    let with_form = format!(
        "{body}\n{}",
        panel(
            "Sign in",
            &format!(
                "<form method=\"post\" action=\"/login\">\n{}\n<p class=\"form-actions\"><button class=\"action action-primary\" type=\"submit\">Sign in</button></p></form>",
                field(
                    "token",
                    "Owner token",
                    None,
                    None,
                    &input("token", "token", "password", None, true, Some("off")),
                ),
            ),
            None,
        )
    );
    shared::page(&meta, &with_form)
}

/// Escape helper re-exported for tests of this module.
#[cfg(test)]
fn escaped(text: &str) -> String {
    apex_ui::escape::html_escape(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unconfigured_state_explains_the_requirement() {
        let html = unconfigured(1_791_965_400, "APEX_SALES_WEB_TOKEN is not set");
        assert!(html.contains("This deployment is not configured"));
        assert!(html.contains("APEX_SALES_WEB_TOKEN"));
        assert!(html.contains("no session can be minted"));
        assert!(!html.contains("<form"));
    }

    #[test]
    fn the_form_posts_to_login_and_escapes_the_problem() {
        let html = form(1_791_965_400, ".apex/sales", Some(LOGIN_FAILED));
        assert!(html.contains("<form method=\"post\" action=\"/login\">"));
        assert!(html.contains("name=\"token\""));
        assert!(html.contains("type=\"password\""));
        assert!(html.contains("role=\"alert\""));
        assert!(html.contains(LOGIN_FAILED));
        let hostile = form(0, ".apex/sales", Some("<script>alert(1)</script>"));
        assert!(!hostile.contains("<script>"));
        assert_eq!(escaped("<"), "&lt;");
    }
}
