//! The payment-event processing path: verified provider events to durable
//! facts, notifications and metrics.
//!
//! The billing boundary (the webhook handler owned by the API workstream)
//! verifies Adyen deliveries and records each accepted item through
//! [`apex_billing::CollectionStore`]. The runner then owns the reaction: it
//! reads the append-only collection file (`APEX_PAYMENT_EVENTS`), classifies
//! each provider event into a bounded class, emits the analytics events
//! (`payment_received / reconciled / refunded / disputed`), records
//! `apex_payment_events_total`, and sends the customer messages
//! (`payment_received`, `payment_failed`).
//!
//! `payment_retry` is sent only for a *real* scheduled retry: a row in the
//! retry schedule file (`APEX_PAYMENT_RETRIES`, written by the finance
//! boundary) whose state is `scheduled` and whose `retry_on` is a calendar
//! day. Without one, no retry message exists, and the template refuses to
//! render a date-less retry.
//!
//! The `payment_received` copy states the remaining balance, so the processor
//! needs the collected reference's total: the runner's monthly collection
//! path appends a row per created payment reference to `APEX_PAYMENT_LEDGER`.
//! Without a ledger row the balance is unknown and the received message is
//! skipped (missing is not zero), while the analytics event still lands.
//!
//! ## Consumed-offset cursor
//!
//! The collection file is append-only. The daily pass consumes it through a
//! cursor file (`APEX_PAYMENT_CURSOR`, default `<collection>.cursor`) that
//! records the byte offset of the last consumed line. Each pass reads only
//! the new, complete lines, processes them, and writes the new offset
//! atomically (temp file plus rename) after success. A crash between
//! processing and the cursor write re-processes the last batch, which is
//! safe by construction: the analytics emits are dedup-keyed on the event
//! identity and the delivery log deduplicates the messages. A truncated or
//! rewound file (cursor past the end, or a cursor that is not a line
//! boundary) restarts from the beginning of the file for the same reason:
//! nothing may be skipped, and re-processing loses nothing.

use std::path::{Path, PathBuf};

use apex_billing::{CollectionEvent, MICROS_PER_MINOR_UNIT};
use serde::{Deserialize, Serialize};

use crate::notify::NotifyPlan;
use crate::telemetry::RunnerTelemetry;

/// The environment variable naming the consumed-offset cursor file.
pub const PAYMENT_CURSOR_ENV: &str = "APEX_PAYMENT_CURSOR";

/// One collected payment reference and its due amount, appended by the
/// monthly collection path when payment links are created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentLedgerRow {
    /// The provider reference (the payment link's reference).
    pub reference: String,
    /// The amount due on that reference, in micros.
    pub amount_micros: i64,
    /// ISO-4217 currency.
    pub currency: String,
    /// When the reference was created, unix milliseconds.
    pub at_unix_ms: i64,
}

/// One scheduled payment retry from the finance boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledRetry {
    /// The collected reference the retry belongs to.
    pub invoice_reference: String,
    /// The unpaid balance, in micros.
    pub amount_micros: i64,
    /// ISO-4217 currency.
    pub currency: String,
    /// The day the retry is scheduled for (`YYYY-MM-DD`).
    #[serde(default)]
    pub retry_on: Option<String>,
    /// The retry's own reference.
    pub reference: String,
    /// `scheduled` or `cancelled`.
    pub state: String,
    /// When the schedule was recorded, unix milliseconds.
    pub at_unix_ms: i64,
}

/// Why the payment files could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PaymentError {
    /// A file could not be read.
    #[error("payment file {path}: {reason}")]
    Io {
        /// The file involved.
        path: String,
        /// The reason.
        reason: String,
    },
    /// A stored line is corrupt; the processor fails closed.
    #[error("payment file {path} line {line} is corrupt: {reason}")]
    Corrupt {
        /// The file involved.
        path: String,
        /// The 1-based line.
        line: usize,
        /// The parse failure.
        reason: String,
    },
}

/// What one processing pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PaymentsReport {
    /// Successful authorisations processed.
    pub received: u32,
    /// Refused authorisations processed.
    pub failed: u32,
    /// Scheduled retries that produced a message.
    pub retries: u32,
    /// Reconciliation events.
    pub reconciled: u32,
    /// Refund events.
    pub refunded: u32,
    /// Dispute events.
    pub disputed: u32,
    /// Provider events that carry no bounded class (or a received message
    /// without a known balance).
    pub skipped: u32,
}

