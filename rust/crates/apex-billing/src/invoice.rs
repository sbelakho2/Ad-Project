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

use apex_core::{lift_fee_micros, Micros, MAX_LIFT_RATE_BP, MIN_LIFT_RATE_BP, RESULTS_RATE_BP};
use apex_proof::LedgerRecord;
use serde::{Deserialize, Serialize};

use crate::errors::BillingError;
use crate::profile::{BillingProfile, CustomerProfile};
use crate::vat::{
    treatment_for_shape, treatment_for_with_vies, HttpViesClient, VatTreatment, ViesEvidence,
    ViesOutcome,
};

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

/// The A3 holdout reading an invoice line shows: the measured gain, its
/// range, the revenue it applies to and the signed records behind it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldoutLine {
    pub measured_lift: f64,
    pub lower_90: f64,
    pub upper_90: f64,
    /// The treated (exposed) revenue the fee was applied to.
    pub treated_revenue: Micros,
    /// Signed ledger references behind the reading.
    #[serde(default)]
    pub ledger_references: Vec<String>,
    /// The "no fee this month" note when the measured gain is zero or below.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_fee_reason: Option<String>,
}

/// One billable line: a re-verified proof with its recomputed net fee and the
/// VAT split applied to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceLine {
    pub seq: u64,
    pub experiment_id: String,
    pub proven_lift: Micros,
    pub rate_bp: u16,
    /// The net fee (the proven-lift rate). VAT is added on top.
    pub fee_micros: Micros,
    /// The VAT on the net fee, at the invoice's rate.
    #[serde(default)]
    pub vat_micros: Micros,
    /// The gross amount collected: net + VAT.
    #[serde(default)]
    pub gross_micros: Micros,
    /// The A3 holdout reading behind this line, when it is a results-fee
    /// line. Legacy lift-fee lines carry `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holdout: Option<HoldoutLine>,
}

/// A monthly invoice: the verified lines (ordered by `seq`) and their checked
/// total.
///
/// Zero-fee lines are kept when a positive proof truncates to a zero fee, but
/// [`crate::create_payment_links`] intentionally omits them from the receipt:
/// `receipt.items` can be shorter than `line_items`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Invoice {
    /// The gapless sequential number assigned by
    /// [`crate::InvoiceRegister::issue`]; 0 while the invoice is still an
    /// unregistered draft. Never reused.
    #[serde(default)]
    pub number: u64,
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
    /// The customer's legal identity, required before a VAT document can be
    /// rendered (see [`Invoice::with_customer`]). Optional because callers
    /// predating the VAT work may not have it yet; the renderer refuses
    /// rather than printing blanks.
    #[serde(default)]
    pub customer: Option<CustomerProfile>,
    /// The decided VAT treatment (see [`Invoice::apply_vat_treatment`]).
    /// [`VatTreatment::NotApplied`] marks a draft the renderer and the
    /// register refuse.
    #[serde(default)]
    pub vat_treatment: VatTreatment,
    /// The EU VIES answer the treatment was decided from, when one was
    /// consulted. Kept as evidence on the invoice and in the register line;
    /// a reverse charge without a matching valid answer cannot render.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vies_evidence: Option<ViesEvidence>,
}

impl Invoice {
    /// Apply a VAT rate to the invoice: VAT is computed per line (round
    /// half-up to the micro) and added to the net to form the gross. A
    /// non-positive rate marks the invoice as not subject to VAT.
    ///
    /// Every step is checked: a VAT or gross total that does not fit in
    /// `i64` micros is [`BillingError::Overflow`], never a saturated total.
    ///
    /// This is the low-level rate application; it does not decide *which*
    /// treatment applies. Use [`Invoice::apply_vat_treatment`] for that — an
    /// invoice carrying only a rate stays [`VatTreatment::NotApplied`] and
    /// [`Invoice::render_vat_invoice`] refuses it.
    pub fn with_vat(mut self, vat_rate_bp: u16) -> Result<Self, BillingError> {
        self.vat_rate_bp = vat_rate_bp;
        let mut vat_total: Micros = 0;
        for line in &mut self.line_items {
            line.vat_micros = vat_micros(line.fee_micros, vat_rate_bp)?;
            line.gross_micros = line
                .fee_micros
                .checked_add(line.vat_micros)
                .ok_or(BillingError::Overflow)?;
            vat_total = vat_total
                .checked_add(line.vat_micros)
                .ok_or(BillingError::Overflow)?;
        }
        self.vat_micros = vat_total;
        self.gross_micros = self
            .total_micros
            .checked_add(vat_total)
            .ok_or(BillingError::Overflow)?;
        Ok(self)
    }

