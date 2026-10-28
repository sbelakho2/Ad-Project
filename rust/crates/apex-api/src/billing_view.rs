//! The billing read model: fees from the signed records, payments from the
//! collection log.
//!
//! The console does not recompute financial truth. Every amount comes from a
//! signed record: the monthly fee lines are `apex_lift::records::BillingRecord`
//! payloads inside verified ledger entries, and the payment facts are the
//! collection events the Mollie webhook recorded (read-only). The design
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

use std::collections::HashMap;
use std::path::Path;

use apex_billing::CollectionEvent;
use apex_lift::records::LiftRecord;
use apex_proof::LedgerRecord;

use crate::error::ApiError;

/// VAT rate of the workspace in basis points (unset means no rate is
/// recorded; the invoice shows VAT as unavailable, never as zero).
pub const VAT_RATE_ENV: &str = "APEX_BILLING_VAT_RATE_BP";
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
    /// Why this month billed nothing, when it did: the same typed reason the
    /// signed holdout line carries.
    pub no_fee_reason: Option<String>,
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
    /// Verified collected amount for the period, same currency only.
    pub collected_micros: i64,
    /// Successful receipts in another currency for this period. Their
    /// amounts are never summed with the balance currency; the page renders
    /// them as an explicit exclusion note.
    pub excluded_foreign_receipts: usize,
    /// Whether this is the newest (current) period.
    pub is_current: bool,
}

/// The managed ad-ops plan state, from the publisher's recorded plan state
/// file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedView {
    /// Whether the plan is recorded as on.
    pub active: bool,
    /// The recorded monthly fee in micros.
    pub fee_micros: Option<i64>,
    /// The recorded effective day (`YYYY-MM-DD`).
    pub since: Option<String>,
    /// Cancelled plans: the last period (`YYYY-MM`) the plan bills.
    pub stops_after: Option<String>,
}

impl ManagedView {
    /// A plan with nothing recorded.
    pub fn not_recorded() -> Self {
        Self {
            active: false,
            fee_micros: None,
            since: None,
            stops_after: None,
        }
    }

    /// The view of one recorded plan state.
    pub fn from_state(state: &apex_billing::PlanState) -> Self {
        Self {
            active: state.active,
            fee_micros: Some(state.monthly_fee_micros),
            since: (!state.since.trim().is_empty()).then(|| state.since.clone()),
            stops_after: state.stops_after.clone(),
        }
    }

    /// The plan label: active, cancelled, or not recorded.
    pub fn selected_label(&self) -> &'static str {
        if !self.active {
            "Not recorded"
        } else if self.stops_after.is_some() {
            "Cancelled"
        } else {
            "Active"
        }
    }

    /// Whether a cancellation is recorded.
    pub fn cancelled(&self) -> bool {
        self.stops_after.is_some()
    }
}

/// Read one publisher's plan state file (`<root>/<publisher>.<slug>.json`).
/// No connections root or no file means nothing is recorded; a malformed
/// file is a typed refusal, never a fabricated plan.
pub fn plan_state_view(
    root: Option<&Path>,
    publisher: &str,
    plan: apex_billing::PlanId,
) -> Result<Option<apex_billing::PlanState>, ApiError> {
    let Some(root) = root else {
        return Ok(None);
    };
    let path = apex_billing::PlanState::path_for(root, publisher, plan);
    apex_billing::PlanState::load(&path)
        .map_err(|error| ApiError::Internal(format!("plan state: {error}")))
}

