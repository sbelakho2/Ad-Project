//! Platform operations abstraction + a fully-real scripted implementation used
//! by integration tests and by dry-run deployments.

use std::collections::VecDeque;
use std::sync::Mutex;

use apex_core::{ExperimentResult, ExperimentSpec, PlatformError};
use apex_proof::ChangeKind;
use async_trait::async_trait;

use crate::model::Action;

/// Daily per-arm pulls from the platform reporting APIs: the lift engine's
/// measurement input. An adapter that cannot read reporting refuses instead
/// of fabricating days.
#[derive(Debug, Clone, PartialEq)]
pub struct ExperimentDays {
    pub control: Vec<apex_lift::window::ArmDay>,
    pub test: Vec<apex_lift::window::ArmDay>,
}

/// Per-player evidence for the L1 estimator and the L3 unique-player split
/// check. Rows are the platform's impression/user revenue feed; the pretest
/// map is the pre-period revenue per resolved player for CUPED.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerEvidence {
    pub rows: Vec<apex_lift::player_store::PlayerDay>,
    pub pretest: std::collections::BTreeMap<String, apex_core::Micros>,
    /// The causal scope the platform feed actually measures. A feed whose
    /// scope is not the pre-registered ad unit cannot size that unit's fee.
    pub measurement_scope: Option<String>,
    /// Eligible randomization units per arm, including zero-revenue users the
    /// impression feed cannot show.
    pub roster: Option<apex_lift::holdout::AssignedRoster>,
    /// Per-user pre-assignment envelopes and priors for the finite-sample
    /// certificate. Absent means no fee can be certified unless the adapter
    /// supplies its own certified bound below.
    pub eligible: Option<Vec<EligibleUserEvidence>>,
    /// A platform-computed one-sided lower bound on the absolute revenue the
    /// treated subset added, used only when no per-user envelopes are present.
    pub certified_gain_lower: Option<i64>,
    /// The certificate's population lower bound, required with the absolute
    /// bound before a fee exists.
    pub certified_delta_lower: Option<f64>,
    /// Assigned unique players per arm when the platform exposes the roster;
    /// the initial split guardrail prefers these over observed players.
    pub assigned_test_control: Option<(u64, u64)>,
}

/// One eligible user's fixed certificate inputs.
#[derive(Debug, Clone, PartialEq)]
pub struct EligibleUserEvidence {
    pub user_id: String,
    /// Assigned to the exposed (treated) configuration.
    pub exposed: bool,
    pub prior_micros: i64,
    pub envelope_micros: i64,
}

impl PlayerEvidence {
    /// Fold the evidence into the player window the estimator and guardrail
    /// consume. A player in both arms is a typed refusal.
    pub fn window(
        &self,
    ) -> Result<apex_lift::player_store::PlayerWindow, apex_lift::player_store::PlayerStoreError>
    {
        apex_lift::player_store::player_window(&self.rows, Some(&self.pretest))
    }

    /// The unique-player split counts (test, control).
    pub fn split_counts(&self) -> Result<(u64, u64), apex_lift::player_store::PlayerStoreError> {
        let window = apex_lift::player_store::player_window(&self.rows, None)?;
        Ok((window.test_players, window.control_players))
    }
}

/// The promotion state of a kept change on an adapter that needs a manual
/// dashboard action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromotionStatus {
    Pending,
    Confirmed,
    Declined,
}

/// Everything an adapter needs to start the platform-native experiment that
/// carries the proposed change in its **test arm only**. The deterministic
/// `experiment_id` is the pre-registered name the ledger already pins.
#[derive(Debug, Clone, PartialEq)]
pub struct ExperimentRequest {
    /// Deterministic, pre-registered experiment id (the platform-native name).
    pub experiment_id: String,
    /// The platform experiment configuration.
    pub spec: ExperimentSpec,
    /// The single change the test arm must carry. The control arm and the
    /// live ad unit are left untouched.
    pub change: ChangeKind,
    /// The setting's value as read immediately before the call; the ledger
    /// already pins it in the pre-registration.
    pub prev_value: String,
}

/// Everything an adapter needs to open the 10% old-setting holdout after a
/// change is kept. The test arm carries the **old** configuration; the live
/// unit remains the kept change.
#[derive(Debug, Clone, PartialEq)]
pub struct HoldoutRequest {
    /// Deterministic holdout experiment name.
    pub experiment_id: String,
    /// The experiment configuration (10% test group).
    pub spec: ExperimentSpec,
    /// The kept change, so adapters can decide how to represent the inverse.
    pub change: ChangeKind,
    /// The setting value before the change, canonically encoded.
    pub prev_value: String,
}

/// The ended state of a platform test whose API reports no per-variant
/// measurement, plus the platform's own verdict on which variant led.
///
/// This is the whole measurement a platform such as AdMob can provide: a
/// `variantLeader` verdict, never revenue per arm. The settle path records
/// the conclusion durably and never turns the verdict into measured lift or a
/// fee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmeasuredConclusion {
    /// The platform's post-end experiment state (for example `ENDED`).
    pub experiment_state: String,
    /// The platform's `variantLeader` verdict, in the platform's own spelling.
    pub variant_leader: Option<String>,
    /// Whether the treatment arm led, so the change stays; otherwise the
    /// original control lines are the outcome.
    pub keep_treatment: bool,
}

