//! The billing read model: fees from the signed records, payments from the
//! collection log.
//!
//! The console does not recompute financial truth. Every amount comes from a
//! signed record: the monthly fee lines are `apex_lift::records::BillingRecord`
//! payloads inside verified ledger entries, and the payment facts are the
//! collection events the Adyen webhook recorded (read-only). The design
//! document's financial language is enforced as states:
//!
//! - **Draft is not invoiced** (the current month's signed lines, no invoice
//!   record exists in this workspace),
//! - **Invoiced is not collected** (no invoice register exists here, so a
//!   past month without payment evidence reads "Not collected",
//!   never "paid"),
//! - **Pending is not paid** (partly collected weeks stay Pending with the
//!   balance due),
//! - **Credits, taxes and monitoring remain distinct** lines,
//! - a missing VAT rate is "Not available", never a fabricated tax.

use std::path::Path;

use apex_billing::CollectionEvent;
use apex_lift::records::LiftRecord;
use apex_proof::LedgerRecord;

use crate::error::ApiError;

/// VAT rate of the workspace in basis points (unset means no rate is
/// recorded; the invoice shows VAT as unavailable, never as zero).
pub const VAT_RATE_ENV: &str = "APEX_BILLING_VAT_RATE_BP";
/// Monitoring plan monthly fee in micros (unset means nothing is selected).
pub const MONITORING_FEE_ENV: &str = "APEX_MONITORING_FEE_MICROS";
/// Monitoring plan effective day (`YYYY-MM-DD`), optional.
pub const MONITORING_SINCE_ENV: &str = "APEX_MONITORING_SINCE";
/// Read cap for the collection log.
pub const MAX_COLLECTION_BYTES: u64 = 16 * 1024 * 1024;

/// One signed monthly fee line.
#[derive(Debug, Clone, PartialEq)]
pub struct BillingFeeLine {
    /// The change the fee is based on.
    pub change_id: String,
    /// The billing month `YYYY-MM`.
    pub month: String,
    /// The pre-registered scope's platform-reported revenue for the month.
    pub scope_revenue: i64,
    /// Billable lambda from the signed decision.
    pub lambda: f64,
    /// Combined factor over every active change.
    pub factor: f64,
    /// Attributed incremental revenue.
    pub incremental: i64,
    /// This change's log share of the combined factor.
    pub log_share: f64,
    /// Contract rate in basis points.
    pub rate_bp: u16,
    /// The fee before tax.
    pub fee_micros: i64,
    /// Whole days billable this month.
    pub billed_days: u32,
    /// Days in the month.
    pub month_days: u32,
    /// The signed record's ledger sequence (a proof reference).
    pub seq: u64,
    /// The signed record's payload digest (a proof reference).
    pub payload_sha256: String,
}

/// One month's invoice status, without pretending an invoice register exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvoiceStateView {
    /// The current month's signed lines; nothing invoiced or collected.
    Draft,
    /// Verified payments cover the signed total.
    Paid,
    /// Verified payments cover part of the signed total.
    Pending,
    /// A past month with no verified payment recorded.
    NotCollected,
}

impl InvoiceStateView {
    /// The state label.
    pub fn label(&self) -> &'static str {
        match self {
            InvoiceStateView::Draft => "Preview",
            InvoiceStateView::Paid => "Paid",
            InvoiceStateView::Pending => "Pending",
            InvoiceStateView::NotCollected => "Not collected",
        }
    }

    /// The invoice column the register shows next to the period.
    pub fn invoice_column(&self, period: &str) -> String {
        match self {
            InvoiceStateView::Draft => format!("Draft {period}"),
            _ => "Not available (no invoice record in this workspace)".to_string(),
        }
    }

    /// One factual note per state (the design document's financial language).
    pub fn note(&self) -> &'static str {
        match self {
            InvoiceStateView::Draft => "Invoice is still a draft.",
            InvoiceStateView::Paid => "Verified payment covers this period.",
            InvoiceStateView::Pending => "Pending is not paid; the balance is due.",
            InvoiceStateView::NotCollected => "No verified payment is recorded for this period.",
        }
    }
}

