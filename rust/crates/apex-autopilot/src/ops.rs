//! Platform operations abstraction + a fully-real scripted implementation used
//! by integration tests and by dry-run deployments.

use std::collections::VecDeque;
use std::sync::Mutex;

use apex_core::{ExperimentResult, ExperimentSpec, PlatformError};
use async_trait::async_trait;

use crate::model::Action;

#[async_trait]
pub trait PlatformOps: Send + Sync {
    /// Verify-live gate: may the autopilot write through this operation?
    async fn writable(&self, operation: &str) -> bool;
    async fn apply(&self, action: &Action, idempotency_key: &str) -> Result<(), PlatformError>;
    async fn start_experiment(&self, spec: &ExperimentSpec) -> Result<String, PlatformError>;
    async fn experiment(&self, experiment_id: &str) -> Result<ExperimentResult, PlatformError>;
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
    apply_script: Mutex<VecDeque<Result<(), PlatformError>>>,
    start_script: Mutex<VecDeque<Result<String, PlatformError>>>,
    rollback_script: Mutex<VecDeque<Result<(), PlatformError>>>,
    experiments: Mutex<std::collections::HashMap<String, ExperimentResult>>,
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

    async fn apply(&self, action: &Action, idempotency_key: &str) -> Result<(), PlatformError> {
        push_log(
            &self.log,
            format!("apply:{}:{idempotency_key}", key_of(action)),
        );
        pop_or_default(&self.apply_script, Ok(()))
    }

    async fn start_experiment(&self, spec: &ExperimentSpec) -> Result<String, PlatformError> {
        push_log(&self.log, format!("start:{}", spec.name));
        pop_or_default(&self.start_script, Ok("exp-default".to_string()))
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
