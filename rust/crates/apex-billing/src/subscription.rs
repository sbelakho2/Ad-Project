//! Flat plan lines: managed ad ops.
//!
//! A3 made monitoring, the audit and the app-ads.txt watch free with every
//! connection. No monitoring tier, monitoring invoice line, cancellation
//! price or Setup Watch price exists. The only fixed-fee plan that remains is
//! Managed ad ops, priced from the publisher's reported monthly revenue.
//!
//! # Managed ad ops
//!
//! - **Base** — $2,500/mo at or below $1M of monthly revenue
//! - **Top** — $5,000/mo strictly above $1M
//!
//! The account team records the plan; the runner's monthly pass reads the
//! recorded state and issues one line on the invoice. Cancelled plans are
//! recorded, never deleted: `stops_after` names the last period the plan
//! bills.
//!
//! # State files
//!
//! One JSON [`PlanState`] per publisher under the connections root:
//! `<root>/<publisher>.managed.json`. A missing file is "not recorded"; a
//! malformed file is a typed refusal, never a fabricated plan.
//!
//! # Currency
//!
//! Every plan fee is denominated in [`PLAN_CURRENCY`], fixed to the collection
//! currency (USD) so a plan line always converts exactly to a Mollie amount.

use std::path::{Path, PathBuf};

use apex_core::{Micros, MICROS_PER_UNIT};
use serde::{Deserialize, Serialize};

use crate::errors::BillingError;
use crate::invoice::InvoiceLine;

/// The currency every flat plan fee is denominated in. Explicit on every
/// plan line; fixed to the collection currency so a plan line always converts
/// exactly to a Mollie amount.
pub const PLAN_CURRENCY: &str = crate::mollie::BILLING_CURRENCY;

// ---------------------------------------------------------------------------
// Managed ad ops
// ---------------------------------------------------------------------------

/// Managed base price: $2,500/mo.
pub const MANAGED_BASE_FEE_MICROS: Micros = 2_500 * MICROS_PER_UNIT;
/// Managed top price: $5,000/mo.
pub const MANAGED_TOP_FEE_MICROS: Micros = 5_000 * MICROS_PER_UNIT;
/// The top price starts strictly above this monthly revenue ($1M).
pub const MANAGED_TOP_ABOVE_REVENUE_MICROS: Micros = 1_000_000 * MICROS_PER_UNIT;

/// The written managed-ad-ops scope, as data. The console renders these
/// items verbatim; the crate is the single source of the scope text.
pub const MANAGED_SCOPE: [&str; 4] = [
    "Network onboarding",
    "Bidder setup",
    "app-ads.txt setup",
    "A monthly review",
];

/// The managed ad ops plan: a flat line, priced by monthly revenue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedPlan {
    /// The plan currency (always [`PLAN_CURRENCY`] today).
    pub currency: String,
}

impl ManagedPlan {
    /// The published plan: plan currency.
    pub fn new() -> Self {
        Self {
            currency: PLAN_CURRENCY.to_string(),
        }
    }

    /// The monthly price for `revenue_30d`: $2,500, or $5,000 strictly above
    /// $1M.
    pub fn fee_micros(revenue_30d: Micros) -> Micros {
        if revenue_30d > MANAGED_TOP_ABOVE_REVENUE_MICROS {
            MANAGED_TOP_FEE_MICROS
        } else {
            MANAGED_BASE_FEE_MICROS
        }
    }

    /// Activate the plan for a publisher from `revenue_30d`.
    pub fn activate(&self, revenue_30d: Micros, since: &str) -> PlanState {
        PlanState {
            plan: PlanId::Managed,
            active: true,
            since: since.to_string(),
            monthly_fee_micros: Self::fee_micros(revenue_30d),
            currency: self.currency.clone(),
            revenue_30d: Some(revenue_30d),
            stops_after: None,
        }
    }
}

impl Default for ManagedPlan {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Plan identity, lines and state
// ---------------------------------------------------------------------------

/// The identity of a flat plan line. Managed ad ops is the only paid plan
/// that remains after A3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanId {
    /// Managed ad ops (flat, by monthly revenue).
    Managed,
}

impl PlanId {
    /// The stable slug used in state file names and line ids.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Managed => "managed",
        }
    }

    /// The console label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Managed => "Managed ad ops",
        }
    }
}

/// A flat plan line: a named fee in an explicit currency. Converted into one
/// invoice line per billed month.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanLine {
    /// Which plan this line is.
    pub plan: PlanId,
    /// The monthly fee in micros.
    pub monthly_fee_micros: Micros,
    /// The currency the fee is denominated in.
    pub currency: String,
}

