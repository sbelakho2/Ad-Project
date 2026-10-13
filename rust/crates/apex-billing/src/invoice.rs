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

/// One billable line: a re-verified proof with its recomputed fee.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceLine {
    pub seq: u64,
    pub experiment_id: String,
    pub proven_lift: Micros,
    pub rate_bp: u16,
    pub fee_micros: Micros,
}

/// A monthly invoice: the verified lines (ordered by `seq`) and their checked
/// total.
///
/// Zero-fee lines are kept when a positive proof truncates to a zero fee, but
/// [`crate::bill_invoice`] intentionally omits them from the Stripe receipt:
/// `receipt.items` can be shorter than `line_items`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invoice {
    pub publisher_id: String,
    pub period: BillingPeriod,
    pub line_items: Vec<InvoiceLine>,
    pub total_micros: Micros,
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
    })
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