    /// Attach the customer's legal identity to the invoice. The renderer
    /// needs it (typed refusal without it); the register refuses to issue an
    /// invoice without it.
    #[must_use]
    pub fn with_customer(mut self, customer: CustomerProfile) -> Self {
        self.customer = Some(customer);
        self
    }

    /// Append one flat plan line (managed ad ops):
    /// the checked net total grows by the line's fee, exactly as the lift
    /// lines were summed. Call before the VAT treatment is applied so the
    /// line is taxed with every other line; the totals arithmetic is the
    /// same `checked_add` shape as [`compute_invoice`]'s.
    pub fn push_plan_line(&mut self, line: InvoiceLine) -> Result<(), BillingError> {
        self.total_micros = self
            .total_micros
            .checked_add(line.fee_micros)
            .ok_or(BillingError::Overflow)?;
        self.line_items.push(line);
        Ok(())
    }

    /// Decide and apply the VAT treatment for the attached customer against
    /// `seller`'s profile using the production VIES client. See
    /// [`Self::apply_vat_treatment_with_vies`] for the injected seam.
    pub fn apply_vat_treatment(mut self, seller: &BillingProfile) -> Result<Self, BillingError> {
        self = self.apply_vat_treatment_with_vies(seller, &HttpViesClient::default())?;
        Ok(self)
    }

    /// Decide and apply the VAT treatment for the attached customer against
    /// `seller`'s profile, consulting `vies` before any reverse charge.
    ///
    /// The decided treatment and the VIES answer (when one was consulted) are
    /// recorded on the invoice. Both profiles are validated first, so a
    /// missing field is a typed refusal naming it. An unavailable register
    /// refuses reverse charge with [`BillingError::ViesUnavailable`]; an
    /// invalid or unknown number applies domestic VAT.
    pub fn apply_vat_treatment_with_vies(
        mut self,
        seller: &BillingProfile,
        vies: &dyn crate::vat::ViesClient,
    ) -> Result<Self, BillingError> {
        let customer = self
            .customer
            .as_ref()
            .ok_or(BillingError::MissingCustomerProfile)?;
        let decision = treatment_for_with_vies(seller, customer, vies)?;
        // `treatment_for_with_vies` returns only decided treatments: the
        // domestic rate, reverse charge, or no VAT outside the EU.
        let rate_bp = if decision.treatment == VatTreatment::Domestic {
            seller.vat_rate_bp
        } else {
            0
        };
        self = self.with_vat(rate_bp)?;
        self.vat_treatment = decision.treatment;
        self.vies_evidence = decision.evidence;
        Ok(self)
    }

