//! The setup-watch sign-up page and its confirmation, rendered with the
//! marketing shell of the design system.
//!
//! The request page mirrors the free-audit conventions: sentence-case
//! labels, a validation failure re-renders it with the safe values
//! preserved and the exact field copy linked at each field, and a storage
//! failure re-renders it with the shared submission-failure notice. A3 made
//! the watch free: there is no checkout link, tier price or first invoice.

use apex_ui::components::{field, form_error_summary, input, notice, panel, split, textarea};
use apex_ui::status::StatusKind;

use super::{html_escape, marketing_document, ConsoleCtx};
use crate::leads::{SUBMISSION_FAILURE_BODY, SUBMISSION_FAILURE_TITLE, TRY_AGAIN_LABEL};

/// The request form action; the POST handler validates and stores.
pub const WATCH_ACTION: &str = "/watch";
/// Historical env name kept for a deployment that still sets it; the free
/// watch never shows a checkout link.
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
pub const WATCH_FREE_LINE: &str = "Confirm to start the free watch. There is no payment.";
/// The honest line when the App Store lookup could not answer.
pub const WATCH_VERIFICATION_UNAVAILABLE: &str =
    "We could not verify some of your named apps against the app stores; only confirmed \
     apps and developer catalogues count toward your plan.";
/// The most distinct apps one submission may name.
pub const MAX_WATCH_APPS: usize = 50;
/// Exact field copy for a malformed or oversized apps textarea.
pub const WATCH_APPS_ERROR: &str =
    "Enter one app-store id, package id, or URL per line, up to 50 lines.";
/// The watch activation action label.
/// The preserved form values of one submission.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchFormValues {
    /// The submitted domain.
    pub domain: String,
    /// The submitted email.
    pub email: String,
    /// The submitted account-id textarea.
    pub accounts: String,
    /// The submitted app textarea: one app-store id or URL per line.
    pub apps: String,
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
    let privacy = apex_ui::shell::site_href("/privacy/");
    let form = format!(
        "<form method=\"post\" action=\"{WATCH_ACTION}\">{}{}{}{}\
         <div class=\"form-actions\">\
         <button class=\"action action-primary\" type=\"submit\">{submit_label}</button></div>\
         <p class=\"field-help\">We use these details to run the watch and send its alerts. \
         <a href=\"{privacy}\">Read the privacy notice</a>.</p></form>",
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
            "apps",
            "Apps (optional)",
            Some(
                "One App Store bundle id, Google Play package id, or store URL per line. Ten or more apps in the verified developer catalogues make this a core portfolio. The lookup checks at most 20 named apps, so put the ones you most want counted first.",
            ),
            error_for("apps"),
            &textarea("apps", "apps", Some(&values.apps), 5, None),
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
        "<p>Free with every connection. No monitoring tier, no Setup Watch fee and no checkout.</p>\
         <p>A daily check of the public app-ads.txt file and an email when something leaks.</p>",
        None,
    );
    aside.push_str(&format!("<p class=\"table-note\">{WATCH_NO_KEYS_NOTE}</p>"));
    body.push_str(&split(&form, &aside));
    marketing_document(ctx, "Setup watch", &body)
}

/// Render a pending confirmation. The watch is free: the page carries only
/// the confirmation id, the free line and the next step, never a checkout,
/// tier price or first invoice.
pub fn render_confirmation(
    ctx: &ConsoleCtx,
    reference: Option<&str>,
    verified_app_count: Option<u32>,
) -> String {
    let mut body = String::from("<h1>Your watch is requested.</h1>");
    body.push_str(
        "<p>Confirm the email we sent and the free daily check starts. Every alert goes \
         to the email address you gave us.</p>",
    );
    if let Some(count) = verified_app_count {
        body.push_str(&format!(
            "<p class=\"table-note\">{} app{} are in the verified store catalogue.</p>",
            count,
            if count == 1 { "" } else { "s" }
        ));
    }
    body.push_str(&format!("<p class=\"table-note\">{WATCH_FREE_LINE}</p>"));
    if let Some(reference) = reference {
        body.push_str(&format!(
            "<p class=\"table-note\">Confirmation ID: <code>{}</code></p>",
            html_escape(reference)
        ));
    }
    let how_it_works = apex_ui::shell::site_href("/how-it-works/");
    body.push_str(&format!(
        "<p><a class=\"action action-secondary\" href=\"{how_it_works}\">Read how the watch works</a></p>"
    ));
    marketing_document(ctx, "Watch requested", &body)
}

