//! The transactional email catalog: the finished copy of section 06 pages
//! 93 to 95, rendered as a 600 px HTML body with a matching plain-text part.
//!
//! The copy is frozen. Each render function interpolates only the values the
//! design marks as data (amounts, dates, references); the sentences, section
//! order and action labels are exactly as written. Editorial rules are
//! enforced in data, not by convention:
//!
//! - `payment_retry` refuses to render without a scheduled retry date
//!   ("Show this message only when a retry actually exists."), and its copy
//!   says the payment link was re-sent — never that a saved payment method
//!   will be charged, because nothing runs a mandate retry (MM),
//! - `invoice_ready` refuses to render without a real https payment link,
//!   and its dedup identity carries the delivery attempt so a dunning
//!   reminder is never suppressed as a duplicate,
//! - `payment_received` always states the received amount and the remaining
//!   balance, so a partial payment reads explicitly,
//! - `restore_failed` carries the incident block (what is known / who is
//!   affected / what we are doing / next update / reference) and derives its
//!   dedup identity from the incident id, so retries reuse the same incident,
//! - `approval_requested` always includes "This message does not grant
//!   approval.".
//!
//! Every message has exactly one primary action, a system-font stack and
//! safe presentation tables.

use apex_metrics::vocab::TemplateClass;

use crate::money::{
    format_day, format_money_or_not_available, format_usd, month_name, FormatError,
};

/// Why a message could not be rendered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    /// A field that the copy renders was empty.
    #[error("template field '{field}' is required")]
    MissingField {
        /// The missing field.
        field: &'static str,
    },
    /// An amount was negative.
    #[error(transparent)]
    Format(#[from] FormatError),
    /// The retry message was asked to render without a scheduled retry.
    #[error("a payment retry message needs a scheduled retry date")]
    RetryWithoutDate,
    /// The action URL is not an https URL.
    #[error("action url '{url}' must be an https:// console link")]
    InvalidActionUrl {
        /// The refused URL.
        url: String,
    },
    /// A field interpolated into a header carries control characters.
    #[error("template field '{field}' must not contain control characters")]
    UnsafeField {
        /// The offending field.
        field: &'static str,
    },
}

/// A rendered message, ready for a [`crate::NotificationSink`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Message {
    /// The subject line, exactly as the design writes it.
    pub subject: String,
    /// The 600 px HTML part.
    pub html: String,
    /// The plain-text part.
    pub text: String,
    /// The template this message came from.
    #[serde(with = "crate::vocab_serde")]
    pub template_class: TemplateClass,
    /// The deduplication identity; the same `(template_class, dedup_key)`
    /// never sends twice.
    pub dedup_key: String,
    /// The durable reference (draft, invoice, incident, experiment or
    /// proposal identity) the reader can quote.
    pub reference: Option<String>,
}

/// The incident block of the restore-failed message (page 95): "What is
/// known / who is affected / what we are doing / next update / reference.
/// Keep the same incident ID across replies."
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncidentBlock {
    /// What is known about the current platform state.
    pub what_is_known: String,
    /// Who is affected.
    pub who_is_affected: String,
    /// What Apex is doing about it.
    pub what_we_are_doing: String,
    /// When the next update will be sent.
    pub next_update: String,
}

/// Data for the fee-preview message (page 93).
#[derive(Debug, Clone)]
pub struct FeePreview {
    /// The billing month, `YYYY-MM`.
    pub month: String,
    /// The draft lift fee before tax, in micros.
    pub amount_micros: i64,
    /// The draft reference the reader can quote.
    pub draft_reference: String,
    /// The authenticated calculation page.
    pub action_url: String,
}

/// Data for the invoice message: the issued invoice and the payment link
/// that pays it. The link is the provider's durable Payment Links API URL,
/// so it does not expire the way a checkout payment does (E7).
#[derive(Debug, Clone)]
pub struct InvoiceReady {
    /// The invoice reference.
    pub invoice_reference: String,
    /// The invoice total due, in micros.
    pub amount_micros: i64,
    /// The invoice currency; `None` or a malformed code renders the
    /// not-available reason, never USD.
    pub amount_currency: Option<String>,
    /// The payment link that pays this invoice; the message's one primary
    /// action.
    pub payment_url: String,
    /// Which delivery this is (1 = the first send). The dunning re-sends
    /// carry the attempt, so a reminder is never suppressed as a duplicate
    /// of the original message.
    pub attempt: u32,
}

/// Data for the payment-retry message (page 93).
#[derive(Debug, Clone)]
pub struct PaymentRetry {
    /// The invoice reference.
    pub invoice_reference: String,
    /// The unpaid balance, in micros.
    pub balance_micros: i64,
    /// The balance's ISO-4217 currency from the durable record; `None` or a
    /// malformed code renders the not-available reason, never USD.
    pub balance_currency: Option<String>,
    /// The scheduled retry day, `YYYY-MM-DD`; `None` refuses to render.
    pub retry_day: Option<String>,
    /// The authenticated invoice page.
    pub action_url: String,
}

/// Data for the payment-failed message (page 93).
#[derive(Debug, Clone)]
pub struct PaymentFailed {
    /// The invoice reference.
    pub invoice_reference: String,
    /// The amount that could not be collected, in micros.
    pub amount_micros: i64,
    /// The amount's ISO-4217 currency from the provider event; `None` or a
    /// malformed code renders the not-available reason, never USD.
    pub amount_currency: Option<String>,
    /// The provider event key; identity only, never rendered.
    pub provider_event_key: String,
    /// The authenticated invoice page.
    pub action_url: String,
}

/// Data for the payment-received message (page 93).
#[derive(Debug, Clone)]
pub struct PaymentReceived {
    /// The invoice reference.
    pub invoice_reference: String,
    /// The amount received, in micros.
    pub received_micros: i64,
    /// The remaining balance after this payment, in micros.
    pub remaining_micros: i64,
    /// The provider event's ISO-4217 currency, used for both amounts;
    /// `None` or a malformed code renders the not-available reason, never
    /// USD.
    pub amount_currency: Option<String>,
    /// The day the payment was applied, `YYYY-MM-DD`.
    pub applied_day: String,
    /// The provider event key; identity only, never rendered.
    pub provider_event_key: String,
    /// The authenticated receipt page.
    pub action_url: String,
}

/// Data for the audit-ready message (page 94).
#[derive(Debug, Clone)]
pub struct AuditReady {
    /// The platform label the copy shows ("MAX").
    pub platform_label: String,
    /// The snapshot day, `YYYY-MM-DD`.
    pub snapshot_day: String,
    /// The authenticated audit page.
    pub action_url: String,
}

