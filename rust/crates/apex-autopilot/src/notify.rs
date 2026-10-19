//! Runner-side notification wiring: the seven design trigger points.
//!
//! - `fee_preview` before the monthly invoice is issued,
//! - `payment_failed` / `payment_received` / `payment_retry` from the
//!   payment-event processing path ([`crate::payment_events`]),
//! - `audit_ready` after the snapshot refresh publishes,
//! - `reporting_access` on an auth-refused report pull,
//! - `guardrail_stop` when a guardrail stops a test,
//! - `restore_failed` when a restore is not confirmed (the incident id is
//!   derived from the experiment and therefore reused across retries),
//! - `proof_hold` when a fee proof or reconciliation fails,
//! - `approval_requested` when a proposal is created in approval mode.
//!
//! Contacts come only from `APEX_NOTIFY_CONTACTS`
//! (`{"publisher": ["ops@example.com"]}`). An absent value disables
//! notifications entirely and returns a clear log line, never a panic. The
//! console base (`APEX_CONSOLE_URL`) is required when contacts are configured
//! and must be an `https://` URL so action links are authenticated pages.

use std::sync::Arc;

use apex_metrics::Registry;
use apex_notify::contacts::{ContactBook, ContactError};
use apex_notify::notifier::{Notifier, NotifyReport};
use apex_notify::sink::{select_sink, NotificationSink, SinkError};
use apex_notify::state::{DeliveryLog, StateError};
use apex_notify::template::{
    self, ApprovalRequested, AuditReady, FeePreview, GuardrailStop, IncidentBlock, PaymentFailed,
    PaymentReceived, PaymentRetry, ProofHold, ReconnectionRequested, ReportingAccess,
    RestoreFailed,
};

/// The environment variable naming the contact map.
pub const CONTACTS_ENV: &str = "APEX_NOTIFY_CONTACTS";
/// The environment variable naming the console base URL.
pub const CONSOLE_URL_ENV: &str = "APEX_CONSOLE_URL";

/// The runner's notification plan: one notifier, one contact book, one
/// console base.
#[derive(Debug)]
pub struct NotifyPlan {
    notifier: Arc<Notifier>,
    book: ContactBook,
    console_base: String,
}

