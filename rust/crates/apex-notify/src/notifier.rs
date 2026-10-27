//! The notifier: dedup, delivery states and the attempts metric.
//!
//! One `send_message` call drives the whole delivery contract for every
//! authorized contact:
//!
//! 1. a `(template_class, dedup_key, to)` already accepted or delivered is
//!    skipped (counted as a duplicate, nothing is sent),
//! 2. a `requested` row is fsynced to the delivery log in the same critical
//!    section that checks for an in-flight send,
//! 3. the sink is called; its answer (`accepted` or `delivered`) or the
//!    typed failure is appended as its own row,
//! 4. every step records `apex_notification_attempts_total` through the
//!    injected registry ([`Registry`]).

use std::sync::Arc;

use apex_metrics::catalog;
use apex_metrics::vocab::{Channel, NotificationOutcome, TemplateClass};
use apex_metrics::Registry;

use crate::contacts::Contact;
use crate::sink::{DeliveryMessage, DeliveryReceipt, DeliveryState, NotificationSink, SinkError};
use crate::state::{Claim, DeliveryLog, DeliveryStateRow, StateError, StateKind};
use crate::template::Message;

/// What one `send_message` call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NotifyReport {
    /// Contacts the message was requested for.
    pub attempted: u32,
    /// Provider acceptances recorded.
    pub accepted: u32,
    /// Provider delivery confirmations recorded.
    pub delivered: u32,
    /// Failed sends.
    pub failed: u32,
    /// Skips because the same `(template_class, dedup_key)` already sent to
    /// that contact.
    pub duplicates: u32,
    /// State rows that could not be written (the send still happened, but
    /// the durable record is incomplete and that is reported, not hidden).
    pub state_errors: u32,
}