/// The A3 free watch: no payment, no tier, no billing form.
pub fn render_free_watch(
    ctx: &ConsoleCtx,
    reference: Option<&str>,
    verified_app_count: Option<u32>,
) -> String {
    let mut body = String::from("<h1>Your watch is active.</h1>");
    body.push_str(
        "<p>The daily check starts now. Monitoring, the audit and the app-ads.txt watch are \
         free; every alert goes to the email address you confirmed.</p>",
    );
    if let Some(count) = verified_app_count {
        body.push_str(&format!(
            "<p class=\"table-note\">{} app{} are in the verified store catalogue.</p>",
            count,
            if count == 1 { "" } else { "s" }
        ));
    }
    if let Some(reference) = reference {
        body.push_str(&format!(
            "<p class=\"table-note\">Confirmation ID: <code>{}</code></p>",
            html_escape(reference)
        ));
    }
    let how_it_works = apex_ui::shell::site_href("/how-it-works/");
    body.push_str(&format!(
        "<p><a class=\"action action-secondary\" href=\"{how_it_works}\">Read how the watch works</a></p>"
    ));
    marketing_document(ctx, "Watch active", &body)
}

/// Render the inert email-confirmation landing page. The GET only shows a
/// button that POSTs the token back, so a mail scanner that follows the link
/// cannot consume it. The token is the capability; the form needs no CSRF
/// value of its own.
pub fn render_email_confirmation_form(ctx: &ConsoleCtx, token: &str) -> String {
    let body = format!(
        "<h1>Confirm your watch email.</h1>\
         <p>The button confirms the email address this link was sent to. The link works once; \
         opening it again does not confirm anything.</p>\
         <form method=\"post\" action=\"{}\">\
         <input type=\"hidden\" name=\"token\" value=\"{}\">\
         <div class=\"form-actions\">\
         <button class=\"action action-primary\" type=\"submit\">Confirm my email</button></div>\
         </form>",
        crate::signin::WATCH_CONFIRM_PATH,
        html_escape(token)
    );
    marketing_document(ctx, "Confirm watch email", &body)
}