impl PaymentsReport {
    /// Whether the pass observed anything at all.
    pub fn any(&self) -> bool {
        !(self.received == 0
            && self.failed == 0
            && self.retries == 0
            && self.reconciled == 0
            && self.refunded == 0
            && self.disputed == 0
            && self.skipped == 0)
    }
}

/// Append one collected-reference row (the monthly collection path records
/// every created payment reference here).
pub fn append_payment_ledger_row(path: &Path, row: &PaymentLedgerRow) -> Result<(), PaymentError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| PaymentError::Io {
                path: parent.display().to_string(),
                reason: error.to_string(),
            })?;
        }
    }
    let line = serde_json::to_string(row).map_err(|error| PaymentError::Io {
        path: path.display().to_string(),
        reason: error.to_string(),
    })?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| PaymentError::Io {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
    writeln!(file, "{line}").map_err(|error| PaymentError::Io {
        path: path.display().to_string(),
        reason: error.to_string(),
    })
}

/// Read the collection file; a missing file is an empty history.
pub fn read_collection_events(path: &Path) -> Result<Vec<CollectionEvent>, PaymentError> {
    read_jsonl(path)
}

/// The default cursor file for a collection file: `<collection>.cursor`.
#[must_use]
pub fn default_cursor_path(collection: &Path) -> PathBuf {
    let mut name = collection.as_os_str().to_owned();
    name.push(".cursor");
    PathBuf::from(name)
}

/// Read the consumed byte offset; a missing or unparsable cursor is 0, which
/// re-processes the file safely (the emits are dedup-keyed).
pub fn read_consumed_offset(path: &Path) -> Result<u64, PaymentError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text.trim().parse().unwrap_or(0)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(PaymentError::Io {
            path: path.display().to_string(),
            reason: error.to_string(),
        }),
    }
}

/// Write the consumed offset atomically (temp file plus rename) so a crash
/// never leaves a half-written cursor that would skip or repeat wrongly.
pub fn write_consumed_offset(path: &Path, offset: u64) -> Result<(), PaymentError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| PaymentError::Io {
                path: parent.display().to_string(),
                reason: error.to_string(),
            })?;
        }
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, format!("{offset}\n")).map_err(|error| PaymentError::Io {
        path: tmp.display().to_string(),
        reason: error.to_string(),
    })?;
    std::fs::rename(&tmp, path).map_err(|error| PaymentError::Io {
        path: path.display().to_string(),
        reason: error.to_string(),
    })
}

/// Read the collection events after `offset` bytes, with the new consumed
/// offset: only complete lines advance the cursor, so a crash mid-append
/// leaves the partial tail for the next pass.
///
/// A cursor past the file's end (the file was truncated or the cursor was
/// rewound) restarts at 0, and a cursor that is not at a line boundary
/// restarts at 0 as well: re-processing the file is safe because every emit
/// is dedup-keyed, while skipping a record would not be.
pub fn read_new_collection_events(
    path: &Path,
    offset: u64,
) -> Result<(Vec<CollectionEvent>, u64), PaymentError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), 0)),
        Err(error) => {
            return Err(PaymentError::Io {
                path: path.display().to_string(),
                reason: error.to_string(),
            })
        }
    };
    let mut start = usize::try_from(offset)
        .ok()
        .filter(|start| *start <= bytes.len())
        .unwrap_or(0);
    if start > 0 && bytes.get(start - 1) != Some(&b'\n') {
        // The cursor does not sit on a line boundary (a hand-edited or
        // rewound cursor). Re-process from the start; never parse a tail.
        start = 0;
    }
    let base_lines = bytes[..start].iter().filter(|byte| **byte == b'\n').count();
    let mut consumed = start;
    let mut rows = Vec::new();
    for (index, line) in bytes[start..]
        .split_inclusive(|byte| *byte == b'\n')
        .enumerate()
    {
        if !line.ends_with(b"\n") {
            // An incomplete tail line (the writer is mid-append): leave it
            // for the next pass instead of parsing a fragment.
            break;
        }
        consumed += line.len();
        let text = String::from_utf8_lossy(line);
        if text.trim().is_empty() {
            continue;
        }
        rows.push(
            serde_json::from_str(text.trim()).map_err(|error| PaymentError::Corrupt {
                path: path.display().to_string(),
                line: base_lines + index + 1,
                reason: error.to_string(),
            })?,
        );
    }
    Ok((rows, consumed as u64))
}

