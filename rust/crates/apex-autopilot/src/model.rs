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
    /// Raise one LevelPlay waterfall instance price (the variant group's
    /// instance). `instance_id` is the platform instance resource id and
    /// `price_micros` the new eCPM floor in micros.
    SetLevelplayInstancePrice {
        instance_id: String,
        price_micros: i64,
    },
    /// Set an AdMob mediation-group line floor. `group_id` is the mediation
    /// group and `floor_micros` the new eCPM floor in micros.
    SetAdmobMediationFloor { group_id: String, floor_micros: i64 },
    /// Set a MAX frequency cap in the experiment's test group. The cap type
    /// selects the limit field; `countries` is the exact country set the
    /// setting covers (an empty list means the platform's global setting).
    SetMaxFrequencyCap {
        ad_unit_id: String,
        cap_type: MaxCapType,
        countries: Vec<String>,
        session_limit: Option<u32>,
        day_limit: Option<u32>,
        minute_frequency: Option<u32>,
    },
    /// Set a LevelPlay group-B session/interstitial/day cap. The group is the
    /// dashboard variant arm; group A is never touched.
    SetLevelplayGroupCapping {
        group_id: String,
        session_limit: Option<u32>,
        interstitial_limit: Option<u32>,
        day_limit: Option<u32>,
    },
}

/// The frequency-cap dimension MAX's `frequency_capping_settings` accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaxCapType {
    #[serde(rename = "session")]
    Session,
    /// A time cap carries `day_limit` and/or `minute_frequency`.
    #[serde(rename = "time")]
    Time,
}

impl MaxCapType {
    /// The exact documented wire spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Time => "time",
        }
    }
}