/// Render the one-time email confirmation's success state.
pub fn render_email_confirmed(ctx: &ConsoleCtx, email: &str) -> String {
    let body = format!(
        "<h1>Your email is confirmed.</h1>\
         <p>We recorded {} as the contact for this watch. The daily check starts when the \
         billing details are recorded; there is no payment. Every alert goes to that address.</p>\
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

/// The cancel page path.
pub const CANCEL_PATH: &str = "/watch/cancel";

/// Render the inert cancel confirmation (the token is the capability; GET
/// never cancels, matching mail-scanner safety).
pub fn render_cancel_form(ctx: &ConsoleCtx, token: &str) -> String {
    let body = format!(
        "<h1>Cancel your setup watch?</h1>\
         <p>Cancelling stops the daily checks and every alert for this watch.</p>\
         <form method=\"post\" action=\"{}\" class=\"form\">\
         <input type=\"hidden\" name=\"token\" value=\"{}\">\
         <div class=\"form-actions\">\
         <button class=\"action action-primary\" type=\"submit\">Cancel this watch</button>\
         <a class=\"action action-secondary\" href=\"/watch/\">Keep watching</a>\
         </div></form>",
        CANCEL_PATH,
        html_escape(token)
    );
    marketing_document(ctx, "Cancel watch", &body)
}

/// Render the cancelled state.
pub fn render_cancel_done(ctx: &ConsoleCtx, domain: &str) -> String {
    let body = format!(
        "<h1>Your setup watch is cancelled.</h1>\
         <p>No further checks or alerts will be sent for {}. The cancellation is recorded.</p>\
         <p><a class=\"action action-secondary\" href=\"/watch/\">Start another watch</a></p>",
        html_escape(domain)
    );
    marketing_document(ctx, "Watch cancelled", &body)
}

/// Render the refusal state for an unknown, used or expired cancel token.
pub fn render_cancel_refused(ctx: &ConsoleCtx) -> String {
    let body = "<h1>This cancellation link is no longer valid.</h1>\
                <p>The watch was already cancelled, the link expired, or it never existed.</p>\
                <p><a class=\"action action-secondary\" href=\"/watch/\">Back to the setup watch</a></p>";
    marketing_document(ctx, "Cancellation link refused", body)
}

/// A generic invalid-reference page: never claims a filtered-out reference
/// was requested or recorded.
pub fn render_reference_not_found(ctx: &ConsoleCtx) -> String {
    let body = "<h1>This link is not valid or has expired.</h1>\
                <p>The reference is unknown or no longer active. A fresh sign-up starts a new \
                watch and sends its own links.</p>\
                <p><a class=\"action action-secondary\" href=\"/watch/\">Start a new watch</a></p>";
    marketing_document(ctx, "Link not found", body)
}

/// The billing-profile form action.
pub const BILLING_PROFILE_ACTION: &str = "/watch/billing-profile";
/// The stable result page the form redirects to.
pub const BILLING_PROFILE_RECORDED_PATH: &str = "/watch/billing-profile/recorded";
/// Legal-name field copy.
pub const LEGAL_NAME_ERROR: &str = "Enter the legal name that should appear on the invoice.";
/// Address field copy.
pub const ADDRESS_ERROR: &str =
    "Enter one address line per line (at least one, at most ten, 500 characters each).";
/// Country field copy.
pub const COUNTRY_ERROR: &str = "Enter the two-letter country code.";
/// VAT field copy.
pub const VAT_ERROR: &str = "Enter a VAT number of 4 to 14 letters and digits, or leave it blank.";
/// The free-watch note shown while the email is still pending.
pub const INVOICE_AT_ACTIVATION: &str =
    "The free watch starts when your billing details are recorded.";

/// One billing-profile field error, with the exact house copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BillingProfileError {
    /// The field id the error belongs to.
    pub field: &'static str,
    /// The exact message.
    pub message: &'static str,
}

/// The values the billing-profile form re-renders after a refusal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BillingProfileFormValues {
    /// The one-time profile token (hidden).
    pub profile_token: String,
    /// The subscriber reference (hidden).
    pub reference: String,
    /// The customer's legal name.
    pub legal_name: String,
    /// Newline-separated address lines.
    pub address_lines: String,
    /// The two-letter country code.
    pub country: String,
    /// The optional VAT number.
    pub vat_number: String,
}