/// The managed ad-ops plan view for one publisher.
pub fn managed_state_view(root: Option<&Path>, publisher: &str) -> Result<ManagedView, ApiError> {
    Ok(
        match plan_state_view(root, publisher, apex_billing::PlanId::Managed)? {
            Some(state) => ManagedView::from_state(&state),
            None => ManagedView::not_recorded(),
        },
    )
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
            .filter(|event| reference_period(&event.reference).as_deref() == Some(period))
            .collect()
    }

    /// The successfully collected amount for a period. Only receipts in the
    /// invoice currency count (case-insensitive code compare): a foreign
    /// currency amount is never summed into this balance.
    pub fn collected_for(&self, period: &str) -> i64 {
        self.payments_for(period)
            .iter()
            .filter(|event| event.status == "paid")
            .filter(|event| event.amount_currency.eq_ignore_ascii_case(&self.currency))
            .map(|event| event.amount_minor_units.saturating_mul(10_000))
            .sum()
    }

    /// How many successful receipts for a period are in another currency
    /// than the invoice. They are excluded from [`Self::collected_for`] and
    /// shown as the explicit exclusion note.
    pub fn excluded_foreign_for(&self, period: &str) -> usize {
        self.payments_for(period)
            .iter()
            .filter(|event| event.status == "paid")
            .filter(|event| !event.amount_currency.eq_ignore_ascii_case(&self.currency))
            .count()
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
pub fn reference_period(reference: &str) -> Option<String> {
    let candidate = reference.rsplit(':').next()?;
    if is_period(candidate) {
        Some(candidate.to_string())
    } else {
        None
    }
}

/// The honest note for successful receipts in another currency that were
/// excluded from a balance, when any exist. The count is stated; the amounts
/// are never summed across currencies.
pub fn excluded_foreign_note(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some("1 receipt in another currency is excluded from this balance.".to_string()),
        count => Some(format!(
            "{count} receipts in another currency are excluded from this balance."
        )),
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
                        no_fee_reason: billing
                            .holdout
                            .as_ref()
                            .and_then(|holdout| holdout.no_fee_reason.clone()),
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
            .then_with(|| a.payment_id.cmp(&b.payment_id))
    });

    let current_month =
        apex_analytics::UtcMillis::from_unix_millis(now_unix_ms).to_day_string()[..7].to_string();
    let mut months: Vec<String> = fee_lines.iter().map(|line| line.month.clone()).collect();
    months.sort();
    months.dedup();
    months.reverse();
    // One pass over the fee lines and one over the payments, grouped by
    // period: per-month scans of both lists would grow with the register
    // squared in a ledger with many periods.
    let mut total_by_month: HashMap<&str, i64> = HashMap::new();
    for line in &fee_lines {
        *total_by_month.entry(line.month.as_str()).or_default() += line.fee_micros;
    }
    let mut collected_by_month: HashMap<String, i64> = HashMap::new();
    let mut excluded_by_month: HashMap<String, usize> = HashMap::new();
    for event in &payments {
        let Some(period) = reference_period(&event.reference) else {
            continue;
        };
        if event.status != "paid" {
            continue;
        }
        if event.amount_currency.eq_ignore_ascii_case(currency) {
            *collected_by_month.entry(period).or_default() +=
                event.amount_minor_units.saturating_mul(10_000);
        } else {
            *excluded_by_month.entry(period).or_default() += 1;
        }
    }
    let register: Vec<InvoiceRegisterRow> = months
        .iter()
        .map(|period| {
            let total = total_by_month.get(period.as_str()).copied().unwrap_or(0);
            let collected = collected_by_month.get(period).copied().unwrap_or(0);
            let excluded_foreign_receipts = excluded_by_month.get(period).copied().unwrap_or(0);
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
                excluded_foreign_receipts,
                is_current: period == &current_month,
            }
        })
        .collect();

    BillingView {
        currency: currency.to_string(),
        current_month,
        fee_lines,
        billing_changes,
        payments,
        register,
    }
}

