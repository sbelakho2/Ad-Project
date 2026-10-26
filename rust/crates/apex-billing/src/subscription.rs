//! Flat plan lines: monitoring, managed ad ops and the setup watch line.
//!
//! Continuous monitoring is a flat monthly fee, independent of proven lift:
//! the free audit, the daily measurement pass, guardrail monitoring and alerts
//! run whether or not a change is under test. It is a normal invoice line, so
//! it flows through the same re-derivation check, VAT application and Mollie
//! collection as a lift fee — only the amount is fixed rather than derived
//! from a proof.
//!
//! # Monitoring: tiers, activation and cancellation
//!
//! Monitoring is ON at connection — there is no opt-in action. The tier is
//! chosen from the publisher's reported 30-day revenue
//! (`apex_audit::RevenueTotals::revenue_30d` of the snapshot the runner's
//! cycle already holds):
//!
//! - **Base** — strictly under $100k: $500/mo
//! - **Mid** — from $100k through $500k inclusive: $1,500/mo
//! - **Top** — strictly above $500k: $3,000/mo
//!
//! The activation seam is `apex_autopilot`: the runner CLI's `cycle` command
//! (a publisher only gets a cycle once its console connection validated)
//! calls `Runner::activate_monitoring_from_snapshot`, which records the
//! chosen tier in the publisher's state file. The monthly pass reads that
//! recorded state; a monthly pass that runs before the connection's first
//! cycle records the base tier so a connected publisher always gets the plan
//! line, and the first cycle then replaces a revenue-less activation with the
//! snapshot-chosen tier exactly once.
//!
//! A cancellation keeps the plan state (cancelled, never deleted) with the
//! period it stops: the plan bills the current period only, and the next
//! period is zero. The line itself records the stop period in its
//! `experiment_id` (`monitoring:<month>:cancelled:stops-<month>`), so the
//! invoiced evidence shows why the line ended.
//!
//! # State files
//!
//! One JSON [`PlanState`] per publisher and plan, beside the customer billing
//! profile under the connections root the console writes:
//!
//! - `<root>/<publisher>.monitoring.json`
//! - `<root>/<publisher>.managed.json`
//! - `<root>/<publisher>.setup-watch.json`
//!
//! The console's settings pages render the state and write the cancel action;
//! the runner's monthly pass reads it. A missing file is "not recorded"; a
//! malformed file is a typed refusal, never a fabricated plan.
//!
//! # Environment
//!
//! [`MonitoringPrices::from_env`] reads the test/deployment overrides
//! `APEX_MONITORING_TIER_BASE_MICROS`, `APEX_MONITORING_TIER_MID_MICROS` and
//! `APEX_MONITORING_TIER_TOP_MICROS`. The historical
//! `APEX_MONITORING_FEE_MICROS` remains the base-tier fallback and is
//! announced as deprecated through the caller's warning sink; a legacy value
//! of `0` (or less) keeps the old "not selected" meaning and turns monitoring
//! off, because deployments used it as the off switch.
//!
//! # Setup watch (frozen interface)
//!
//! [`watch_plan`] is the frozen interface for the setup-watch service wave:
//! the service calls it with its price provider and currency, gets a
//! [`PlanLine`] back, and activates/cancels it through the same
//! [`PlanState`] this module persists. The service itself is not built here.

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
// Monitoring tiers
// ---------------------------------------------------------------------------

/// Base tier price: $500/mo.
pub const MONITORING_BASE_FEE_MICROS: Micros = 500 * MICROS_PER_UNIT;
/// Mid tier price: $1,500/mo.
pub const MONITORING_MID_FEE_MICROS: Micros = 1_500 * MICROS_PER_UNIT;
/// Top tier price: $3,000/mo.
pub const MONITORING_TOP_FEE_MICROS: Micros = 3_000 * MICROS_PER_UNIT;
/// The mid tier starts at or above this 30-day revenue ($100k).
pub const MONITORING_MID_FROM_REVENUE_MICROS: Micros = 100_000 * MICROS_PER_UNIT;
/// The top tier starts strictly above this 30-day revenue ($500k).
pub const MONITORING_TOP_ABOVE_REVENUE_MICROS: Micros = 500_000 * MICROS_PER_UNIT;

