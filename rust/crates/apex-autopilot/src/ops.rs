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

#[async_trait]
pub trait PlatformOps: Send + Sync {
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

    /// Read the experiment's own configuration/state (not its lift).
    async fn experiment(&self, experiment_id: &str) -> Result<ExperimentResult, PlatformError>;

    /// Daily reporting pulls for the lift engine; adapters without reporting
    /// access refuse, and the settle path then fails closed.
    async fn experiment_days(&self, experiment_id: &str) -> Result<ExperimentDays, PlatformError> {
        Err(PlatformError::Validation(format!(
            "experiment_days is not supported by this adapter for {experiment_id}"
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
    /// detect a publisher edit during a test. An adapter that cannot read the
    /// scope refuses; the runner then skips the edit check rather than voiding
    /// on a failure to look.
    async fn current_settings(&self, ad_unit_id: &str) -> Result<String, PlatformError> {
        Err(PlatformError::Validation(format!(
            "current_settings is not supported by this adapter for {ad_unit_id}"
        )))
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
    writable: Mutex<std::collections::HashMap<String, bool>>,
    change_in_test_arm: Mutex<bool>,
    apply_script: Mutex<VecDeque<Result<(), PlatformError>>>,
    start_script: Mutex<VecDeque<Result<String, PlatformError>>>,
    promote_script: Mutex<VecDeque<Result<(), PlatformError>>>,
    deprecate_script: Mutex<VecDeque<Result<(), PlatformError>>>,
    rollback_script: Mutex<VecDeque<Result<(), PlatformError>>>,
    experiments: Mutex<std::collections::HashMap<String, ExperimentResult>>,
    days: Mutex<std::collections::HashMap<String, ExperimentDays>>,
    days_script: Mutex<VecDeque<Result<ExperimentDays, PlatformError>>>,
    scope_revenues: Mutex<std::collections::HashMap<String, apex_core::Micros>>,
    current_settings: Mutex<std::collections::HashMap<String, String>>,
    log: Mutex<Vec<String>>,
}

impl RecordingOps {
    pub fn new() -> Self {
        Self::default()
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

    /// Script the reported revenue of one scope (ad unit) for the billing pass.
    pub fn put_scope_revenue(&self, ad_unit_id: &str, micros: apex_core::Micros) -> &Self {
        if let Ok(mut map) = self.scope_revenues.lock() {
            map.insert(ad_unit_id.to_string(), micros);
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

    fn change_in_test_arm(&self) -> bool {
        self.change_in_test_arm
            .lock()
            .map(|flag| *flag)
            .unwrap_or(false)
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

    async fn scope_revenue(
        &self,
        ad_unit_id: &str,
        _start: &str,
        _end: &str,
    ) -> Result<apex_core::Micros, PlatformError> {
        push_log(&self.log, format!("scope_revenue:{ad_unit_id}"));
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
        }
    }

    #[tokio::test]
    async fn adapters_without_reporting_refuse_instead_of_fabricating() {
        let err = BareOps.experiment_days("exp-1").await.expect_err("refuse");
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