impl PlanLine {
    /// The fee in minor units (cents for a two-decimal currency).
    pub fn monthly_minor_units(&self) -> i64 {
        self.monthly_fee_micros / MICROS_PER_UNIT
    }

    /// The invoice line for one billed month. The amount is flat; VAT is
    /// applied by the invoice like any other line.
    pub fn monthly_line(&self, seq: u64, month: &str) -> InvoiceLine {
        plan_invoice_line(self.plan, self.monthly_fee_micros, seq, month, None)
    }
}

/// The persisted state of one flat plan for one publisher. Cancelled plans
/// are recorded, never deleted: `stops_after` names the last period the plan
/// bills.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanState {
    /// Which plan this state is.
    pub plan: PlanId,
    /// Whether the plan is on.
    #[serde(default)]
    pub active: bool,
    /// The day the plan started (`YYYY-MM-DD`).
    #[serde(default)]
    pub since: String,
    /// The recorded monthly fee in micros.
    pub monthly_fee_micros: Micros,
    /// The currency the fee is denominated in.
    pub currency: String,
    /// The reported 30-day revenue the price was chosen from, when a
    /// snapshot supplied one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revenue_30d: Option<Micros>,
    /// Cancelled plans: the last period (`YYYY-MM`) the plan bills.
    /// Cancellation stops the next period; it never removes this record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stops_after: Option<String>,
}

impl PlanState {
    /// Whether the plan bills `month` (`YYYY-MM`): active, and not past a
    /// recorded cancellation.
    pub fn bills_period(&self, month: &str) -> bool {
        let started = self.since.get(..7).unwrap_or("");
        self.active
            && (started.is_empty() || month >= started)
            && match &self.stops_after {
                Some(stops) => month <= stops.as_str(),
                None => true,
            }
    }

    /// Whether a cancellation is recorded (the plan is cancelled-not-deleted).
    pub fn cancelled(&self) -> bool {
        self.stops_after.is_some()
    }

    /// Record a cancellation effective after `month` (`YYYY-MM`): the plan
    /// bills the current period only, the next period is zero. Cancelling an
    /// already-cancelled plan never re-dates the recorded stop period.
    pub fn cancel(&mut self, month: &str) {
        if self.active && self.stops_after.is_none() {
            self.stops_after = Some(month.to_string());
        }
    }

    /// The invoice line for `month`, when the plan bills it. A cancelled
    /// plan's final line records the stop period in its id
    /// (`<slug>:<month>:cancelled:stops-<stops>`), so the invoiced evidence
    /// shows the plan ended rather than vanished. `None` means the period is
    /// not billed (inactive or past the stop period).
    pub fn monthly_line(&self, seq: u64, month: &str) -> Option<InvoiceLine> {
        if !self.bills_period(month) {
            return None;
        }
        Some(plan_invoice_line(
            self.plan,
            self.monthly_fee_micros,
            seq,
            month,
            self.stops_after.as_deref(),
        ))
    }

    /// The state file path for one publisher and plan under `root`:
    /// `<root>/<publisher>.<slug>.json`.
    pub fn path_for(root: &Path, publisher: &str, plan: PlanId) -> PathBuf {
        root.join(format!("{publisher}.{}.json", plan.slug()))
    }

    /// Load one publisher's plan state. A missing file is `Ok(None)`; a
    /// malformed or unreadable file is a typed refusal — never a fabricated
    /// plan.
    pub fn load(path: &Path) -> Result<Option<Self>, BillingError> {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(BillingError::PlanStateCorrupt {
                    detail: format!("{} could not be read: {error}", path.display()),
                })
            }
        };
        let state: Self =
            serde_json::from_str(&raw).map_err(|error| BillingError::PlanStateCorrupt {
                detail: format!("{} is not valid plan state: {error}", path.display()),
            })?;
        Ok(Some(state))
    }

    /// Persist the state atomically (temp file then rename), creating the
    /// parent directory. The console's cancel action and the runner's
    /// activation both write through here.
    pub fn save(&self, path: &Path) -> Result<(), BillingError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    BillingError::PlanStateCorrupt {
                        detail: format!("{}: parent directory: {error}", path.display()),
                    }
                })?;
            }
        }
        let json =
            serde_json::to_string_pretty(self).map_err(|error| BillingError::PlanStateCorrupt {
                detail: format!("plan state could not be serialized: {error}"),
            })?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &json).map_err(|error| BillingError::PlanStateCorrupt {
            detail: format!("{} could not be written: {error}", tmp.display()),
        })?;
        std::fs::rename(&tmp, path).map_err(|error| BillingError::PlanStateCorrupt {
            detail: format!("{} could not be renamed into place: {error}", tmp.display()),
        })?;
        Ok(())
    }
}