/// Read the payment ledger; a missing file is an empty history.
pub fn read_payment_ledger(path: &Path) -> Result<Vec<PaymentLedgerRow>, PaymentError> {
    read_jsonl(path)
}

/// Read the retry schedule; a missing file is an empty schedule.
pub fn read_scheduled_retries(path: &Path) -> Result<Vec<ScheduledRetry>, PaymentError> {
    read_jsonl(path)
}

fn read_jsonl<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Vec<T>, PaymentError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(PaymentError::Io {
                path: path.display().to_string(),
                reason: error.to_string(),
            })
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut rows = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        rows.push(
            serde_json::from_str(line).map_err(|error| PaymentError::Corrupt {
                path: path.display().to_string(),
                line: index + 1,
                reason: error.to_string(),
            })?,
        );
    }
    Ok(rows)
}

/// The bounded class of a provider event code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaymentClass {
    Authorisation,
    Refund,
    Dispute,
    Reconciliation,
}

fn classify(event_code: &str) -> Option<PaymentClass> {
    match event_code.to_ascii_uppercase().as_str() {
        "AUTHORISATION" => Some(PaymentClass::Authorisation),
        "REFUND" => Some(PaymentClass::Refund),
        "DISPUTE" | "CHARGEBACK" | "NOTIFICATION_OF_CHARGEBACK" => Some(PaymentClass::Dispute),
        "RECONCILIATION" => Some(PaymentClass::Reconciliation),
        _ => None,
    }
}

fn micros_of(minor_units: i64) -> Option<i64> {
    minor_units.checked_mul(MICROS_PER_MINOR_UNIT)
}

/// The currency to render with: the record's own non-blank code, or `None`
/// (the templates then render the not-available reason, never USD).
fn currency_of(code: &str) -> Option<&str> {
    let code = code.trim();
    (!code.is_empty()).then_some(code)
}

fn day_of(at_unix_ms: i64) -> String {
    crate::calendar::format_day(at_unix_ms.div_euclid(86_400_000))
}