#[async_trait]
pub trait PlatformOps: Send + Sync {
    /// The platform behind this adapter. The report's mode line, the fleet's
    /// mode aggregation and the fee-eligibility gate read it; adapters for
    /// platforms other than AppLovin MAX override it.
    fn platform(&self) -> apex_core::PlatformId {
        apex_core::PlatformId::AppLovinMax
    }

    /// Verify-live gate: may the autopilot write through this operation?
    async fn writable(&self, operation: &str) -> bool;

    /// Start the platform-native experiment whose **test arm** carries
    /// `request.change`; the live ad unit is never mutated on this path.
    /// Returns the platform experiment id.
    async fn start_experiment(&self, request: &ExperimentRequest) -> Result<String, PlatformError>;

    /// Start a null (A/A) experiment: the same configuration on both arms, no
    /// change under test. Used to calibrate whether the split and the pipeline
    /// are honest before any real change rides on them. The default adapter
    /// cannot, and refuses.
    async fn start_null_experiment(&self, spec: &ExperimentSpec) -> Result<String, PlatformError> {
        Err(PlatformError::Validation(format!(
            "start_null_experiment is not supported by this adapter for {}",
            spec.name
        )))
    }

    /// Whether [`start_experiment`](Self::start_experiment) carried the change
    /// in a test arm (true: the live unit is untouched, so a failed start has
    /// nothing to roll back) or applied it to the unit first (false).
    fn change_in_test_arm(&self) -> bool {
        false
    }

    /// Whether ending a test requires the publisher to act in the platform's
    /// dashboard. LevelPlay promotes only from the dashboard, so a winner is
    /// recorded as pending until the next read confirms the change serves
    /// everyone.
    fn promotion_requires_manual_action(&self) -> bool {
        false
    }

    /// The promotion state of a kept change: confirmed when it now serves
    /// everyone, declined when the publisher kept the control arm, pending
    /// while the platform test still runs. Automatic adapters are already
    /// confirmed after their own promote call.
    async fn promotion_status(
        &self,
        _experiment_id: &str,
        _change: &apex_proof::ChangeKind,
    ) -> Result<PromotionStatus, PlatformError> {
        Ok(PromotionStatus::Confirmed)
    }

    /// Whether this adapter can create a real platform-native 90/10 holdout.
    /// Results-fee billing is only possible when it can.
    fn holdout_supported(&self) -> bool {
        false
    }

    /// Open the 10% old-setting holdout after a keep. The default is a typed
    /// Unsupported error: never report a holdout that no platform created.
    async fn start_holdout(&self, request: &HoldoutRequest) -> Result<String, PlatformError> {
        Err(PlatformError::Unsupported {
            platform: self.platform(),
            operation: "start_holdout".to_string(),
            detail: format!(
                "this adapter cannot create the holdout for {}",
                request.experiment_id
            ),
        })
    }

    /// Whether this adapter can supply a registered per-player evidence feed.
    /// A failure or an empty answer from a supporting platform voids; a
    /// platform that documents no such feed legitimately decides on the daily
    /// series.
    fn supports_player_feed(&self) -> bool;

    /// Whether this adapter can produce per-variant daily reporting.
    ///
    /// A platform that documents no per-variant metric (AdMob reports only a
    /// `variantLeader` verdict) returns `false`; the daily pass then never
    /// calls [`experiment_days`](Self::experiment_days) for it and settlement
    /// concludes from the platform verdict instead of failing on a measurement
    /// that cannot exist.
    fn measures_variants(&self) -> bool {
        true
    }

    /// Read the experiment's own configuration/state (not its lift).
    async fn experiment(&self, experiment_id: &str) -> Result<ExperimentResult, PlatformError>;

    /// End a test on an adapter that reports no per-variant measurement and
    /// return the platform verdict: stop it through the platform's documented
    /// end call, then report whether the treatment led. Adapters with
    /// measurement refuse; nothing here produces a lift estimate.
    async fn conclude_unmeasured_experiment(
        &self,
        experiment_id: &str,
    ) -> Result<UnmeasuredConclusion, PlatformError> {
        Err(PlatformError::Validation(format!(
            "conclude_unmeasured_experiment is not supported by this adapter for {experiment_id}"
        )))
    }

    /// Daily reporting pulls for the lift engine; adapters without reporting
    /// access refuse, and the settle path then fails closed.
    async fn experiment_days(&self, experiment_id: &str) -> Result<ExperimentDays, PlatformError> {
        Err(PlatformError::Validation(format!(
            "experiment_days is not supported by this adapter for {experiment_id}"
        )))
    }

