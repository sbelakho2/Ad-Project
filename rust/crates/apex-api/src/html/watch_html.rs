//! The setup-watch sign-up page and its confirmation, rendered with the
//! marketing shell of the design system.
//!
//! The request page mirrors the free-audit conventions: sentence-case
//! labels, a validation failure re-renders it with the safe values
//! preserved and the exact field copy linked at each field, and a storage
//! failure re-renders it with the shared submission-failure notice. The
//! confirmation shows the checkout link when the deployment configures one
//! and the honest "we will email a payment link" line otherwise.

use apex_ui::components::{field, form_error_summary, input, notice, panel, split, textarea};
use apex_ui::status::StatusKind;

use super::{html_escape, marketing_document, ConsoleCtx};
use crate::leads::{SUBMISSION_FAILURE_BODY, SUBMISSION_FAILURE_TITLE, TRY_AGAIN_LABEL};

/// The request form action; the POST handler validates and stores.
pub const WATCH_ACTION: &str = "/watch";
/// Environment variable naming the checkout link shown on the confirmation.
pub const WATCH_CHECKOUT_ENV: &str = "APEX_WATCH_CHECKOUT_URL";
/// Where the marketing site explains how the watch works.
pub const HOW_IT_WORKS_HREF: &str = "/how-it-works/";
/// The privacy notice of the marketing site.
pub const PRIVACY_HREF: &str = "/privacy/";
/// The copy above the form.
pub const WATCH_INTRO: &str =
    "Tell us the domain and we will check its public app-ads.txt every morning.";
/// The aside trust line.
pub const WATCH_NO_KEYS_NOTE: &str = "No platform keys are needed for the watch.";
/// The honest confirmation line when no checkout URL is configured.
pub const WATCH_PAYMENT_LINE: &str = "We will email a payment link before the watch starts.";
/// The checkout action label.
pub const WATCH_CHECKOUT_LABEL: &str = "Complete the checkout";
/// The preserved form values of one submission.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchFormValues {
    /// The submitted domain.
    pub domain: String,
    /// The submitted email.
    pub email: String,
    /// The submitted account-id textarea.
    pub accounts: String,
}

/// What the request page is rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchPageState {
    /// The untouched form.
    Fresh,
    /// A validation failure: preserved values plus field errors.
    Invalid {
        /// The submitted values, preserved safely.
        values: WatchFormValues,
        /// The field errors in submission order.
        errors: Vec<apex_watch::WatchFieldError>,
    },
    /// The submit action itself failed (storage); preserved values and the
    /// submission-failure copy.
    SubmissionFailed(Box<WatchFormValues>),
}

