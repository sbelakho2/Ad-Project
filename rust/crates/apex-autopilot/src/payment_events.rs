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
//! records the byte offset of the last consumed line, plus the number of
//! complete lines before it. Each pass seeks to the offset and reads only the
//! new tail — never the growing prefix — processes the complete lines, and
//! writes the new cursor atomically (temp file plus rename) after success. A
//! crash between processing and the cursor write re-processes the last batch,
//! which is safe by construction: the analytics emits are dedup-keyed on the
//! event identity and the delivery log deduplicates the messages. A truncated
//! or rewound file (cursor past the end, or a cursor that is not a line
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
    /// A reference's received total overflowed i64 micros while accumulating
    /// its receipts; the pass fails rather than fabricating a balance. The
    /// offending provider event is named.
    #[error("payment event {psp_reference} for reference {reference}: the received total overflows i64 micros")]
    Overflow {
        /// The provider event that tipped the total over.
        psp_reference: String,
        /// The collected reference being accumulated.
        reference: String,
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

/// The consumed-offset cursor: the byte offset of the last consumed newline
/// plus the number of complete lines before it. The line count makes a
/// corrupt-line error exact without ever reading the file's prefix; a cursor
/// written by an older build carries only the offset and falls back to
/// counting the prefix only when an error actually needs a line number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PaymentCursor {
    /// Bytes consumed (always a line boundary).
    pub offset: u64,
    /// Complete lines before `offset`, when known.
    pub line: Option<u64>,
}

/// Read the consumed cursor; a missing or unparsable cursor is the empty
/// cursor, which re-processes the file safely (the emits are dedup-keyed).
pub fn read_payment_cursor(path: &Path) -> Result<PaymentCursor, PaymentError> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let mut fields = text.split_whitespace();
            let offset = match fields.next().and_then(|value| value.parse::<u64>().ok()) {
                Some(offset) => offset,
                // A malformed cursor re-processes from the start, exactly as
                // the previous single-number format did; a stray line token
                // is not trusted either.
                None => return Ok(PaymentCursor::default()),
            };
            let line = fields.next().and_then(|value| value.parse().ok());
            Ok(PaymentCursor { offset, line })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(PaymentCursor::default()),
        Err(error) => Err(PaymentError::Io {
            path: path.display().to_string(),
            reason: error.to_string(),
        }),
    }
}

/// Read the consumed byte offset; a missing or unparsable cursor is 0, which
/// re-processes the file safely (the emits are dedup-keyed).
pub fn read_consumed_offset(path: &Path) -> Result<u64, PaymentError> {
    read_payment_cursor(path).map(|cursor| cursor.offset)
}

/// Write the consumed cursor atomically (temp file plus rename) so a crash
/// never leaves a half-written cursor that would skip or repeat wrongly.
pub fn write_payment_cursor(path: &Path, cursor: PaymentCursor) -> Result<(), PaymentError> {
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
    let body = match cursor.line {
        Some(line) => format!("{} {line}\n", cursor.offset),
        None => format!("{}\n", cursor.offset),
    };
    std::fs::write(&tmp, body).map_err(|error| PaymentError::Io {
        path: tmp.display().to_string(),
        reason: error.to_string(),
    })?;
    std::fs::rename(&tmp, path).map_err(|error| PaymentError::Io {
        path: path.display().to_string(),
        reason: error.to_string(),
    })
}

/// Write a bare consumed offset (the legacy cursor shape).
pub fn write_consumed_offset(path: &Path, offset: u64) -> Result<(), PaymentError> {
    write_payment_cursor(path, PaymentCursor { offset, line: None })
}

/// Read the collection events after `cursor`, with the new consumed cursor:
/// only complete lines advance the cursor, so a crash mid-append leaves the
/// partial tail for the next pass.
///
/// The read is a seek to the cursor offset: the file's prefix is never read
/// when the cursor is valid, and the cursor carries the line count so a
/// corrupt tail reports its exact 1-based line.
pub fn read_new_collection_events_from(
    path: &Path,
    cursor: PaymentCursor,
) -> Result<(Vec<CollectionEvent>, PaymentCursor), PaymentError> {
    let file = std::fs::File::open(path);
    let file = match file {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), PaymentCursor::default()))
        }
        Err(error) => {
            return Err(PaymentError::Io {
                path: path.display().to_string(),
                reason: error.to_string(),
            })
        }
    };
    read_new_collection_events_with_reader(path, file, cursor)
}