    /// Daily reporting pulls keyed by a caller-supplied experiment instance.
    /// The platform experiment id is not always unique per run (MAX returns
    /// the ad unit id), so the durable two-pull evidence must use the
    /// pre-registered experiment instance plus its start day instead. The
    /// default keeps adapters that already have a unique id unchanged.
    async fn experiment_days_for(
        &self,
        experiment_id: &str,
        _instance_key: &str,
    ) -> Result<ExperimentDays, PlatformError> {
        self.experiment_days(experiment_id).await
    }

    /// Per-player revenue evidence for the L1 estimator and the L3 split
    /// check. Adapters without a user-level feed refuse; the settle path then
    /// keeps the daily label-free estimate instead of inventing players.
    async fn experiment_players(
        &self,
        experiment_id: &str,
        _instance_key: &str,
    ) -> Result<PlayerEvidence, PlatformError> {
        Err(PlatformError::Validation(format!(
            "experiment_players is not supported by this adapter for {experiment_id}"
        )))
    }

    /// The platform-reported revenue of one pre-registered scope (ad unit) over
    /// `[start, end]`, in micros. The monthly billing pass bills this scope's
    /// incremental revenue; an adapter without reporting refuses.
    async fn scope_revenue(
        &self,
        ad_unit_id: &str,
        start: &str,
        end: &str,
    ) -> Result<apex_core::Micros, PlatformError> {
        Err(PlatformError::Validation(format!(
            "scope_revenue is not supported by this adapter for {ad_unit_id} ({start}..{end})"
        )))
    }

    /// The scope's current settings, canonically encoded, so the runner can
    /// detect a publisher edit during a test. A read failure on an adapter
    /// that declares [`supports_settings_read`](Self::supports_settings_read)
    /// voids the test (a failure to look is never "no edit"); an adapter that
    /// declares no capability simply leaves the edit check unarmed.
    async fn current_settings(&self, ad_unit_id: &str) -> Result<String, PlatformError> {
        Err(PlatformError::Validation(format!(
            "current_settings is not supported by this adapter for {ad_unit_id}"
        )))
    }

    /// Whether this adapter can read a scope's current settings at all. This
    /// is a capability declaration, not a health check: `false` disarms the
    /// publisher-edit check (nothing to compare against), while `true` arms
    /// it so that a failed read is `Unknown`, never a silent "no edit".
    fn supports_settings_read(&self) -> bool {
        false
    }

    /// Apply the change to all users and end the test: the kept outcome.
    async fn promote_experiment(&self, experiment_id: &str) -> Result<(), PlatformError>;

    /// Revert to the control configuration and end the test: the rollback and
    /// void outcome.
    async fn deprecate_experiment(&self, experiment_id: &str) -> Result<(), PlatformError>;

    /// Apply the change directly to the live unit. Only adapters that cannot
    /// express a test-arm change, that is when `change_in_test_arm` is false.
    async fn apply(&self, action: &Action, idempotency_key: &str) -> Result<(), PlatformError>;

    /// Undo a directly-applied change.
    async fn rollback(&self, action: &Action) -> Result<(), PlatformError>;

    /// Recorded call log (for the write adapters this is the audit trail the
    /// console shows; tests assert exact call sequences on it).
    fn call_log(&self) -> Vec<String>;
}

/// Scripted, fully functional implementation: every behavior is parametrized,
/// nothing is mocked away at the type level.
#[derive(Debug, Default)]
pub struct RecordingOps {
    platform: Mutex<Option<apex_core::PlatformId>>,
    writable: Mutex<std::collections::HashMap<String, bool>>,
    change_in_test_arm: Mutex<bool>,
    measures_variants: Mutex<Option<bool>>,
    holdout_supported: Mutex<Option<bool>>,
    player_feed_capability: Mutex<Option<bool>>,
    settings_read_capability: Mutex<Option<bool>>,
    manual_promotion: Mutex<bool>,
    promotion_status: Mutex<Option<PromotionStatus>>,
    apply_script: Mutex<VecDeque<Result<(), PlatformError>>>,
    start_script: Mutex<VecDeque<Result<String, PlatformError>>>,
    promote_script: Mutex<VecDeque<Result<(), PlatformError>>>,
    deprecate_script: Mutex<VecDeque<Result<(), PlatformError>>>,
    rollback_script: Mutex<VecDeque<Result<(), PlatformError>>>,
    experiments: Mutex<std::collections::HashMap<String, ExperimentResult>>,
    conclusions: Mutex<std::collections::HashMap<String, UnmeasuredConclusion>>,
    conclusion_script: Mutex<VecDeque<Result<UnmeasuredConclusion, PlatformError>>>,
    days: Mutex<std::collections::HashMap<String, ExperimentDays>>,
    days_script: Mutex<VecDeque<Result<ExperimentDays, PlatformError>>>,
    players: Mutex<std::collections::HashMap<String, PlayerEvidence>>,
    players_script: Mutex<VecDeque<Result<PlayerEvidence, PlatformError>>>,
    scope_revenues: Mutex<std::collections::HashMap<String, apex_core::Micros>>,
    day_scope_revenues: Mutex<std::collections::HashMap<(String, String), apex_core::Micros>>,
    current_settings: Mutex<std::collections::HashMap<String, String>>,
    log: Mutex<Vec<String>>,
}

