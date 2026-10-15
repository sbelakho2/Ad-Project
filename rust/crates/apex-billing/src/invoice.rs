//! Invoice construction from signed billing proofs.
//!
//! The billing model is lift-based: the publisher is charged 15%..20% of the
//! *platform-proven* increment, monthly. A proof only becomes a line item
//! after it is re-verified:
//!
//! 1. the proof timestamp must fall inside `[period.start, period.end)`;
//! 2. the proof publisher must be the invoiced publisher;
//! 3. `proven_lift > 0`, `rate_bp` inside `[MIN_LIFT_RATE_BP,
//!    MAX_LIFT_RATE_BP]` and the stored fee must equal
//!    [`apex_core::lift_fee_micros`] of those two values — the stored fee is
//!    never trusted;
//! 4. neither the ledger `seq` nor the `experiment_id` may repeat, so the same
//!    experiment can never be billed twice.
//!
//! Only then is the fee (recomputed, not stored) summed with `checked_add` and
//! the invoice returned with its lines ordered by `seq`.

use std::collections::HashSet;

use apex_core::{lift_fee_micros, Micros, MAX_LIFT_RATE_BP, MIN_LIFT_RATE_BP};
use apex_proof::LedgerRecord;
use serde::{Deserialize, Serialize};

use crate::errors::BillingError;

/// The billing window an invoice covers. `year_month` is the human-facing
/// `YYYY-MM` label; `start_unix_ms` is inclusive and `end_unix_ms` exclusive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingPeriod {
    pub year_month: String,
    pub start_unix_ms: i64,
    pub end_unix_ms: i64,
}

/// A `BillingProof` ledger record flattened for billing. Build it from a
/// sealed ledger record with [`BillingProofRecord::from_ledger_record`]; the
/// fields mirror [`LedgerRecord::BillingProof`] one-to-one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingProofRecord {
    pub seq: u64,
    pub at_unix_ms: i64,
    pub publisher_id: String,
    pub experiment_id: String,
    pub proven_lift: Micros,
    pub rate_bp: u16,
    pub fee_micros: Micros,
}

impl BillingProofRecord {
    /// Flatten a [`LedgerRecord`] into a billing proof record. Any other
    /// ledger variant is refused: only billing proofs can be billed.
    pub fn from_ledger_record(record: &LedgerRecord) -> Result<Self, BillingError> {
        match record {
            LedgerRecord::BillingProof {
                seq,
                at_unix_ms,
                publisher_id,
                experiment_id,
                proven_lift,
                rate_bp,
                fee_micros,
                ..
            } => Ok(Self {
                seq: *seq,
                at_unix_ms: *at_unix_ms,
                publisher_id: publisher_id.clone(),
                experiment_id: experiment_id.clone(),
                proven_lift: *proven_lift,
                rate_bp: *rate_bp,
                fee_micros: *fee_micros,
            }),
            other => Err(BillingError::Malformed(format!(
                "ledger record seq {} is not a billingProof record",
                other.seq()
            ))),
        }
    }
}

/// One billable line: a re-verified proof with its recomputed net fee and the
/// VAT split applied to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceLine {
    pub seq: u64,
    pub experiment_id: String,
    pub proven_lift: Micros,
    pub rate_bp: u16,
    /// The net fee (the proven-lift rate). VAT is added on top.
    pub fee_micros: Micros,
    /// VAT on the net fee, at the invoice's rate.
    #[serde(default)]
    pub vat_micros: Micros,
    /// The gross amount collected: net + VAT.
    #[serde(default)]
    pub gross_micros: Micros,
}

/// A monthly invoice: the verified lines (ordered by `seq`) and their checked
/// total.
///
/// Zero-fee lines are kept when a positive proof truncates to a zero fee, but
/// [`crate::create_payment_links`] intentionally omits them from the receipt:
/// `receipt.items` can be shorter than `line_items`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invoice {
    pub publisher_id: String,
    pub period: BillingPeriod,
    pub line_items: Vec<InvoiceLine>,
    /// Net total (sum of the line fees).
    pub total_micros: Micros,
    /// VAT rate applied to every line, in basis points (0 = no VAT).
    #[serde(default)]
    pub vat_rate_bp: u16,
    /// Total VAT over the net.
    #[serde(default)]
    pub vat_micros: Micros,
    /// Gross total collected: net + VAT.
    #[serde(default)]
    pub gross_micros: Micros,
}

impl Invoice {
    /// Apply a VAT rate to the invoice: VAT is computed per line (round
    /// half-up to the micro) and added to the net to form the gross. A
    /// non-positive rate marks the invoice as not subject to VAT.
    #[must_use]
    pub fn with_vat(mut self, vat_rate_bp: u16) -> Self {
        self.vat_rate_bp = vat_rate_bp;
        let mut vat_total: Micros = 0;
        for line in &mut self.line_items {
            line.vat_micros = vat_micros(line.fee_micros, vat_rate_bp);
            line.gross_micros = line.fee_micros.saturating_add(line.vat_micros);
            vat_total = vat_total.saturating_add(line.vat_micros);
        }
        self.vat_micros = vat_total;
        self.gross_micros = self.total_micros.saturating_add(vat_total);
        self
    }