/// The seek-based reader seam: `reader` starts at any position and is used
/// only from the validated cursor to the end of the file. Tests wrap a
/// counting reader here to prove the prefix is never read.
pub(crate) fn read_new_collection_events_with_reader<R: std::io::Read + std::io::Seek>(
    path: &Path,
    mut reader: R,
    cursor: PaymentCursor,
) -> Result<(Vec<CollectionEvent>, PaymentCursor), PaymentError> {
    use std::io::SeekFrom;

    let io_error = |error: std::io::Error| PaymentError::Io {
        path: path.display().to_string(),
        reason: error.to_string(),
    };
    let length = reader.seek(SeekFrom::End(0)).map_err(io_error)?;
    let mut offset = cursor.offset.min(length);
    let mut base_line = cursor.line;
    let mut restart = cursor.offset > length;
    if !restart && offset > 0 {
        reader.seek(SeekFrom::Start(offset - 1)).map_err(io_error)?;
        let mut previous = [0u8; 1];
        reader.read_exact(&mut previous).map_err(io_error)?;
        if previous[0] != b'\n' {
            // The cursor does not sit on a line boundary (a hand-edited or
            // rewound cursor). Re-process from the start; never parse a tail.
            restart = true;
        }
    }
    if restart {
        offset = 0;
        base_line = Some(0);
    } else if offset == 0 && base_line.is_none() {
        // A fresh cursor starts at line 0 even when the file was absent on
        // the previous pass. From here the line count is carried forward.
        base_line = Some(0);
    }
    reader.seek(SeekFrom::Start(offset)).map_err(io_error)?;
    let mut tail = Vec::new();
    reader.read_to_end(&mut tail).map_err(io_error)?;

    let base_lines = base_line;
    let mut consumed = offset;
    let mut consumed_lines = base_lines;
    let mut rows = Vec::new();
    for (index, line) in tail.split_inclusive(|byte| *byte == b'\n').enumerate() {
        if !line.ends_with(b"\n") {
            // An incomplete tail line (the writer is mid-append): leave it
            // for the next pass instead of parsing a fragment.
            break;
        }
        consumed += line.len() as u64;
        if let Some(lines) = consumed_lines.as_mut() {
            *lines += 1;
        }
        let text = String::from_utf8_lossy(line);
        if text.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(text.trim()) {
            Ok(row) => rows.push(row),
            Err(error) => {
                // A legacy cursor has no line count. Only this error path
                // counts the prefix; the success path never reads it.
                let base = match base_lines {
                    Some(line) => line,
                    None => count_lines_before(&mut reader, path, offset)?,
                };
                return Err(PaymentError::Corrupt {
                    path: path.display().to_string(),
                    line: usize::try_from(base)
                        .unwrap_or(usize::MAX)
                        .saturating_add(index)
                        .saturating_add(1),
                    reason: error.to_string(),
                });
            }
        }
    }
    Ok((
        rows,
        PaymentCursor {
            offset: consumed,
            line: consumed_lines,
        },
    ))
}

/// Count newline bytes in `[0, offset)` using the already-open reader. This is
/// the legacy-cursor fallback and runs only when an error needs an exact line.
fn count_lines_before<R: std::io::Read + std::io::Seek>(
    reader: &mut R,
    path: &Path,
    offset: u64,
) -> Result<u64, PaymentError> {
    use std::io::SeekFrom;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|error| PaymentError::Io {
            path: path.display().to_string(),
            reason: error.to_string(),
        })?;
    let mut remaining = offset;
    let mut lines = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    while remaining > 0 {
        let wanted = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = reader
            .read(&mut buffer[..wanted])
            .map_err(|error| PaymentError::Io {
                path: path.display().to_string(),
                reason: error.to_string(),
            })?;
        if read == 0 {
            break;
        }
        lines += buffer[..read].iter().filter(|byte| **byte == b'\n').count() as u64;
        remaining -= read as u64;
    }
    Ok(lines)
}