impl RecordingOps {
    pub fn new() -> Self {
        Self::default()
    }

    /// The platform this scripted adapter reports; without an override it is
    /// AppLovin MAX, the platform the default test actions target.
    pub fn set_platform(&self, platform: apex_core::PlatformId) -> &Self {
        if let Ok(mut slot) = self.platform.lock() {
            *slot = Some(platform);
        }
        self
    }

    /// All operations default to writable; mark specific operations verify-live.
    pub fn set_writable(&self, operation: &str, writable: bool) -> &Self {
        if let Ok(mut map) = self.writable.lock() {
            map.insert(operation.to_string(), writable);
        }
        self
    }

    /// Mark this adapter as carrying the change in the experiment's test arm
    /// (the MAX path): no separate apply, and a failed start rolls nothing
    /// back because the live unit was never touched.
    pub fn set_change_in_test_arm(&self, value: bool) -> &Self {
        if let Ok(mut flag) = self.change_in_test_arm.lock() {
            *flag = value;
        }
        self
    }

    /// Mark this adapter as unable to measure per-variant reporting (the AdMob
    /// shape). The settle path then concludes from the scripted conclusion.
    /// Script a manual-promotion adapter (LevelPlay) and its current verdict.
    pub fn set_manual_promotion(&self, status: PromotionStatus) -> &Self {
        if let Ok(mut flag) = self.manual_promotion.lock() {
            *flag = true;
        }
        if let Ok(mut slot) = self.promotion_status.lock() {
            *slot = Some(status);
        }
        self
    }

    /// Change the verdict returned by later promotion reads.
    pub fn set_promotion_status(&self, status: PromotionStatus) -> &Self {
        if let Ok(mut slot) = self.promotion_status.lock() {
            *slot = Some(status);
        }
        self
    }

    pub fn set_measures_variants(&self, value: bool) -> &Self {
        if let Ok(mut flag) = self.measures_variants.lock() {
            *flag = Some(value);
        }
        self
    }

    /// Script whether this adapter supplies a registered per-player feed.
    pub fn set_supports_player_feed(&self, value: bool) -> &Self {
        if let Ok(mut flag) = self.player_feed_capability.lock() {
            *flag = Some(value);
        }
        self
    }

    /// Script whether this adapter can read a scope's current settings. On
    /// `true`, the publisher-edit check arms and a failed (unscripted) read
    /// settles as `Unknown`; on `false` (the default) the check stays unarmed.
    pub fn set_supports_settings_read(&self, value: bool) -> &Self {
        if let Ok(mut flag) = self.settings_read_capability.lock() {
            *flag = Some(value);
        }
        self
    }

    /// Script whether this adapter can open a platform-native holdout.
    /// `false` keeps a kept change on the legacy billing path, which is how a
    /// pre-holdout keep is billed on later months.
    pub fn set_holdout_supported(&self, value: bool) -> &Self {
        if let Ok(mut flag) = self.holdout_supported.lock() {
            *flag = Some(value);
        }
        self
    }

    /// Script the verdict returned after the platform's end call.
    pub fn put_conclusion(&self, id: &str, conclusion: UnmeasuredConclusion) -> &Self {
        if let Ok(mut map) = self.conclusions.lock() {
            map.insert(id.to_string(), conclusion);
        }
        self
    }

    /// Script conclusion results in call order (failures included); the queue
    /// is consumed before the stored conclusions are consulted.
    pub fn script_conclusion(
        &self,
        results: Vec<Result<UnmeasuredConclusion, PlatformError>>,
    ) -> &Self {
        if let Ok(mut q) = self.conclusion_script.lock() {
            *q = results.into();
        }
        self
    }

    pub fn script_apply(&self, results: Vec<Result<(), PlatformError>>) -> &Self {
        if let Ok(mut q) = self.apply_script.lock() {
            *q = results.into();
        }
        self
    }

    pub fn script_start(&self, results: Vec<Result<String, PlatformError>>) -> &Self {
        if let Ok(mut q) = self.start_script.lock() {
            *q = results.into();
        }
        self
    }

    pub fn script_promote(&self, results: Vec<Result<(), PlatformError>>) -> &Self {
        if let Ok(mut q) = self.promote_script.lock() {
            *q = results.into();
        }
        self
    }

    pub fn script_deprecate(&self, results: Vec<Result<(), PlatformError>>) -> &Self {
        if let Ok(mut q) = self.deprecate_script.lock() {
            *q = results.into();
        }
        self
    }

    pub fn script_rollback(&self, results: Vec<Result<(), PlatformError>>) -> &Self {
        if let Ok(mut q) = self.rollback_script.lock() {
            *q = results.into();
        }
        self
    }

    pub fn put_experiment(&self, id: &str, result: ExperimentResult) -> &Self {
        if let Ok(mut map) = self.experiments.lock() {
            map.insert(id.to_string(), result);
        }
        self
    }