/// One flat invoice line: `proven_lift` is the fee itself and the rate is
/// 10_000 bp (100%) by construction, because a flat plan recovers no measured
/// lift. `stops_after` is recorded in the id when the plan is cancelled.
fn plan_invoice_line(
    plan: PlanId,
    fee_micros: Micros,
    seq: u64,
    month: &str,
    stops_after: Option<&str>,
) -> InvoiceLine {
    let slug = plan.slug();
    let experiment_id = match stops_after {
        Some(stops) => format!("{slug}:{month}:cancelled:stops-{stops}"),
        None => format!("{slug}:{month}"),
    };
    InvoiceLine {
        seq,
        experiment_id,
        proven_lift: fee_micros,
        rate_bp: 10_000,
        fee_micros,
        vat_micros: 0,
        gross_micros: fee_micros,
        holdout: None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn the_managed_points_shade_above_one_million() {
        assert_eq!(
            ManagedPlan::fee_micros(999_999 * MICROS_PER_UNIT),
            MANAGED_BASE_FEE_MICROS
        );
        assert_eq!(
            ManagedPlan::fee_micros(1_000_000 * MICROS_PER_UNIT),
            MANAGED_BASE_FEE_MICROS
        );
        assert_eq!(
            ManagedPlan::fee_micros(1_000_000 * MICROS_PER_UNIT + 1),
            MANAGED_TOP_FEE_MICROS
        );
        assert_eq!(MANAGED_SCOPE.len(), 4);
        let plan = ManagedPlan::default();
        let state = plan.activate(2_000_000 * MICROS_PER_UNIT, "2026-10-01");
        assert_eq!(state.monthly_fee_micros, 5_000 * MICROS_PER_UNIT);
        let line = state.monthly_line(1, "2026-10").expect("billed");
        assert_eq!(line.experiment_id, "managed:2026-10");
        assert_eq!(PLAN_CURRENCY, "USD");
    }

    #[test]
    fn cancellation_bills_the_current_period_only_and_is_not_deleted() {
        let plan = ManagedPlan::default();
        let mut state = plan.activate(50 * MICROS_PER_UNIT, "2026-09-01");
        state.cancel("2026-10");
        assert!(state.cancelled());
        assert_eq!(state.stops_after.as_deref(), Some("2026-10"));
        let line = state.monthly_line(1, "2026-10").expect("stop period bills");
        assert_eq!(
            line.experiment_id,
            "managed:2026-10:cancelled:stops-2026-10"
        );
        assert_eq!(state.monthly_line(2, "2026-11"), None);
        assert!(state.monthly_line(3, "2026-08").is_none());
        state.cancel("2026-12");
        assert_eq!(state.stops_after.as_deref(), Some("2026-10"));
    }

    #[test]
    fn an_inactive_state_bills_nothing() {
        let mut state = ManagedPlan::default().activate(50 * MICROS_PER_UNIT, "2026-10-01");
        state.active = false;
        assert_eq!(state.monthly_line(1, "2026-10"), None);
        state.cancel("2026-10");
        assert_eq!(
            state.stops_after, None,
            "an inactive plan records no cancel"
        );
    }

    #[test]
    fn plan_state_round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = PlanState::path_for(dir.path(), "northstar", PlanId::Managed);
        assert!(path.ends_with("northstar.managed.json"));
        assert_eq!(
            PlanState::load(&path).unwrap(),
            None,
            "missing is not corrupt"
        );
        let state = ManagedPlan::default().activate(50 * MICROS_PER_UNIT, "2026-10-01");
        state.save(&path).unwrap();
        let back = PlanState::load(&path).unwrap().expect("persisted");
        assert_eq!(back, state);
        assert!(back.active);
        let mut back = back;
        back.cancel("2026-10");
        back.save(&path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"stopsAfter\""), "{raw}");

        // A malformed file is a typed refusal, never a fabricated plan.
        std::fs::write(&path, "{ not json").unwrap();
        assert!(matches!(
            PlanState::load(&path),
            Err(BillingError::PlanStateCorrupt { .. })
        ));
        std::fs::write(&path, "{}").unwrap();
        assert!(matches!(
            PlanState::load(&path),
            Err(BillingError::PlanStateCorrupt { .. })
        ));
    }

    #[test]
    fn plan_line_is_denominated_in_the_canonical_currency() {
        let line = PlanLine {
            plan: PlanId::Managed,
            monthly_fee_micros: MANAGED_BASE_FEE_MICROS,
            currency: PLAN_CURRENCY.to_string(),
        };
        assert_eq!(line.currency, "USD");
        assert_eq!(line.monthly_minor_units(), 2_500);
        assert_eq!(line.monthly_line(7, "2026-10").seq, 7);
    }
}
