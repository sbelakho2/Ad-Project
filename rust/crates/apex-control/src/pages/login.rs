//! The operator sign-in pages (page 96): password form, authenticator-code
//! form, the unconfigured state, the rate-limited state and the
//! session-expired recovery page.

use apex_ui::components::{action, field, input, notice};
use apex_ui::escape::html_escape;
use apex_ui::status::StatusKind;

use crate::copy;

use super::{document, fixtures_banner, PageMeta};

/// The sign-in page states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginState {
    /// The password step, optionally with the frozen failure copy.
    Password {
        /// The frozen failure copy to show, when the password failed.
        problem: Option<&'static str>,
        /// The operator name to preserve (a safe field).
        preserved_operator: String,
        /// The validated return path to preserve (a safe field).
        preserved_next: Option<String>,
        /// Whether this is a step-up re-authentication.
        reauth: bool,
    },
    /// The authenticator-code step.
    Code {
        /// The hidden pending-login token.
        pending_token: String,
        /// The operator display name.
        operator_name: String,
        /// The frozen failure copy to show, when the code failed.
        problem: Option<&'static str>,
    },
    /// The password verified but the operator has no second factor.
    NoSecondFactor {
        /// The operator display name.
        operator_name: String,
    },
    /// The rate-limited state with its lockout deadline.
    RateLimited {
        /// Unix milliseconds when the lockout ends.
        until_ms: i64,
    },
    /// The pending login expired between the two steps.
    StepExpired,
    /// The deployment has no operator configuration.
    Unconfigured {
        /// Why (already bounded; rendered verbatim).
        reason: String,
    },
}

/// Render the sign-in page for `state`.
pub fn render(meta: &PageMeta, state: &LoginState) -> String {
    let body = match state {
        LoginState::Unconfigured { reason } => notice(
            StatusKind::Attention,
            copy::UNCONFIGURED_HEADLINE,
            &format!("{} Configuration: {reason}", copy::UNCONFIGURED_BODY),
            None,
        ),
        LoginState::NoSecondFactor { operator_name } => notice(
            StatusKind::Failure,
            copy::OPERATOR_NO_SECOND_FACTOR_HEADLINE,
            &format!(
                "{} Operator: {operator_name}.",
                copy::OPERATOR_NO_SECOND_FACTOR_BODY
            ),
            None,
        ),
        LoginState::RateLimited { until_ms } => notice(
            StatusKind::Attention,
            copy::LOGIN_RATE_LIMITED_HEADLINE,
            &copy::LOGIN_RATE_LIMITED_BODY
                .replace("{until}", &apex_ui::format::utc_datetime(*until_ms)),
            Some(("Return to sign-in", "/control/login")),
        ),
        LoginState::StepExpired => notice(
            StatusKind::Attention,
            copy::LOGIN_STEP_EXPIRED_HEADLINE,
            copy::LOGIN_STEP_EXPIRED_BODY,
            Some(("Start again", "/control/login")),
        ),
        LoginState::Password {
            problem,
            preserved_operator,
            preserved_next,
            reauth,
        } => password_form(
            *problem,
            preserved_operator,
            preserved_next.as_deref(),
            *reauth,
        ),
        LoginState::Code {
            pending_token,
            operator_name,
            problem,
        } => code_form(pending_token, operator_name, *problem),
    };
    let heading = if matches!(state, LoginState::Code { .. }) {
        copy::LOGIN_CODE_HEADLINE
    } else {
        copy::LOGIN_HEADLINE
    };
    let lead = if matches!(state, LoginState::Code { .. }) {
        copy::LOGIN_CODE_LEAD
    } else {
        copy::LOGIN_LEAD
    };
    let document_body = format!(
        "{}<h1 class=\"page-title\">{}</h1>\
         <p class=\"panel-body\">{}</p>\
         <p class=\"panel-body\">{}</p>{body}",
        fixtures_banner(meta),
        html_escape(heading),
        html_escape(lead),
        html_escape(copy::LOGIN_SEPARATION)
    );
    document(meta, copy::LOGIN_HEADLINE, &["Sign in"], &document_body)
}

fn password_form(
    problem: Option<&str>,
    preserved_operator: &str,
    preserved_next: Option<&str>,
    reauth: bool,
) -> String {
    let mut out = String::new();
    if reauth {
        out.push_str(&notice(
            StatusKind::Neutral,
            copy::STEP_UP_HEADLINE,
            copy::STEP_UP_BODY,
            None,
        ));
    }
    if let Some(problem) = problem {
        out.push_str(&notice(StatusKind::Failure, problem, "", None));
    }
    out.push_str("<form method=\"post\" action=\"/control/login\">");
    out.push_str(&field(
        "operator",
        copy::LOGIN_OPERATOR_LABEL,
        None,
        None,
        &input(
            "operator",
            "operator",
            "text",
            Some(preserved_operator),
            true,
            Some("username"),
        ),
    ));
    out.push_str(&field(
        "password",
        copy::LOGIN_PASSWORD_LABEL,
        None,
        None,
        &input(
            "password",
            "password",
            "password",
            None,
            true,
            Some("current-password"),
        ),
    ));
    if let Some(next) = preserved_next {
        out.push_str(&super::hidden("next", next));
    }
    if reauth {
        out.push_str(&super::hidden("reauth", "1"));
    }
    out.push_str(
        "<button class=\"action action-primary\" type=\"submit\">Continue</button></form>",
    );
    out.push_str(&format!(
        "<p class=\"table-note\">{}</p>",
        html_escape(copy::LOGIN_NEXT_STEP)
    ));
    out
}

fn code_form(pending_token: &str, operator_name: &str, problem: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(problem) = problem {
        out.push_str(&notice(StatusKind::Failure, problem, "", None));
    }
    out.push_str("<form method=\"post\" action=\"/control/login/code\">");
    out.push_str(&super::hidden("pending", pending_token));
    out.push_str(&field(
        "code",
        copy::LOGIN_CODE_LABEL,
        Some("Six digits from the operator's authenticator app."),
        None,
        &input("code", "code", "text", None, true, Some("one-time-code")),
    ));
    out.push_str(&format!(
        "<p class=\"table-note\">Operator: {} / {} seconds per code.</p>",
        html_escape(operator_name),
        crate::totp::STEP_SECONDS
    ));
    out.push_str(
        "<button class=\"action action-primary\" type=\"submit\">Continue</button></form>",
    );
    out
}

/// The session-expired page: the safe GET recovery view.
pub fn session_expired(meta: &PageMeta) -> String {
    let body = format!(
        "{}<h1 class=\"page-title\">{}</h1>\
         <p class=\"panel-body\">{}</p>\
         <p class=\"panel-body\">{}</p>",
        fixtures_banner(meta),
        html_escape(copy::SESSION_EXPIRED_HEADLINE),
        html_escape(copy::SESSION_EXPIRED_BODY),
        action("Sign in again", "/control/login", true)
    );
    document(meta, copy::SESSION_EXPIRED_HEADLINE, &["Sign in"], &body)
}

/// The unconfigured-deployment page shown for every authenticated route when
/// no operators exist.
pub fn unconfigured(meta: &PageMeta, reason: &str) -> String {
    render(
        meta,
        &LoginState::Unconfigured {
            reason: reason.to_string(),
        },
    )
}