/// Why the notification plan could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NotifyPlanError {
    /// The contact map is not the documented JSON shape.
    #[error(transparent)]
    Contacts(#[from] ContactError),
    /// The console base is missing or not an https URL.
    #[error("{CONSOLE_URL_ENV} must be an https:// console URL when {CONTACTS_ENV} is set")]
    ConsoleUrl,
    /// The sink configuration failed.
    #[error(transparent)]
    Sink(#[from] SinkError),
    /// The delivery state store failed to open.
    #[error(transparent)]
    State(#[from] StateError),
}

impl NotifyPlan {
    /// Build a plan from its parts.
    pub fn new(
        notifier: Arc<Notifier>,
        book: ContactBook,
        console_base: impl Into<String>,
    ) -> Self {
        Self {
            notifier,
            book,
            console_base: console_base.into(),
        }
    }

    /// Build from the environment. Returns `Ok((None, log lines))` when
    /// `APEX_NOTIFY_CONTACTS` is unset or blank (notifications disabled).
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
        registry: Arc<Registry>,
    ) -> Result<(Option<Arc<Self>>, Vec<String>), NotifyPlanError> {
        let mut logs = Vec::new();
        let Some(raw) = lookup(CONTACTS_ENV).filter(|value| !value.trim().is_empty()) else {
            logs.push(format!(
                "apex-notify: {CONTACTS_ENV} is not set; notifications are disabled."
            ));
            return Ok((None, logs));
        };
        let book = ContactBook::from_json(&raw)?;
        let Some(console_base) = lookup(CONSOLE_URL_ENV).filter(|value| !value.trim().is_empty())
        else {
            return Err(NotifyPlanError::ConsoleUrl);
        };
        let console_base = console_base.trim().trim_end_matches('/').to_string();
        if !console_base.starts_with("https://") || console_base.len() <= "https://".len() {
            return Err(NotifyPlanError::ConsoleUrl);
        }
        // One sink, selected by the sink owner's shared rule: an explicit
        // `APEX_NOTIFY_SINK` kind (email/webhook/file), otherwise the legacy
        // order (webhook, then file). The runner never falls back to a
        // different provider, and contacts without a sink are a typed
        // configuration refusal.
        let sink: Arc<dyn NotificationSink> = select_sink(&lookup)?.ok_or_else(|| {
            NotifyPlanError::Sink(SinkError::Config(format!(
                "no notification sink configured; set APEX_NOTIFY_SINK (email, webhook or file) or APEX_NOTIFY_WEBHOOK or APEX_NOTIFY_FILE when {CONTACTS_ENV} is set"
            )))
        })?;
        let log = match lookup("APEX_NOTIFY_STATE").filter(|value| !value.trim().is_empty()) {
            Some(path) => DeliveryLog::open(Some(std::path::Path::new(path.trim())))?,
            None => {
                logs.push(
                    "apex-notify: APEX_NOTIFY_STATE is not set; delivery deduplication is process-local."
                        .to_string(),
                );
                DeliveryLog::open(None)?
            }
        };
        let notifier = Arc::new(Notifier::new(sink, log, registry));
        Ok((
            Some(Arc::new(Self::new(notifier, book, console_base))),
            logs,
        ))
    }

    /// The publisher display label shown in message copy ("Northstar").
    pub fn publisher_label(publisher: &str) -> String {
        let mut chars = publisher.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => "Publisher".to_string(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.console_base)
    }

    async fn send(
        &self,
        publisher: &str,
        message: Result<template::Message, template::TemplateError>,
        now_unix_ms: i64,
    ) -> NotifyReport {
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                eprintln!("apex-notify: refusing to render a message: {error}");
                return NotifyReport::default();
            }
        };
        let contacts = self.book.for_publisher(publisher);
        if contacts.is_empty() {
            return NotifyReport::default();
        }
        self.notifier
            .send_message(contacts, &message, now_unix_ms)
            .await
    }

    /// `fee_preview` before the monthly invoice is issued.
    pub async fn fee_preview(
        &self,
        publisher: &str,
        month: &str,
        amount_micros: i64,
        monitoring_selected: bool,
        now_unix_ms: i64,
    ) -> NotifyReport {
        let draft_reference = draft_reference(month);
        self.send(
            publisher,
            template::fee_preview(&FeePreview {
                month: month.to_string(),
                amount_micros,
                draft_reference,
                monitoring_selected,
                action_url: self.url("/console/billing"),
            }),
            now_unix_ms,
        )
        .await
    }

    /// `payment_retry` from the payment-event processing path. The balance is
    /// rendered in the retry row's own currency; a missing code renders the
    /// not-available reason, never USD.
    pub async fn payment_retry(
        &self,
        publisher: &str,
        invoice_reference: &str,
        balance_micros: i64,
        balance_currency: Option<&str>,
        retry_day: Option<&str>,
        now_unix_ms: i64,
    ) -> NotifyReport {
        self.send(
            publisher,
            template::payment_retry(&PaymentRetry {
                invoice_reference: invoice_reference.to_string(),
                balance_micros,
                balance_currency: balance_currency.map(str::to_string),
                retry_day: retry_day.map(str::to_string),
                action_url: self.url("/console/billing/invoices"),
            }),
            now_unix_ms,
        )
        .await
    }

    /// `payment_failed` from the payment-event processing path. The amount is
    /// rendered in the provider event's own currency; a missing code renders
    /// the not-available reason, never USD.
    pub async fn payment_failed(
        &self,
        publisher: &str,
        invoice_reference: &str,
        amount_micros: i64,
        amount_currency: Option<&str>,
        provider_event_key: &str,
        now_unix_ms: i64,
    ) -> NotifyReport {
        self.send(
            publisher,
            template::payment_failed(&PaymentFailed {
                invoice_reference: invoice_reference.to_string(),
                amount_micros,
                amount_currency: amount_currency.map(str::to_string),
                provider_event_key: provider_event_key.to_string(),
                action_url: self.url("/console/billing/invoices"),
            }),
            now_unix_ms,
        )
        .await
    }

    /// `payment_received` from the payment-event processing path. The
    /// remaining balance comes from the durable payment ledger; without it the
    /// message is skipped rather than guessed (missing is not zero). Both
    /// amounts render in the provider event's own currency.
    #[allow(clippy::too_many_arguments)]
    pub async fn payment_received(
        &self,
        publisher: &str,
        invoice_reference: &str,
        received_micros: i64,
        remaining_micros: i64,
        amount_currency: Option<&str>,
        applied_day: &str,
        provider_event_key: &str,
        now_unix_ms: i64,
    ) -> NotifyReport {
        self.send(
            publisher,
            template::payment_received(&PaymentReceived {
                invoice_reference: invoice_reference.to_string(),
                received_micros,
                remaining_micros,
                amount_currency: amount_currency.map(str::to_string),
                applied_day: applied_day.to_string(),
                provider_event_key: provider_event_key.to_string(),
                action_url: self.url("/console/billing/invoices"),
            }),
            now_unix_ms,
        )
        .await
    }

    /// `audit_ready` after the snapshot refresh publishes.
    pub async fn audit_ready(
        &self,
        publisher: &str,
        snapshot_day: &str,
        now_unix_ms: i64,
    ) -> NotifyReport {
        self.send(
            publisher,
            template::audit_ready(&AuditReady {
                platform_label: "MAX".to_string(),
                snapshot_day: snapshot_day.to_string(),
                action_url: self.url("/console/audit"),
            }),
            now_unix_ms,
        )
        .await
    }

    /// `reporting_access` on an auth-refused report pull.
    pub async fn reporting_access(
        &self,
        publisher: &str,
        day: &str,
        now_unix_ms: i64,
    ) -> NotifyReport {
        self.send(
            publisher,
            template::reporting_access(&ReportingAccess {
                platform_label: "MAX".to_string(),
                publisher_label: Self::publisher_label(publisher),
                day: day.to_string(),
                action_url: self.url("/console/connections"),
            }),
            now_unix_ms,
        )
        .await
    }

    /// The operator-requested reconnection notice. Distinct from
    /// [`Self::reporting_access`]: it names the publisher, platform and the
    /// access to restore, and deduplicates on the request's own day.
    pub async fn reconnection_requested(
        &self,
        publisher: &str,
        platform_label: &str,
        access_label: &str,
        day: &str,
        now_unix_ms: i64,
    ) -> NotifyReport {
        self.send(
            publisher,
            template::reconnection_requested(&ReconnectionRequested {
                platform_label: platform_label.to_string(),
                publisher_label: Self::publisher_label(publisher),
                access_label: access_label.to_string(),
                day: day.to_string(),
                action_url: self.url("/console/connections"),
            }),
            now_unix_ms,
        )
        .await
    }

    /// `guardrail_stop` when a guardrail stops a test.
    pub async fn guardrail_stop(
        &self,
        publisher: &str,
        experiment_id: &str,
        now_unix_ms: i64,
    ) -> NotifyReport {
        self.send(
            publisher,
            template::guardrail_stop(&GuardrailStop {
                experiment_id: experiment_id.to_string(),
                action_url: self.url(&format!("/console/experiments/{experiment_id}")),
            }),
            now_unix_ms,
        )
        .await
    }

    /// `restore_failed` when a restore is not confirmed; the incident id is
    /// stable per experiment, so a retry reuses it.
    pub async fn restore_failed(
        &self,
        publisher: &str,
        experiment_id: &str,
        incident_id: &str,
        now_unix_ms: i64,
    ) -> NotifyReport {
        self.send(
            publisher,
            template::restore_failed(&RestoreFailed {
                experiment_id: experiment_id.to_string(),
                incident_id: incident_id.to_string(),
                incident: IncidentBlock {
                    what_is_known: "The platform did not confirm the stop request for this test; the last confirmed state is unchanged."
                        .to_string(),
                    who_is_affected: "The test's audience on your MAX account.".to_string(),
                    what_we_are_doing:
                        "Apex is reconciling the current platform state before any retry.".to_string(),
                    next_update: "The next daily pass.".to_string(),
                },
                action_url: self.url(&format!("/console/experiments/{experiment_id}")),
            }),
            now_unix_ms,
        )
        .await
    }

    /// `proof_hold` when a fee proof or reconciliation fails.
    pub async fn proof_hold(
        &self,
        publisher: &str,
        draft_reference: &str,
        now_unix_ms: i64,
    ) -> NotifyReport {
        self.send(
            publisher,
            template::proof_hold(&ProofHold {
                draft_reference: draft_reference.to_string(),
                action_url: self.url("/console/billing"),
            }),
            now_unix_ms,
        )
        .await
    }

    /// `approval_requested` when a proposal is created in approval mode.
    pub async fn approval_requested(
        &self,
        publisher: &str,
        proposal_id: &str,
        test_label: &str,
        country: &str,
        now_unix_ms: i64,
    ) -> NotifyReport {
        self.send(
            publisher,
            template::approval_requested(&ApprovalRequested {
                proposal_id: proposal_id.to_string(),
                test_label: test_label.to_string(),
                country: country.to_string(),
                action_url: self.url(&format!("/console/proposals/{proposal_id}")),
            }),
            now_unix_ms,
        )
        .await
    }
}