/// Render the sign-up request page (`GET /watch/`).
pub fn render_request_page(ctx: &ConsoleCtx, state: &WatchPageState) -> String {
    let (values, errors, failed) = match state {
        WatchPageState::Fresh => (WatchFormValues::default(), Vec::new(), false),
        WatchPageState::Invalid { values, errors } => (values.clone(), errors.clone(), false),
        WatchPageState::SubmissionFailed(values) => ((**values).clone(), Vec::new(), true),
    };
    let mut body = String::from("<h1>Watch your app-ads.txt every day.</h1>");
    body.push_str(&format!("<p>{WATCH_INTRO}</p>"));
    if failed {
        body.push_str(&notice(
            StatusKind::Failure,
            SUBMISSION_FAILURE_TITLE,
            SUBMISSION_FAILURE_BODY,
            None,
        ));
    }
    if !errors.is_empty() {
        if !errors.iter().any(|error| error.field == "summary") {
            body.push_str(&format!(
                "<p class=\"form-errors-headline\">{}</p>",
                html_escape(apex_watch::FORM_SUMMARY)
            ));
        }
        let summary: Vec<(String, String)> = errors
            .iter()
            .map(|error| (error.field.to_string(), error.message.to_string()))
            .collect();
        body.push_str(&form_error_summary(&summary));
    }
    let error_for = |field_id: &str| {
        errors
            .iter()
            .find(|error| error.field == field_id)
            .map(|error| error.message)
    };
    let submit_label = if failed {
        TRY_AGAIN_LABEL
    } else {
        "Start the setup watch"
    };
    let form = format!(
        "<form method=\"post\" action=\"{WATCH_ACTION}\">{}{}{}\
         <div class=\"form-actions\">\
         <button class=\"action action-primary\" type=\"submit\">{submit_label}</button></div>\
         <p class=\"field-help\">We use these details to run the watch and send its alerts. \
         <a href=\"{PRIVACY_HREF}\">Read the privacy notice</a>.</p></form>",
        field(
            "domain",
            "Domain",
            Some("The website whose /app-ads.txt we should watch."),
            error_for("domain"),
            &input(
                "domain",
                "domain",
                "text",
                Some(&values.domain),
                false,
                Some("url")
            ),
        ),
        field(
            "email",
            "Email",
            None,
            error_for("email"),
            &input(
                "email",
                "email",
                "email",
                Some(&values.email),
                false,
                Some("email")
            ),
        ),
        field(
            "accounts",
            "Network account ids (optional)",
            Some("One network = account id per line. Do not include passwords."),
            error_for("accounts"),
            &textarea(
                "accounts",
                "accounts",
                Some(&values.accounts),
                5,
                Some(apex_watch::MAX_ACCOUNT_TEXT_CHARS)
            ),
        ),
    );
    let mut aside = panel(
        "What you get",
        "<p>$99 per month. $199 per month for core portfolios (ten or more publisher account ids in your public app-ads.txt).</p>\
         <p>A daily check of the public app-ads.txt file and an email when something leaks.</p>",
        None,
    );
    aside.push_str(&format!("<p class=\"table-note\">{WATCH_NO_KEYS_NOTE}</p>"));
    body.push_str(&split(&form, &aside));
    marketing_document(ctx, "Setup watch", &body)
}

/// Render the confirmation (`GET /watch/requested`). `checkout_url` is the
/// configured payment link; `None` renders the honest email line instead.
pub fn render_confirmation(
    ctx: &ConsoleCtx,
    reference: Option<&str>,
    checkout_url: Option<&str>,
    tier: Option<&str>,
) -> String {
    let mut body = String::from("<h1>Your watch is requested.</h1>");
    body.push_str(
        "<p>The daily check starts after the first payment. Every alert goes to the email \
         address you gave us.</p>",
    );
    if let Some(tier) = tier {
        body.push_str(&format!(
            "<p class=\"table-note\">Plan from your app count: {}</p>",
            html_escape(tier)
        ));
    }
    match checkout_url {
        Some(url) => {
            body.push_str(&format!(
                "<p><a class=\"action action-primary\" href=\"{}\">{WATCH_CHECKOUT_LABEL}</a></p>",
                html_escape(url)
            ));
            body.push_str(
                "<p class=\"table-note\">The link opens the payment page for the setup watch.</p>",
            );
        }
        None => {
            body.push_str(&format!("<p class=\"table-note\">{WATCH_PAYMENT_LINE}</p>"));
        }
    }
    if let Some(reference) = reference {
        body.push_str(&format!(
            "<p class=\"table-note\">Confirmation ID: <code>{}</code></p>",
            html_escape(reference)
        ));
    }
    body.push_str(&format!(
        "<p><a class=\"action action-secondary\" href=\"{HOW_IT_WORKS_HREF}\">Read how the watch \
         works</a></p>"
    ));
    marketing_document(ctx, "Watch requested", &body)
}

/// Render the one-time email confirmation's success state.
pub fn render_email_confirmed(ctx: &ConsoleCtx, email: &str) -> String {
    let body = format!(
        "<h1>Your email is confirmed.</h1>\
         <p>We recorded {} as the contact for this watch. The daily check starts after the \
         first payment; every alert goes to that address.</p>\
         <p class=\"table-note\">The confirmation link can be used once.</p>",
        html_escape(email)
    );
    marketing_document(ctx, "Email confirmed", &body)
}