/// Data for the reporting-access message (page 94).
#[derive(Debug, Clone)]
pub struct ReportingAccess {
    /// The platform label the copy shows ("MAX").
    pub platform_label: String,
    /// The publisher display name ("Northstar").
    pub publisher_label: String,
    /// The day of the refused pull, for deduplication.
    pub day: String,
    /// The authenticated connections page.
    pub action_url: String,
}

/// Data for the operator-requested reconnection message. This is distinct
/// from an organic reporting-access refusal: it names the publisher, the
/// platform and the access to restore, and its dedup identity is the job's
/// request rather than a pull day.
#[derive(Debug, Clone)]
pub struct ReconnectionRequested {
    /// The platform label the copy shows ("MAX").
    pub platform_label: String,
    /// The publisher display name ("Northstar").
    pub publisher_label: String,
    /// What access the operator asked to restore ("reporting access").
    pub access_label: String,
    /// The request day, for deduplication.
    pub day: String,
    /// The authenticated connections page.
    pub action_url: String,
}

/// Data for the guardrail-stop message (page 95).
#[derive(Debug, Clone)]
pub struct GuardrailStop {
    /// The experiment identity.
    pub experiment_id: String,
    /// The authenticated experiment-result page.
    pub action_url: String,
}

/// Data for the restore-failed message (page 95).
#[derive(Debug, Clone)]
pub struct RestoreFailed {
    /// The experiment identity.
    pub experiment_id: String,
    /// The incident identity; the same id is kept across retries.
    pub incident_id: String,
    /// The incident block.
    pub incident: IncidentBlock,
    /// The authenticated incident page.
    pub action_url: String,
}

/// Data for the proof-hold message (page 95).
#[derive(Debug, Clone)]
pub struct ProofHold {
    /// The draft fee reference.
    pub draft_reference: String,
    /// The authenticated fee-status page.
    pub action_url: String,
}

/// Data for the approval-requested message (page 94).
#[derive(Debug, Clone)]
pub struct ApprovalRequested {
    /// The proposal identity (deduplication only; the copy names the test).
    pub proposal_id: String,
    /// The proposed change ("Rewarded video").
    pub test_label: String,
    /// The country ("Germany").
    pub country: String,
    /// The authenticated proposal page.
    pub action_url: String,
}

/// Data for the levelplay-promotion-required message.
#[derive(Debug, Clone)]
pub struct PromotionRequired {
    /// The platform experiment identity.
    pub experiment_id: String,
    /// What the publisher must do in the platform dashboard.
    pub instructions: String,
    /// The last day a promotion is accepted, as a human date.
    pub deadline: String,
    /// The authenticated console page.
    pub action_url: String,
}

/// The shared layout of one message.
struct Layout {
    subject: String,
    paragraphs: Vec<String>,
    action_label: &'static str,
    action_url: String,
    closing: Option<String>,
    incident: Option<(String, IncidentBlock)>,
    footer: &'static str,
    reference_line: Option<String>,
}

impl Layout {
    fn render(
        self,
        template_class: TemplateClass,
        dedup_key: String,
        reference: Option<String>,
    ) -> Message {
        let subject = self.subject.clone();
        let text = self.render_text();
        let html = self.render_html();
        Message {
            subject,
            html,
            text,
            template_class,
            dedup_key,
            reference,
        }
    }

    fn render_text(&self) -> String {
        let mut out = String::new();
        out.push_str("Apex\n\n");
        out.push_str(&self.subject);
        out.push_str("\n\n");
        for paragraph in &self.paragraphs {
            out.push_str(paragraph);
            out.push_str("\n\n");
        }
        out.push_str(self.action_label);
        out.push_str(": ");
        out.push_str(&self.action_url);
        out.push('\n');
        if let Some(closing) = &self.closing {
            out.push('\n');
            out.push_str(closing);
            out.push('\n');
        }
        if let Some((_, incident)) = &self.incident {
            out.push('\n');
            out.push_str("What is known: ");
            out.push_str(&incident.what_is_known);
            out.push('\n');
            out.push_str("Who is affected: ");
            out.push_str(&incident.who_is_affected);
            out.push('\n');
            out.push_str("What we are doing: ");
            out.push_str(&incident.what_we_are_doing);
            out.push('\n');
            out.push_str("Next update: ");
            out.push_str(&incident.next_update);
            out.push('\n');
        }
        out.push('\n');
        out.push_str(self.footer);
        out.push('\n');
        if let Some(reference) = &self.reference_line {
            out.push_str("Reference: ");
            out.push_str(reference);
            out.push('\n');
        }
        out
    }

    fn render_html(&self) -> String {
        let paragraph = |text: &str| {
            format!(
                "<tr><td style=\"padding:8px 32px;font-size:16px;line-height:26px;\">{}</td></tr>",
                escape_html(text)
            )
        };
        let mut rows = String::new();
        rows.push_str(
            "<tr><td style=\"padding:32px 32px 8px;font-size:20px;font-weight:700;\">Apex</td></tr>",
        );
        rows.push_str(&format!(
            "<tr><td style=\"padding:0 32px 8px;font-size:24px;font-weight:700;line-height:32px;\">{}</td></tr>",
            escape_html(&self.subject)
        ));
        for text in &self.paragraphs {
            rows.push_str(&paragraph(text));
        }
        rows.push_str(&format!(
            "<tr><td style=\"padding:16px 32px;\"><a href=\"{url}\" style=\"display:inline-block;background:#2849d8;color:#ffffff;text-decoration:none;padding:12px 24px;border-radius:6px;font-weight:700;\">{label}</a></td></tr>",
            url = escape_html_attribute(&self.action_url),
            label = escape_html(self.action_label)
        ));
        if let Some(closing) = &self.closing {
            rows.push_str(&paragraph(closing));
        }
        if let Some((incident_id, incident)) = &self.incident {
            let mut incident_rows = String::new();
            for (label, value) in [
                ("What is known", &incident.what_is_known),
                ("Who is affected", &incident.who_is_affected),
                ("What we are doing", &incident.what_we_are_doing),
                ("Next update", &incident.next_update),
                ("Reference", &incident_id.to_string()),
            ] {
                incident_rows.push_str(&format!(
                    "<tr><td style=\"padding:2px 12px 2px 0;font-size:14px;line-height:20px;color:#536568;\">{}</td><td style=\"padding:2px 0;font-size:14px;line-height:20px;\">{}</td></tr>",
                    escape_html(label),
                    escape_html(value)
                ));
            }
            rows.push_str(&format!(
                "<tr><td style=\"padding:8px 32px;\"><table role=\"presentation\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\">{incident_rows}</table></td></tr>"
            ));
        }
        let mut footer = escape_html(self.footer);
        if let Some(reference) = &self.reference_line {
            footer.push_str("<br>Reference: ");
            footer.push_str(&escape_html(reference));
        }
        rows.push_str(&format!(
            "<tr><td style=\"padding:16px 32px 32px;color:#536568;font-size:13px;line-height:20px;\">{footer}</td></tr>"
        ));
        format!(
            "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title}</title></head>\n<body style=\"margin:0;padding:0;background:#f5f6f1;\">\n<table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\"><tr><td align=\"center\" style=\"padding:24px 12px;\">\n<table role=\"presentation\" width=\"600\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"width:600px;max-width:600px;background:#ffffff;border-radius:8px;font-family:-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif;color:#182c31;\">\n{rows}</table>\n</td></tr></table>\n</body>\n</html>\n",
            title = escape_html(&self.subject)
        )
    }
}