/// One month of the register.
#[derive(Debug, Clone, PartialEq)]
pub struct InvoiceRegisterRow {
    /// The billing month `YYYY-MM`.
    pub period: String,
    /// The state.
    pub state: InvoiceStateView,
    /// The signed total before tax.
    pub total_micros: i64,
    /// Verified collected amount for the period.
    pub collected_micros: i64,
    /// Whether this is the newest (current) period.
    pub is_current: bool,
}

/// The monitoring plan state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitoringView {
    /// The monthly fee, when a plan is selected.
    pub fee_micros: Option<i64>,
    /// The effective day, when one is recorded.
    pub since: Option<String>,
}

impl MonitoringView {
    /// The frequency label: whether a monitoring plan is selected.
    pub fn selected_label(&self) -> &'static str {
        match self.fee_micros {
            Some(_) => "Selected",
            None => "Not selected",
        }
    }
}

/// One recorded billing change (override, re-check, term end).
#[derive(Debug, Clone, PartialEq)]
pub struct BillingChangeView {
    /// The change the billing change belongs to.
    pub change_id: String,
    /// The day it took effect.
    pub day: String,
    /// The recorded reason.
    pub reason: String,
    /// The billed lift after the change (0 when billing stopped).
    pub billed_lambda: f64,
    /// The signed record's ledger sequence.
    pub seq: u64,
}

/// The whole billing read model.
#[derive(Debug, Clone, PartialEq)]
pub struct BillingView {
    /// The display currency.
    pub currency: String,
    /// The current billing month at the read's as-of time.
    pub current_month: String,
    /// Signed fee lines per month (newest first).
    pub fee_lines: Vec<BillingFeeLine>,
    /// Recorded billing changes (overrides, re-checks, term ends).
    pub billing_changes: Vec<BillingChangeView>,
    /// The monitoring plan.
    pub monitoring: MonitoringView,
    /// Collection events (newest first, every recorded event).
    pub payments: Vec<CollectionEvent>,
    /// The register, one row per month with signed fees (newest first).
    pub register: Vec<InvoiceRegisterRow>,
}

impl BillingView {
    /// The fee lines of one month.
    pub fn lines_for(&self, period: &str) -> Vec<&BillingFeeLine> {
        self.fee_lines
            .iter()
            .filter(|line| line.month == period)
            .collect()
    }

    /// The signed total of one month.
    pub fn total_for(&self, period: &str) -> i64 {
        self.lines_for(period)
            .iter()
            .map(|line| line.fee_micros)
            .sum()
    }

    /// The register row of one period, when present.
    pub fn register_row(&self, period: &str) -> Option<&InvoiceRegisterRow> {
        self.register.iter().find(|row| row.period == period)
    }

    /// Collection events whose merchant reference names the period.
    pub fn payments_for(&self, period: &str) -> Vec<&CollectionEvent> {
        self.payments
            .iter()
            .filter(|event| {
                merchant_reference_period(&event.merchant_reference).as_deref() == Some(period)
            })
            .collect()
    }

    /// The successfully collected amount for a period.
    pub fn collected_for(&self, period: &str) -> i64 {
        self.payments_for(period)
            .iter()
            .filter(|event| event.success.eq_ignore_ascii_case("true"))
            .map(|event| event.amount_value.saturating_mul(10_000))
            .sum()
    }

    /// The balance due for a period: signed total minus verified collections.
    pub fn balance_due_for(&self, period: &str) -> i64 {
        self.total_for(period)
            .saturating_sub(self.collected_for(period))
    }

    /// Validate a period id (`YYYY-MM`) the invoices route accepts.
    pub fn has_period(&self, period: &str) -> bool {
        self.fee_lines.iter().any(|line| line.month == period)
    }
}

/// The period a payment link's merchant reference names, when one does.
/// The deterministic reference is `apex-fee:<publisher>:<experiment>:<period>`.
pub fn merchant_reference_period(reference: &str) -> Option<String> {
    let candidate = reference.rsplit(':').next()?;
    if is_period(candidate) {
        Some(candidate.to_string())
    } else {
        None
    }
}