    /// Render the invoice as the human-facing VAT document Adyen's invoice
    /// template and the customer see.
    pub fn render_vat_invoice(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Apex invoice {} for {}\n",
            self.period.year_month, self.publisher_id
        ));
        for line in &self.line_items {
            out.push_str(&format!(
                "  #{} {}: net {} micros, VAT {} micros, gross {} micros\n",
                line.seq, line.experiment_id, line.fee_micros, line.vat_micros, line.gross_micros
            ));
        }
        out.push_str(&format!(
            "  net {} micros\n  VAT ({} bp) {} micros\n  gross {} micros\n",
            self.total_micros, self.vat_rate_bp, self.vat_micros, self.gross_micros
        ));
        out
    }
}

/// VAT on `net` at `rate_bp` basis points, rounded half-up to the micro.
fn vat_micros(net: Micros, rate_bp: u16) -> Micros {
    if net <= 0 || rate_bp == 0 {
        return 0;
    }
    (net.saturating_mul(i64::from(rate_bp)) + 5_000) / 10_000
}

/// Re-verify `proofs` and build the invoice for `publisher_id` in `period`.
///
/// An empty proof set is not an error: it yields a zero invoice (no lift, no
/// fee). Any offending proof refuses the whole invoice with a typed error —
/// there is no partial acceptance.
///
/// # Provenance precondition
///
/// This function re-verifies **arithmetic only**: period, publisher, positive
/// lift, rate band and the recomputed fee. It cannot verify the ledger chain —
/// a [`BillingProofRecord`] is freely constructible and its fee can be made
/// consistent with any lift/rate pair. Callers MUST pass records flattened
/// from a chain verified against the expected public key (for example via
/// [`apex_proof::Ledger::open`] or [`apex_proof::Ledger::verify`]); otherwise a
/// fabricated yet arithmetically consistent record would be billed.
pub fn compute_invoice(
    publisher_id: &str,
    period: BillingPeriod,
    proofs: &[BillingProofRecord],
) -> Result<Invoice, BillingError> {
    let mut seen_seqs: HashSet<u64> = HashSet::with_capacity(proofs.len());
    let mut seen_experiments: HashSet<&str> = HashSet::with_capacity(proofs.len());
    let mut line_items = Vec::with_capacity(proofs.len());

    for proof in proofs {
        if proof.at_unix_ms < period.start_unix_ms || proof.at_unix_ms >= period.end_unix_ms {
            return Err(BillingError::OutOfPeriod { seq: proof.seq });
        }
        if proof.publisher_id != publisher_id {
            return Err(BillingError::WrongPublisher { seq: proof.seq });
        }
        let fee_micros = verify_proof(proof)?;
        if !seen_seqs.insert(proof.seq) || !seen_experiments.insert(proof.experiment_id.as_str()) {
            return Err(BillingError::Duplicate {
                seq: proof.seq,
                experiment_id: proof.experiment_id.clone(),
            });
        }
        line_items.push(InvoiceLine {
            seq: proof.seq,
            experiment_id: proof.experiment_id.clone(),
            proven_lift: proof.proven_lift,
            rate_bp: proof.rate_bp,
            fee_micros,
            vat_micros: 0,
            gross_micros: fee_micros,
        });
    }

    line_items.sort_by_key(|line| line.seq);

    let mut total_micros: Micros = 0;
    for line in &line_items {
        total_micros = total_micros
            .checked_add(line.fee_micros)
            .ok_or(BillingError::Overflow)?;
    }

    Ok(Invoice {
        publisher_id: publisher_id.to_string(),
        period,
        line_items,
        total_micros,
        vat_rate_bp: 0,
        vat_micros: 0,
        gross_micros: total_micros,
    })
}

/// One month's fee basis, flattened from the signed `monthlyBilling` ledger
/// record. The fee re-derives from these fields alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BillingBasisRecord {
    /// The ledger sequence of the signed monthly-billing record.
    pub seq: u64,
    pub at_unix_ms: i64,
    pub change_id: String,
    /// The pre-registered scope's platform-reported revenue for the month.
    #[allow(dead_code)]
    pub scope_revenue: Micros,
    /// The scope's attributed incremental revenue for the month.
    pub incremental: Micros,
    /// This change's log share of the combined factor.
    pub log_share: f64,
    pub rate_bp: u16,
    pub fee_micros: Micros,
    /// Whole days the change was billable this month.
    pub billed_days: u32,
    /// Days in the month, for the pro-rata re-derivation.
    pub month_days: u32,
}

