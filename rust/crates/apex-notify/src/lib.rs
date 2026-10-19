//! apex-notify — transactional messages: templates, sinks and delivery.
//!
//! The authority is section 06 of the design document (pages 92 to 95). The
//! copy is frozen: every subject and sentence in [`template`] is reproduced
//! verbatim from the design, and the layout follows page 92 (a 600 px body,
//! safe HTML tables, system-font fallback, one primary action, a matching
//! plain-text part).
//!
//! The delivery contract (page 94) is implemented here, not left to callers:
//!
//! - recipients are typed [`contacts::Contact`] addresses; the only source is
//!   the configured contact book,
//! - [`notifier::Notifier`] records `requested`, `accepted`, `delivered` and
//!   `failed` as distinct rows in the append-only
//!   [`state::DeliveryLog`] and never sends the same
//!   `(template_class, dedup_key)` to the same recipient twice,
//! - every attempt records `apex_notification_attempts_total` through the
//!   injected [`apex_metrics::Registry`],
//! - the webhook payload keeps security-sensitive values out of subjects and
//!   bodies (provider event keys ride only in the dedup identity),
//! - [`sink::EmailSink`] hands the composed RFC 5322 message to the local
//!   MTA's `sendmail`, the same transport the sales engine uses, with the
//!   sender from `APEX_NOTIFY_FROM` and the executable from
//!   `APEX_NOTIFY_SENDMAIL`; a local MTA accepts but never confirms
//!   delivery, so `accepted` is terminal for that sink. The transactional
//!   sending domain must be kept apart from the sales engine's domain — DNS
//!   cannot be enforced from here, so the configuration is the contract.

#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )
)]

pub mod contacts;
mod mail;
pub mod money;
pub mod notifier;
pub mod sink;
pub mod state;
pub mod template;
mod text;

/// Serde adapters for the `apex-metrics` bounded vocabularies (which are
/// dependency-free and therefore carry no serde derives).
pub(crate) mod vocab_serde {
    use apex_metrics::vocab::TemplateClass;
    use serde::{Deserialize, Deserializer, Serializer};

    /// Serialize a [`TemplateClass`] as its stable string value.
    pub(crate) fn serialize<S: Serializer>(
        value: &TemplateClass,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(value.as_str())
    }

    /// Deserialize a [`TemplateClass`] from its stable string value.
    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<TemplateClass, D::Error> {
        let text = String::deserialize(deserializer)?;
        TemplateClass::parse(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown template class '{text}'")))
    }
}

pub use contacts::{Contact, ContactBook, ContactError};
pub use money::{
    format_day, format_money, format_money_or_not_available, format_usd, is_currency_code,
    month_name, FormatError, CURRENCY_UNAVAILABLE,
};
pub use notifier::{Notifier, NotifyError, NotifyReport};
pub use sink::{
    select_sink, DeliveryMessage, DeliveryReceipt, DeliveryState, EmailSink, FileSink,
    NotificationSink, SinkError, WebhookSink, DEFAULT_SENDMAIL, FILE_ENV, FROM_ENV, SENDMAIL_ENV,
    SINK_ENV, WEBHOOK_ENV, WEBHOOK_TOKEN_ENV,
};
pub use state::{DeliveryLog, DeliveryStateRow, StateError, StateKind, STATE_ENV};
pub use template::{
    approval_requested, audit_ready, fee_preview, guardrail_stop, payment_failed, payment_received,
    payment_retry, proof_hold, reconnection_requested, reporting_access, restore_failed,
    ApprovalRequested, AuditReady, FeePreview, GuardrailStop, IncidentBlock, Message,
    PaymentFailed, PaymentReceived, PaymentRetry, ProofHold, ReconnectionRequested,
    ReportingAccess, RestoreFailed, TemplateError,
};