/// HTML element escaping for interpolated copy.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Attribute escaping (quotes included).
fn escape_html_attribute(text: &str) -> String {
    escape_html(text)
}

fn require(field: &'static str, value: &str) -> Result<(), TemplateError> {
    if value.trim().is_empty() {
        Err(TemplateError::MissingField { field })
    } else {
        Ok(())
    }
}

/// A subject/header field: required and single-line, so a stored or operator
/// supplied value can never smuggle a header line.
fn require_header_text(field: &'static str, value: &str) -> Result<(), TemplateError> {
    require(field, value)?;
    if value.chars().any(char::is_control) {
        Err(TemplateError::UnsafeField { field })
    } else {
        Ok(())
    }
}

fn check_url(url: &str) -> Result<(), TemplateError> {
    if url.starts_with("https://") && url.len() > "https://".len() {
        Ok(())
    } else {
        Err(TemplateError::InvalidActionUrl {
            url: url.to_string(),
        })
    }
}

/// The fee-preview message (page 93): "Your October Apex fee preview".
pub fn fee_preview(data: &FeePreview) -> Result<Message, TemplateError> {
    require("draft_reference", &data.draft_reference)?;
    check_url(&data.action_url)?;
    let month = month_name(&data.month)?;
    let amount = format_usd(data.amount_micros)?;
    let monitoring = "Monitoring, the audit and app-ads.txt watch are free.";
    let layout = Layout {
        subject: format!("Your {month} Apex fee preview"),
        paragraphs: vec![
            format!("Your draft lift fee for {month} is {amount} before tax. {monitoring}"),
            "Open the calculation to review the eligible revenue, billable lift, agreed rate and proof references. The page also shows the invoice and collection dates when they are confirmed.".to_string(),
        ],
        action_label: "Review fee calculation",
        action_url: data.action_url.clone(),
        closing: Some(
            "If something looks wrong, contact us with the draft reference before the invoice is issued."
                .to_string(),
        ),
        incident: None,
        footer: "Apex / Billing",
        reference_line: Some(data.draft_reference.clone()),
    };
    Ok(layout.render(
        TemplateClass::FeePreview,
        format!("fee_preview:{}", data.draft_reference),
        Some(data.draft_reference.clone()),
    ))
}

/// The invoice message: "Your Apex invoice is ready". The provider payment
/// link is the one primary action and never renders without a real https
/// link. Dunning re-sends carry the attempt number in the dedup identity, so
/// each reminder delivers exactly once.
pub fn invoice_ready(data: &InvoiceReady) -> Result<Message, TemplateError> {
    require("invoice_reference", &data.invoice_reference)?;
    require("payment_url", &data.payment_url)?;
    check_url(&data.payment_url)?;
    if data.attempt == 0 {
        return Err(TemplateError::MissingField { field: "attempt" });
    }
    let amount =
        format_money_or_not_available(data.amount_currency.as_deref(), data.amount_micros)?;
    let layout = Layout {
        subject: "Your Apex invoice is ready".to_string(),
        paragraphs: vec![
            format!("Invoice {} is ready. The total due is {amount}.", data.invoice_reference),
            "Pay online with the button above. The payment link does not expire; the calculation, the proof references and the paid status stay available in the console.".to_string(),
        ],
        action_label: "Pay invoice",
        action_url: data.payment_url.clone(),
        closing: Some(
            "If the amount looks wrong, contact us with the invoice reference before paying."
                .to_string(),
        ),
        incident: None,
        footer: "Apex / Billing",
        reference_line: Some(data.invoice_reference.clone()),
    };
    Ok(layout.render(
        TemplateClass::InvoiceReady,
        format!("invoice_ready:{}:{}", data.invoice_reference, data.attempt),
        Some(data.invoice_reference.clone()),
    ))
}

/// The payment-retry message: the payment link was re-sent. Nothing runs a
/// mandate retry and no saved payment method exists, so the copy never
/// claims a card charge will happen (MM); the scheduled retry date is the
/// next link reminder. Refuses to render without a real retry date.
pub fn payment_retry(data: &PaymentRetry) -> Result<Message, TemplateError> {
    require("invoice_reference", &data.invoice_reference)?;
    check_url(&data.action_url)?;
    let Some(retry_day) = data
        .retry_day
        .as_deref()
        .filter(|day| !day.trim().is_empty())
    else {
        return Err(TemplateError::RetryWithoutDate);
    };
    let retry_day = format_day(retry_day)?;
    let balance =
        format_money_or_not_available(data.balance_currency.as_deref(), data.balance_micros)?;
    let layout = Layout {
        subject: "Your Apex invoice payment link was re-sent".to_string(),
        paragraphs: vec![
            format!("Invoice {} has an unpaid balance of {balance}. We have re-sent the payment link for it; nothing is charged to a saved payment method.", data.invoice_reference),
            format!("Pay with the link in the invoice, or contact us with the reference if you have already arranged a different payment. The next payment-link reminder is scheduled for {retry_day}."),
        ],
        action_label: "Review invoice",
        action_url: data.action_url.clone(),
        closing: None,
        incident: None,
        footer: "Apex / Billing",
        reference_line: Some(data.invoice_reference.clone()),
    };
    Ok(layout.render(
        TemplateClass::PaymentRetry,
        format!(
            "payment_retry:{}:{}",
            data.invoice_reference,
            data.retry_day.as_deref().unwrap_or_default()
        ),
        Some(data.invoice_reference.clone()),
    ))
}