/// Env override for the base-tier price, in micros.
pub const MONITORING_TIER_BASE_ENV: &str = "APEX_MONITORING_TIER_BASE_MICROS";
/// Env override for the mid-tier price, in micros.
pub const MONITORING_TIER_MID_ENV: &str = "APEX_MONITORING_TIER_MID_MICROS";
/// Env override for the top-tier price, in micros.
pub const MONITORING_TIER_TOP_ENV: &str = "APEX_MONITORING_TIER_TOP_MICROS";
/// The historical base-tier fee variable (deprecated; still read as the base
/// fallback when [`MONITORING_TIER_BASE_ENV`] is unset).
pub const MONITORING_FEE_ENV: &str = "APEX_MONITORING_FEE_MICROS";

/// The three monitoring tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MonitoringTier {
    /// Strictly under $100k of reported 30-day revenue.
    Base,
    /// From $100k through $500k inclusive.
    Mid,
    /// Strictly above $500k.
    Top,
}

impl MonitoringTier {
    /// The tier's console label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Base => "Base",
            Self::Mid => "Mid",
            Self::Top => "Top",
        }
    }

    /// The tier's default monthly price in micros.
    pub fn default_fee_micros(self) -> Micros {
        match self {
            Self::Base => MONITORING_BASE_FEE_MICROS,
            Self::Mid => MONITORING_MID_FEE_MICROS,
            Self::Top => MONITORING_TOP_FEE_MICROS,
        }
    }

    /// Select the tier for a publisher's reported 30-day revenue. Exact
    /// boundaries: $99,999.99 is Base, $100k is Mid, $500k is Mid,
    /// $500k + 1 micro is Top.
    pub fn for_revenue_30d(revenue_30d: Micros) -> Self {
        if revenue_30d < MONITORING_MID_FROM_REVENUE_MICROS {
            Self::Base
        } else if revenue_30d <= MONITORING_TOP_ABOVE_REVENUE_MICROS {
            Self::Mid
        } else {
            Self::Top
        }
    }
}

/// The tier price table the runner bills from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitoringPrices {
    /// Base-tier monthly price in micros.
    pub base_micros: Micros,
    /// Mid-tier monthly price in micros.
    pub mid_micros: Micros,
    /// Top-tier monthly price in micros.
    pub top_micros: Micros,
}

impl MonitoringPrices {
    /// The published tiers: $500 / $1,500 / $3,000.
    pub const DEFAULT: Self = Self {
        base_micros: MONITORING_BASE_FEE_MICROS,
        mid_micros: MONITORING_MID_FEE_MICROS,
        top_micros: MONITORING_TOP_FEE_MICROS,
    };

    /// The price of one tier.
    pub fn fee_micros(&self, tier: MonitoringTier) -> Micros {
        match tier {
            MonitoringTier::Base => self.base_micros,
            MonitoringTier::Mid => self.mid_micros,
            MonitoringTier::Top => self.top_micros,
        }
    }

    /// Read the environment overrides. Returns `Ok(None)` only when the
    /// deprecated `APEX_MONITORING_FEE_MICROS` is set to `0` (or less): the
    /// legacy off switch. A present-but-unparseable or non-positive tier
    /// override is a typed refusal, never a silently dropped price.
    pub fn from_env(
        lookup: &impl Fn(&str) -> Option<String>,
        warn: &mut dyn FnMut(&str),
    ) -> Result<Option<Self>, BillingError> {
        let base = match env_nonempty(lookup, MONITORING_TIER_BASE_ENV) {
            Some(raw) => parse_price(MONITORING_TIER_BASE_ENV, &raw)?,
            None => match env_nonempty(lookup, MONITORING_FEE_ENV) {
                Some(raw) => {
                    let legacy: i64 = raw.trim().parse().map_err(|_| {
                        BillingError::Malformed(format!(
                            "{MONITORING_FEE_ENV} must be a number of micros"
                        ))
                    })?;
                    if legacy <= 0 {
                        return Ok(None);
                    }
                    warn(&format!(
                        "{MONITORING_FEE_ENV} is deprecated; use {MONITORING_TIER_BASE_ENV} \
                         (the base-tier override)"
                    ));
                    legacy
                }
                None => MONITORING_BASE_FEE_MICROS,
            },
        };
        let mid = match env_nonempty(lookup, MONITORING_TIER_MID_ENV) {
            Some(raw) => parse_price(MONITORING_TIER_MID_ENV, &raw)?,
            None => MONITORING_MID_FEE_MICROS,
        };
        let top = match env_nonempty(lookup, MONITORING_TIER_TOP_ENV) {
            Some(raw) => parse_price(MONITORING_TIER_TOP_ENV, &raw)?,
            None => MONITORING_TOP_FEE_MICROS,
        };
        Ok(Some(Self {
            base_micros: base,
            mid_micros: mid,
            top_micros: top,
        }))
    }
}