/// Why the notifier could not be built from the environment.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NotifyError {
    /// A sink failure surfaced while building.
    #[error(transparent)]
    Sink(#[from] SinkError),
    /// The state store failed.
    #[error(transparent)]
    State(#[from] StateError),
}

/// Delivery with deduplication, state recording and metrics.
#[derive(Debug)]
pub struct Notifier {
    sink: Arc<dyn NotificationSink>,
    log: DeliveryLog,
    registry: Arc<Registry>,
    channel: Channel,
}

impl Notifier {
    /// Build a notifier over `sink`, `log` and `registry`.
    pub fn new(sink: Arc<dyn NotificationSink>, log: DeliveryLog, registry: Arc<Registry>) -> Self {
        Self {
            sink,
            log,
            registry,
            channel: Channel::Email,
        }
    }

    /// The delivery log this notifier deduplicates through.
    pub fn log(&self) -> &DeliveryLog {
        &self.log
    }

    /// Send `message` to every contact, with the full delivery contract.
    pub async fn send_message(
        &self,
        contacts: &[Contact],
        message: &Message,
        now_unix_ms: i64,
    ) -> NotifyReport {
        let mut report = NotifyReport::default();
        // One batch per send_message call: duplicates are decided first, then
        // the remaining recipients are handed to the sink in one batch. The
        // file sink opens/flushes once; the default batch loops `send`, so
        // other transports keep per-recipient behavior.
        let mut pending: Vec<DeliveryMessage> = Vec::new();
        for contact in contacts {
            let to = contact.email();
            let claim = self.log.claim(&DeliveryStateRow {
                at_unix_ms: now_unix_ms,
                template_class: message.template_class.as_str().to_string(),
                dedup_key: message.dedup_key.clone(),
                to: to.to_string(),
                state: StateKind::Requested.as_str().to_string(),
                provider_reference: None,
                detail: None,
            });
            match claim {
                Ok(Claim::Claimed) => {}
                Ok(Claim::AlreadySent | Claim::InFlight) => {
                    report.duplicates += 1;
                    self.count(message.template_class, NotificationOutcome::Duplicate);
                    continue;
                }
                Err(_) => {
                    // Fail closed: without a durable claim no provider call.
                    report.state_errors += 1;
                    continue;
                }
            }
            report.attempted += 1;
            self.count(message.template_class, NotificationOutcome::Attempted);
            pending.push(DeliveryMessage::from_message(message, to));
        }
        if pending.is_empty() {
            return report;
        }
        let results = self.sink.send_batch(&pending).await;
        for (payload, result) in pending.iter().zip(results) {
            let to = payload.to.as_str();
            match result {
                Ok(receipt) => {
                    let (kind, outcome, provider_reference) = match receipt {
                        DeliveryReceipt {
                            state: DeliveryState::Accepted,
                            provider_reference,
                        } => (
                            StateKind::Accepted,
                            NotificationOutcome::Accepted,
                            provider_reference,
                        ),
                        DeliveryReceipt {
                            state: DeliveryState::Delivered,
                            provider_reference,
                        } => (
                            StateKind::Delivered,
                            NotificationOutcome::Delivered,
                            provider_reference,
                        ),
                    };
                    if !self.record_state(message, to, kind, provider_reference, None, now_unix_ms)
                    {
                        report.state_errors += 1;
                    }
                    match kind {
                        StateKind::Accepted => report.accepted += 1,
                        StateKind::Delivered => report.delivered += 1,
                        _ => {}
                    }
                    self.count(message.template_class, outcome);
                }
                Err(error) => {
                    report.failed += 1;
                    if !self.record_state(
                        message,
                        to,
                        StateKind::Failed,
                        None,
                        Some(bounded_detail(&error)),
                        now_unix_ms,
                    ) {
                        report.state_errors += 1;
                    }
                    self.count(message.template_class, NotificationOutcome::Failed);
                }
            }
        }
        report
    }

    /// Append one state row; failures are counted by the caller.
    fn record_state(
        &self,
        message: &Message,
        to: &str,
        kind: StateKind,
        provider_reference: Option<String>,
        detail: Option<String>,
        now_unix_ms: i64,
    ) -> bool {
        self.log
            .record(&DeliveryStateRow {
                at_unix_ms: now_unix_ms,
                template_class: message.template_class.as_str().to_string(),
                dedup_key: message.dedup_key.clone(),
                to: to.to_string(),
                state: kind.as_str().to_string(),
                provider_reference,
                detail,
            })
            .is_ok()
    }

    /// Record one attempts-metric outcome.
    ///
    /// Declared label values cannot be refused by the registry, so a failed
    /// write here is impossible by construction; the delivery log remains the
    /// durable record either way.
    fn count(&self, template_class: TemplateClass, outcome: NotificationOutcome) {
        let _ = self.registry.increment(
            &catalog::NOTIFICATION_ATTEMPTS_TOTAL,
            &[
                ("channel", self.channel.as_str()),
                ("template_class", template_class.as_str()),
                ("outcome", outcome.as_str()),
            ],
        );
    }
}

/// Bound a failure detail so a provider's verbose error cannot bloat the
/// state file. The bound counts characters, never bytes.
fn bounded_detail(error: &SinkError) -> String {
    crate::text::bounded_chars(&error.to_string(), 200)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Mutex;

    use super::*;
    use crate::contacts::ContactBook;
    use crate::sink::FileSink;
    use crate::template::{payment_failed, PaymentFailed};

    fn message() -> Message {
        payment_failed(&PaymentFailed {
            invoice_reference: "INV-2026-104".into(),
            amount_micros: 300_000_000,
            amount_currency: Some("USD".into()),
            provider_event_key: "psp_evt_88".into(),
            action_url: "https://console.apexmediation.ee/invoices/INV-2026-104".into(),
        })
        .expect("message")
    }

    fn contacts() -> Vec<Contact> {
        vec![
            Contact::new("ops@example.com").expect("ops"),
            Contact::new("fin@example.com").expect("fin"),
        ]
    }

    /// A sink that reports whatever it is told, counting calls.
    #[derive(Debug)]
    struct ScriptedSink {
        outcome: Mutex<Result<DeliveryState, String>>,
        calls: Mutex<u32>,
        provider_reference: Mutex<Option<String>>,
    }

    impl ScriptedSink {
        fn new(outcome: Result<DeliveryState, String>) -> Self {
            Self {
                outcome: Mutex::new(outcome),
                calls: Mutex::new(0),
                provider_reference: Mutex::new(None),
            }
        }

        fn calls(&self) -> u32 {
            *self.calls.lock().unwrap()
        }
    }

    #[async_trait::async_trait]
    impl NotificationSink for ScriptedSink {
        async fn send(&self, _message: &DeliveryMessage) -> Result<DeliveryReceipt, SinkError> {
            *self.calls.lock().unwrap() += 1;
            match &*self.outcome.lock().unwrap() {
                Ok(state) => Ok(DeliveryReceipt {
                    state: *state,
                    provider_reference: self.provider_reference.lock().unwrap().clone(),
                }),
                Err(reason) => Err(SinkError::Transport(reason.clone())),
            }
        }
    }

    fn outcome_count(registry: &Registry, outcome: NotificationOutcome) -> u64 {
        registry
            .counter_value(
                &catalog::NOTIFICATION_ATTEMPTS_TOTAL,
                &[
                    ("channel", "email"),
                    ("template_class", "payment_failed"),
                    ("outcome", outcome.as_str()),
                ],
            )
            .expect("metric")
    }

    #[tokio::test]
    async fn a_send_records_requested_and_accepted_rows_and_metrics() {
        let dir = std::env::temp_dir().join(format!("apex-notify-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("delivery.jsonl");
        let log = DeliveryLog::open(Some(&path)).expect("log");
        let registry = Arc::new(Registry::new());
        let sink = Arc::new(ScriptedSink::new(Ok(DeliveryState::Accepted)));
        let notifier = Notifier::new(sink.clone(), log, registry.clone());
        let report = notifier
            .send_message(&contacts(), &message(), 1_700_000_000_000)
            .await;
        assert_eq!(
            report,
            NotifyReport {
                attempted: 2,
                accepted: 2,
                delivered: 0,
                failed: 0,
                duplicates: 0,
                state_errors: 0,
            }
        );
        assert_eq!(sink.calls(), 2);
        let rows = DeliveryLog::read_rows(&path).expect("rows");
        assert_eq!(rows.len(), 4, "requested + accepted per contact");
        assert_eq!(rows[0].state, "requested");
        assert_eq!(rows[1].state, "requested");
        assert_eq!(rows[2].state, "accepted");
        assert_eq!(rows[3].state, "accepted");
        assert_eq!(rows[0].template_class, "payment_failed");
        assert_eq!(rows[0].dedup_key, "payment_failed:INV-2026-104:psp_evt_88");
        assert_eq!(outcome_count(&registry, NotificationOutcome::Attempted), 2);
        assert_eq!(outcome_count(&registry, NotificationOutcome::Accepted), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_same_template_and_dedup_key_never_sends_twice() {
        let log = DeliveryLog::open(None).expect("log");
        let registry = Arc::new(Registry::new());
        let sink = Arc::new(ScriptedSink::new(Ok(DeliveryState::Delivered)));
        let notifier = Notifier::new(sink.clone(), log, registry.clone());
        let first = notifier.send_message(&contacts(), &message(), 1).await;
        assert_eq!(first.delivered, 2);
        assert_eq!(sink.calls(), 2);
        // A retry of the same message to the same contacts is a no-op.
        let second = notifier.send_message(&contacts(), &message(), 2).await;
        assert_eq!(second.duplicates, 2);
        assert_eq!(second.attempted, 0);
        assert_eq!(sink.calls(), 2, "no second delivery");
        assert_eq!(outcome_count(&registry, NotificationOutcome::Duplicate), 2);
        assert_eq!(outcome_count(&registry, NotificationOutcome::Delivered), 2);
    }

    #[tokio::test]
    async fn failures_are_recorded_and_may_be_retried() {
        let dir = std::env::temp_dir().join(format!("apex-notify-retry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("delivery.jsonl");
        let log = DeliveryLog::open(Some(&path)).expect("log");
        let registry = Arc::new(Registry::new());
        let sink = Arc::new(ScriptedSink::new(Err("provider down".into())));
        let notifier = Notifier::new(sink.clone(), log, registry.clone());
        let report = notifier.send_message(&contacts(), &message(), 3).await;
        assert_eq!(report.failed, 2);
        assert_eq!(report.attempted, 2);
        let rows = DeliveryLog::read_rows(&path).expect("rows");
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[2].state, "failed");
        assert!(rows[2].detail.as_deref().unwrap().contains("provider down"));
        assert_eq!(outcome_count(&registry, NotificationOutcome::Failed), 2);

        // Nothing was accepted, so a retry attempts again.
        *sink.outcome.lock().unwrap() = Ok(DeliveryState::Accepted);
        let retry = notifier.send_message(&contacts(), &message(), 4).await;
        assert_eq!(retry.attempted, 2);
        assert_eq!(retry.accepted, 2);
        assert_eq!(sink.calls(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_file_sink_delivers_end_to_end_and_the_book_selects_contacts() {
        let dir = std::env::temp_dir().join(format!("apex-notify-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sink = Arc::new(FileSink::new(dir.join("outbox.jsonl")));
        let log = DeliveryLog::open(Some(&dir.join("delivery.jsonl"))).expect("log");
        let registry = Arc::new(Registry::new());
        let notifier = Notifier::new(sink, log, registry);
        let book = ContactBook::from_json(r#"{"northstar": ["ops@example.com"]}"#).expect("book");
        let report = notifier
            .send_message(book.for_publisher("northstar"), &message(), 5)
            .await;
        assert_eq!(report.accepted, 1);
        // An unknown publisher has no contacts and therefore sends nothing.
        let report = notifier
            .send_message(book.for_publisher("acme"), &message(), 5)
            .await;
        assert_eq!(report, NotifyReport::default());
        let outbox = std::fs::read_to_string(dir.join("outbox.jsonl")).expect("outbox");
        assert_eq!(outbox.lines().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notifier_debug_and_error_surfaces_work() {
        let sink = Arc::new(ScriptedSink::new(Ok(DeliveryState::Accepted)));
        let notifier = Notifier::new(
            sink,
            DeliveryLog::open(None).expect("log"),
            Arc::new(Registry::new()),
        );
        assert!(format!("{notifier:?}").contains("Notifier"));
        assert!(notifier.log().path().is_none());
        let error = NotifyError::Sink(SinkError::Transport("x".into()));
        assert!(error.to_string().contains("transport"));
        let error = NotifyError::from(StateError::UnknownState {
            state: "queued".into(),
        });
        assert!(error.to_string().contains("queued"));
        let long = SinkError::Transport("e".repeat(500));
        assert!(bounded_detail(&long).len() <= 200);
    }
}