/// The payment-failed message (page 93): "Payment needs attention for
/// invoice INV-2026-104".
pub fn payment_failed(data: &PaymentFailed) -> Result<Message, TemplateError> {
    require("invoice_reference", &data.invoice_reference)?;
    require("provider_event_key", &data.provider_event_key)?;
    check_url(&data.action_url)?;
    let amount =
        format_money_or_not_available(data.amount_currency.as_deref(), data.amount_micros)?;
    let layout = Layout {
        subject: format!(
            "Payment needs attention for invoice {}",
            data.invoice_reference
        ),
        paragraphs: vec![
            format!("We could not collect {amount} for this invoice. No payment is confirmed."),
            "Open the invoice to review the payment status and available next steps. We will show a retry date only when it is scheduled.".to_string(),
        ],
        action_label: "Review payment",
        action_url: data.action_url.clone(),
        closing: Some(
            "Do not email card or bank credentials. Contact us if the invoice or amount needs explaining."
                .to_string(),
        ),
        incident: None,
        footer: "Apex / Billing",
        reference_line: Some(data.invoice_reference.clone()),
    };
    Ok(layout.render(
        TemplateClass::PaymentFailed,
        format!(
            "payment_failed:{}:{}",
            data.invoice_reference, data.provider_event_key
        ),
        Some(data.invoice_reference.clone()),
    ))
}

/// The payment-received message (page 93): "Payment received for invoice
/// INV-2026-104". A partial payment always states the received amount and the
/// remaining balance.
pub fn payment_received(data: &PaymentReceived) -> Result<Message, TemplateError> {
    require("invoice_reference", &data.invoice_reference)?;
    require("provider_event_key", &data.provider_event_key)?;
    check_url(&data.action_url)?;
    let received =
        format_money_or_not_available(data.amount_currency.as_deref(), data.received_micros)?;
    let remaining =
        format_money_or_not_available(data.amount_currency.as_deref(), data.remaining_micros)?;
    let applied = format_day(&data.applied_day)?;
    let layout = Layout {
        subject: format!("Payment received for invoice {}", data.invoice_reference),
        paragraphs: vec![
            format!("We received {received} and applied it to this invoice on {applied}. The remaining balance is {remaining}."),
            "Your receipt and calculation remain available in the console.".to_string(),
        ],
        action_label: "View receipt",
        action_url: data.action_url.clone(),
        closing: None,
        incident: None,
        footer: "Apex / Billing",
        reference_line: Some(data.invoice_reference.clone()),
    };
    Ok(layout.render(
        TemplateClass::PaymentReceived,
        format!(
            "payment_received:{}:{}",
            data.invoice_reference, data.provider_event_key
        ),
        Some(data.invoice_reference.clone()),
    ))
}

/// The audit-ready message (page 94): "Your Apex audit is ready".
pub fn audit_ready(data: &AuditReady) -> Result<Message, TemplateError> {
    require("platform_label", &data.platform_label)?;
    check_url(&data.action_url)?;
    let snapshot = format_day(&data.snapshot_day)?;
    let layout = Layout {
        subject: "Your Apex audit is ready".to_string(),
        paragraphs: vec![
            format!(
                "Your {} audit is available for review. It covers the snapshot captured on {snapshot}. The report identifies the findings, evidence and any missing inputs.",
                data.platform_label
            ),
            "Nothing was changed during this audit.".to_string(),
        ],
        action_label: "Open audit",
        action_url: data.action_url.clone(),
        closing: None,
        incident: None,
        footer: "Apex / Service",
        reference_line: Some(data.snapshot_day.clone()),
    };
    Ok(layout.render(
        TemplateClass::AuditReady,
        format!("audit_ready:{}", data.snapshot_day),
        Some(data.snapshot_day.clone()),
    ))
}

/// The reporting-access message (page 94): "Reconnect MAX reporting for
/// Northstar".
pub fn reporting_access(data: &ReportingAccess) -> Result<Message, TemplateError> {
    require_header_text("platform_label", &data.platform_label)?;
    require_header_text("publisher_label", &data.publisher_label)?;
    require("day", &data.day)?;
    check_url(&data.action_url)?;
    let layout = Layout {
        subject: format!(
            "Reconnect {} reporting for {}",
            data.platform_label, data.publisher_label
        ),
        paragraphs: vec![
            "The latest reporting request was refused. New experiment data is not available. Open Connections to review the account and reconnect the reporting credential.".to_string(),
            "Do not send the key by email.".to_string(),
        ],
        action_label: "Review connection",
        action_url: data.action_url.clone(),
        closing: None,
        incident: None,
        footer: "Apex / Service",
        reference_line: Some(data.platform_label.clone()),
    };
    Ok(layout.render(
        TemplateClass::ReportingAccess,
        format!("reporting_access:{}:{}", data.platform_label, data.day),
        Some(data.platform_label.clone()),
    ))
}

/// The operator-requested reconnection message: names the publisher, the
/// platform and the access to restore. It uses the frozen `reporting_access`
/// metric class (the vocabulary has no separate value) but a distinct
/// `reconnection_requested:` dedup identity, so it never suppresses or is
/// suppressed by an organic reporting-access notice.
pub fn reconnection_requested(data: &ReconnectionRequested) -> Result<Message, TemplateError> {
    require_header_text("platform_label", &data.platform_label)?;
    require_header_text("publisher_label", &data.publisher_label)?;
    require_header_text("access_label", &data.access_label)?;
    require("day", &data.day)?;
    check_url(&data.action_url)?;
    let layout = Layout {
        subject: format!(
            "Restore {} access for {}",
            data.platform_label, data.publisher_label
        ),
        paragraphs: vec![
            format!(
                "An operator asked Apex to restore {} for {} on {}. The account's reporting credential is not usable, so new experiment data is not available until the platform access is restored.",
                data.access_label, data.publisher_label, data.platform_label
            ),
            format!(
                "Open Connections to restore {} for {} on {}.",
                data.access_label, data.publisher_label, data.platform_label
            ),
            "Do not send the key by email.".to_string(),
        ],
        action_label: "Review connection",
        action_url: data.action_url.clone(),
        closing: None,
        incident: None,
        footer: "Apex / Service",
        reference_line: Some(data.platform_label.clone()),
    };
    Ok(layout.render(
        TemplateClass::ReportingAccess,
        format!(
            "reconnection_requested:{}:{}:{}",
            data.platform_label, data.publisher_label, data.day
        ),
        Some(data.platform_label.clone()),
    ))
}