/// The draft-fee reference used by the preview and the hold: `DF-202610`.
pub fn draft_reference(month: &str) -> String {
    format!("DF-{}", month.replace('-', ""))
}

/// A stable incident id per experiment (`inc_` plus three hex digits), so
/// retries reuse the same incident.
pub fn incident_id_for(experiment_id: &str) -> String {
    format!("inc_{:03x}", fnv1a(experiment_id) & 0xfff)
}

/// The customer-facing label of a proposed action: the change kind and the
/// country when the action names one.
pub fn proposal_label(action: &crate::model::Action) -> (String, String) {
    use crate::model::Action;
    match action {
        Action::SetCountryFloor { country, .. } => {
            ("Country floor".to_string(), country.to_uppercase())
        }
        Action::SetMappingEcpm { .. } => ("eCPM mapping".to_string(), "all countries".to_string()),
        Action::SetWaterfallPrice { .. } => {
            ("Waterfall price".to_string(), "all countries".to_string())
        }
        Action::CreateWaterfallEntry { .. } => {
            ("Waterfall entry".to_string(), "all countries".to_string())
        }
        Action::RemoveWaterfallEntry { .. } => (
            "Waterfall entry removal".to_string(),
            "all countries".to_string(),
        ),
        Action::SetLevelplayInstancePrice { .. } => {
            ("Instance price".to_string(), "all countries".to_string())
        }
        Action::SetAdmobMediationFloor { .. } => {
            ("Mediation floor".to_string(), "all countries".to_string())
        }
    }
}