impl Action {
    /// The platform resource the action is scoped to. MAX and LevelPlay entry
    /// actions name an ad unit; the instance-price and mediation-floor actions
    /// name their platform resource directly (`instanceId`/`mediationGroupId`),
    /// so that resource id is the scope the ledger and pre-registration carry.
    pub fn ad_unit_id(&self) -> &str {
        match self {
            Action::SetCountryFloor { ad_unit_id, .. }
            | Action::SetMappingEcpm { ad_unit_id, .. }
            | Action::SetWaterfallPrice { ad_unit_id, .. }
            | Action::CreateWaterfallEntry { ad_unit_id, .. }
            | Action::RemoveWaterfallEntry { ad_unit_id, .. } => ad_unit_id,
            Action::SetLevelplayInstancePrice { instance_id, .. } => instance_id,
            Action::SetAdmobMediationFloor { group_id, .. } => group_id,
            Action::SetMaxFrequencyCap { ad_unit_id, .. } => ad_unit_id,
            Action::SetLevelplayGroupCapping { group_id, .. } => group_id,
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

    /// The canonical JSON of the complete effective policy: every field that
    /// constrains a write (caps, windows, share, burn-in, thresholds, minimum
    /// point lift, rate, code hash and blackout/approval switches). Approvals
    /// bind to its SHA-256, never to a version string alone.
    pub fn canonical_json(&self) -> Result<String, String> {
        serde_json::to_string(self)
            .map_err(|error| format!("policy could not be serialized: {error}"))
    }

    /// SHA-256 of [`Self::canonical_json`], lowercase hex.
    pub fn canonical_hash(&self) -> Result<String, String> {
        use sha2::Digest as _;
        let json = self.canonical_json()?;
        Ok(hex::encode(sha2::Sha256::digest(json.as_bytes())))
    }

    /// The conservative semantic bounds an operator may set through the
    /// control plane (and the runner re-checks every configured policy).
    pub fn validate(&self) -> Result<(), String> {
        if self.max_actions_per_day == 0 || self.max_actions_per_day > 50 {
            return Err(format!(
                "maxActionsPerDay must be between 1 and 50 (given {})",
                self.max_actions_per_day
            ));
        }
        if !(1..=2_000).contains(&self.max_step_bp) {
            return Err(format!(
                "maxStepBp must be between 1 and 2000 (given {})",
                self.max_step_bp
            ));
        }
        if !matches!(self.window_days, 7 | 14 | 21 | 28) {
            return Err(format!(
                "windowDays must be 7, 14, 21 or 28 (given {})",
                self.window_days
            ));
        }
        let share_ok = [0.1_f64, 0.25, 0.5]
            .iter()
            .any(|allowed| (self.test_share - allowed).abs() < 1e-9);
        if !share_ok {
            return Err(format!(
                "testShare must be 0.1, 0.25 or 0.5 (given {})",
                self.test_share
            ));
        }
        if self.burn_in_days > 14 {
            return Err(format!(
                "burnInDays must be at most 14 (given {})",
                self.burn_in_days
            ));
        }
        if !(1_500..=2_000).contains(&self.rate_bp) {
            return Err(format!(
                "rateBp must be between 1500 and 2000 (given {})",
                self.rate_bp
            ));
        }
        if !self.min_point_lift.is_finite()
            || self.min_point_lift < 0.0
            || self.min_point_lift > 1.0
        {
            return Err(format!(
                "minPointLift must be a finite number in [0, 1] (given {})",
                self.min_point_lift
            ));
        }
        // The guardrail thresholds are part of the effective policy too: a
        // policy that loosens a stop condition must never reach the runner.
        self.thresholds.validate()?;
        Ok(())
    }
}

/// Read one `AutopilotPolicy` from `path`, fail closed on every error.
pub fn load_policy_file(path: &std::path::Path) -> Result<AutopilotPolicy, String> {
    let raw =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let policy: AutopilotPolicy = serde_json::from_str(&raw)
        .map_err(|error| format!("{}: not an AutopilotPolicy: {error}", path.display()))?;
    policy.validate()?;
    Ok(policy)
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
    /// The platform cannot carry this experiment (for example MAX refuses to
    /// split a waterfall unit). The pre-registration stays in the ledger as an
    /// audit-only fact with the platform's exact refusal; nothing is retried
    /// and no experiment is started.
    AuditOnly {
        reason: String,
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
    /// Proposals held because the platform adapter has not verified the
    /// operation as writable (`verify-live`): one typed reason per held
    /// proposal, `awaiting live verification for <platform>: <operation>`,
    /// in proposal order. A held proposal writes nothing and the pass
    /// continues past it.
    #[serde(default)]
    pub awaiting_verification: Vec<String>,
    /// Provider write calls that failed this pass (start/rollback refusals,
    /// transport failures). Business skips and policy holds are NOT counted:
    /// this is execution health, separate from business success.
    #[serde(default)]
    pub provider_write_failures: u32,
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
        /// True when the platform reports no per-variant measurement: the
        /// decision came from the platform's own verdict, the lift is zero and
        /// the change is fee-ineligible.
        #[serde(default)]
        unmeasured: bool,
        /// The platform verdict that produced an unmeasured keep
        /// (`TREATMENT`), when this is one; absent for measured keeps.
        #[serde(default)]
        variant_leader: Option<String>,
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
        /// True when the rollback is the conclusion of an unmeasured test
        /// (the platform verdict was not `TREATMENT`). No guardrail fired.
        #[serde(default)]
        unmeasured: bool,
        /// The platform verdict behind an unmeasured rollback, when one exists.
        #[serde(default)]
        variant_leader: Option<String>,
    },
    /// A winner whose platform needs a manual dashboard promotion: the keep
    /// is recorded, but no fee starts until a later read confirms it.
    PromotionPending {
        experiment_id: String,
        instructions: String,
        deadline_unix_ms: i64,
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

    #[error("apply failed: {source}")]
    ApplyFailed { source: PlatformError },

    #[error("rollback failed: {reason}")]
    RollbackFailed { reason: String },

    #[error("ledger error: {0}")]
    Ledger(#[from] apex_proof::LedgerError),

    /// A signed lift payload in the ledger is not valid JSON. The scans refuse
    /// the pass instead of skipping the record: a hostile payload must never
    /// silently shrink the set of experiments that settle.
    #[error("ledger record {seq} has an unparseable lift payload: {detail}")]
    MalformedLiftRecord { seq: u64, detail: String },

    /// A billing/VAT refusal, typed by the billing crate: a missing seller
    /// profile, an unusable invoice register, or a refused VAT treatment
    /// carries its own variant and field names instead of a string.
    #[error("billing: {0}")]
    Billing(#[from] apex_billing::BillingError),

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

    #[test]
    fn conservative_policy_passes_the_semantic_bounds() {
        AutopilotPolicy::conservative().validate().expect("valid");
        for (window, share) in [(7, 0.1), (14, 0.25), (21, 0.5), (28, 0.25)] {
            let mut policy = AutopilotPolicy::conservative();
            policy.window_days = window;
            policy.test_share = share;
            policy.validate().expect("documented combination");
        }
    }

    #[test]
    fn out_of_bounds_policy_values_are_refused() {
        let mut policy = AutopilotPolicy::conservative();
        policy.max_step_bp = 0;
        assert!(policy.validate().is_err());
        policy.max_step_bp = 2_001;
        assert!(policy.validate().is_err());
        policy.max_step_bp = 2_000;
        policy.window_days = 10;
        assert!(policy.validate().is_err());
        policy.window_days = 14;
        policy.test_share = 0.2;
        assert!(policy.validate().is_err());
        policy.test_share = 0.25;
        policy.burn_in_days = 15;
        assert!(policy.validate().is_err());
        policy.burn_in_days = 14;
        policy.rate_bp = 1_000;
        assert!(policy.validate().is_err());
        policy.rate_bp = 1_500;
        policy.max_actions_per_day = 0;
        assert!(policy.validate().is_err());
        policy.max_actions_per_day = 3;
        policy.validate().expect("restored bounds");
    }

    #[test]
    fn the_canonical_policy_hash_covers_every_effective_field() {
        let base = AutopilotPolicy::conservative();
        let base_hash = base.canonical_hash().expect("hash");
        assert_eq!(base_hash, base.clone().canonical_hash().expect("hash"));
        assert_eq!(base_hash.len(), 64);

        let mut changed = base.clone();
        changed.min_point_lift = 0.02;
        assert_ne!(
            base_hash,
            changed.canonical_hash().expect("hash"),
            "the keep floor is part of the effective policy"
        );

        let mut threshold = base.clone();
        threshold.thresholds.harm_point = -0.04;
        assert_ne!(base_hash, threshold.canonical_hash().expect("hash"));

        let mut burn = base.clone();
        burn.burn_in_days = 2;
        assert_ne!(base_hash, burn.canonical_hash().expect("hash"));
    }

    #[test]
    fn thresholds_and_the_minimum_point_lift_are_validated() {
        let mut policy = AutopilotPolicy::conservative();
        policy.min_point_lift = f64::NAN;
        assert!(policy.validate().is_err());
        policy.min_point_lift = -0.1;
        assert!(policy.validate().is_err());
        policy.min_point_lift = 2.0;
        assert!(policy.validate().is_err());
        policy.min_point_lift = 0.02;
        policy.thresholds.split_chi_square = f64::NAN;
        assert!(
            policy.validate().is_err(),
            "a non-finite guardrail threshold is refused even when the caps are valid"
        );
    }

    #[test]
    fn load_policy_file_is_fail_closed() {
        let dir = std::env::temp_dir().join(format!("apex-policy-load-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("effective.json");
        assert!(load_policy_file(&path).is_err(), "missing file");
        std::fs::write(&path, "{not json}").expect("write");
        assert!(load_policy_file(&path).is_err(), "invalid JSON");
        let mut invalid = AutopilotPolicy::conservative();
        invalid.max_step_bp = 9_000;
        std::fs::write(&path, serde_json::to_vec(&invalid).expect("json")).expect("write");
        assert!(load_policy_file(&path).is_err(), "out of bounds");
        std::fs::write(
            &path,
            serde_json::to_vec(&AutopilotPolicy::conservative()).expect("json"),
        )
        .expect("write");
        assert_eq!(
            load_policy_file(&path).expect("valid"),
            AutopilotPolicy::conservative()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