/// The guardrail-stop message (page 95): "An Apex test was stopped".
pub fn guardrail_stop(data: &GuardrailStop) -> Result<Message, TemplateError> {
    require("experiment_id", &data.experiment_id)?;
    check_url(&data.action_url)?;
    let layout = Layout {
        subject: "An Apex test was stopped".to_string(),
        paragraphs: vec![format!(
            "Experiment {} triggered a registered stop condition. The console shows the input, threshold and confirmed platform outcome. This result does not create a lift fee.",
            data.experiment_id
        )],
        action_label: "View test result",
        action_url: data.action_url.clone(),
        closing: None,
        incident: None,
        footer: "Apex / Service",
        reference_line: Some(data.experiment_id.clone()),
    };
    Ok(layout.render(
        TemplateClass::GuardrailStop,
        format!("guardrail_stop:{}", data.experiment_id),
        Some(data.experiment_id.clone()),
    ))
}

/// The restore-failed message (page 95): "Action required for experiment
/// exp_0247", with the incident block and a stable incident id.
pub fn restore_failed(data: &RestoreFailed) -> Result<Message, TemplateError> {
    require("experiment_id", &data.experiment_id)?;
    require("incident_id", &data.incident_id)?;
    require("what_is_known", &data.incident.what_is_known)?;
    require("who_is_affected", &data.incident.who_is_affected)?;
    require("what_we_are_doing", &data.incident.what_we_are_doing)?;
    require("next_update", &data.incident.next_update)?;
    check_url(&data.action_url)?;
    let layout = Layout {
        subject: format!("Action required for experiment {}", data.experiment_id),
        paragraphs: vec![
            format!(
                "The platform did not confirm the stop request. Apex has not marked the test restored. We are checking the current state under incident {}.",
                data.incident_id
            ),
            "Open the incident to see the last confirmed state and next update. Do not assume that retrying a request is safe without checking the current platform state.".to_string(),
        ],
        action_label: "View incident",
        action_url: data.action_url.clone(),
        closing: None,
        incident: Some((data.incident_id.clone(), data.incident.clone())),
        footer: "Apex / Service",
        reference_line: Some(data.incident_id.clone()),
    };
    Ok(layout.render(
        TemplateClass::RestoreFailed,
        format!("restore_failed:{}", data.incident_id),
        Some(data.incident_id.clone()),
    ))
}

/// The proof-hold message (page 95): "A fee is on hold while we check its
/// evidence".
pub fn proof_hold(data: &ProofHold) -> Result<Message, TemplateError> {
    require("draft_reference", &data.draft_reference)?;
    check_url(&data.action_url)?;
    let layout = Layout {
        subject: "A fee is on hold while we check its evidence".to_string(),
        paragraphs: vec![
            format!(
                "A required verification check for draft fee {} did not pass. We have not issued or collected this fee from that state.",
                data.draft_reference
            ),
            "The console shows the affected period and reference. We will update the draft when the evidence is verified or the line is removed.".to_string(),
        ],
        action_label: "View fee status",
        action_url: data.action_url.clone(),
        closing: None,
        incident: None,
        footer: "Apex / Billing",
        reference_line: Some(data.draft_reference.clone()),
    };
    Ok(layout.render(
        TemplateClass::ProofHold,
        format!("proof_hold:{}", data.draft_reference),
        Some(data.draft_reference.clone()),
    ))
}

/// A kept change whose platform needs a manual promotion before it can
/// serve everyone (and before any fee).
pub fn promotion_required(data: &PromotionRequired) -> Result<Message, TemplateError> {
    require("experiment_id", &data.experiment_id)?;
    require("instructions", &data.instructions)?;
    require("deadline", &data.deadline)?;
    check_url(&data.action_url)?;
    let layout = Layout {
        subject: "Promote your test to keep the change".to_string(),
        paragraphs: vec![
            data.instructions.clone(),
            format!(
                "Until the promotion is confirmed by the next daily read (by {}), the change is recorded but no fee is billed.",
                data.deadline
            ),
        ],
        action_label: "Open experiments",
        action_url: data.action_url.clone(),
        closing: None,
        incident: None,
        footer: "Apex / Service",
        reference_line: Some(data.experiment_id.clone()),
    };
    Ok(layout.render(
        TemplateClass::ApprovalRequested,
        format!("promotion_required:{}", data.experiment_id),
        Some(data.experiment_id.clone()),
    ))
}

