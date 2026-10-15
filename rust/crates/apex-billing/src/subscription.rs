//! The optional flat monitoring plan.
//!
//! Continuous monitoring is a flat monthly fee, independent of proven lift:
//! the free audit, the daily measurement pass, guardrail monitoring and alerts
//! run whether or not a change is under test. It is a normal invoice line, so
//! it flows through the same re-derivation check, VAT application and Adyen
//! collection as a lift fee — only the amount is fixed rather than derived
//! from a proof. A subscriber can start and stop it month to month.

use apex_core::{Micros, MICROS_PER_UNIT};

use crate::invoice::InvoiceLine;

/// One monitoring plan: a flat monthly fee in micros.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitoringPlan {
    /// The flat monthly fee in micros (currency minor units times
    /// [`MICROS_PER_UNIT`]).
    pub monthly_fee_micros: Micros,
}

impl MonitoringPlan {
    /// The standard plan: a flat monthly fee.
    pub const STANDARD: Self = Self {
        monthly_fee_micros: 500 * MICROS_PER_UNIT,
    };

    /// The plan's fee in minor units (cents for a two-decimal currency).
    pub fn monthly_minor_units(&self) -> i64 {
        self.monthly_fee_micros / MICROS_PER_UNIT
    }

    /// The invoice line for one month of monitoring. The amount is flat; VAT
    /// is applied by the invoice like any other line.
    pub fn monthly_line(&self, seq: u64, month: &str) -> InvoiceLine {
        InvoiceLine {
            seq,
            experiment_id: format!("monitoring:{month}"),
            // Monitoring recovers no measured lift, so the proven-lift basis is
            // the fee itself and the rate is 10_000 bp (100%) by construction.
            proven_lift: self.monthly_fee_micros,
            rate_bp: 10_000,
            fee_micros: self.monthly_fee_micros,
            vat_micros: 0,
            gross_micros: self.monthly_fee_micros,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn the_standard_plan_is_a_flat_monthly_fee() {
        assert_eq!(MonitoringPlan::STANDARD.monthly_minor_units(), 500);
        let line = MonitoringPlan::STANDARD.monthly_line(3, "2026-10");
        assert_eq!(line.seq, 3);
        assert_eq!(line.experiment_id, "monitoring:2026-10");
        assert_eq!(line.fee_micros, MonitoringPlan::STANDARD.monthly_fee_micros);
    }
}