/// Read the collection events after `offset` bytes, keeping the historical
/// signature. New callers use [`read_new_collection_events_from`] so the
/// cursor's line count also advances.
pub fn read_new_collection_events(
    path: &Path,
    offset: u64,
) -> Result<(Vec<CollectionEvent>, u64), PaymentError> {
    let (rows, cursor) =
        read_new_collection_events_from(path, PaymentCursor { offset, line: None })?;
    Ok((rows, cursor.offset))
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

/// The publisher a billing reference belongs to: payment links carry
/// `apex-fee:<publisher>:<experiment>:<period>` and invoice references carry
/// `INV-<YYYY-MM>-<publisher>`. The fleet-level payment pass uses this to
/// scope each group's customer messages to the owning publisher; an
/// unrecognized or charset-invalid reference yields `None` (processed with an
/// empty publisher scope, never silently attributed to someone).
pub fn publisher_of_reference(reference: &str) -> Option<String> {
    let candidate = reference
        .strip_prefix("apex-fee:")
        .and_then(|rest| rest.split(':').next())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .or_else(|| {
            // `YYYY-MM-<publisher>`: fixed-shape month, then the publisher.
            let rest = reference.strip_prefix("INV-")?;
            let bytes = rest.as_bytes();
            if rest.len() < 10
                || !bytes[..4].iter().all(u8::is_ascii_digit)
                || bytes[4] != b'-'
                || !bytes[5..7].iter().all(u8::is_ascii_digit)
                || bytes[7] != b'-'
            {
                return None;
            }
            Some(rest[8..].to_string())
        })?;
    let valid = candidate.len() <= 64
        && candidate
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then_some(candidate)
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
///
/// The pass fails with a typed [`PaymentError::Overflow`] when a reference's
/// received total cannot be accumulated in i64 micros: the remaining balance
/// is never fabricated from a wrapped total (the batch is re-processed after
/// the corrupt source is fixed; every emit is dedup-keyed).
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
) -> Result<PaymentsReport, PaymentError> {
    use apex_analytics::EventType;
    use apex_metrics::vocab::{PaymentEventClass, PaymentResult};

    let mut report = PaymentsReport::default();
    // The due amount per reference, indexed once: the old shape rescanned the
    // whole payment history for every received event. The first row per
    // reference wins, exactly like the previous linear `find`.
    let due_by_reference: Option<std::collections::HashMap<&str, i64>> = notifier.map(|_| {
        let mut due = std::collections::HashMap::with_capacity(ledger.len());
        for row in ledger {
            due.entry(row.reference.as_str())
                .or_insert(row.amount_micros);
        }
        due
    });
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
                // A corrupt row must not wrap the total: fail the pass,
                // naming the event, and let the batch retry after the
                // source is fixed.
                *total =
                    total
                        .checked_add(amount_micros)
                        .ok_or_else(|| PaymentError::Overflow {
                            psp_reference: event.psp_reference.clone(),
                            reference: event.merchant_reference.clone(),
                        })?;
                if let Some(notifier) = notifier {
                    let due = due_by_reference
                        .as_ref()
                        .and_then(|due| due.get(event.merchant_reference.as_str()).copied());
                    match due {
                        Some(due) => {
                            // A ledger row below the received total (or at
                            // i64::MIN) saturates to a zero remainder; it
                            // never underflows or fabricates a balance.
                            let remaining = due.saturating_sub(*total).max(0);
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
    Ok(report)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// Counts bytes actually read from the underlying file, so a test can
    /// prove the seek read never touched the prefix.
    struct CountingReader {
        inner: std::fs::File,
        bytes: std::sync::Arc<std::sync::atomic::AtomicU64>,
    }

    impl std::io::Read for CountingReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let read = std::io::Read::read(&mut self.inner, buffer)?;
            self.bytes
                .fetch_add(read as u64, std::sync::atomic::Ordering::SeqCst);
            Ok(read)
        }
    }

    impl std::io::Seek for CountingReader {
        fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
            std::io::Seek::seek(&mut self.inner, position)
        }
    }

    fn counting_reader(
        path: &Path,
    ) -> (CountingReader, std::sync::Arc<std::sync::atomic::AtomicU64>) {
        let bytes = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        (
            CountingReader {
                inner: std::fs::File::open(path).unwrap(),
                bytes: std::sync::Arc::clone(&bytes),
            },
            bytes,
        )
    }

    fn event(code: &str, success: &str, psp: &str, amount: i64) -> CollectionEvent {
        CollectionEvent {
            at_unix_ms: 1_797_235_200_000, // 2026-12-14 UTC
            event_code: code.to_string(),
            psp_reference: psp.to_string(),
            merchant_reference: "apex-fee:northstar:ch_0246:2026-10".to_string(),
            success: success.to_string(),
            amount_value: amount,
            amount_currency: "USD".to_string(),
            stored_payment_method_id: String::new(),
        }
    }

    fn eur_event(code: &str, success: &str, psp: &str, amount: i64) -> CollectionEvent {
        CollectionEvent {
            amount_currency: "EUR".to_string(),
            ..event(code, success, psp, amount)
        }
    }

    #[test]
    fn reference_publishers_are_parsed_strictly() {
        assert_eq!(
            publisher_of_reference("apex-fee:northstar:ch_0246:2026-10").as_deref(),
            Some("northstar")
        );
        assert_eq!(
            publisher_of_reference("INV-2026-09-northstar").as_deref(),
            Some("northstar")
        );
        assert_eq!(
            publisher_of_reference("INV-2026-09-pub_1-a").as_deref(),
            Some("pub_1-a")
        );
        for bad in [
            "",
            "PSP123",
            "apex-fee:",
            "apex-fee::ch:2026-10",
            "apex-fee:../evil:ch:2026-10",
            "INV-2026-09-",
            "INV-26-09-northstar",
            "INV-2026/09-northstar",
            "invoice-2026-09-northstar",
        ] {
            assert_eq!(publisher_of_reference(bad), None, "{bad:?}");
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
        .await
        .expect("pass");
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
        .await
        .expect("pass");
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
        .await
        .expect("pass");
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
        .await
        .expect("pass");
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
        .await
        .expect("pass");
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

    #[tokio::test]
    async fn overflowing_totals_fail_typed_and_extreme_ledgers_saturate() {
        // Two receipts for one reference whose micros sum past i64::MAX:
        // the pass fails, naming the offending event, and never fabricates
        // a wrapped remaining balance.
        let events = vec![
            event("AUTHORISATION", "true", "psp_1", 922_337_203_685_477),
            event("AUTHORISATION", "true", "psp_2", 922_337_203_685_477),
        ];
        let error = process_payment_events(
            &events,
            &[],
            &[],
            None,
            None,
            None,
            "northstar",
            1_797_235_200_000,
        )
        .await
        .expect_err("overflow");
        assert!(
            matches!(
                &error,
                PaymentError::Overflow { psp_reference, reference }
                    if psp_reference == "psp_2"
                        && reference == "apex-fee:northstar:ch_0246:2026-10"
            ),
            "{error:?}"
        );
        assert!(error.to_string().contains("psp_2"), "{error}");

        // An i64::MIN ledger row saturates to a zero remainder instead of
        // underflowing; an i64::MAX row still renders the exact balance.
        let dir = std::env::temp_dir().join(format!("apex-payments-ext-{}", std::process::id()));
        let min_plan = plan(&dir).await;
        let min_ledger = vec![PaymentLedgerRow {
            reference: "apex-fee:northstar:ch_0246:2026-10".into(),
            amount_micros: i64::MIN,
            currency: "USD".into(),
            at_unix_ms: 0,
        }];
        let report = process_payment_events(
            &[event("AUTHORISATION", "true", "psp_1", 300)],
            &min_ledger,
            &[],
            Some(&min_plan),
            None,
            None,
            "northstar",
            1_797_235_200_000,
        )
        .await
        .expect("i64::MIN ledger row");
        assert_eq!(report.received, 1);
        let rows = outbox(&dir);
        let text = rows[0]["text"].as_str().unwrap();
        assert!(
            text.contains("The remaining balance is USD 0.00."),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);

        let dir = std::env::temp_dir().join(format!("apex-payments-max-{}", std::process::id()));
        let max_plan = plan(&dir).await;
        let max_ledger = vec![PaymentLedgerRow {
            reference: "apex-fee:northstar:ch_0246:2026-10".into(),
            amount_micros: i64::MAX,
            currency: "USD".into(),
            at_unix_ms: 0,
        }];
        let report = process_payment_events(
            &[event("AUTHORISATION", "true", "psp_7", 300)],
            &max_ledger,
            &[],
            Some(&max_plan),
            None,
            None,
            "northstar",
            1_797_235_200_000,
        )
        .await
        .expect("i64::MAX ledger row");
        assert_eq!(report.received, 1);
        let expected = apex_notify::format_usd(i64::MAX - 3_000_000).expect("balance renders");
        let rows = outbox(&dir);
        let text = rows[0]["text"].as_str().unwrap();
        assert!(
            text.contains(&format!("The remaining balance is {expected}.")),
            "{text}"
        );
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
    fn the_cursor_file_carries_the_line_count_and_reads_legacy_shape() {
        let dir = std::env::temp_dir().join(format!("apex-payments-curfmt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cursor_path = dir.join("collection.jsonl.cursor");
        let cursor = PaymentCursor {
            offset: 123,
            line: Some(7),
        };
        write_payment_cursor(&cursor_path, cursor).unwrap();
        assert_eq!(read_payment_cursor(&cursor_path).unwrap(), cursor);
        assert_eq!(read_consumed_offset(&cursor_path).unwrap(), 123);
        // An older build wrote only the offset; it is still read, with the
        // line count unknown until an error actually needs it.
        std::fs::write(&cursor_path, "456\n").unwrap();
        assert_eq!(
            read_payment_cursor(&cursor_path).unwrap(),
            PaymentCursor {
                offset: 456,
                line: None
            }
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_seek_read_matches_the_legacy_offsets_and_reports_exact_error_lines() {
        let dir = std::env::temp_dir().join(format!("apex-payments-tail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let collection = dir.join("collection.jsonl");
        let line = serde_json::to_string(&event("AUTHORISATION", "true", "psp_1", 300)).unwrap();
        let mut body = String::new();
        for _ in 0..40 {
            body.push_str(&line);
            body.push('\n');
        }
        std::fs::write(&collection, &body).unwrap();
        let prefix = body.len() as u64;

        // Success path: new cursor and legacy offset agree, and the line
        // count advances without reading the prefix.
        let cursor = PaymentCursor {
            offset: 0,
            line: Some(0),
        };
        let (rows, advanced) = read_new_collection_events_from(&collection, cursor).unwrap();
        let (legacy_rows, legacy_offset) = read_new_collection_events(&collection, 0).unwrap();
        assert_eq!(rows, legacy_rows);
        assert_eq!(advanced.offset, legacy_offset);
        assert_eq!(advanced.line, Some(40));
        assert_eq!(advanced.offset, prefix);

        // A corrupt line at the tail reports the exact 1-based line from the
        // cursor, and the counting reader proves the prefix was not read.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&collection)
            .unwrap();
        use std::io::Write as _;
        writeln!(file, "{{not json}}").unwrap();
        file.flush().unwrap();
        let (mut reader, bytes) = counting_reader(&collection);
        let error = read_new_collection_events_with_reader(
            &collection,
            &mut reader,
            PaymentCursor {
                offset: prefix,
                line: Some(40),
            },
        )
        .expect_err("corrupt tail");
        assert!(
            matches!(&error, PaymentError::Corrupt { line: 41, .. }),
            "{error:?}"
        );
        assert!(
            bytes.load(std::sync::atomic::Ordering::SeqCst) < 1024,
            "the seek read read {} bytes, not the {prefix}-byte prefix",
            bytes.load(std::sync::atomic::Ordering::SeqCst)
        );
        // The legacy cursor shape computes the same line only on the error.
        let error = read_new_collection_events(&collection, prefix).expect_err("corrupt tail");
        assert!(matches!(&error, PaymentError::Corrupt { line: 41, .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_one_gigabyte_prefix_reads_only_the_tail() {
        let dir = std::env::temp_dir().join(format!("apex-payments-gib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let collection = dir.join("collection.jsonl");
        const PREFIX: u64 = 1_000_000_000;
        let line = serde_json::to_string(&event("AUTHORISATION", "true", "psp_tail", 300)).unwrap();
        {
            let mut file = std::fs::File::create(&collection).unwrap();
            file.set_len(PREFIX).unwrap();
            use std::io::{Seek, SeekFrom, Write};
            file.seek(SeekFrom::Start(PREFIX - 1)).unwrap();
            file.write_all(b"\n").unwrap();
            file.write_all(line.as_bytes()).unwrap();
            file.write_all(b"\n").unwrap();
            file.flush().unwrap();
        }
        let (mut reader, bytes) = counting_reader(&collection);
        let (rows, cursor) = read_new_collection_events_with_reader(
            &collection,
            &mut reader,
            PaymentCursor {
                offset: PREFIX,
                line: Some(500_000_000),
            },
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].psp_reference, "psp_tail");
        assert_eq!(cursor.offset, PREFIX + line.len() as u64 + 1);
        assert_eq!(cursor.line, Some(500_000_001));
        let read = bytes.load(std::sync::atomic::Ordering::SeqCst);
        eprintln!("1GB collection: read {read} bytes (prefix {PREFIX})");
        assert!(
            read < 16 * 1024,
            "a 1GB prefix must not be read; read {read} bytes"
        );
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