    /// Render the invoice as the human-facing VAT document the billing provider's invoice
    /// template and the customer see.
    ///
    /// This is the compliance gate: it refuses with a typed error instead of
    /// printing a half-empty document when the register number is missing,
    /// either party's profile is incomplete, no treatment was applied, or the
    /// stored figures do not re-derive from the applied treatment. The
    /// "reverse charge" wording stands exactly where the tax would otherwise
    /// be stated; a non-EU customer has no VAT line at all.
    ///
    /// The re-derivation compares against the *current* seller profile, so a
    /// profile change (country or domestic rate) makes historical re-renders
    /// refuse rather than silently state a different rate: the register line
    /// stays the immutable record.
    pub fn render_vat_invoice(&self, seller: &BillingProfile) -> Result<String, BillingError> {
        if self.number == 0 {
            return Err(BillingError::UnregisteredInvoice);
        }
        seller.validate()?;
        let customer = self
            .customer
            .as_ref()
            .ok_or(BillingError::MissingCustomerProfile)?;
        customer.validate()?;
        if self.vat_treatment == VatTreatment::NotApplied {
            return Err(BillingError::VatTreatmentNotApplied);
        }
        let expected = if self.vat_treatment == VatTreatment::ReverseCharge {
            // A reverse charge is lawful only with the matching VIES
            // confirmation recorded when the treatment was applied. Rendering
            // never calls the register again, so the recorded evidence is the
            // authority.
            let evidence =
                self.vies_evidence
                    .as_ref()
                    .ok_or_else(|| BillingError::InconsistentVat {
                        detail: "reverse charge requires a recorded VIES confirmation".to_string(),
                    })?;
            if !evidence.confirms(customer) {
                return Err(BillingError::InconsistentVat {
                    detail: format!(
                        "reverse charge evidence does not confirm the customer's VAT number: {}",
                        evidence.detail
                    ),
                });
            }
            VatTreatment::ReverseCharge
        } else {
            let shape = treatment_for_shape(seller, customer)?;
            if shape == VatTreatment::ReverseCharge {
                // The parties are cross-border EU with a number, but the
                // applied treatment is domestic: only an invalid/unknown
                // VIES answer that matches the customer may explain that.
                match &self.vies_evidence {
                    Some(evidence)
                        if evidence.confirms_country_and_number(customer)
                            && matches!(
                                evidence.outcome,
                                ViesOutcome::Invalid | ViesOutcome::Unknown
                            ) =>
                    {
                        VatTreatment::Domestic
                    }
                    _ => {
                        return Err(BillingError::InconsistentVat {
                            detail: format!(
                                "the invoice states {} without a VIES answer that supports it",
                                self.vat_treatment
                            ),
                        })
                    }
                }
            } else {
                shape
            }
        };
        if self.vat_treatment != expected {
            return Err(BillingError::InconsistentVat {
                detail: format!(
                    "the invoice states {} but the parties' profiles derive {expected}",
                    self.vat_treatment
                ),
            });
        }
        let expected_rate = if expected == VatTreatment::Domestic {
            seller.vat_rate_bp
        } else {
            0
        };
        if self.vat_rate_bp != expected_rate {
            return Err(BillingError::InconsistentVat {
                detail: format!(
                    "rate {} bp does not match the {expected} treatment ({expected_rate} bp)",
                    self.vat_rate_bp
                ),
            });
        }
        let mut vat_total: Micros = 0;
        for line in &self.line_items {
            let vat = vat_micros(line.fee_micros, self.vat_rate_bp)?;
            let expected_gross = line
                .fee_micros
                .checked_add(vat)
                .ok_or(BillingError::Overflow)?;
            if line.vat_micros != vat || line.gross_micros != expected_gross {
                return Err(BillingError::InconsistentVat {
                    detail: format!(
                        "line #{} carries VAT {} micros / gross {} micros",
                        line.seq, line.vat_micros, line.gross_micros
                    ),
                });
            }
            vat_total = vat_total.checked_add(vat).ok_or(BillingError::Overflow)?;
        }
        let expected_gross = self
            .total_micros
            .checked_add(vat_total)
            .ok_or(BillingError::Overflow)?;
        if self.vat_micros != vat_total || self.gross_micros != expected_gross {
            return Err(BillingError::InconsistentVat {
                detail: format!(
                    "invoice totals carry VAT {} micros / gross {} micros",
                    self.vat_micros, self.gross_micros
                ),
            });
        }

        let mut out = String::new();
        out.push_str(&format!(
            "VAT invoice No. {} for {}\n",
            self.number, self.period.year_month
        ));
        out.push_str("Seller:\n");
        out.push_str(&format!("{}\n", seller.legal_name));
        for line in &seller.address_lines {
            out.push_str(&format!("{line}\n"));
        }
        out.push_str(&format!("{}\n", seller.country));
        out.push_str(&format!("VAT number: {}\n", seller.vat_number));
        out.push_str("Customer:\n");
        out.push_str(&format!("{}\n", customer.legal_name));
        for line in &customer.address_lines {
            out.push_str(&format!("{line}\n"));
        }
        out.push_str(&format!("{}\n", customer.country));
        if let Some(vat_number) = &customer.vat_number {
            out.push_str(&format!("VAT number: {vat_number}\n"));
        }
        out.push_str("Line items:\n");
        for line in &self.line_items {
            out.push_str(&format!(
                "  #{} {}: net {} micros, VAT {} micros, gross {} micros\n",
                line.seq, line.experiment_id, line.fee_micros, line.vat_micros, line.gross_micros
            ));
        }
        out.push_str(&format!("Net total: {} micros\n", self.total_micros));
        if self.vat_treatment == VatTreatment::Domestic {
            out.push_str(&format!(
                "VAT ({} bp): {} micros\n",
                self.vat_rate_bp, self.vat_micros
            ));
        } else if self.vat_treatment == VatTreatment::ReverseCharge {
            // The stated-tax position, exactly as the reverse charge is
            // worded on an intra-EU B2B invoice.
            out.push_str("VAT: reverse charge\n");
        }
        if let Some(evidence) = &self.vies_evidence {
            // The register answer is part of the document, not a hidden
            // annotation: it is what makes the stated treatment auditable.
            out.push_str(&format!("VIES: {}\n", evidence.detail));
        }
        out.push_str(&format!("Gross total: {} micros\n", self.gross_micros));
        Ok(out)
    }
}