/// Read the collection log the Mollie webhook records into
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

    fn payment(period: &str, amount_minor: i64, status: &str, payment_id: &str) -> CollectionEvent {
        payment_in(period, amount_minor, status, payment_id, "USD")
    }

    fn payment_in(
        period: &str,
        amount_minor: i64,
        status: &str,
        payment_id: &str,
        currency: &str,
    ) -> CollectionEvent {
        CollectionEvent {
            at_unix_ms: 1_792_055_400_000,
            payment_id: payment_id.to_string(),
            reference: format!("apex-fee:northstar:{period}"),
            status: match status {
                "true" => "paid".to_string(),
                "false" => "failed".to_string(),
                other => other.to_string(),
            },
            amount_minor_units: amount_minor,
            amount_currency: currency.to_string(),
            stored_customer_id: String::new(),
            stored_mandate_id: String::new(),
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
        let view = build(&records, payments, 1_792_055_400_000, "USD");
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
    fn a_foreign_currency_receipt_never_pays_a_usd_invoice() {
        let records = sealed(vec![billing_record("ch_0246", "2026-09", 100_000_000)]);
        let view = build(
            &records,
            vec![payment_in("2026-09", 300, "true", "psp-eur", "EUR")],
            1_792_055_400_000,
            "USD",
        );
        assert_eq!(view.collected_for("2026-09"), 0);
        assert_eq!(view.balance_due_for("2026-09"), 100_000_000);
        let row = view.register_row("2026-09").expect("row");
        assert_eq!(row.state, InvoiceStateView::NotCollected);
        assert_eq!(row.collected_micros, 0);
        assert_eq!(row.excluded_foreign_receipts, 1);
        assert_eq!(view.excluded_foreign_for("2026-09"), 1);
        assert_eq!(
            excluded_foreign_note(row.excluded_foreign_receipts).as_deref(),
            Some("1 receipt in another currency is excluded from this balance.")
        );
        assert_eq!(
            view.payments_for("2026-09").len(),
            1,
            "the excluded receipt is still shown"
        );
    }

    #[test]
    fn same_currency_receipts_count_case_insensitively_and_alone() {
        let records = sealed(vec![billing_record("ch_1", "2026-09", 100_000_000)]);
        let view = build(
            &records,
            vec![
                payment_in("2026-09", 300, "true", "psp-eur", "EUR"),
                payment_in("2026-09", 10_000, "true", "psp-usd", "usd"),
            ],
            1_792_055_400_000,
            "USD",
        );
        assert_eq!(
            view.collected_for("2026-09"),
            100_000_000,
            "the lowercase usd receipt counts; the EUR amount is never summed in"
        );
        assert_eq!(view.balance_due_for("2026-09"), 0);
        let row = view.register_row("2026-09").expect("row");
        assert_eq!(row.state, InvoiceStateView::Paid);
        assert_eq!(row.excluded_foreign_receipts, 1);
        assert_eq!(excluded_foreign_note(0), None);
        assert_eq!(
            excluded_foreign_note(2).as_deref(),
            Some("2 receipts in another currency are excluded from this balance.")
        );
    }

    #[test]
    fn reference_periods_are_parsed_strictly() {
        assert_eq!(
            reference_period("apex-fee:northstar:exp_0247:2026-10").as_deref(),
            Some("2026-10")
        );
        assert_eq!(reference_period("apex-fee:northstar:exp_0247"), None);
        assert_eq!(reference_period("2026-13"), None);
        assert_eq!(reference_period(""), None);
        assert!(is_period("2026-01"));
        assert!(is_period("2026-12"));
        assert!(!is_period("2026-00"));
        assert!(!is_period("26-10"));
    }

    #[test]
    fn empty_records_yield_an_empty_register_never_a_zero_row() {
        let view = build(&[], vec![], 1_792_055_400_000, "USD");
        assert!(view.register.is_empty());
        assert!(view.fee_lines.is_empty());
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
        assert_eq!(events[0].payment_id, "psp-1");
        std::fs::write(&path, "{}\n").expect("write");
        assert!(matches!(
            read_collection_log(&path),
            Err(ApiError::Internal(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vat_comes_from_configuration_and_plans_from_recorded_state() {
        let lookup = |key: &str| match key {
            VAT_RATE_ENV => Some("2000".to_string()),
            _ => None,
        };
        assert_eq!(vat_rate_from_env(&lookup), Some(2_000));
        assert_eq!(vat_rate_from_env(&|_| Some("99999".to_string())), None);
        assert_eq!(vat_rate_from_env(&|_| None), None);

        // The managed view reads its own recorded state (or nothing).
        let dir = std::env::temp_dir().join(format!("apex-billing-managed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let none = managed_state_view(Some(&dir), "northstar").expect("view");
        assert_eq!(none.selected_label(), "Not recorded");
        let managed =
            apex_billing::ManagedPlan::default().activate(900_000 * 1_000_000, "2026-05-01");
        managed
            .save(&apex_billing::PlanState::path_for(
                &dir,
                "northstar",
                apex_billing::PlanId::Managed,
            ))
            .expect("save");
        let view = managed_state_view(Some(&dir), "northstar").expect("view");
        assert_eq!(view.selected_label(), "Active");
        assert_eq!(view.fee_micros, Some(2_500_000_000));
        let _ = std::fs::remove_dir_all(&dir);
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

    /// The pre-fix register build: per-month scans of the whole fee-line and
    /// payment lists. Kept verbatim as the naive reference.
    fn register_naive(
        fee_lines: &[BillingFeeLine],
        payments: &[CollectionEvent],
        currency: &str,
        now_unix_ms: i64,
    ) -> Vec<InvoiceRegisterRow> {
        let current_month = apex_analytics::UtcMillis::from_unix_millis(now_unix_ms)
            .to_day_string()[..7]
            .to_string();
        let mut months: Vec<String> = fee_lines.iter().map(|line| line.month.clone()).collect();
        months.sort();
        months.dedup();
        months.reverse();
        months
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
                        reference_period(&event.reference).as_deref() == Some(period.as_str())
                    })
                    .filter(|event| event.status == "paid")
                    .filter(|event| event.amount_currency.eq_ignore_ascii_case(currency))
                    .map(|event| event.amount_minor_units.saturating_mul(10_000))
                    .sum();
                let excluded_foreign_receipts = payments
                    .iter()
                    .filter(|event| {
                        reference_period(&event.reference).as_deref() == Some(period.as_str())
                    })
                    .filter(|event| event.status == "paid")
                    .filter(|event| !event.amount_currency.eq_ignore_ascii_case(currency))
                    .count();
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
                    excluded_foreign_receipts,
                    is_current: period == &current_month,
                }
            })
            .collect()
    }

    #[test]
    fn the_register_agrees_with_the_naive_reference_at_ten_thousand_records() {
        // 8_000 billing lines over 120 distinct months, 2_000 billing-change
        // records, and 10_000 payment events (9_000 same-currency, 1_000
        // foreign, a tenth unsuccessful).
        let mut records: Vec<LedgerRecord> = Vec::with_capacity(10_000);
        for i in 0..8_000 {
            let month = format!("20{:02}-{:02}", 10 + (i / 12) % 10, i % 12 + 1);
            records.push(billing_record(
                &format!("ch_{i:05}"),
                &month,
                1_000_000 + i as i64,
            ));
        }
        for i in 0..2_000 {
            let payload = format!(
                r#"{{"type":"billingChange","changeId":"ch_{i:05}","day":"2026-10-01","reason":"publisher override","billedLambda":0.0}}"#
            );
            let digest = hex::encode(Sha256::digest(payload.as_bytes()));
            records.push(LedgerRecord::lift_record(
                1_792_055_400_000,
                &digest,
                &payload,
            ));
        }
        let payments: Vec<CollectionEvent> = (0..10_000)
            .map(|i| {
                let period = format!("20{:02}-{:02}", 10 + (i / 12) % 10, i % 12 + 1);
                payment_in(
                    &period,
                    3_000,
                    if i % 10 == 9 { "false" } else { "true" },
                    &format!("psp-{i}"),
                    if i % 10 == 5 { "EUR" } else { "USD" },
                )
            })
            .collect();

        let view = build(&records, payments, 1_792_055_400_000, "USD");
        assert_eq!(view.fee_lines.len(), 8_000);
        assert_eq!(view.payments.len(), 10_000);
        let mut months: Vec<&str> = view.fee_lines.iter().map(|l| l.month.as_str()).collect();
        months.sort();
        months.dedup();
        assert_eq!(months.len(), 120);
        assert_eq!(
            view.register,
            register_naive(&view.fee_lines, &view.payments, "USD", 1_792_055_400_000),
            "the grouped register must agree with the naive per-month scan"
        );
        // The register really carries the grouping work (not a vacuous
        // comparison): one row per month, and collections split across
        // several states.
        assert_eq!(view.register.len(), 120);
        assert!(view
            .register
            .iter()
            .any(|row| row.state == InvoiceStateView::NotCollected));
        assert!(view
            .register
            .iter()
            .any(|row| row.excluded_foreign_receipts > 0));
    }
}