/// Render the billing-profile form shown after a confirmed watch email.
pub fn render_billing_profile_form(
    ctx: &ConsoleCtx,
    email: Option<&str>,
    values: &BillingProfileFormValues,
    errors: &[BillingProfileError],
) -> String {
    let mut body = String::from("<h1>Confirm your billing details.</h1>");
    match email {
        Some(email) => body.push_str(&format!(
            "<p>We record the legal details this watch and its monthly invoices are issued to. The contact email {} was confirmed.</p>",
            html_escape(email)
        )),
        None => body.push_str(
            "<p>We record the legal details this watch and its monthly invoices are issued to. The contact email on this watch was confirmed.</p>",
        ),
    }
    if !errors.is_empty() {
        body.push_str(&format!(
            "<p class=\"form-errors-headline\">{}</p>",
            html_escape(apex_watch::FORM_SUMMARY)
        ));
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
    let privacy = apex_ui::shell::site_href("/privacy/");
    let form = format!(
        "<form method=\"post\" action=\"{BILLING_PROFILE_ACTION}\">\
         <input type=\"hidden\" name=\"profile_token\" value=\"{}\">\
         <input type=\"hidden\" name=\"reference\" value=\"{}\">\
         {}{}{}{}\
         <div class=\"form-actions\">\
         <button class=\"action action-primary\" type=\"submit\">Save billing details</button>\
         </div>\
         <p class=\"field-help\">The billing profile is stored once; a resubmission of this          form is refused. <a href=\"{privacy}\">Read the privacy notice</a>.</p></form>",
        html_escape(&values.profile_token),
        html_escape(&values.reference),
        field(
            "legal_name",
            "Legal name",
            Some("The company or person the invoice is addressed to."),
            error_for("legal_name"),
            &input(
                "legal_name",
                "legal_name",
                "text",
                Some(&values.legal_name),
                false,
                None
            ),
        ),
        field(
            "address_lines",
            "Address",
            Some("One line per row."),
            error_for("address_lines"),
            &textarea(
                "address_lines",
                "address_lines",
                Some(&values.address_lines),
                4,
                None
            ),
        ),
        field(
            "country",
            "Country",
            Some("Two-letter country code, for example EE."),
            error_for("country"),
            &input(
                "country",
                "country",
                "text",
                Some(&values.country),
                false,
                None
            ),
        ),
        field(
            "vat_number",
            "VAT number (optional)",
            None,
            error_for("vat_number"),
            &input(
                "vat_number",
                "vat_number",
                "text",
                Some(&values.vat_number),
                false,
                None
            ),
        ),
    );
    body.push_str(&form);
    marketing_document(ctx, "Billing details", &body)
}

/// Render the stable recorded page after a billing profile is stored.
pub fn render_billing_profile_recorded(
    ctx: &ConsoleCtx,
    reference: &str,
    active: bool,
    invoice_number: Option<u64>,
) -> String {
    let mut body = String::from("<h1>Your billing details are recorded.</h1>");
    body.push_str(
        "<p>The watch is free; no invoice is issued. The billing details cannot be \
         replaced from this form.</p>",
    );
    if invoice_number.is_some() {
        body.push_str(
            "<p class=\"table-note\">A historical invoice reference is kept for the archive.</p>",
        );
    } else if active {
        body.push_str("<p>The free watch is active.</p>");
    } else {
        body.push_str(&format!(
            "<p class=\"table-note\">{INVOICE_AT_ACTIVATION}</p>"
        ));
    }
    if !reference.is_empty() {
        body.push_str(&format!(
            "<p class=\"table-note\">Confirmation ID: <code>{}</code></p>",
            html_escape(reference)
        ));
    }
    body.push_str(
        "<p><a class=\"action action-secondary\" href=\"/watch/\">Back to the setup watch</a></p>",
    );
    marketing_document(ctx, "Billing details recorded", &body)
}

/// Render the refusal state for a used, expired or unknown profile token.
pub fn render_billing_profile_refused(ctx: &ConsoleCtx) -> String {
    let body = "<h1>This billing link is no longer valid.</h1>\
                <p>The link was used already, expired, or never existed. The billing details\
                cannot be changed from an old link.</p>\
                <p><a class=\"action action-secondary\" href=\"/watch/\">Back to the setup watch</a></p>";
    marketing_document(ctx, "Billing link refused", body)
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
            "Free with every connection. No monitoring tier, no Setup Watch fee and no checkout.",
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
                    apps: "com.example.app".into(),
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
                apps: "com.example.app".into(),
            })),
        );
        assert!(html.contains(SUBMISSION_FAILURE_TITLE));
        assert!(html.contains(">Try again</button>"));
        assert!(!html.contains(">Start the setup watch</button>"));
        assert!(html.contains("value=\"northstar.example\""));
    }

    #[test]
    fn pending_confirmation_is_free_and_escapes_the_reference() {
        let html = render_confirmation(&ctx(), Some("sub_0123456789"), Some(10));
        assert!(html.contains("<h1>Your watch is requested.</h1>"));
        assert!(html.contains(WATCH_FREE_LINE));
        assert!(html.contains("10 apps are in the verified store catalogue"));
        assert!(html.contains("Confirmation ID: <code>sub_0123456789</code>"));
        assert!(!html.contains("checkout"));
        assert!(!html.contains("payment link"));

        let without = render_confirmation(&ctx(), None, None);
        assert!(without.contains(WATCH_FREE_LINE));
        assert!(!without.contains("Confirmation ID"));

        let hostile = render_confirmation(&ctx(), Some("sub_<script>alert(1)</script>"), None);
        assert!(!hostile.contains("<script>alert(1)"));
        assert!(hostile.contains("&lt;script&gt;"));
    }
}