/// Render the one-time email confirmation's refusal state (used, expired or
/// unknown token). The token is never echoed.
pub fn render_email_confirmation_refused(ctx: &ConsoleCtx) -> String {
    let body = "<h1>This confirmation link is no longer valid.</h1>\
                <p>The link was used already, expired, or never existed. A fresh sign-up \
                sends a new confirmation link.</p>\
                <p><a class=\"action action-secondary\" href=\"/watch/\">Start a new watch</a></p>";
    marketing_document(ctx, "Confirmation link refused", body)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn ctx() -> ConsoleCtx {
        ConsoleCtx::default()
    }

    #[test]
    fn request_page_carries_the_house_copy_and_the_exact_form() {
        let html = render_request_page(&ctx(), &WatchPageState::Fresh);
        assert!(html.contains("<h1>Watch your app-ads.txt every day.</h1>"));
        assert!(html.contains(WATCH_INTRO));
        for needle in [
            ">Domain</label>",
            ">Email</label>",
            ">Network account ids (optional)</label>",
            ">Start the setup watch</button>",
            "$199 per month for core portfolios (ten or more publisher account ids in your public app-ads.txt).",
            WATCH_NO_KEYS_NOTE,
            "Read the privacy notice",
        ] {
            assert!(html.contains(needle), "missing {needle}");
        }
        assert!(html.contains("<form method=\"post\" action=\"/watch\">"));
        assert!(html.contains("<header class=\"marketing-header\">"));
        assert!(!html.contains("<script"));
    }

    #[test]
    fn validation_state_preserves_safe_values_and_links_errors() {
        let html = render_request_page(
            &ctx(),
            &WatchPageState::Invalid {
                values: WatchFormValues {
                    domain: "\" onmouseover=\"alert(1)".into(),
                    email: "founder@northstar.example".into(),
                    accounts: "admob.com = pub-1".into(),
                },
                errors: vec![
                    apex_watch::WatchFieldError {
                        field: "domain",
                        message: apex_watch::DOMAIN_ERROR,
                    },
                    apex_watch::WatchFieldError {
                        field: "accounts",
                        message: apex_watch::ACCOUNTS_ERROR,
                    },
                ],
            },
        );
        assert!(html.contains("role=\"alert\""));
        assert!(html.contains(&format!(
            "<a href=\"#domain\">{}</a>",
            apex_watch::DOMAIN_ERROR
        )));
        assert!(html.contains("value=\"&quot; onmouseover=&quot;alert(1)\""));
        assert!(!html.contains("onmouseover=\"alert(1)\""));
        assert!(html.contains("admob.com = pub-1"));
        assert!(!html.contains("type=\"checkbox\""), "{html}");
        assert!(!html.contains(SUBMISSION_FAILURE_TITLE));
    }

    #[test]
    fn submission_failure_switches_the_action_to_try_again() {
        let html = render_request_page(
            &ctx(),
            &WatchPageState::SubmissionFailed(Box::new(WatchFormValues {
                domain: "northstar.example".into(),
                email: "founder@northstar.example".into(),
                accounts: String::new(),
            })),
        );
        assert!(html.contains(SUBMISSION_FAILURE_TITLE));
        assert!(html.contains(">Try again</button>"));
        assert!(!html.contains(">Start the setup watch</button>"));
        assert!(html.contains("value=\"northstar.example\""));
    }

    #[test]
    fn confirmation_shows_the_checkout_link_only_when_configured() {
        let with_link = render_confirmation(
            &ctx(),
            Some("sub_0123456789"),
            Some("https://checkout.example/pay?x=1&y=2"),
            Some("core ($199 per month)"),
        );
        assert!(with_link.contains("<h1>Your watch is requested.</h1>"));
        assert!(with_link.contains(WATCH_CHECKOUT_LABEL));
        assert!(with_link.contains("href=\"https://checkout.example/pay?x=1&amp;y=2\""));
        assert!(with_link.contains("Confirmation ID: <code>sub_0123456789</code>"));
        assert!(!with_link.contains(WATCH_PAYMENT_LINE));
        assert!(with_link.contains("Plan from your app count: core ($199 per month)"));

        let without = render_confirmation(&ctx(), None, None, None);
        assert!(without.contains(WATCH_PAYMENT_LINE));
        assert!(!without.contains(WATCH_CHECKOUT_LABEL));
        assert!(!without.contains("Confirmation ID"));

        let hostile =
            render_confirmation(&ctx(), Some("sub_<script>alert(1)</script>"), None, None);
        assert!(!hostile.contains("<script>alert(1)"));
        assert!(hostile.contains("&lt;script&gt;"));
    }
}