    /// Script daily reporting pulls in call order (failures included); the
    /// scripted queue is consumed before the stored days are consulted.
    pub fn script_experiment_days(
        &self,
        results: Vec<Result<ExperimentDays, PlatformError>>,
    ) -> &Self {
        if let Ok(mut q) = self.days_script.lock() {
            *q = results.into();
        }
        self
    }

    /// Script the daily reporting pulls for one experiment.
    pub fn put_experiment_days(&self, id: &str, days: ExperimentDays) -> &Self {
        if let Ok(mut map) = self.days.lock() {
            map.insert(id.to_string(), days);
        }
        self
    }

    /// Script per-player evidence for one experiment.
    pub fn put_experiment_players(&self, id: &str, players: PlayerEvidence) -> &Self {
        if let Ok(mut map) = self.players.lock() {
            map.insert(id.to_string(), players);
        }
        self
    }

    /// Script per-player pulls in call order (failures included).
    pub fn script_experiment_players(
        &self,
        results: Vec<Result<PlayerEvidence, PlatformError>>,
    ) -> &Self {
        if let Ok(mut q) = self.players_script.lock() {
            *q = results.into();
        }
        self
    }

    /// Script the reported revenue of one scope (ad unit) for the billing pass.
    pub fn put_scope_revenue(&self, ad_unit_id: &str, micros: apex_core::Micros) -> &Self {
        if let Ok(mut map) = self.scope_revenues.lock() {
            map.insert(ad_unit_id.to_string(), micros);
        }
        self
    }

    /// Script one exact day's scope revenue, so a quiet (zero-revenue) day in
    /// a holdout window can be confirmed as covered rather than missing.
    pub fn put_day_scope_revenue(
        &self,
        ad_unit_id: &str,
        day: &str,
        micros: apex_core::Micros,
    ) -> &Self {
        if let Ok(mut map) = self.day_scope_revenues.lock() {
            map.insert((ad_unit_id.to_string(), day.to_string()), micros);
        }
        self
    }

    /// Script the scope's current settings, so the runner's publisher-edit
    /// check has something to compare against.
    pub fn put_current_settings(&self, ad_unit_id: &str, settings: &str) -> &Self {
        if let Ok(mut map) = self.current_settings.lock() {
            map.insert(ad_unit_id.to_string(), settings.to_string());
        }
        self
    }
}

fn pop_or_default<T: Clone>(
    queue: &Mutex<VecDeque<Result<T, PlatformError>>>,
    default: Result<T, PlatformError>,
) -> Result<T, PlatformError> {
    match queue.lock() {
        Ok(mut q) => q.pop_front().unwrap_or(default),
        Err(_) => default,
    }
}

fn push_log(log: &Mutex<Vec<String>>, entry: String) {
    if let Ok(mut l) = log.lock() {
        l.push(entry);
    }
}

#[async_trait]
impl PlatformOps for RecordingOps {
    fn platform(&self) -> apex_core::PlatformId {
        self.platform
            .lock()
            .ok()
            .and_then(|slot| *slot)
            .unwrap_or(apex_core::PlatformId::AppLovinMax)
    }

    async fn writable(&self, operation: &str) -> bool {
        push_log(&self.log, format!("writable:{operation}"));
        self.writable
            .lock()
            .ok()
            .and_then(|map| map.get(operation).copied())
            .unwrap_or(true)
    }

    async fn start_experiment(&self, request: &ExperimentRequest) -> Result<String, PlatformError> {
        push_log(&self.log, format!("start:{}", request.experiment_id));
        pop_or_default(&self.start_script, Ok(request.experiment_id.clone()))
    }

    async fn start_null_experiment(&self, spec: &ExperimentSpec) -> Result<String, PlatformError> {
        push_log(&self.log, format!("start_aa:{}", spec.name));
        pop_or_default(&self.start_script, Ok(spec.name.clone()))
    }

    fn supports_player_feed(&self) -> bool {
        self.player_feed_capability
            .lock()
            .ok()
            .and_then(|flag| *flag)
            .unwrap_or(false)
    }

    fn holdout_supported(&self) -> bool {
        self.holdout_supported
            .lock()
            .ok()
            .and_then(|flag| *flag)
            .unwrap_or(true)
    }

    async fn start_holdout(
        &self,
        request: &crate::ops::HoldoutRequest,
    ) -> Result<String, PlatformError> {
        push_log(&self.log, format!("holdout:{}", request.experiment_id));
        Ok(request.experiment_id.clone())
    }

    fn change_in_test_arm(&self) -> bool {
        self.change_in_test_arm
            .lock()
            .map(|flag| *flag)
            .unwrap_or(false)
    }

    fn promotion_requires_manual_action(&self) -> bool {
        self.manual_promotion
            .lock()
            .map(|flag| *flag)
            .unwrap_or(false)
    }