/// VAT on `net` at `rate_bp` basis points, rounded half-up to the micro.
/// Overflow is a typed refusal: a VAT figure is never saturated or wrapped.
fn vat_micros(net: Micros, rate_bp: u16) -> Result<Micros, BillingError> {
    if net <= 0 || rate_bp == 0 {
        return Ok(0);
    }
    let scaled = net
        .checked_mul(i64::from(rate_bp))
        .ok_or(BillingError::Overflow)?;
    scaled
        .checked_add(5_000)
        .ok_or(BillingError::Overflow)
        .map(|rounded| rounded / 10_000)
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
            holdout: None,
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
        number: 0,
        publisher_id: publisher_id.to_string(),
        period,
        line_items,
        total_micros,
        vat_rate_bp: 0,
        vat_micros: 0,
        gross_micros: total_micros,
        customer: None,
        vat_treatment: VatTreatment::NotApplied,
        vies_evidence: None,
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
    /// The A3 holdout reading behind the line. `None` on legacy lift-fee
    /// records; `Some` makes the results-rate (3000 bp) verification apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holdout: Option<HoldoutLine>,
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
        let holdout = record.holdout.clone().map(|mut line| {
            // Every zero-fee month explains itself, including a positive point
            // estimate whose bound did not clear zero.
            if fee_micros == 0 && line.no_fee_reason.is_none() {
                line.no_fee_reason =
                    Some("No fee this month: the holdout measured no gain.".to_string());
            }
            line
        });
        line_items.push(InvoiceLine {
            seq: record.seq,
            experiment_id: record.change_id.clone(),
            proven_lift: record.incremental,
            rate_bp: record.rate_bp,
            fee_micros,
            vat_micros: 0,
            gross_micros: fee_micros,
            holdout,
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
        number: 0,
        publisher_id: publisher_id.to_string(),
        period,
        line_items,
        total_micros,
        vat_rate_bp: 0,
        vat_micros: 0,
        gross_micros: total_micros,
        customer: None,
        vat_treatment: VatTreatment::NotApplied,
        vies_evidence: None,
    })
}

/// Round a fee to a whole minor unit (cent) so the Mollie amount converts
/// exactly. Positive amounts only. A fee within half a minor unit of
/// `i64::MAX` saturates to the largest cent-exact value: the half-cent add
/// must never overflow (debug panic) or wrap the fee negative (release).
pub fn round_to_cent(fee_micros: Micros) -> Micros {
    if fee_micros <= 0 {
        return fee_micros;
    }
    (fee_micros.saturating_add(crate::mollie::MICROS_PER_MINOR_UNIT / 2)
        / crate::mollie::MICROS_PER_MINOR_UNIT)
        * crate::mollie::MICROS_PER_MINOR_UNIT
}