/// Process one batch of verified provider events plus the retry schedule.
///
/// `analytics` and `telemetry` are optional collaborators: tests and dry-run
/// deployments run without them, and the durable collection file stays the
/// source of truth either way.
#[allow(clippy::too_many_arguments)]
pub async fn process_payment_events(
    events: &[CollectionEvent],
    ledger: &[PaymentLedgerRow],
    retries: &[ScheduledRetry],
    notifier: Option<&NotifyPlan>,
    analytics: Option<&crate::events::RunnerEvents>,
    telemetry: Option<&RunnerTelemetry>,
    publisher: &str,
    now_unix_ms: i64,
) -> PaymentsReport {
    use apex_analytics::EventType;
    use apex_metrics::vocab::{PaymentEventClass, PaymentResult};

    let mut report = PaymentsReport::default();
    // Successful receipts per collected reference, in file order, so the
    // remaining balance is correct for partial payments.
    let mut received_by_reference: std::collections::BTreeMap<String, i64> =
        std::collections::BTreeMap::new();
    for event in events {
        let Some(class) = classify(&event.event_code) else {
            report.skipped += 1;
            continue;
        };
        let Some(amount_micros) = micros_of(event.amount_value) else {
            report.skipped += 1;
            continue;
        };
        let occurred = event.at_unix_ms;
        match class {
            PaymentClass::Authorisation if event.success.eq_ignore_ascii_case("true") => {
                report.received += 1;
                if let Some(telemetry) = telemetry {
                    telemetry.payment(PaymentEventClass::Authorisation, PaymentResult::Accepted);
                }
                if let Some(analytics) = analytics {
                    analytics
                        .payment_received(
                            &event.psp_reference,
                            &event.merchant_reference,
                            &event.amount_currency,
                            amount_micros,
                            occurred,
                        )
                        .await;
                }
                let total = received_by_reference
                    .entry(event.merchant_reference.clone())
                    .or_default();
                *total += amount_micros;
                if let Some(notifier) = notifier {
                    let due = ledger
                        .iter()
                        .find(|row| row.reference == event.merchant_reference)
                        .map(|row| row.amount_micros);
                    match due {
                        Some(due) => {
                            let remaining = (due - *total).max(0);
                            notifier
                                .payment_received(
                                    publisher,
                                    &event.merchant_reference,
                                    amount_micros,
                                    remaining,
                                    currency_of(&event.amount_currency),
                                    &day_of(occurred),
                                    &event.psp_reference,
                                    now_unix_ms,
                                )
                                .await;
                        }
                        None => {
                            // The due amount is unknown: the message states a
                            // remaining balance, so it must not be guessed.
                            report.skipped += 1;
                            eprintln!(
                                "apex-payments: reference {} is not in the payment ledger; the received message is skipped",
                                event.merchant_reference
                            );
                        }
                    }
                }
            }
            PaymentClass::Authorisation => {
                report.failed += 1;
                if let Some(telemetry) = telemetry {
                    telemetry.payment(PaymentEventClass::Authorisation, PaymentResult::Failed);
                }
                if let Some(notifier) = notifier {
                    notifier
                        .payment_failed(
                            publisher,
                            &event.merchant_reference,
                            amount_micros,
                            currency_of(&event.amount_currency),
                            &event.psp_reference,
                            now_unix_ms,
                        )
                        .await;
                }
            }
            PaymentClass::Refund => {
                report.refunded += 1;
                if let Some(telemetry) = telemetry {
                    telemetry.payment(PaymentEventClass::Refund, PaymentResult::Accepted);
                }
                if let Some(analytics) = analytics {
                    analytics
                        .payment_event(
                            EventType::PaymentRefunded,
                            &event.psp_reference,
                            &event.merchant_reference,
                            &event.amount_currency,
                            amount_micros,
                            occurred,
                        )
                        .await;
                }
            }
            PaymentClass::Dispute => {
                report.disputed += 1;
                if let Some(telemetry) = telemetry {
                    telemetry.payment(PaymentEventClass::Dispute, PaymentResult::Accepted);
                }
                if let Some(analytics) = analytics {
                    analytics
                        .payment_event(
                            EventType::PaymentDisputed,
                            &event.psp_reference,
                            &event.merchant_reference,
                            &event.amount_currency,
                            amount_micros,
                            occurred,
                        )
                        .await;
                }
            }
            PaymentClass::Reconciliation => {
                report.reconciled += 1;
                if let Some(telemetry) = telemetry {
                    telemetry.payment(PaymentEventClass::Reconciliation, PaymentResult::Reconciled);
                }
                if let Some(analytics) = analytics {
                    analytics
                        .payment_event(
                            EventType::PaymentReconciled,
                            &event.psp_reference,
                            &event.merchant_reference,
                            &event.amount_currency,
                            amount_micros,
                            occurred,
                        )
                        .await;
                }
            }
        }
    }

    // Retry messages exist only for a scheduled retry with a real date.
    for retry in retries {
        if retry.state != "scheduled" {
            continue;
        }
        let Some(notifier) = notifier else {
            continue;
        };
        let report_entry = notifier
            .payment_retry(
                publisher,
                &retry.invoice_reference,
                retry.amount_micros,
                currency_of(&retry.currency),
                retry.retry_on.as_deref(),
                now_unix_ms,
            )
            .await;
        if report_entry.attempted + report_entry.duplicates > 0 {
            report.retries += 1;
        } else {
            report.skipped += 1;
        }
    }
    report
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn event(code: &str, success: &str, psp: &str, amount: i64) -> CollectionEvent {
        CollectionEvent {
            at_unix_ms: 1_797_235_200_000, // 2026-12-14 UTC
            event_code: code.to_string(),
            psp_reference: psp.to_string(),
            merchant_reference: "apex-fee:northstar:ch_0246:2026-10".to_string(),
            success: success.to_string(),
            amount_value: amount,
            amount_currency: "USD".to_string(),
        }
    }

    fn eur_event(code: &str, success: &str, psp: &str, amount: i64) -> CollectionEvent {
        CollectionEvent {
            amount_currency: "EUR".to_string(),
            ..event(code, success, psp, amount)
        }
    }

    #[test]
    fn classes_are_bounded_and_unknown_codes_are_skipped() {
        assert_eq!(classify("AUTHORISATION"), Some(PaymentClass::Authorisation));
        assert_eq!(classify("authorisation"), Some(PaymentClass::Authorisation));
        assert_eq!(classify("REFUND"), Some(PaymentClass::Refund));
        assert_eq!(classify("CHARGEBACK"), Some(PaymentClass::Dispute));
        assert_eq!(
            classify("RECONCILIATION"),
            Some(PaymentClass::Reconciliation)
        );
        assert_eq!(classify("SOMETHING_CUSTOM"), None);
        assert_eq!(classify(""), None);
        assert_eq!(micros_of(300), Some(3_000_000));
        assert_eq!(micros_of(i64::MAX), None);
        assert_eq!(day_of(1_797_235_200_000), "2026-12-14");
    }

    #[tokio::test]
    async fn received_failed_refunded_and_disputed_flow_without_a_notifier() {
        let events = vec![
            event("AUTHORISATION", "true", "psp_1", 300),
            event("AUTHORISATION", "false", "psp_2", 300),
            event("REFUND", "true", "psp_3", 50),
            event("CHARGEBACK", "true", "psp_4", 300),
            event("RECONCILIATION", "true", "psp_5", 300),
            event("MYSTERY_EVENT", "true", "psp_6", 300),
        ];
        let report = process_payment_events(
            &events,
            &[],
            &[],
            None,
            None,
            None,
            "northstar",
            1_797_235_200_000,
        )
        .await;
        assert_eq!(
            report,
            PaymentsReport {
                received: 1,
                failed: 1,
                retries: 0,
                reconciled: 1,
                refunded: 1,
                disputed: 1,
                skipped: 1,
            }
        );
        assert!(report.any());
        assert!(!PaymentsReport::default().any());
    }

    async fn plan(dir: &std::path::Path) -> crate::notify::NotifyPlan {
        let _ = std::fs::remove_dir_all(dir);
        let sink = std::sync::Arc::new(apex_notify::sink::FileSink::new(dir.join("outbox.jsonl")));
        let log =
            apex_notify::state::DeliveryLog::open(Some(&dir.join("delivery.jsonl"))).expect("log");
        let notifier = std::sync::Arc::new(apex_notify::notifier::Notifier::new(
            sink,
            log,
            std::sync::Arc::new(apex_metrics::Registry::new()),
        ));
        let book =
            apex_notify::contacts::ContactBook::from_json(r#"{"northstar": ["ops@example.com"]}"#)
                .expect("book");
        crate::notify::NotifyPlan::new(notifier, book, "https://console.apexmediation.ee")
    }

    fn outbox(dir: &std::path::Path) -> Vec<serde_json::Value> {
        std::fs::read_to_string(dir.join("outbox.jsonl"))
            .expect("outbox")
            .lines()
            .map(|line| serde_json::from_str(line).expect("row"))
            .collect()
    }

    #[tokio::test]
    async fn a_received_payment_states_the_remaining_balance_from_the_ledger() {
        let dir = std::env::temp_dir().join(format!("apex-payments-{}", std::process::id()));
        let plan = plan(&dir).await;
        let ledger = vec![PaymentLedgerRow {
            reference: "apex-fee:northstar:ch_0246:2026-10".into(),
            amount_micros: 5_000_000,
            currency: "USD".into(),
            at_unix_ms: 0,
        }];
        let events = vec![
            event("AUTHORISATION", "true", "psp_1", 300),
            event("AUTHORISATION", "true", "psp_2", 200),
        ];
        let report = process_payment_events(
            &events,
            &ledger,
            &[],
            Some(&plan),
            None,
            None,
            "northstar",
            1_797_235_200_000,
        )
        .await;
        assert_eq!(report.received, 2);
        let rows = outbox(&dir);
        assert_eq!(rows.len(), 2, "one message per receipt");
        let first = rows[0]["text"].as_str().unwrap();
        assert!(first.contains("We received USD 3.00"));
        assert!(first.contains("The remaining balance is USD 2.00."));
        let second = rows[1]["text"].as_str().unwrap();
        assert!(second.contains("We received USD 2.00"));
        assert!(second.contains("The remaining balance is USD 0.00."));
        assert_eq!(
            rows[0]["dedup_key"],
            "payment_received:apex-fee:northstar:ch_0246:2026-10:psp_1"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_unknown_reference_skips_the_received_message_and_a_failure_notifies() {
        let dir = std::env::temp_dir().join(format!("apex-payments-miss-{}", std::process::id()));
        let plan = plan(&dir).await;
        let events = vec![
            event("AUTHORISATION", "true", "psp_1", 300),
            event("AUTHORISATION", "false", "psp_2", 300),
        ];
        let report = process_payment_events(
            &events,
            &[],
            &[],
            Some(&plan),
            None,
            None,
            "northstar",
            1_797_235_200_000,
        )
        .await;
        assert_eq!(report.received, 1);
        assert_eq!(report.failed, 1);
        assert_eq!(report.skipped, 1, "the received message needs a balance");
        let rows = outbox(&dir);
        assert_eq!(rows.len(), 1, "only the failure message was sent");
        assert_eq!(rows[0]["template_class"], "payment_failed");
        let text = rows[0]["text"].as_str().unwrap();
        assert!(text
            .contains("We could not collect USD 3.00 for this invoice. No payment is confirmed."));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn retries_send_only_when_scheduled_with_a_date() {
        let dir = std::env::temp_dir().join(format!("apex-payments-retry-{}", std::process::id()));
        let plan = plan(&dir).await;
        let retries = vec![
            ScheduledRetry {
                invoice_reference: "apex-fee:northstar:ch_0246:2026-10".into(),
                amount_micros: 300_000_000,
                currency: "USD".into(),
                retry_on: Some("2026-12-20".into()),
                reference: "retry_1".into(),
                state: "scheduled".into(),
                at_unix_ms: 0,
            },
            ScheduledRetry {
                invoice_reference: "apex-fee:northstar:ch_0246:2026-10".into(),
                amount_micros: 300_000_000,
                currency: "USD".into(),
                retry_on: Some("2026-12-21".into()),
                reference: "retry_2".into(),
                state: "cancelled".into(),
                at_unix_ms: 0,
            },
            ScheduledRetry {
                invoice_reference: "apex-fee:northstar:ch_0246:2026-10".into(),
                amount_micros: 300_000_000,
                currency: "USD".into(),
                retry_on: None,
                reference: "retry_3".into(),
                state: "scheduled".into(),
                at_unix_ms: 0,
            },
        ];
        let report = process_payment_events(
            &[],
            &[],
            &retries,
            Some(&plan),
            None,
            None,
            "northstar",
            1_797_235_200_000,
        )
        .await;
        assert_eq!(report.retries, 1);
        assert_eq!(report.skipped, 1, "the dateless schedule sends nothing");
        let rows = outbox(&dir);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["template_class"], "payment_retry");
        assert!(rows[0]["text"]
            .as_str()
            .unwrap()
            .contains("A retry is scheduled for 20 December 2026"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn jsonl_readers_fail_closed_on_corruption_and_tolerate_absence() {
        let dir = std::env::temp_dir().join(format!("apex-payments-io-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("missing.jsonl");
        assert!(read_collection_events(&missing).unwrap().is_empty());
        assert!(read_payment_ledger(&missing).unwrap().is_empty());
        assert!(read_scheduled_retries(&missing).unwrap().is_empty());

        let bad = dir.join("bad.jsonl");
        std::fs::write(&bad, "not json\n").unwrap();
        let error = read_collection_events(&bad).expect_err("corrupt");
        assert!(matches!(error, PaymentError::Corrupt { line: 1, .. }));
        assert!(error.to_string().contains("line 1"));
        std::fs::write(&bad, "").unwrap();
        assert!(read_collection_events(&bad).unwrap().is_empty());

        let good = dir.join("good.jsonl");
        std::fs::write(
            &good,
            r#"{"reference":"r","amount_micros":1,"currency":"USD","at_unix_ms":0}"#,
        )
        .unwrap();
        let rows = read_payment_ledger(&good).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].reference, "r");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_non_usd_receipt_renders_the_events_currency() {
        let dir = std::env::temp_dir().join(format!("apex-payments-eur-{}", std::process::id()));
        let plan = plan(&dir).await;
        let ledger = vec![PaymentLedgerRow {
            reference: "apex-fee:northstar:ch_0246:2026-10".into(),
            amount_micros: 5_000_000,
            currency: "EUR".into(),
            at_unix_ms: 0,
        }];
        let events = vec![eur_event("AUTHORISATION", "true", "psp_1", 300)];
        let report = process_payment_events(
            &events,
            &ledger,
            &[],
            Some(&plan),
            None,
            None,
            "northstar",
            1_797_235_200_000,
        )
        .await;
        assert_eq!(report.received, 1);
        let rows = outbox(&dir);
        assert_eq!(rows.len(), 1);
        let text = rows[0]["text"].as_str().unwrap();
        assert!(text.contains("We received EUR 3.00"), "{text}");
        assert!(
            text.contains("The remaining balance is EUR 2.00."),
            "{text}"
        );
        assert!(!text.contains("USD"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_cursor_consumes_only_new_complete_lines() {
        let dir = std::env::temp_dir().join(format!("apex-payments-cur-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let collection = dir.join("collection.jsonl");
        let mut file = std::fs::File::create(&collection).unwrap();
        use std::io::Write as _;
        writeln!(
            file,
            "{}",
            serde_json::to_string(&event("AUTHORISATION", "true", "psp_1", 300)).unwrap()
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::to_string(&event("AUTHORISATION", "true", "psp_2", 200)).unwrap()
        )
        .unwrap();
        file.flush().unwrap();

        let (rows, consumed) = read_new_collection_events(&collection, 0).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].psp_reference, "psp_1");
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&collection)
            .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::to_string(&event("REFUND", "true", "psp_3", 50)).unwrap()
        )
        .unwrap();
        file.flush().unwrap();

        // The second pass sees only the new event.
        let (rows, next) = read_new_collection_events(&collection, consumed).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].psp_reference, "psp_3");
        let (rows, same) = read_new_collection_events(&collection, next).unwrap();
        assert!(rows.is_empty(), "nothing new on a third pass");
        assert_eq!(same, next);

        // A partial tail line is not consumed until it is complete.
        let full = serde_json::to_string(&event("AUTHORISATION", "true", "psp_4", 100)).unwrap();
        let (head, tail) = full.split_at(full.len() / 2);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&collection)
            .unwrap();
        write!(file, "{head}").unwrap();
        file.flush().unwrap();
        let (rows, stuck) = read_new_collection_events(&collection, next).unwrap();
        assert!(rows.is_empty(), "a mid-append fragment is never parsed");
        assert_eq!(stuck, next, "the partial tail does not advance the cursor");
        writeln!(file, "{tail}").unwrap();
        file.flush().unwrap();
        let (rows, advanced) = read_new_collection_events(&collection, stuck).unwrap();
        assert_eq!(rows.len(), 1, "the completed line is consumed");
        assert_eq!(rows[0].psp_reference, "psp_4");
        assert!(advanced > stuck);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_truncated_or_rewound_cursor_reprocesses_safely() {
        let dir = std::env::temp_dir().join(format!("apex-payments-rw-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let collection = dir.join("collection.jsonl");
        let line_one =
            serde_json::to_string(&event("AUTHORISATION", "true", "psp_1", 300)).unwrap();
        let line_two =
            serde_json::to_string(&event("AUTHORISATION", "true", "psp_2", 200)).unwrap();
        std::fs::write(&collection, format!("{line_one}\n{line_two}\n")).unwrap();
        let (rows, consumed) = read_new_collection_events(&collection, 0).unwrap();
        assert_eq!(rows.len(), 2);

        // The file was truncated to fewer bytes than the cursor: the pass
        // restarts at 0 and re-processes the surviving line rather than
        // skipping a record.
        std::fs::write(&collection, format!("{line_one}\n")).unwrap();
        let (rows, rewound) = read_new_collection_events(&collection, consumed).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].psp_reference, "psp_1");
        assert_eq!(rewound, (line_one.len() + 1) as u64);

        // A cursor that is not at a line boundary also restarts at 0.
        let (rows, _) = read_new_collection_events(&collection, 5).unwrap();
        assert_eq!(rows.len(), 1, "mid-line cursors re-process, never skip");

        // The cursor file round-trips atomically, and a missing or corrupt
        // cursor is a safe 0.
        let cursor = default_cursor_path(&collection);
        assert_eq!(cursor, dir.join("collection.jsonl.cursor"));
        assert_eq!(read_consumed_offset(&cursor).unwrap(), 0);
        write_consumed_offset(&cursor, rewound).unwrap();
        assert_eq!(read_consumed_offset(&cursor).unwrap(), rewound);
        assert!(!dir.join("collection.jsonl.cursor.tmp").exists());
        std::fs::write(&cursor, "not a number\n").unwrap();
        assert_eq!(read_consumed_offset(&cursor).unwrap(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
