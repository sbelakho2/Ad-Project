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
    /// Hold every proposal for explicit approval: run the gates and report the
    /// proposal, but write nothing. On by default for a customer's first 30
    /// days (see [`AutopilotPolicy::approval_required`]).
    #[serde(default)]
    pub approval_mode: bool,
    /// When the publisher onboarded, in unix milliseconds. Approval mode is
    /// forced on for the first [`APPROVAL_WINDOW_MS`] after this.
    #[serde(default)]
    pub onboarding_unix_ms: Option<i64>,
}

/// How long a new publisher runs in approval mode by default.
pub const APPROVAL_WINDOW_MS: i64 = 30 * 24 * 60 * 60 * 1000;

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
            approval_mode: false,
            onboarding_unix_ms: None,
        }
    }

    /// Whether a proposal must be held for approval at `now_unix_ms`: explicit
    /// approval mode, or a publisher within the first 30 days of onboarding.
    pub fn approval_required(&self, now_unix_ms: i64) -> bool {
        if self.approval_mode {
            return true;
        }
        match self.onboarding_unix_ms {
            Some(start) => now_unix_ms.saturating_sub(start) < APPROVAL_WINDOW_MS,
            None => false,
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
    /// Approval mode is on: the change is proposed but nothing was written.
    AwaitingApproval {
        idempotency_key: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CycleOutcome {
    pub outcomes: Vec<ActionOutcome>,
    pub applied: u32,
    pub skipped: u32,
    /// Proposals held for explicit approval (approval mode).
    #[serde(default)]
    pub awaiting_approval: u32,
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
        /// The analysis-window log ratios the decision was made on.
        daily_log_ratios: Vec<f64>,
    },
    /// The change was rolled back: no fee.
    RolledBack {
        lift: f64,
        reason: String,
        rollback_seq: u64,
        /// The guardrails that fired and forced the rollback. Not part of
        /// the serialized settle outcome (the guardrail records live in the
        /// ledger); carriers keep the in-process value.
        #[serde(skip)]
        guardrails: Vec<apex_lift::guardrails::GuardrailEvent>,
        /// The analysis-window log ratios the decision was made on.
        daily_log_ratios: Vec<f64>,
    },
    /// The window is not complete; wait.
    Pending { reason: String },
    /// The test was voided (publisher edit, split mismatch, incident); no
    /// decision, no fee.
    Voided {
        reason: String,
        /// The guardrails that fired and voided the test. Not part of the
        /// serialized settle outcome (the guardrail records live in the
        /// ledger).
        #[serde(skip)]
        guardrails: Vec<apex_lift::guardrails::GuardrailEvent>,
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn a_policy_without_the_new_fields_uses_the_defaults() {
        let json = serde_json::json!({
            "maxActionsPerDay": 2,
            "maxStepBp": 2000,
            "burnInDays": 3,
            "windowDays": 7,
            "interimLook": true,
            "extensionAllowed": true,
            "testShare": 0.25,
            "minPointLift": 0.01,
            "thresholds": {
                "harmPoint": -0.05,
                "fillRewarded": -0.05,
                "fillBanner": -0.05,
                "splitChiSquare": 10.83
            },
            "blackoutOnCriticalFindings": true
        });
        let policy: AutopilotPolicy = serde_json::from_value(json).expect("policy");
        assert_eq!(policy.rate_bp, 2_000, "rateBp default");
        assert!(policy.code_version_hash.starts_with("apex-lift-"));
        assert!(
            !policy.approval_required(1_000),
            "no onboarding, no approval"
        );
    }

    #[test]
    fn max_allocation_maps_the_pre_registered_share() {
        let mut policy = AutopilotPolicy::conservative();
        policy.test_share = 0.5;
        assert_eq!(policy.max_allocation(), 50);
        policy.test_share = 0.25;
        assert_eq!(policy.max_allocation(), 25);
        policy.test_share = 0.1;
        assert_eq!(policy.max_allocation(), 10);
    }
}