/// `YYYY-MM` with a plausible month and a four-digit year.
pub fn is_period(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 7
        && bytes[4] == b'-'
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && matches!(
            &bytes[5..],
            [b'0', b'1']
                | [b'0', b'2']
                | [b'0', b'3']
                | [b'0', b'4']
                | [b'0', b'5']
                | [b'0', b'6']
                | [b'0', b'7']
                | [b'0', b'8']
                | [b'0', b'9']
                | [b'1', b'0']
                | [b'1', b'1']
                | [b'1', b'2']
        )
}

/// Build the read model from verified records and recorded payments.
pub fn build(
    records: &[LedgerRecord],
    payments: Vec<CollectionEvent>,
    now_unix_ms: i64,
    monitoring: MonitoringView,
    currency: &str,
) -> BillingView {
    let mut fee_lines: Vec<BillingFeeLine> = Vec::new();
    let mut billing_changes: Vec<BillingChangeView> = Vec::new();
    for record in records {
        if let LedgerRecord::LiftRecord {
            seq,
            payload,
            payload_sha256,
            ..
        } = record
        {
            match serde_json::from_str::<LiftRecord>(payload) {
                Ok(LiftRecord::MonthlyBilling(billing)) => {
                    fee_lines.push(BillingFeeLine {
                        change_id: billing.change_id,
                        month: billing.month,
                        scope_revenue: billing.scope_revenue,
                        lambda: billing.lambda,
                        factor: billing.factor,
                        incremental: billing.incremental,
                        log_share: billing.log_share,
                        rate_bp: billing.rate_bp,
                        fee_micros: billing.fee_micros,
                        billed_days: billing.billed_days,
                        month_days: billing.month_days,
                        seq: *seq,
                        payload_sha256: payload_sha256.clone(),
                    });
                }
                Ok(LiftRecord::BillingChange(change)) => {
                    billing_changes.push(BillingChangeView {
                        change_id: change.change_id,
                        day: change.day,
                        reason: change.reason,
                        billed_lambda: change.billed_lambda,
                        seq: *seq,
                    });
                }
                _ => {}
            }
        }
    }
    fee_lines.sort_by(|a, b| {
        b.month
            .cmp(&a.month)
            .then_with(|| a.change_id.cmp(&b.change_id))
    });
    billing_changes.sort_by(|a, b| b.day.cmp(&a.day).then_with(|| a.seq.cmp(&b.seq)));

    let mut payments = payments;
    payments.sort_by(|a, b| {
        b.at_unix_ms
            .cmp(&a.at_unix_ms)
            .then_with(|| a.psp_reference.cmp(&b.psp_reference))
    });

    let current_month =
        apex_analytics::UtcMillis::from_unix_millis(now_unix_ms).to_day_string()[..7].to_string();
    let mut months: Vec<String> = fee_lines.iter().map(|line| line.month.clone()).collect();
    months.sort();
    months.dedup();
    months.reverse();
    let register: Vec<InvoiceRegisterRow> = months
        .iter()
        .map(|period| {
            let total: i64 = fee_lines
                .iter()
                .filter(|line| &line.month == period)
                .map(|line| line.fee_micros)
                .sum();
            let collected: i64 = payments
                .iter()
                .filter(|event| {
                    merchant_reference_period(&event.merchant_reference).as_deref()
                        == Some(period.as_str())
                })
                .filter(|event| event.success.eq_ignore_ascii_case("true"))
                .map(|event| event.amount_value.saturating_mul(10_000))
                .sum();
            let state = if period == &current_month {
                InvoiceStateView::Draft
            } else if collected >= total && total > 0 {
                InvoiceStateView::Paid
            } else if collected > 0 {
                InvoiceStateView::Pending
            } else {
                InvoiceStateView::NotCollected
            };
            InvoiceRegisterRow {
                period: period.clone(),
                state,
                total_micros: total,
                collected_micros: collected,
                is_current: period == &current_month,
            }
        })
        .collect();

    BillingView {
        currency: currency.to_string(),
        current_month,
        fee_lines,
        billing_changes,
        monitoring,
        payments,
        register,
    }
}