    async fn promotion_status(
        &self,
        _experiment_id: &str,
        _change: &apex_proof::ChangeKind,
    ) -> Result<PromotionStatus, PlatformError> {
        Ok(self
            .promotion_status
            .lock()
            .ok()
            .and_then(|slot| *slot)
            .unwrap_or(PromotionStatus::Confirmed))
    }

    fn measures_variants(&self) -> bool {
        self.measures_variants
            .lock()
            .ok()
            .and_then(|flag| *flag)
            .unwrap_or(true)
    }

    async fn experiment(&self, experiment_id: &str) -> Result<ExperimentResult, PlatformError> {
        push_log(&self.log, format!("experiment:{experiment_id}"));
        match self.experiments.lock() {
            Ok(map) => map
                .get(experiment_id)
                .cloned()
                .ok_or(PlatformError::Schema {
                    platform: apex_core::PlatformId::Admob,
                    operation: "experiment".into(),
                    detail: format!("no scripted result for {experiment_id}"),
                }),
            Err(_) => Err(PlatformError::Network {
                platform: apex_core::PlatformId::Admob,
                operation: "experiment".into(),
                detail: "lock poisoned".into(),
            }),
        }
    }

    async fn conclude_unmeasured_experiment(
        &self,
        experiment_id: &str,
    ) -> Result<UnmeasuredConclusion, PlatformError> {
        push_log(&self.log, format!("conclude:{experiment_id}"));
        if let Ok(mut script) = self.conclusion_script.lock() {
            if let Some(result) = script.pop_front() {
                return result;
            }
        }
        match self.conclusions.lock() {
            Ok(map) => map
                .get(experiment_id)
                .cloned()
                .ok_or(PlatformError::Schema {
                    platform: apex_core::PlatformId::Admob,
                    operation: "concludeUnmeasuredExperiment".into(),
                    detail: format!("no scripted conclusion for {experiment_id}"),
                }),
            Err(_) => Err(PlatformError::Network {
                platform: apex_core::PlatformId::Admob,
                operation: "concludeUnmeasuredExperiment".into(),
                detail: "lock poisoned".into(),
            }),
        }
    }

    async fn experiment_days(&self, experiment_id: &str) -> Result<ExperimentDays, PlatformError> {
        push_log(&self.log, format!("days:{experiment_id}"));
        if let Ok(mut script) = self.days_script.lock() {
            if let Some(result) = script.pop_front() {
                return result;
            }
        }
        match self.days.lock() {
            Ok(map) => map
                .get(experiment_id)
                .cloned()
                .ok_or(PlatformError::Schema {
                    platform: apex_core::PlatformId::Admob,
                    operation: "experimentDays".into(),
                    detail: format!("no scripted days for {experiment_id}"),
                }),
            Err(_) => Err(PlatformError::Network {
                platform: apex_core::PlatformId::Admob,
                operation: "experimentDays".into(),
                detail: "lock poisoned".into(),
            }),
        }
    }

    async fn experiment_players(
        &self,
        experiment_id: &str,
        _instance_key: &str,
    ) -> Result<PlayerEvidence, PlatformError> {
        push_log(&self.log, format!("players:{experiment_id}"));
        if let Ok(mut script) = self.players_script.lock() {
            if let Some(result) = script.pop_front() {
                return result;
            }
        }
        Ok(self
            .players
            .lock()
            .ok()
            .and_then(|map| map.get(experiment_id).cloned())
            .unwrap_or_default())
    }

    async fn scope_revenue(
        &self,
        ad_unit_id: &str,
        start: &str,
        _end: &str,
    ) -> Result<apex_core::Micros, PlatformError> {
        push_log(&self.log, format!("scope_revenue:{ad_unit_id}"));
        if let Ok(map) = self.day_scope_revenues.lock() {
            if let Some(micros) = map.get(&(ad_unit_id.to_string(), start.to_string())) {
                return Ok(*micros);
            }
        }
        match self.scope_revenues.lock() {
            Ok(map) => map.get(ad_unit_id).copied().ok_or(PlatformError::Schema {
                platform: apex_core::PlatformId::Admob,
                operation: "scopeRevenue".into(),
                detail: format!("no scripted scope revenue for {ad_unit_id}"),
            }),
            Err(_) => Err(PlatformError::Network {
                platform: apex_core::PlatformId::Admob,
                operation: "scopeRevenue".into(),
                detail: "lock poisoned".into(),
            }),
        }
    }

    fn supports_settings_read(&self) -> bool {
        // The simulator follows its capability-flag convention (see
        // `set_supports_player_feed`): the settings read is off until a test
        // opts in, so fixture settles are never voided by an unscripted read.
        self.settings_read_capability
            .lock()
            .ok()
            .and_then(|flag| *flag)
            .unwrap_or(false)
    }

    async fn current_settings(&self, ad_unit_id: &str) -> Result<String, PlatformError> {
        push_log(&self.log, format!("current_settings:{ad_unit_id}"));
        match self.current_settings.lock() {
            Ok(map) => map.get(ad_unit_id).cloned().ok_or(PlatformError::Schema {
                platform: apex_core::PlatformId::Admob,
                operation: "currentSettings".into(),
                detail: format!("no scripted settings for {ad_unit_id}"),
            }),
            Err(_) => Err(PlatformError::Network {
                platform: apex_core::PlatformId::Admob,
                operation: "currentSettings".into(),
                detail: "lock poisoned".into(),
            }),
        }
    }