fn env_nonempty(lookup: &impl Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    lookup(name).filter(|value| !value.trim().is_empty())
}

fn parse_price(name: &str, raw: &str) -> Result<Micros, BillingError> {
    let value: i64 = raw
        .trim()
        .parse()
        .map_err(|_| BillingError::Malformed(format!("{name} must be a number of micros")))?;
    if value <= 0 {
        return Err(BillingError::Malformed(format!(
            "{name} must be a positive micros amount"
        )));
    }
    Ok(value)
}

/// The monitoring configuration: the tier table and the plan currency.
/// Presence means monitoring is ON (at connection); the runner holds `None`
/// only when the legacy off switch turned it off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitoringPlan {
    /// The tier prices.
    pub prices: MonitoringPrices,
    /// The plan currency (always [`PLAN_CURRENCY`] today).
    pub currency: String,
}

impl MonitoringPlan {
    /// The published plan: default tiers, plan currency.
    pub fn new(prices: MonitoringPrices) -> Self {
        Self {
            prices,
            currency: PLAN_CURRENCY.to_string(),
        }
    }

    /// Activate the plan for a publisher from `revenue_30d`: the tier chosen
    /// by the table, with the evidence recorded on the state.
    pub fn activate(&self, revenue_30d: Micros, since: &str) -> PlanState {
        let tier = MonitoringTier::for_revenue_30d(revenue_30d);
        self.activate_tier(tier, Some(revenue_30d), since)
    }

    /// Activate the plan without revenue evidence (the first monthly pass
    /// before the connection's cycle): the base tier, marked as chosen
    /// without a snapshot so the first cycle can replace it exactly once.
    pub fn activate_without_snapshot(&self, since: &str) -> PlanState {
        self.activate_tier(MonitoringTier::Base, None, since)
    }

    fn activate_tier(
        &self,
        tier: MonitoringTier,
        revenue_30d: Option<Micros>,
        since: &str,
    ) -> PlanState {
        PlanState {
            plan: PlanId::Monitoring,
            active: true,
            since: since.to_string(),
            monthly_fee_micros: self.prices.fee_micros(tier),
            currency: self.currency.clone(),
            tier: Some(tier),
            revenue_30d,
            stops_after: None,
        }
    }
}

impl Default for MonitoringPlan {
    fn default() -> Self {
        Self::new(MonitoringPrices::DEFAULT)
    }
}

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
            tier: None,
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
// Setup watch (frozen interface)
// ---------------------------------------------------------------------------

/// The setup-watch base price: $99/mo.
pub const WATCH_PRICE_MICROS: Micros = 99 * MICROS_PER_UNIT;
/// The setup-watch core-publisher price: $199/mo.
pub const WATCH_CORE_PRICE_MICROS: Micros = 199 * MICROS_PER_UNIT;