/// Read the collection log the Adyen webhook records into
/// (`APEX_COLLECTION_FILE`), read-only. A missing file is an empty log; a
/// malformed line is a logged internal error, never a fabricated payment.
pub fn read_collection_log(path: &Path) -> Result<Vec<CollectionEvent>, ApiError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(ApiError::Internal(format!("collection log stat: {error}"))),
    };
    crate::sources::ensure_within_cap(metadata.len(), MAX_COLLECTION_BYTES)?;
    let raw = std::fs::read_to_string(path)
        .map_err(|error| ApiError::Internal(format!("collection log read: {error}")))?;
    let mut events = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event: CollectionEvent = serde_json::from_str(line).map_err(|error| {
            ApiError::Internal(format!(
                "collection log parse at line {}: {error}",
                index + 1
            ))
        })?;
        events.push(event);
    }
    Ok(events)
}

/// The workspace VAT rate from the environment, when one is recorded.
pub fn vat_rate_from_env(lookup: &dyn Fn(&str) -> Option<String>) -> Option<u16> {
    lookup(VAT_RATE_ENV)
        .and_then(|raw| raw.trim().parse::<u16>().ok())
        .filter(|rate| *rate <= 10_000)
}

/// The monitoring plan from the environment, when one is selected.
pub fn monitoring_from_env(lookup: &dyn Fn(&str) -> Option<String>) -> MonitoringView {
    let fee = lookup(MONITORING_FEE_ENV)
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .filter(|fee| *fee > 0);
    MonitoringView {
        fee_micros: fee,
        since: lookup(MONITORING_SINCE_ENV).filter(|value| !value.trim().is_empty()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use sha2::{Digest, Sha256};

    use super::*;

    fn billing_record(change: &str, month: &str, fee: i64) -> LedgerRecord {
        let payload = format!(
            r#"{{"type":"monthlyBilling","changeId":"{change}","month":"{month}","scopeRevenue":51500000000,"lambda":0.03,"factor":1.03,"incremental":1500000000,"logShare":1.0,"rateBp":2000,"feeMicros":{fee},"billedDays":31,"monthDays":31}}"#
        );
        let digest = hex::encode(Sha256::digest(payload.as_bytes()));
        LedgerRecord::lift_record(1_792_055_400_000, &digest, &payload)
    }

    fn payment(period: &str, amount_minor: i64, success: &str, psp: &str) -> CollectionEvent {
        CollectionEvent {
            at_unix_ms: 1_792_055_400_000,
            event_code: "AUTHORISATION".to_string(),
            psp_reference: psp.to_string(),
            merchant_reference: format!("apex-fee:northstar:{period}"),
            success: success.to_string(),
            amount_value: amount_minor,
            amount_currency: "USD".to_string(),
        }
    }

    fn sealed(records: Vec<LedgerRecord>) -> Vec<LedgerRecord> {
        let mut ledger =
            apex_proof::InMemoryLedger::new(apex_proof::Keypair::from_bytes(&[3u8; 32]));
        records
            .into_iter()
            .map(|draft| ledger.append(draft).expect("seal"))
            .collect()
    }

    #[test]
    fn the_register_separates_draft_paid_pending_and_not_collected() {
        let records = sealed(vec![
            billing_record("ch_0247", "2026-10", 300_000_000),
            billing_record("ch_0246", "2026-09", 225_000_000),
            billing_record("ch_0242", "2026-08", 100_000_000),
            billing_record("ch_0239", "2026-07", 50_000_000),
        ]);
        let payments = vec![
            payment("2026-09", 22_500, "true", "psp-1"),
            payment("2026-08", 4_000, "true", "psp-2"),
        ];
        let view = build(
            &records,
            payments,
            1_792_055_400_000,
            MonitoringView {
                fee_micros: None,
                since: None,
            },
            "USD",
        );
        assert_eq!(view.current_month, "2026-10");
        assert_eq!(view.total_for("2026-10"), 300_000_000);
        assert_eq!(view.register.len(), 4);
        assert_eq!(
            view.register_row("2026-10").expect("row").state,
            InvoiceStateView::Draft
        );
        assert_eq!(
            view.register_row("2026-09").expect("row").state,
            InvoiceStateView::Paid
        );
        assert_eq!(
            view.register_row("2026-08").expect("row").state,
            InvoiceStateView::Pending
        );
        assert_eq!(view.balance_due_for("2026-08"), 60_000_000);
        assert_eq!(
            view.register_row("2026-07").expect("row").state,
            InvoiceStateView::NotCollected
        );
        assert!(view.has_period("2026-09"));
        assert!(!view.has_period("2025-01"));
        assert_eq!(view.lines_for("2026-09").len(), 1);
        assert_eq!(view.lines_for("2026-09")[0].change_id, "ch_0246");
    }

    #[test]
    fn failed_payments_do_not_count_as_collected() {
        let records = sealed(vec![billing_record("ch_1", "2026-09", 100_000_000)]);
        let view = build(
            &records,
            vec![payment("2026-09", 10_000, "false", "psp-1")],
            1_792_055_400_000,
            MonitoringView {
                fee_micros: None,
                since: None,
            },
            "USD",
        );
        assert_eq!(view.collected_for("2026-09"), 0);
        assert_eq!(
            view.register_row("2026-09").expect("row").state,
            InvoiceStateView::NotCollected
        );
        assert_eq!(view.payments_for("2026-09").len(), 1, "the event is shown");
    }

    #[test]
    fn merchant_reference_periods_are_parsed_strictly() {
        assert_eq!(
            merchant_reference_period("apex-fee:northstar:exp_0247:2026-10").as_deref(),
            Some("2026-10")
        );
        assert_eq!(
            merchant_reference_period("apex-fee:northstar:exp_0247"),
            None
        );
        assert_eq!(merchant_reference_period("2026-13"), None);
        assert_eq!(merchant_reference_period(""), None);
        assert!(is_period("2026-01"));
        assert!(is_period("2026-12"));
        assert!(!is_period("2026-00"));
        assert!(!is_period("26-10"));
    }

    #[test]
    fn empty_records_yield_an_empty_register_never_a_zero_row() {
        let view = build(
            &[],
            vec![],
            1_792_055_400_000,
            MonitoringView {
                fee_micros: None,
                since: None,
            },
            "USD",
        );
        assert!(view.register.is_empty());
        assert!(view.fee_lines.is_empty());
        assert_eq!(view.monitoring.selected_label(), "Not selected");
        assert_eq!(view.total_for("2026-10"), 0);
    }

    #[test]
    fn the_collection_log_roundtrips_and_missing_is_empty() {
        let dir = std::env::temp_dir().join(format!("apex-billing-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(read_collection_log(&dir.join("missing.jsonl"))
            .expect("missing")
            .is_empty());
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("collection.jsonl");
        let event = payment("2026-09", 22_500, "true", "psp-1");
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string(&event).expect("json")),
        )
        .expect("write");
        let events = read_collection_log(&path).expect("read");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].psp_reference, "psp-1");
        std::fs::write(&path, "{}\n").expect("write");
        assert!(matches!(
            read_collection_log(&path),
            Err(ApiError::Internal(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vat_and_monitoring_are_read_from_explicit_configuration() {
        let lookup = |key: &str| match key {
            VAT_RATE_ENV => Some("2000".to_string()),
            MONITORING_FEE_ENV => Some("500000000".to_string()),
            MONITORING_SINCE_ENV => Some("2026-05-01".to_string()),
            _ => None,
        };
        assert_eq!(vat_rate_from_env(&lookup), Some(2_000));
        let monitoring = monitoring_from_env(&lookup);
        assert_eq!(monitoring.fee_micros, Some(500_000_000));
        assert_eq!(monitoring.since.as_deref(), Some("2026-05-01"));
        assert_eq!(monitoring.selected_label(), "Selected");
        assert_eq!(vat_rate_from_env(&|_| Some("99999".to_string())), None);
        assert_eq!(vat_rate_from_env(&|_| None), None);
        let none = monitoring_from_env(&|_| None);
        assert_eq!(none.selected_label(), "Not selected");
    }

    #[test]
    fn invoice_state_columns_and_notes_follow_the_document() {
        assert_eq!(
            InvoiceStateView::Draft.invoice_column("2026-10"),
            "Draft 2026-10"
        );
        assert_eq!(InvoiceStateView::Draft.note(), "Invoice is still a draft.");
        assert_eq!(
            InvoiceStateView::Paid.invoice_column("2026-09"),
            "Not available (no invoice record in this workspace)"
        );
        assert_eq!(
            InvoiceStateView::Pending.note(),
            "Pending is not paid; the balance is due."
        );
        assert!(InvoiceStateView::NotCollected
            .note()
            .contains("No verified payment is recorded"));
    }
}