/// Build the invoice from the signed monthly-billing records.
///
/// Mirrors [`compute_invoice`]'s refusal order, but the fee re-derives from
/// the lambda-model basis — the exact expression the engine's monthly run
/// uses, so a re-derived fee and a signed fee cannot drift apart.
pub fn compute_invoice_from_basis(
    publisher_id: &str,
    period: BillingPeriod,
    records: &[BillingBasisRecord],
) -> Result<Invoice, BillingError> {
    let mut seen_seqs: HashSet<u64> = HashSet::with_capacity(records.len());
    let mut seen_changes: HashSet<&str> = HashSet::with_capacity(records.len());
    let mut line_items = Vec::with_capacity(records.len());

    for record in records {
        if record.at_unix_ms < period.start_unix_ms || record.at_unix_ms >= period.end_unix_ms {
            return Err(BillingError::OutOfPeriod { seq: record.seq });
        }
        if record.change_id.is_empty() {
            return Err(BillingError::TamperedProof {
                seq: record.seq,
                reason: "billing basis has no change id".into(),
            });
        }
        let fee_micros = verify_basis(record)?;
        if !seen_seqs.insert(record.seq) || !seen_changes.insert(record.change_id.as_str()) {
            return Err(BillingError::Duplicate {
                seq: record.seq,
                experiment_id: record.change_id.clone(),
            });
        }
        line_items.push(InvoiceLine {
            seq: record.seq,
            experiment_id: record.change_id.clone(),
            proven_lift: record.incremental,
            rate_bp: record.rate_bp,
            fee_micros,
            vat_micros: 0,
            gross_micros: fee_micros,
        });
    }

    line_items.sort_by_key(|line| line.seq);

    let mut total_micros: Micros = 0;
    for line in &line_items {
        total_micros = total_micros
            .checked_add(line.fee_micros)
            .ok_or(BillingError::Overflow)?;
    }

    Ok(Invoice {
        publisher_id: publisher_id.to_string(),
        period,
        line_items,
        total_micros,
        vat_rate_bp: 0,
        vat_micros: 0,
        gross_micros: total_micros,
    })
}

/// Round a fee to a whole minor unit (cent) so the Adyen amount converts
/// exactly. Positive amounts only.
pub fn round_to_cent(fee_micros: Micros) -> Micros {
    if fee_micros <= 0 {
        return fee_micros;
    }
    (fee_micros + crate::adyen::MICROS_PER_MINOR_UNIT / 2) / crate::adyen::MICROS_PER_MINOR_UNIT
        * crate::adyen::MICROS_PER_MINOR_UNIT
}

/// The pinned verification order per basis record: attributed lift, rate
/// band, then the recomputed fee (the exact expression the engine uses).
fn verify_basis(record: &BillingBasisRecord) -> Result<Micros, BillingError> {
    if record.incremental <= 0 {
        return Err(BillingError::TamperedProof {
            seq: record.seq,
            reason: format!(
                "attributed lift {} micros is not positive (no lift, no fee)",
                record.incremental
            ),
        });
    }
    if !(MIN_LIFT_RATE_BP..=MAX_LIFT_RATE_BP).contains(&record.rate_bp) {
        return Err(BillingError::TamperedProof {
            seq: record.seq,
            reason: format!(
                "rate {} bp outside [{MIN_LIFT_RATE_BP}, {MAX_LIFT_RATE_BP}]",
                record.rate_bp
            ),
        });
    }
    let rate = record.rate_bp as f64 / 10_000.0;
    let base = (rate * record.incremental as f64 * record.log_share).round() as Micros;
    let pro_rated = if record.month_days > 0 && record.billed_days < record.month_days {
        (base as f64 * record.billed_days as f64 / record.month_days as f64).round() as Micros
    } else {
        base
    };
    let fee_micros = round_to_cent(pro_rated);
    if fee_micros != record.fee_micros {
        return Err(BillingError::TamperedProof {
            seq: record.seq,
            reason: format!(
                "stored fee {} micros does not match the re-derived {} micros",
                record.fee_micros, fee_micros
            ),
        });
    }
    Ok(fee_micros)
}

/// The pinned verification order per proof: lift, rate band, then the
/// recomputed fee. Returns the recomputed fee for the line item.
fn verify_proof(proof: &BillingProofRecord) -> Result<Micros, BillingError> {
    if proof.proven_lift <= 0 {
        return Err(BillingError::TamperedProof {
            seq: proof.seq,
            reason: format!(
                "proven lift {} micros is not positive (no lift, no fee)",
                proof.proven_lift
            ),
        });
    }
    if !(MIN_LIFT_RATE_BP..=MAX_LIFT_RATE_BP).contains(&proof.rate_bp) {
        return Err(BillingError::TamperedProof {
            seq: proof.seq,
            reason: format!(
                "rate {} bp outside [{MIN_LIFT_RATE_BP}, {MAX_LIFT_RATE_BP}]",
                proof.rate_bp
            ),
        });
    }
    let recomputed = lift_fee_micros(proof.proven_lift, proof.rate_bp).map_err(|reason| {
        BillingError::TamperedProof {
            seq: proof.seq,
            reason,
        }
    })?;
    if recomputed != proof.fee_micros {
        return Err(BillingError::TamperedProof {
            seq: proof.seq,
            reason: format!(
                "stored fee {} micros but recomputed {} micros",
                proof.fee_micros, recomputed
            ),
        });
    }
    Ok(recomputed)
}
