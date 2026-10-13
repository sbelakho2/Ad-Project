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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutopilotPolicy {
    pub max_actions_per_day: u32,
    /// Magnitude cap per change in basis points (20% = 2000 bp).
    pub max_step_bp: i64,
    /// Minimum platform-measured lift to keep a change (1% = 100 bp).
    pub min_lift_bp: i64,
    /// Minimum platform-reported confidence in basis points (95% = 9500).
    pub min_confidence_bp: i32,
    /// Refuse every write while any Critical audit result exists.
    pub blackout_on_critical_findings: bool,
}

impl AutopilotPolicy {
    /// The documented conservative default.
    pub fn conservative() -> Self {
        Self {
            max_actions_per_day: 3,
            max_step_bp: 2_000,
            min_lift_bp: 100,
            min_confidence_bp: 9_500,
            blackout_on_critical_findings: true,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "result")]
pub enum SettleOutcome {
    Kept {
        lift_bp: i64,
        confidence_bp: i32,
        winner_variant_id: String,
        fee_micros: Option<Micros>,
        billing_seq: Option<u64>,
    },
    RolledBack {
        lift_bp: i64,
        confidence_bp: i32,
        reason: String,
        rollback_seq: u64,
    },
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