/// The pinned verification order per basis record: attributed lift, rate
/// band, then the recomputed fee (the exact expression the engine uses).
fn verify_basis(record: &BillingBasisRecord) -> Result<Micros, BillingError> {
    if let Some(holdout) = &record.holdout {
        return verify_holdout_basis(record, holdout);
    }
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

/// Re-derive an A3 results-fee line: the signed measured lift and bounds, the
/// single 30% rate and the exposed incremental revenue. A month whose bound
/// does not clear zero, or whose bound-derived gain rounds to zero at this
/// revenue, is a valid zero-fee line with its own explanation — never an old
/// 15–20% fee.
fn verify_holdout_basis(
    record: &BillingBasisRecord,
    holdout: &HoldoutLine,
) -> Result<Micros, BillingError> {
    if record.rate_bp != RESULTS_RATE_BP {
        return Err(BillingError::TamperedProof {
            seq: record.seq,
            reason: format!(
                "holdout rate {} bp is not the results rate {RESULTS_RATE_BP} bp",
                record.rate_bp
            ),
        });
    }
    if !holdout.measured_lift.is_finite()
        || !holdout.lower_90.is_finite()
        || !holdout.upper_90.is_finite()
    {
        return Err(BillingError::TamperedProof {
            seq: record.seq,
            reason: "holdout reading carries a non-finite value".into(),
        });
    }
    if holdout.lower_90 > holdout.measured_lift || holdout.measured_lift > holdout.upper_90 {
        return Err(BillingError::TamperedProof {
            seq: record.seq,
            reason: "holdout bounds do not contain the measured lift".into(),
        });
    }
    if !(record.log_share.is_finite() && record.log_share > 0.0 && record.log_share <= 1.0) {
        return Err(BillingError::TamperedProof {
            seq: record.seq,
            reason: format!("holdout log share {} is outside (0, 1]", record.log_share),
        });
    }
    // The fee is sized from the positive 90% lower bound, never the point
    // estimate: a month whose bound does not clear zero must carry a zero
    // incremental and a zero fee even when the point estimate is positive.
    if holdout.lower_90 <= 0.0 {
        if record.incremental != 0 || record.fee_micros != 0 {
            return Err(BillingError::TamperedProof {
                seq: record.seq,
                reason: format!(
                    "a non-positive holdout lower bound cannot bill incremental {} micros / fee {} micros",
                    record.incremental, record.fee_micros
                ),
            });
        }
        return Ok(0);
    }
    if record.incremental < 0 {
        return Err(BillingError::TamperedProof {
            seq: record.seq,
            reason: format!(
                "holdout incremental {} micros is negative",
                record.incremental
            ),
        });
    }
    if record.incremental == 0 {
        // A positive bound whose gain rounds away to zero at this revenue is
        // a legitimate zero-fee month, not tampering: nothing billed.
        if record.fee_micros != 0 {
            return Err(BillingError::TamperedProof {
                seq: record.seq,
                reason: format!(
                    "a zero holdout incremental cannot bill {} micros",
                    record.fee_micros
                ),
            });
        }
        return Ok(0);
    }
    if holdout.treated_revenue <= 0 || holdout.treated_revenue > record.scope_revenue {
        return Err(BillingError::TamperedProof {
            seq: record.seq,
            reason: format!(
                "holdout treated revenue {} micros is outside the scope revenue {} micros",
                holdout.treated_revenue, record.scope_revenue
            ),
        });
    }
    // Re-derive the incremental revenue from the bound itself: the recorded
    // gain must be the exposed revenue at `max(lower_90, 0)`, within the
    // per-interval rounding slack of one month.
    let bound = holdout.lower_90;
    let expected = holdout.treated_revenue as f64 * (1.0 - 1.0 / (1.0 + bound));
    if !expected.is_finite() {
        return Err(BillingError::TamperedProof {
            seq: record.seq,
            reason: "holdout bound does not produce a finite gain".into(),
        });
    }
    let slack = i64::from(record.month_days) * 2 + 64;
    let drift = (record.incremental as f64 - expected).abs();
    if drift > slack as f64 {
        return Err(BillingError::TamperedProof {
            seq: record.seq,
            reason: format!(
                "holdout incremental {} micros does not match the bound-derived gain {} micros",
                record.incremental,
                expected.round()
            ),
        });
    }
    let base =
        (f64::from(RESULTS_RATE_BP) / 10_000.0 * record.incremental as f64 * record.log_share)
            .round() as Micros;
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
                "stored holdout fee {} micros does not match the re-derived {} micros",
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
    let recomputed = match lift_fee_micros(proof.proven_lift, proof.rate_bp) {
        Ok(fee) => fee,
        // Unreachable with the rate band and positivity already checked, but
        // a refusal is never silently dropped.
        Err(reason) => {
            return Err(BillingError::TamperedProof {
                seq: proof.seq,
                reason,
            })
        }
    };
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