/// The frozen setup-watch plan line for one publisher: the `$99 / $199
/// core-publisher` fee, chosen by the caller's provider, in an explicit
/// currency. The watch service wave calls this, activates the returned line
/// through [`PlanState`] (which records the cancel action), and bills it as
/// its own invoice line through [`PlanLine::monthly_line`].
///
/// The provider is called exactly once. A non-positive price yields a
/// zero-fee plan line (kept on the invoice, skipped by Mollie collection like
/// every zero line); the service owns price validation.
pub fn watch_plan(price_micros_provider: impl Fn() -> i64, currency: &str) -> PlanLine {
    PlanLine {
        plan: PlanId::SetupWatch,
        monthly_fee_micros: price_micros_provider(),
        currency: currency.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Plan identity, lines and state
// ---------------------------------------------------------------------------

/// The identity of a flat plan line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanId {
    /// Continuous monitoring (tiered, on at connection).
    Monitoring,
    /// Managed ad ops (flat, by monthly revenue).
    Managed,
    /// The setup watch sign-up service (frozen interface).
    SetupWatch,
}

impl PlanId {
    /// The stable slug used in state file names and line ids.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Monitoring => "monitoring",
            Self::Managed => "managed",
            Self::SetupWatch => "setup-watch",
        }
    }

    /// The console label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Monitoring => "Monitoring",
            Self::Managed => "Managed ad ops",
            Self::SetupWatch => "Setup watch",
        }
    }
}