/// The approval-requested message (page 94): "Review a proposed MAX test".
pub fn approval_requested(data: &ApprovalRequested) -> Result<Message, TemplateError> {
    require("proposal_id", &data.proposal_id)?;
    require("test_label", &data.test_label)?;
    require("country", &data.country)?;
    check_url(&data.action_url)?;
    let layout = Layout {
        subject: "Review a proposed MAX test".to_string(),
        paragraphs: vec![
            format!(
                "A proposal is ready for {} in {}. Review the current and test settings, scope, allocation, analysis window and stop conditions before deciding.",
                data.test_label, data.country
            ),
            "This message does not grant approval. Use the authenticated proposal page to approve or decline.".to_string(),
        ],
        action_label: "Review proposal",
        action_url: data.action_url.clone(),
        closing: None,
        incident: None,
        footer: "Apex / Service",
        reference_line: Some(data.proposal_id.clone()),
    };
    Ok(layout.render(
        TemplateClass::ApprovalRequested,
        format!("approval_requested:{}", data.proposal_id),
        Some(data.proposal_id.clone()),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn url() -> String {
        "https://console.apexmediation.ee/billing/calc".to_string()
    }

    fn incident() -> IncidentBlock {
        IncidentBlock {
            what_is_known: "The stop request was sent but the platform has not confirmed it."
                .into(),
            who_is_affected: "One experiment on your MAX account.".into(),
            what_we_are_doing: "Reconciling the current platform state.".into(),
            next_update: "Today, 18:00 UTC.".into(),
        }
    }

    #[test]
    fn fee_preview_renders_the_frozen_copy() {
        let message = fee_preview(&FeePreview {
            month: "2026-10".into(),
            amount_micros: 300_000_000,
            draft_reference: "DF-104".into(),
            action_url: url(),
        })
        .expect("fee preview");
        assert_eq!(message.subject, "Your October Apex fee preview");
        assert!(message
            .text
            .contains("Your draft lift fee for October is USD 300.00 before tax. Monitoring, the audit and app-ads.txt watch are free."));
        assert!(message.text.contains(
            "Open the calculation to review the eligible revenue, billable lift, agreed rate and proof references. The page also shows the invoice and collection dates when they are confirmed."
        ));
        assert!(message.text.contains("Review fee calculation: https://"));
        assert!(message.text.contains(
            "If something looks wrong, contact us with the draft reference before the invoice is issued."
        ));
        assert_eq!(message.template_class, TemplateClass::FeePreview);
        assert_eq!(message.dedup_key, "fee_preview:DF-104");
        assert!(message.html.contains("width=\"600\""));
        assert!(message.html.contains("role=\"presentation\""));
        assert!(message
            .html
            .contains("-apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif"));
        assert!(message.html.contains("background:#2849d8"));
        assert_eq!(message.html.matches("<a ").count(), 1, "one primary action");
        assert!(message.html.contains("Reference: DF-104"));
    }

    #[test]
    fn payment_retry_refuses_without_a_real_retry() {
        let error = payment_retry(&PaymentRetry {
            invoice_reference: "INV-2026-104".into(),
            balance_micros: 300_000_000,
            balance_currency: Some("USD".into()),
            retry_day: None,
            action_url: url(),
        })
        .expect_err("no retry");
        assert_eq!(error, TemplateError::RetryWithoutDate);
        let message = payment_retry(&PaymentRetry {
            invoice_reference: "INV-2026-104".into(),
            balance_micros: 300_000_000,
            balance_currency: Some("USD".into()),
            retry_day: Some("2026-10-20".into()),
            action_url: url(),
        })
        .expect("retry");
        assert_eq!(message.subject, "Your Apex invoice payment link was re-sent");
        assert!(message.text.contains(
            "Invoice INV-2026-104 has an unpaid balance of USD 300.00. We have re-sent the payment link for it; nothing is charged to a saved payment method."
        ));
        assert!(message.text.contains(
            "Pay with the link in the invoice, or contact us with the reference if you have already arranged a different payment. The next payment-link reminder is scheduled for 20 October 2026."
        ));
        assert!(message.text.contains("Review invoice: https://"));
    }

    #[test]
    fn invoice_ready_renders_the_payment_link_and_refuses_fakes() {
        let message = invoice_ready(&InvoiceReady {
            invoice_reference: "INV-2026-10-pub-1".into(),
            amount_micros: 300_000_000,
            amount_currency: Some("USD".into()),
            payment_url: "https://pay.mollie.com/pl_9".into(),
            attempt: 1,
        })
        .expect("invoice");
        assert_eq!(message.subject, "Your Apex invoice is ready");
        assert!(message
            .text
            .contains("Invoice INV-2026-10-pub-1 is ready. The total due is USD 300.00."));
        assert!(message
            .text
            .contains("Pay invoice: https://pay.mollie.com/pl_9"));
        assert!(message.text.contains(
            "The payment link does not expire; the calculation, the proof references and the paid status stay available in the console."
        ));
        assert!(message.text.contains(
            "If the amount looks wrong, contact us with the invoice reference before paying."
        ));
        assert_eq!(message.template_class, TemplateClass::InvoiceReady);
        assert_eq!(message.dedup_key, "invoice_ready:INV-2026-10-pub-1:1");
        assert_eq!(message.reference.as_deref(), Some("INV-2026-10-pub-1"));
        assert_eq!(message.html.matches("<a ").count(), 1, "one primary action");
        assert!(message
            .html
            .contains("href=\"https://pay.mollie.com/pl_9\""));

        // A dunning reminder is its own delivery: the attempt is in the dedup
        // identity, so the reminder is never suppressed as a duplicate.
        let reminder = invoice_ready(&InvoiceReady {
            attempt: 2,
            ..InvoiceReady {
                invoice_reference: "INV-2026-10-pub-1".into(),
                amount_micros: 300_000_000,
                amount_currency: Some("USD".into()),
                payment_url: "https://pay.mollie.com/pl_9".into(),
                attempt: 1,
            }
        })
        .expect("reminder");
        assert_eq!(reminder.dedup_key, "invoice_ready:INV-2026-10-pub-1:2");
        assert_ne!(message.dedup_key, reminder.dedup_key);

        // No payment link, an insecure link or a zero attempt never render.
        let error = invoice_ready(&InvoiceReady {
            payment_url: String::new(),
            ..InvoiceReady {
                invoice_reference: "INV-2026-10-pub-1".into(),
                amount_micros: 300_000_000,
                amount_currency: Some("USD".into()),
                payment_url: "https://pay.mollie.com/pl_9".into(),
                attempt: 1,
            }
        })
        .expect_err("missing link");
        assert!(matches!(error, TemplateError::MissingField { .. }));
        let error = invoice_ready(&InvoiceReady {
            payment_url: "http://pay.mollie.com/pl_9".into(),
            ..InvoiceReady {
                invoice_reference: "INV-2026-10-pub-1".into(),
                amount_micros: 300_000_000,
                amount_currency: Some("USD".into()),
                payment_url: "https://pay.mollie.com/pl_9".into(),
                attempt: 1,
            }
        })
        .expect_err("insecure link");
        assert!(matches!(error, TemplateError::InvalidActionUrl { .. }));
        let error = invoice_ready(&InvoiceReady {
            attempt: 0,
            ..InvoiceReady {
                invoice_reference: "INV-2026-10-pub-1".into(),
                amount_micros: 300_000_000,
                amount_currency: Some("USD".into()),
                payment_url: "https://pay.mollie.com/pl_9".into(),
                attempt: 1,
            }
        })
        .expect_err("zero attempt");
        assert!(matches!(error, TemplateError::MissingField { .. }));
    }

    #[test]
    fn payment_failed_and_received_render_received_and_remaining() {
        let failed = payment_failed(&PaymentFailed {
            invoice_reference: "INV-2026-104".into(),
            amount_micros: 300_000_000,
            amount_currency: Some("USD".into()),
            provider_event_key: "psp_evt_88".into(),
            action_url: url(),
        })
        .expect("failed");
        assert_eq!(
            failed.subject,
            "Payment needs attention for invoice INV-2026-104"
        );
        assert!(failed.text.contains(
            "We could not collect USD 300.00 for this invoice. No payment is confirmed."
        ));
        assert!(failed.text.contains(
            "Open the invoice to review the payment status and available next steps. We will show a retry date only when it is scheduled."
        ));
        assert!(failed.text.contains("Review payment: https://"));
        assert!(failed.text.contains(
            "Do not email card or bank credentials. Contact us if the invoice or amount needs explaining."
        ));
        assert!(failed.text.contains("Reference: INV-2026-104"));

        let partial = payment_received(&PaymentReceived {
            invoice_reference: "INV-2026-104".into(),
            received_micros: 120_000_000,
            remaining_micros: 180_000_000,
            amount_currency: Some("USD".into()),
            applied_day: "2026-10-20".into(),
            provider_event_key: "psp_evt_89".into(),
            action_url: url(),
        })
        .expect("partial");
        assert_eq!(partial.subject, "Payment received for invoice INV-2026-104");
        assert!(partial.text.contains(
            "We received USD 120.00 and applied it to this invoice on 20 October 2026. The remaining balance is USD 180.00."
        ));
        assert!(partial
            .text
            .contains("Your receipt and calculation remain available in the console."));
        assert!(partial.text.contains("View receipt: https://"));
        // The provider key is identity, never rendered.
        assert!(!partial.text.contains("psp_evt_89"));
        assert!(!partial.html.contains("psp_evt_89"));
    }

    #[test]
    fn payment_templates_render_the_events_own_currency() {
        let eur = payment_received(&PaymentReceived {
            invoice_reference: "INV-2026-104".into(),
            received_micros: 120_000_000,
            remaining_micros: 180_000_000,
            amount_currency: Some("EUR".into()),
            applied_day: "2026-10-20".into(),
            provider_event_key: "psp_evt_90".into(),
            action_url: url(),
        })
        .expect("eur");
        assert!(eur.text.contains(
            "We received EUR 120.00 and applied it to this invoice on 20 October 2026. The remaining balance is EUR 180.00."
        ));
        assert!(!eur.text.contains("USD"));

        let eur_retry = payment_retry(&PaymentRetry {
            invoice_reference: "INV-2026-104".into(),
            balance_micros: 300_000_000,
            balance_currency: Some("GBP".into()),
            retry_day: Some("2026-10-20".into()),
            action_url: url(),
        })
        .expect("gbp retry");
        assert!(eur_retry
            .text
            .contains("has an unpaid balance of GBP 300.00"));

        let eur_failed = payment_failed(&PaymentFailed {
            invoice_reference: "INV-2026-104".into(),
            amount_micros: 300_000_000,
            amount_currency: Some("EUR".into()),
            provider_event_key: "psp_evt_91".into(),
            action_url: url(),
        })
        .expect("eur failed");
        assert!(eur_failed
            .text
            .contains("We could not collect EUR 300.00 for this invoice."));

        // An absent currency renders the not-available reason, never USD.
        let unknown = payment_received(&PaymentReceived {
            invoice_reference: "INV-2026-104".into(),
            received_micros: 120_000_000,
            remaining_micros: 180_000_000,
            amount_currency: None,
            applied_day: "2026-10-20".into(),
            provider_event_key: "psp_evt_92".into(),
            action_url: url(),
        })
        .expect("unknown currency");
        assert!(unknown
            .text
            .contains("We received Not available (currency unavailable) and applied it"));
        assert!(unknown
            .text
            .contains("The remaining balance is Not available (currency unavailable)."));
        assert!(!unknown.text.contains("USD"));
        assert!(!unknown.html.contains("USD"));
    }

    #[test]
    fn reconnection_requested_names_the_publisher_platform_and_access() {
        let message = reconnection_requested(&ReconnectionRequested {
            platform_label: "MAX".into(),
            publisher_label: "Northstar".into(),
            access_label: "reporting access".into(),
            day: "2026-10-16".into(),
            action_url: "https://console.apexmediation.ee/connections".into(),
        })
        .expect("reconnection");
        assert_eq!(message.subject, "Restore MAX access for Northstar");
        assert!(message
            .text
            .contains("An operator asked Apex to restore reporting access for Northstar on MAX."));
        assert!(message
            .text
            .contains("Open Connections to restore reporting access for Northstar on MAX."));
        assert!(message.text.contains("Do not send the key by email."));
        assert_eq!(
            message.dedup_key,
            "reconnection_requested:MAX:Northstar:2026-10-16"
        );
        assert_eq!(message.template_class, TemplateClass::ReportingAccess);
        // The recipient is the only personal identifier; the copy never
        // carries an address, credential or provider reference.
        assert!(!message.subject.contains('@'));
        assert!(!message.text.contains('@'));
        // A control character in a header-bound field is refused rather than
        // rendered into a Subject line.
        let error = reconnection_requested(&ReconnectionRequested {
            platform_label: "MAX\r\nBcc: evil@example.com".into(),
            publisher_label: "Northstar".into(),
            access_label: "reporting access".into(),
            day: "2026-10-16".into(),
            action_url: "https://console.apexmediation.ee/connections".into(),
        })
        .expect_err("header injection");
        assert!(matches!(error, TemplateError::UnsafeField { .. }));
    }

    #[test]
    fn service_messages_render_the_frozen_copy() {
        let audit = audit_ready(&AuditReady {
            platform_label: "MAX".into(),
            snapshot_day: "2026-10-14".into(),
            action_url: "https://console.apexmediation.ee/audits/1".into(),
        })
        .expect("audit");
        assert_eq!(audit.subject, "Your Apex audit is ready");
        assert!(audit.text.contains(
            "Your MAX audit is available for review. It covers the snapshot captured on 14 October 2026. The report identifies the findings, evidence and any missing inputs."
        ));
        assert!(audit
            .text
            .contains("Nothing was changed during this audit."));
        assert!(audit.text.contains("Open audit: https://"));

        let access = reporting_access(&ReportingAccess {
            platform_label: "MAX".into(),
            publisher_label: "Northstar".into(),
            day: "2026-10-16".into(),
            action_url: "https://console.apexmediation.ee/connections".into(),
        })
        .expect("access");
        assert_eq!(access.subject, "Reconnect MAX reporting for Northstar");
        assert!(access.text.contains(
            "The latest reporting request was refused. New experiment data is not available. Open Connections to review the account and reconnect the reporting credential."
        ));
        assert!(access.text.contains("Do not send the key by email."));
        assert!(access.text.contains("Review connection: https://"));

        let stop = guardrail_stop(&GuardrailStop {
            experiment_id: "exp_0242".into(),
            action_url: "https://console.apexmediation.ee/experiments/exp_0242".into(),
        })
        .expect("stop");
        assert_eq!(stop.subject, "An Apex test was stopped");
        assert!(stop.text.contains(
            "Experiment exp_0242 triggered a registered stop condition. The console shows the input, threshold and confirmed platform outcome. This result does not create a lift fee."
        ));
        assert!(stop.text.contains("View test result: https://"));

        let hold = proof_hold(&ProofHold {
            draft_reference: "DF-104".into(),
            action_url: "https://console.apexmediation.ee/fees/DF-104".into(),
        })
        .expect("hold");
        assert_eq!(hold.subject, "A fee is on hold while we check its evidence");
        assert!(hold.text.contains(
            "A required verification check for draft fee DF-104 did not pass. We have not issued or collected this fee from that state."
        ));
        assert!(hold.text.contains(
            "The console shows the affected period and reference. We will update the draft when the evidence is verified or the line is removed."
        ));
        assert!(hold.text.contains("View fee status: https://"));

        let approval = approval_requested(&ApprovalRequested {
            proposal_id: "p_0247".into(),
            test_label: "Rewarded video".into(),
            country: "Germany".into(),
            action_url: "https://console.apexmediation.ee/proposals/p_0247".into(),
        })
        .expect("approval");
        assert_eq!(approval.subject, "Review a proposed MAX test");
        assert!(approval.text.contains(
            "A proposal is ready for Rewarded video in Germany. Review the current and test settings, scope, allocation, analysis window and stop conditions before deciding."
        ));
        assert!(approval
            .text
            .contains("This message does not grant approval. Use the authenticated proposal page to approve or decline."));
        assert!(approval.text.contains("Review proposal: https://"));
    }

    #[test]
    fn restore_failed_carries_the_incident_block_and_a_stable_incident_id() {
        let rendered = |next_update: &str| {
            restore_failed(&RestoreFailed {
                experiment_id: "exp_0247".into(),
                incident_id: "inc_018".into(),
                incident: IncidentBlock {
                    next_update: next_update.to_string(),
                    ..incident()
                },
                action_url: "https://console.apexmediation.ee/incidents/inc_018".into(),
            })
            .expect("restore failed")
        };
        let first = rendered("Today, 18:00 UTC.");
        assert_eq!(first.subject, "Action required for experiment exp_0247");
        assert!(first.text.contains(
            "The platform did not confirm the stop request. Apex has not marked the test restored. We are checking the current state under incident inc_018."
        ));
        assert!(first.text.contains(
            "Open the incident to see the last confirmed state and next update. Do not assume that retrying a request is safe without checking the current platform state."
        ));
        assert!(first.text.contains("What is known: "));
        assert!(first.text.contains("Who is affected: "));
        assert!(first.text.contains("What we are doing: "));
        assert!(first.text.contains("Next update: "));
        assert!(first.text.contains("Reference: inc_018"));
        assert!(first.html.contains("What is known"));
        assert!(first.html.contains("Who is affected"));
        // A later update reuses the same incident id and therefore the same
        // dedup identity: retries never fan out into new incidents.
        let second = rendered("Tomorrow, 12:00 UTC.");
        assert_eq!(first.dedup_key, second.dedup_key);
        assert_eq!(first.dedup_key, "restore_failed:inc_018");
    }

    #[test]
    fn dynamic_text_is_escaped_and_urls_are_validated() {
        let message = guardrail_stop(&GuardrailStop {
            experiment_id: "exp_<script>".into(),
            action_url: "https://console.apexmediation.ee/experiments/1".into(),
        })
        .expect("stop");
        assert!(!message.html.contains("<script>"));
        assert!(message.html.contains("exp_&lt;script&gt;"));

        let error = guardrail_stop(&GuardrailStop {
            experiment_id: "exp_0242".into(),
            action_url: "http://insecure.example/stop".into(),
        })
        .expect_err("http refused");
        assert!(matches!(error, TemplateError::InvalidActionUrl { .. }));

        let error = guardrail_stop(&GuardrailStop {
            experiment_id: "".into(),
            action_url: url(),
        })
        .expect_err("missing id");
        assert!(matches!(error, TemplateError::MissingField { .. }));
    }

    #[test]
    fn every_template_has_one_action_and_a_matching_text_part() {
        let messages = vec![
            fee_preview(&FeePreview {
                month: "2026-10".into(),
                amount_micros: 300_000_000,
                draft_reference: "DF-104".into(),
                action_url: url(),
            })
            .unwrap(),
            invoice_ready(&InvoiceReady {
                invoice_reference: "INV-2026-10-pub-1".into(),
                amount_micros: 300_000_000,
                amount_currency: Some("USD".into()),
                payment_url: "https://pay.mollie.com/pl_9".into(),
                attempt: 1,
            })
            .unwrap(),
            payment_retry(&PaymentRetry {
                invoice_reference: "INV-2026-104".into(),
                balance_micros: 300_000_000,
                balance_currency: Some("USD".into()),
                retry_day: Some("2026-10-20".into()),
                action_url: url(),
            })
            .unwrap(),
            payment_failed(&PaymentFailed {
                invoice_reference: "INV-2026-104".into(),
                amount_micros: 300_000_000,
                amount_currency: Some("USD".into()),
                provider_event_key: "psp_evt_88".into(),
                action_url: url(),
            })
            .unwrap(),
            payment_received(&PaymentReceived {
                invoice_reference: "INV-2026-104".into(),
                received_micros: 300_000_000,
                remaining_micros: 0,
                amount_currency: Some("USD".into()),
                applied_day: "2026-10-20".into(),
                provider_event_key: "psp_evt_89".into(),
                action_url: url(),
            })
            .unwrap(),
            audit_ready(&AuditReady {
                platform_label: "MAX".into(),
                snapshot_day: "2026-10-14".into(),
                action_url: url(),
            })
            .unwrap(),
            reporting_access(&ReportingAccess {
                platform_label: "MAX".into(),
                publisher_label: "Northstar".into(),
                day: "2026-10-16".into(),
                action_url: url(),
            })
            .unwrap(),
            guardrail_stop(&GuardrailStop {
                experiment_id: "exp_0242".into(),
                action_url: url(),
            })
            .unwrap(),
            restore_failed(&RestoreFailed {
                experiment_id: "exp_0247".into(),
                incident_id: "inc_018".into(),
                incident: incident(),
                action_url: url(),
            })
            .unwrap(),
            proof_hold(&ProofHold {
                draft_reference: "DF-104".into(),
                action_url: url(),
            })
            .unwrap(),
            approval_requested(&ApprovalRequested {
                proposal_id: "p_0247".into(),
                test_label: "Rewarded video".into(),
                country: "Germany".into(),
                action_url: url(),
            })
            .unwrap(),
        ];
        assert_eq!(messages.len(), TemplateClass::VALUES.len());
        for message in &messages {
            assert_eq!(
                message.html.matches("<a ").count(),
                1,
                "{:?}",
                message.template_class
            );
            assert!(!message.subject.is_empty());
            assert!(
                message.text.starts_with("Apex\n"),
                "{:?}",
                message.template_class
            );
            assert!(message.text.contains(&message.subject));
            assert!(!message.dedup_key.is_empty());
            assert!(message.html.contains("width=\"600\""));
        }
        let classes: Vec<TemplateClass> = messages.iter().map(|m| m.template_class).collect();
        for class in TemplateClass::VALUES {
            let parsed = TemplateClass::parse(class).expect("class");
            assert!(classes.contains(&parsed), "{class}");
        }
    }
}