/// A stable proposal id derived from the change's idempotency key.
pub fn proposal_id_for(idempotency_key: &str) -> String {
    format!("prop_{:08x}", fnv1a(idempotency_key))
}

/// A stable change hash for events (fnv-1a, 16 hex digits).
pub fn change_hash(change_json: &str) -> String {
    format!("{:016x}", fnv1a64(change_json))
}

/// Stable 64-bit FNV-1a over the change payload.
pub fn fnv1a64(data: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in data.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 32-bit fold of the 64-bit hash.
fn fnv1a(data: &str) -> u32 {
    (fnv1a64(data) & 0xffff_ffff) as u32
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use apex_notify::contacts::Contact;
    use apex_notify::sink::FileSink;

    fn plan(label: &str, book_json: &str) -> Arc<NotifyPlan> {
        let dir =
            std::env::temp_dir().join(format!("apex-notify-plan-{}-{label}", std::process::id(),));
        let _ = std::fs::remove_dir_all(&dir);
        let sink = Arc::new(FileSink::new(dir.join("outbox.jsonl")));
        let log = DeliveryLog::open(Some(&dir.join("delivery.jsonl"))).expect("log");
        let notifier = Arc::new(Notifier::new(sink, log, Arc::new(Registry::new())));
        let book = ContactBook::from_json(book_json).expect("book");
        Arc::new(NotifyPlan::new(
            notifier,
            book,
            "https://console.apexmediation.ee",
        ))
    }

    #[test]
    fn from_lookup_disables_cleanly_and_requires_the_console_url() {
        let registry = Arc::new(Registry::new());
        let (plan, logs) = NotifyPlan::from_lookup(|_| None, registry.clone()).expect("unset");
        assert!(plan.is_none());
        assert!(logs[0].contains("notifications are disabled"));

        let error = NotifyPlan::from_lookup(
            |name| match name {
                CONTACTS_ENV => Some(r#"{"northstar": ["ops@example.com"]}"#.into()),
                _ => None,
            },
            registry,
        )
        .expect_err("no console url");
        assert_eq!(error, NotifyPlanError::ConsoleUrl);

        let (plan, logs) = NotifyPlan::from_lookup(
            |name| match name {
                CONTACTS_ENV => Some(r#"{"northstar": ["ops@example.com"]}"#.into()),
                CONSOLE_URL_ENV => Some("https://console.apexmediation.ee".into()),
                "APEX_NOTIFY_FILE" => Some("/tmp/apex-notify-test-outbox.jsonl".into()),
                _ => None,
            },
            Arc::new(Registry::new()),
        )
        .expect("configured");
        assert!(plan.is_some());
        assert!(logs[0].contains("process-local"));

        let error = NotifyPlan::from_lookup(
            |name| match name {
                CONTACTS_ENV => Some(r#"{"northstar": ["not an email"]}"#.into()),
                CONSOLE_URL_ENV => Some("https://console.apexmediation.ee".into()),
                _ => None,
            },
            Arc::new(Registry::new()),
        )
        .expect_err("bad contact");
        assert!(matches!(error, NotifyPlanError::Contacts(_)));
    }

    #[test]
    fn sink_selection_follows_the_shared_select_sink_rule() {
        let registry = Arc::new(Registry::new());
        let configured = |name: &str, sink: Option<&'static str>| match name {
            CONTACTS_ENV => Some(r#"{"northstar": ["ops@example.com"]}"#.to_string()),
            CONSOLE_URL_ENV => Some("https://console.apexmediation.ee".to_string()),
            "APEX_NOTIFY_SINK" => sink.map(str::to_string),
            _ => None,
        };

        // No sink variable at all: the typed configuration refusal, never a
        // silent fallback.
        let error = NotifyPlan::from_lookup(|name| configured(name, None), registry.clone())
            .expect_err("no sink");
        assert!(
            matches!(error, NotifyPlanError::Sink(SinkError::Config(_))),
            "{error:?}"
        );
        assert!(
            error
                .to_string()
                .contains("no notification sink configured"),
            "{error}"
        );

        // The legacy order is preserved: a webhook URL wins over a file
        // path, so deployments that relied on the inline selection keep it.
        let (plan, _) = NotifyPlan::from_lookup(
            |name| match name {
                "APEX_NOTIFY_WEBHOOK" => Some("https://hooks.example.test/apex".to_string()),
                "APEX_NOTIFY_FILE" => Some("/tmp/apex-notify-both.jsonl".to_string()),
                other => configured(other, None),
            },
            registry.clone(),
        )
        .expect("legacy order");
        assert!(plan.is_some());

        // The explicit email kind is now selectable through the shared rule.
        let (plan, logs) = NotifyPlan::from_lookup(
            |name| match name {
                "APEX_NOTIFY_FROM" => Some("Apex <hello@apex-transactional.example>".to_string()),
                other => configured(other, Some("email")),
            },
            registry.clone(),
        )
        .expect("email sink");
        assert!(plan.is_some());
        assert!(logs[0].contains("process-local"));

        // A kind whose required variable is missing refuses typed instead of
        // falling back to another provider.
        let error = NotifyPlan::from_lookup(
            |name| match name {
                "APEX_NOTIFY_FILE" => Some("/tmp/apex-notify-legacy.jsonl".to_string()),
                other => configured(other, Some("webhook")),
            },
            registry.clone(),
        )
        .expect_err("webhook kind without a url");
        assert!(
            error
                .to_string()
                .contains("APEX_NOTIFY_WEBHOOK is required"),
            "{error}"
        );

        // An unknown kind is refused outright.
        let error = NotifyPlan::from_lookup(
            |name| match name {
                "APEX_NOTIFY_FILE" => Some("/tmp/apex-notify-legacy.jsonl".to_string()),
                other => configured(other, Some("pigeon")),
            },
            registry,
        )
        .expect_err("unknown kind");
        assert!(
            error.to_string().contains("'email', 'webhook' or 'file'"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn every_trigger_builds_a_message_for_a_configured_publisher() {
        let plan = plan("all", r#"{"northstar": ["ops@example.com"]}"#);
        let now = 1_700_000_000_000;
        let reports = vec![
            plan.fee_preview("northstar", "2026-10", 300_000_000, false, now)
                .await,
            plan.payment_retry(
                "northstar",
                "INV-2026-104",
                300_000_000,
                Some("USD"),
                Some("2026-10-20"),
                now,
            )
            .await,
            plan.payment_failed(
                "northstar",
                "INV-2026-104",
                300_000_000,
                Some("USD"),
                "psp_evt_88",
                now,
            )
            .await,
            plan.payment_received(
                "northstar",
                "INV-2026-104",
                300_000_000,
                0,
                Some("USD"),
                "2026-10-20",
                "psp_evt_88",
                now,
            )
            .await,
            plan.audit_ready("northstar", "2026-10-14", now).await,
            plan.reporting_access("northstar", "2026-10-16", now).await,
            plan.guardrail_stop("northstar", "exp_0242", now).await,
            plan.restore_failed("northstar", "exp_0247", "inc_018", now)
                .await,
            plan.proof_hold("northstar", "DF-202610", now).await,
            plan.approval_requested("northstar", "prop_00000001", "Country floor", "EE", now)
                .await,
        ];
        for report in &reports {
            assert_eq!(report.attempted, 1, "{report:?}");
            assert_eq!(report.accepted, 1, "{report:?}");
        }
        let dupes = plan.guardrail_stop("northstar", "exp_0242", now + 1).await;
        assert_eq!(dupes.duplicates, 1);
    }

    #[tokio::test]
    async fn retry_without_a_scheduled_date_sends_nothing() {
        let plan = plan("retry", r#"{"northstar": ["ops@example.com"]}"#);
        let report = plan
            .payment_retry(
                "northstar",
                "INV-2026-104",
                300_000_000,
                Some("USD"),
                None,
                1,
            )
            .await;
        assert_eq!(report, NotifyReport::default());
    }

    #[tokio::test]
    async fn an_unknown_publisher_sends_nothing() {
        let plan = plan("unknown", r#"{"northstar": ["ops@example.com"]}"#);
        let report = plan.guardrail_stop("acme", "exp_1", 1).await;
        assert_eq!(report, NotifyReport::default());
        assert!(plan.book.for_publisher("acme").is_empty());
        assert_eq!(
            Contact::new("ops@example.com").unwrap().email(),
            "ops@example.com"
        );
    }

    #[test]
    fn identity_helpers_are_stable_and_bounded() {
        assert_eq!(incident_id_for("exp_0247"), incident_id_for("exp_0247"));
        assert!(incident_id_for("exp_0247").starts_with("inc_"));
        assert_eq!(incident_id_for("exp_0247").len(), 7);
        assert_ne!(incident_id_for("exp_0247"), incident_id_for("exp_0248"));
        assert!(proposal_id_for("k").starts_with("prop_"));
        assert_eq!(proposal_id_for("k").len(), 13);
        assert_eq!(change_hash("{}").len(), 16);
        assert_eq!(
            NotifyPlan::publisher_label("northstar"),
            "Northstar".to_string()
        );
        assert_eq!(NotifyPlan::publisher_label(""), "Publisher".to_string());
        assert_eq!(draft_reference("2026-10"), "DF-202610");
        let (label, country) = proposal_label(&crate::model::Action::SetCountryFloor {
            ad_unit_id: "u1".into(),
            country: "ee".into(),
            floor_ecpm: 1,
        });
        assert_eq!(label, "Country floor");
        assert_eq!(country, "EE");
        let (label, country) = proposal_label(&crate::model::Action::SetMappingEcpm {
            mediation_group_id: "g".into(),
            ad_unit_id: "u1".into(),
            ecpm: 1,
        });
        assert_eq!(label, "eCPM mapping");
        assert_eq!(country, "all countries");
    }
}