/// A purchasable flat plan line: a named fee in an explicit currency.
/// Returned by [`watch_plan`] and the activation helpers; converted into one
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
    /// The recorded monitoring tier, when the plan is monitoring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<MonitoringTier>,
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
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn the_tier_table_prices_the_published_points() {
        assert_eq!(
            MonitoringTier::Base.default_fee_micros(),
            500 * MICROS_PER_UNIT
        );
        assert_eq!(
            MonitoringTier::Mid.default_fee_micros(),
            1_500 * MICROS_PER_UNIT
        );
        assert_eq!(
            MonitoringTier::Top.default_fee_micros(),
            3_000 * MICROS_PER_UNIT
        );
        assert_eq!(MonitoringTier::Base.label(), "Base");
        assert_eq!(MonitoringTier::Mid.label(), "Mid");
        assert_eq!(MonitoringTier::Top.label(), "Top");
        assert_eq!(PLAN_CURRENCY, "USD");
    }

    #[test]
    fn tiers_shade_by_revenue_boundaries() {
        // $99,999.99 (one cent under $100k) is Base; $100k is Mid.
        assert_eq!(
            MonitoringTier::for_revenue_30d(100_000 * MICROS_PER_UNIT - 10_000),
            MonitoringTier::Base
        );
        assert_eq!(
            MonitoringTier::for_revenue_30d(100_000 * MICROS_PER_UNIT),
            MonitoringTier::Mid
        );
        // $500k exactly is Mid; one micro above is Top.
        assert_eq!(
            MonitoringTier::for_revenue_30d(500_000 * MICROS_PER_UNIT),
            MonitoringTier::Mid
        );
        assert_eq!(
            MonitoringTier::for_revenue_30d(500_000 * MICROS_PER_UNIT + 1),
            MonitoringTier::Top
        );
        assert_eq!(MonitoringTier::for_revenue_30d(0), MonitoringTier::Base);
        assert_eq!(
            MonitoringTier::for_revenue_30d(-1),
            MonitoringTier::Base,
            "a negative report is still the base tier, never a panic"
        );
    }

    #[test]
    fn activation_records_the_tier_and_evidence() {
        let plan = MonitoringPlan::default();
        let state = plan.activate(350_000 * MICROS_PER_UNIT, "2026-10-16");
        assert_eq!(state.plan, PlanId::Monitoring);
        assert!(state.active);
        assert_eq!(state.tier, Some(MonitoringTier::Mid));
        assert_eq!(state.monthly_fee_micros, 1_500 * MICROS_PER_UNIT);
        assert_eq!(state.revenue_30d, Some(350_000 * MICROS_PER_UNIT));
        assert_eq!(state.since, "2026-10-16");
        assert!(!state.cancelled());
        let line = state.monthly_line(3, "2026-10").expect("billed");
        assert_eq!(line.experiment_id, "monitoring:2026-10");
        assert_eq!(line.fee_micros, 1_500 * MICROS_PER_UNIT);
        assert_eq!(line.rate_bp, 10_000);
        assert_eq!(line.proven_lift, line.fee_micros);
    }

    #[test]
    fn cancellation_bills_the_current_period_only_and_is_not_deleted() {
        let plan = MonitoringPlan::default();
        let mut state = plan.activate(50 * MICROS_PER_UNIT, "2026-09-01");
        state.cancel("2026-10");
        assert!(state.cancelled());
        assert_eq!(state.stops_after.as_deref(), Some("2026-10"));
        // The stop period still bills, and its line records the stop.
        let line = state.monthly_line(1, "2026-10").expect("stop period bills");
        assert_eq!(
            line.experiment_id,
            "monitoring:2026-10:cancelled:stops-2026-10"
        );
        assert_eq!(line.fee_micros, 500 * MICROS_PER_UNIT);
        // The next period is zero.
        assert_eq!(state.monthly_line(2, "2026-11"), None);
        // The activation month bills; a month before activation never does.
        assert_eq!(
            state
                .monthly_line(3, "2026-09")
                .expect("activation month")
                .experiment_id,
            "monitoring:2026-09:cancelled:stops-2026-10"
        );
        assert!(state.monthly_line(4, "2026-08").is_none());
        assert!(state.monthly_line(5, "2026-07").is_none());
        // Cancelling again never re-dates the stop period.
        state.cancel("2026-12");
        assert_eq!(state.stops_after.as_deref(), Some("2026-10"));
    }

    #[test]
    fn an_inactive_state_bills_nothing() {
        let mut state = MonitoringPlan::default().activate_without_snapshot("2026-10-01");
        state.active = false;
        assert_eq!(state.monthly_line(1, "2026-10"), None);
        state.cancel("2026-10");
        assert_eq!(
            state.stops_after, None,
            "an inactive plan records no cancel"
        );
    }

    #[test]
    fn the_legacy_env_is_the_base_tier_fallback_and_warns_once() {
        let legacy = |name: &str| match name {
            MONITORING_FEE_ENV => Some("600000000".to_string()),
            _ => None,
        };
        let mut warnings = Vec::new();
        let prices =
            MonitoringPrices::from_env(&legacy, &mut |message| warnings.push(message.to_string()))
                .unwrap()
                .expect("on");
        assert_eq!(prices.base_micros, 600_000_000);
        assert_eq!(prices.mid_micros, MONITORING_MID_FEE_MICROS, "default");
        assert_eq!(prices.top_micros, MONITORING_TOP_FEE_MICROS, "default");
        assert_eq!(warnings.len(), 1, "exactly one deprecation warning");
        assert!(warnings[0].contains("deprecated"), "{warnings:?}");
        assert!(warnings[0].contains(MONITORING_FEE_ENV), "{warnings:?}");

        // The legacy 0 stays the off switch: monitoring is not selected.
        let off = |name: &str| (name == MONITORING_FEE_ENV).then(|| "0".to_string());
        let mut warnings = Vec::new();
        assert_eq!(
            MonitoringPrices::from_env(&off, &mut |m| warnings.push(m.to_string())),
            Ok(None)
        );
        assert!(
            warnings.is_empty(),
            "the off switch is not a deprecation use"
        );

        // The tier override wins over the legacy name, without a warning.
        let both = |name: &str| match name {
            MONITORING_TIER_BASE_ENV => Some("700000000".to_string()),
            MONITORING_FEE_ENV => Some("600000000".to_string()),
            _ => None,
        };
        let mut warnings = Vec::new();
        let prices = MonitoringPrices::from_env(&both, &mut |m| warnings.push(m.to_string()))
            .unwrap()
            .expect("on");
        assert_eq!(prices.base_micros, 700_000_000);
        assert!(warnings.is_empty());
    }

    #[test]
    fn the_env_defaults_are_the_published_tiers() {
        let mut warnings = Vec::new();
        let prices = MonitoringPrices::from_env(&|_| None, &mut |m| warnings.push(m.to_string()))
            .unwrap()
            .expect("on by default");
        assert_eq!(prices, MonitoringPrices::DEFAULT);
        assert!(warnings.is_empty());

        // Tier overrides shape every price.
        let overrides = |name: &str| match name {
            MONITORING_TIER_BASE_ENV => Some("1000000".to_string()),
            MONITORING_TIER_MID_ENV => Some("2000000".to_string()),
            MONITORING_TIER_TOP_ENV => Some("3000000".to_string()),
            _ => None,
        };
        let prices = MonitoringPrices::from_env(&overrides, &mut |_| {})
            .unwrap()
            .expect("on");
        assert_eq!(prices.fee_micros(MonitoringTier::Base), 1_000_000);
        assert_eq!(prices.fee_micros(MonitoringTier::Mid), 2_000_000);
        assert_eq!(prices.fee_micros(MonitoringTier::Top), 3_000_000);
    }

    #[test]
    fn malformed_or_non_positive_env_values_are_typed_refusals() {
        for raw in ["lots", "0", "-5"] {
            let value = raw.to_string();
            let lookup = move |name: &str| (name == MONITORING_TIER_MID_ENV).then(|| value.clone());
            assert!(
                matches!(
                    MonitoringPrices::from_env(&lookup, &mut |_| {}),
                    Err(BillingError::Malformed(_))
                ),
                "{raw:?} must be refused"
            );
        }
        // A blank value reads as unset, exactly like the rest of the
        // configuration surface.
        let blank = |name: &str| (name == MONITORING_TIER_MID_ENV).then(String::new);
        let prices = MonitoringPrices::from_env(&blank, &mut |_| {})
            .unwrap()
            .expect("on");
        assert_eq!(prices.mid_micros, MONITORING_MID_FEE_MICROS);
        let bad_legacy = |name: &str| (name == MONITORING_FEE_ENV).then(|| "lots".to_string());
        assert!(matches!(
            MonitoringPrices::from_env(&bad_legacy, &mut |_| {}),
            Err(BillingError::Malformed(_))
        ));
    }

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
        assert_eq!(state.tier, None);
        let line = state.monthly_line(1, "2026-10").expect("billed");
        assert_eq!(line.experiment_id, "managed:2026-10");
    }

    #[test]
    fn watch_plan_is_the_frozen_core_line_interface() {
        let line = watch_plan(|| WATCH_PRICE_MICROS, PLAN_CURRENCY);
        assert_eq!(line.plan, PlanId::SetupWatch);
        assert_eq!(line.monthly_fee_micros, 99 * MICROS_PER_UNIT);
        assert_eq!(line.currency, "USD");
        assert_eq!(line.monthly_minor_units(), 99);
        let core = watch_plan(|| WATCH_CORE_PRICE_MICROS, PLAN_CURRENCY);
        assert_eq!(core.monthly_fee_micros, 199 * MICROS_PER_UNIT);
        let invoice_line = core.monthly_line(7, "2026-10");
        assert_eq!(invoice_line.experiment_id, "setup-watch:2026-10");
        assert_eq!(invoice_line.seq, 7);
        assert_eq!(invoice_line.fee_micros, 199 * MICROS_PER_UNIT);
        // A non-positive provider yields a zero-fee line (kept, never billed
        // through Mollie); the service owns validation.
        let zero = watch_plan(|| 0, PLAN_CURRENCY);
        assert_eq!(zero.monthly_line(1, "2026-10").fee_micros, 0);
    }

    #[test]
    fn plan_state_round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = PlanState::path_for(dir.path(), "northstar", PlanId::Monitoring);
        assert!(path.ends_with("northstar.monitoring.json"));
        assert_eq!(
            PlanState::load(&path).unwrap(),
            None,
            "missing is not corrupt"
        );
        let state = MonitoringPlan::default().activate(50 * MICROS_PER_UNIT, "2026-10-01");
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
    fn the_state_file_names_follow_the_plan_slug() {
        let root = Path::new("/connections");
        assert!(PlanState::path_for(root, "pub", PlanId::Managed).ends_with("pub.managed.json"));
        assert!(
            PlanState::path_for(root, "pub", PlanId::SetupWatch).ends_with("pub.setup-watch.json")
        );
    }
}
