//! Proposal/action model, policy and outcome types.

use apex_core::{Micros, PlatformError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    SetCountryFloor {
        ad_unit_id: String,
        country: String,
        floor_ecpm: Micros,
    },
    SetMappingEcpm {
        mediation_group_id: String,
        ad_unit_id: String,
        ecpm: Micros,
    },
    SetWaterfallPrice {
        ad_unit_id: String,
        entry_id: String,
        price_ecpm: Micros,
    },
    CreateWaterfallEntry {
        ad_unit_id: String,
        network: String,
        instance_name: String,
        price_ecpm: Micros,
    },
    RemoveWaterfallEntry {
        ad_unit_id: String,
        entry_id: String,
    },
}

impl Action {
    pub fn ad_unit_id(&self) -> &str {
        match self {
            Action::SetCountryFloor { ad_unit_id, .. }
            | Action::SetMappingEcpm { ad_unit_id, .. }
            | Action::SetWaterfallPrice { ad_unit_id, .. }
            | Action::CreateWaterfallEntry { ad_unit_id, .. }
            | Action::RemoveWaterfallEntry { ad_unit_id, .. } => ad_unit_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub action: Action,
    /// Human-auditable single change under test; becomes the experiment's
    /// change description verbatim.
    pub rationale: String,
    /// Expected effect in basis points; must be in (0, policy.max_step_bp].
    pub expected_effect_bp: i64,
    /// The setting's value as read from the platform immediately before
    /// proposing, so the pre-registration can pin the prior state.
    pub prev_value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutopilotPolicy {
    pub max_actions_per_day: u32,
    /// Magnitude cap per change in basis points (20% = 2000 bp).
    pub max_step_bp: i64,
    /// Pre-registration burn-in days (default 3).
    pub burn_in_days: u32,
    /// Analysis window in whole weeks (7, 14, 21 or 28 days).
    pub window_days: usize,
    /// Whether the single 99%-bound interim look at the window midpoint is
    /// allowed.
    pub interim_look: bool,
    /// Whether the one-week extension is allowed when the point estimate is
    /// positive but the bound is not.
    pub extension_allowed: bool,
    /// Planned test-arm share of users (0.25 default, 0.5 for low-risk
    /// changes under 1% worst-case loss).
    pub test_share: f64,
    /// Minimum point lift to keep a change (1% = 0.01), from the spec's keep
    /// condition 3.
    pub min_point_lift: f64,
    /// Guardrail thresholds, tightenable from the spec defaults.
    pub thresholds: apex_lift::decision::GuardrailThresholds,
    /// The contracted fee rate in basis points, 1500..=2000 (15%..=20%).
    #[serde(default = "default_rate_bp")]
    pub rate_bp: u16,
    /// Hash of the analysis code that will judge the test. Pinned into every
    /// pre-registration; defaults to the running engine version.
    #[serde(default = "default_code_version_hash")]
    pub code_version_hash: String,
    /// Refuse every write while any Critical audit result exists.
    pub blackout_on_critical_findings: bool,
}

fn default_rate_bp() -> u16 {
    2_000
}

fn default_code_version_hash() -> String {
    format!("apex-lift-{}", apex_lift::ENGINE_VERSION)
}

impl AutopilotPolicy {
    /// The documented conservative default.
    pub fn conservative() -> Self {
        Self {
            max_actions_per_day: 3,
            max_step_bp: 2_000,
            burn_in_days: 3,
            window_days: 14,
            interim_look: true,
            extension_allowed: true,
            test_share: 0.25,
            min_point_lift: 0.01,
            thresholds: Default::default(),
            rate_bp: 2_000,
            code_version_hash: default_code_version_hash(),
            blackout_on_critical_findings: true,
        }
    }

    /// The MAX `test_group_allocation` the pre-registered share maps to.
    /// MAX accepts only its documented set (5, 10, 25, 50); the engine's three
    /// shares map cleanly onto 10, 25 and 50.
    pub fn max_allocation(&self) -> u8 {
        match self.test_share {
            share if share >= 0.5 => 50,
            share if share >= 0.25 => 25,
            _ => 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum ActionOutcome {
    Applied {
        change_seq: u64,
        experiment_id: String,
        idempotency_key: String,
    },
    Skipped {
        reason: String,
    },
    RolledBack {
        change_seq: u64,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CycleOutcome {
    pub outcomes: Vec<ActionOutcome>,
    pub applied: u32,
    pub skipped: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "result")]
pub enum SettleOutcome {
    /// The change was kept: billable lambda comes from the engine's lower
    /// bound, and the signed result record is already in the ledger.
    Kept {
        experiment_id: String,
        /// Point lift from the log-ratio estimator.
        lift: f64,
        /// Billable lift: the one-sided 90% (or 99% interim) lower bound.
        lambda: f64,
        lower_90: f64,
        bootstrap_lower: f64,
        /// Ledger sequence of the signed result record.
        result_seq: u64,
    },
    /// The change was rolled back: no fee.
    RolledBack {
        lift: f64,
        reason: String,
        rollback_seq: u64,
    },
    /// The window is not complete; wait.
    Pending { reason: String },
    /// The test was voided (publisher edit, split mismatch, incident); no
    /// decision, no fee.
    Voided { reason: String },
}

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("policy violation: {reason}")]
    PolicyViolation { reason: String },

    #[error("daily action cap reached ({n} applied today)")]
    DailyCapReached { n: u32 },

    #[error("no baseline for comparison: {reason}")]
    NoBaseline { reason: String },

    #[error("endpoint for operation '{operation}' is not writable (verify-live)")]
    EndpointNotWritable { operation: String },

    #[error("apply failed: {source}")]
    ApplyFailed { source: PlatformError },

    #[error("rollback failed: {reason}")]
    RollbackFailed { reason: String },

    #[error("ledger error: {0}")]
    Ledger(#[from] apex_proof::LedgerError),

    #[error("validation: {0}")]
    Validation(String),
}