    async fn promote_experiment(&self, experiment_id: &str) -> Result<(), PlatformError> {
        push_log(&self.log, format!("promote:{experiment_id}"));
        pop_or_default(&self.promote_script, Ok(()))
    }

    async fn deprecate_experiment(&self, experiment_id: &str) -> Result<(), PlatformError> {
        push_log(&self.log, format!("deprecate:{experiment_id}"));
        pop_or_default(&self.deprecate_script, Ok(()))
    }

    async fn apply(&self, action: &Action, idempotency_key: &str) -> Result<(), PlatformError> {
        push_log(
            &self.log,
            format!("apply:{}:{idempotency_key}", key_of(action)),
        );
        pop_or_default(&self.apply_script, Ok(()))
    }

    async fn rollback(&self, action: &Action) -> Result<(), PlatformError> {
        push_log(&self.log, format!("rollback:{}", key_of(action)));
        pop_or_default(&self.rollback_script, Ok(()))
    }

    fn call_log(&self) -> Vec<String> {
        self.log.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

/// Short human tag for logs (stable across runs).
fn key_of(action: &Action) -> String {
    match action {
        Action::SetCountryFloor {
            ad_unit_id,
            country,
            ..
        } => format!("floor:{ad_unit_id}:{country}"),
        Action::SetMappingEcpm {
            mediation_group_id,
            ad_unit_id,
            ..
        } => {
            format!("ecpm:{mediation_group_id}:{ad_unit_id}")
        }
        Action::SetWaterfallPrice {
            ad_unit_id,
            entry_id,
            ..
        } => format!("price:{ad_unit_id}:{entry_id}"),
        Action::CreateWaterfallEntry {
            ad_unit_id,
            network,
            ..
        } => format!("create:{ad_unit_id}:{network}"),
        Action::RemoveWaterfallEntry {
            ad_unit_id,
            entry_id,
        } => format!("remove:{ad_unit_id}:{entry_id}"),
        Action::SetLevelplayInstancePrice {
            instance_id,
            price_micros,
        } => format!("instance-price:{instance_id}:{price_micros}"),
        Action::SetAdmobMediationFloor {
            group_id,
            floor_micros,
        } => format!("mediation-floor:{group_id}:{floor_micros}"),
        Action::SetMaxFrequencyCap {
            ad_unit_id,
            cap_type,
            ..
        } => format!("frequency-cap:{ad_unit_id}:{}", cap_type.as_str()),
        Action::SetLevelplayGroupCapping { group_id, .. } => {
            format!("group-capping:{group_id}")
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use apex_core::{ExperimentResult, ExperimentSpec};

    /// An adapter with no reporting access: the default method must refuse.
    struct BareOps;

    #[async_trait]
    impl PlatformOps for BareOps {
        fn supports_player_feed(&self) -> bool {
            false
        }

        async fn writable(&self, _operation: &str) -> bool {
            false
        }
        async fn start_experiment(
            &self,
            _request: &ExperimentRequest,
        ) -> Result<String, PlatformError> {
            Ok(String::new())
        }
        async fn experiment(&self, _id: &str) -> Result<ExperimentResult, PlatformError> {
            Err(PlatformError::Validation("none".into()))
        }
        async fn promote_experiment(&self, _id: &str) -> Result<(), PlatformError> {
            Ok(())
        }
        async fn deprecate_experiment(&self, _id: &str) -> Result<(), PlatformError> {
            Ok(())
        }
        async fn apply(&self, _action: &Action, _key: &str) -> Result<(), PlatformError> {
            Ok(())
        }
        async fn rollback(&self, _action: &Action) -> Result<(), PlatformError> {
            Ok(())
        }
        fn call_log(&self) -> Vec<String> {
            Vec::new()
        }
    }

    fn arm_day(day: &str, revenue: i64) -> apex_lift::window::ArmDay {
        apex_lift::window::ArmDay {
            day: day.to_string(),
            revenue,
            users: None,
            second_pull_revenue: Some(revenue),
            second_pull_users: None,
            fill: None,
            second_pull_fill: None,
            first_observed_at_unix_s: Some(1_700_000_000),
            second_observed_at_unix_s: Some(
                1_700_000_000 + apex_lift::window::SECOND_PULL_MIN_SECS,
            ),
            first_source_sha256: Some("a".repeat(64)),
            second_source_sha256: Some("b".repeat(64)),
        }
    }

    #[tokio::test]
    async fn adapters_without_reporting_refuse_instead_of_fabricating() {
        let err = BareOps.experiment_days("exp-1").await.expect_err("refuse");
        assert!(err.to_string().contains("not supported"), "{err}");
    }

    #[tokio::test]
    async fn the_default_measurement_capability_is_on_and_bare_conclusion_refuses() {
        assert!(BareOps.measures_variants());
        let err = BareOps
            .conclude_unmeasured_experiment("exp-1")
            .await
            .expect_err("an adapter without the capability must not conclude");
        assert!(err.to_string().contains("not supported"), "{err}");
    }

    #[tokio::test]
    async fn bare_ops_every_method_is_reachable() {
        assert!(!BareOps.writable("op").await);
        assert!(BareOps.experiment("x").await.is_err());
        assert!(BareOps.promote_experiment("x").await.is_ok());
        assert!(BareOps.deprecate_experiment("x").await.is_ok());
        let action = Action::RemoveWaterfallEntry {
            ad_unit_id: "u".into(),
            entry_id: "e".into(),
        };
        assert!(BareOps.apply(&action, "k").await.is_ok());
        assert!(BareOps.rollback(&action).await.is_ok());
        assert!(BareOps.call_log().is_empty());
        assert!(BareOps
            .start_experiment(&ExperimentRequest {
                experiment_id: "e".into(),
                spec: ExperimentSpec {
                    name: "e".into(),
                    instance_id: String::new(),
                    ad_unit_id: "u".into(),
                    metric: apex_core::ExperimentMetric::Revenue,
                    variants: 2,
                    change_description: "c".into(),
                    test_group_allocation: 50,
                },
                change: apex_proof::ChangeKind::CountryFloorSet {
                    country: "EE".into(),
                    floor_ecpm: 1,
                },
                prev_value: "{}".into(),
            })
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn default_capabilities_refuse_instead_of_guessing() {
        // The trait defaults must fail closed: a bare adapter cannot read a
        // scope's revenue, read its settings, or start an A/A run.
        let err = BareOps
            .scope_revenue("u1", "2026-10-01", "2026-10-31")
            .await
            .expect_err("refuse");
        assert!(err.to_string().contains("not supported"), "{err}");
        assert!(BareOps.current_settings("u1").await.is_err());
        let spec = ExperimentSpec {
            name: "apex-aa".into(),
            instance_id: String::new(),
            ad_unit_id: "u1".into(),
            metric: apex_core::ExperimentMetric::Revenue,
            variants: 2,
            change_description: "aa".into(),
            test_group_allocation: 50,
        };
        assert!(BareOps.start_null_experiment(&spec).await.is_err());
    }

    #[tokio::test]
    async fn every_action_shape_logs_a_stable_key() {
        let ops = RecordingOps::new();
        let actions = [
            Action::SetCountryFloor {
                ad_unit_id: "u".into(),
                country: "EE".into(),
                floor_ecpm: 1,
            },
            Action::SetMappingEcpm {
                mediation_group_id: "g".into(),
                ad_unit_id: "u".into(),
                ecpm: 2,
            },
            Action::SetWaterfallPrice {
                ad_unit_id: "u".into(),
                entry_id: "e".into(),
                price_ecpm: 3,
            },
            Action::CreateWaterfallEntry {
                ad_unit_id: "u".into(),
                network: "n".into(),
                instance_name: "i".into(),
                price_ecpm: 4,
            },
            Action::RemoveWaterfallEntry {
                ad_unit_id: "u".into(),
                entry_id: "e".into(),
            },
        ];
        for action in &actions {
            ops.apply(action, "k").await.expect("apply");
            ops.rollback(action).await.expect("rollback");
        }
        let log = ops.call_log();
        assert_eq!(log.len(), actions.len() * 2);
        assert!(log.iter().any(|entry| entry == "apply:floor:u:EE:k"));
        assert!(log.iter().any(|entry| entry == "apply:create:u:n:k"));
        assert!(log.iter().any(|entry| entry == "rollback:remove:u:e"));
    }

    #[tokio::test]
    async fn scripted_promote_deprecate_and_writable_flags_are_honored() {
        let ops = RecordingOps::new();
        ops.set_writable("updateEntryPrice", false);
        assert!(!ops.writable("updateEntryPrice").await);
        assert!(ops.writable("anything-else").await);
        ops.script_promote(vec![Err(PlatformError::Validation("no".into()))]);
        assert!(ops.promote_experiment("exp").await.is_err());
        ops.script_deprecate(vec![Err(PlatformError::Validation("no".into()))]);
        assert!(ops.deprecate_experiment("exp").await.is_err());
    }

    #[tokio::test]
    async fn recording_ops_returns_scripted_days_and_logs_the_pull() {
        let ops = RecordingOps::new();
        ops.put_experiment_days(
            "exp-1",
            ExperimentDays {
                control: vec![arm_day("2026-10-01", 1_000_000)],
                test: vec![arm_day("2026-10-01", 1_060_000)],
            },
        );
        let days = ops.experiment_days("exp-1").await.expect("days");
        assert_eq!(days.control.len(), 1);
        assert_eq!(days.test[0].revenue, 1_060_000);
        assert!(ops.call_log().iter().any(|line| line == "days:exp-1"));
        assert!(ops.experiment_days("missing").await.is_err());
    }
}
