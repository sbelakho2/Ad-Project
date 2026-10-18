//! apex-autopilot — the propose/apply/experiment/keep-or-rollback loop.
//!
//! Anchored rules (each pinned by tests):
//! - a change never ships without a platform-native experiment; if the platform
//!   cannot start the experiment, the change is rolled back immediately,
//! - a Critical audit result blackouts all writes (policy-configurable),
//! - the daily cap is counted from the signed ledger, never from caller state,
//! - verify-live endpoints refuse before any side effect,
//! - billing proofs exist only for platform-proven positive lift.

#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )
)]

pub mod alerts;
pub mod audit_job;
pub mod calendar;
pub mod cli;
pub mod cycle;
pub mod events;
pub mod keys;
pub(crate) mod ledger_index;
pub mod live_admob;
pub mod live_levelplay;
pub mod live_max;
pub mod model;
pub mod notify;
pub mod operator_jobs;
pub mod ops;
pub mod payment_events;
pub mod plan;
pub mod projections_job;
pub mod proposals;
pub mod registry;
pub mod runner;
pub mod settle;
pub mod telemetry;
pub mod text;
pub mod verify;

pub use alerts::{Alert, AlertSink, FileAlertSink, NoopAlertSink, RecordingAlertSink};
pub use audit_job::{
    account_snapshot, admob_account_snapshot, audited_platform_snapshot, audited_snapshot,
    levelplay_account_snapshot, max_account_snapshot, write_snapshot, SnapshotAccount,
};
pub use cycle::CycleJobReport;
pub use events::RunnerEvents;
pub use keys::{experiment_name, idempotency_key, platform_of};
pub use model::{
    Action, ActionOutcome, AutopilotPolicy, CycleOutcome, PlanError, Proposal, SettleOutcome,
};
pub use notify::NotifyPlan;
pub use operator_jobs::{consume_operator_jobs, OperatorJob, OperatorJobsReport};
pub use ops::{PlatformOps, RecordingOps};
pub use payment_events::{PaymentLedgerRow, PaymentsReport, ScheduledRetry};
pub use plan::{run_cycle, run_cycle_observed};
pub use proposals::floor_proposals;
pub use registry::{
    PublisherRegistry, RegistryEntry, RegistryError, RunnablePublisher, SkipReason,
    SkippedPublisher,
};
pub use runner::{DailyReport, MonthlyReport, Runner};
pub use settle::settle_experiment;
pub use telemetry::RunnerTelemetry;
pub use verify::{FileLedgerVerifier, LedgerCheck, LedgerVerifier};

#[cfg(test)]
mod tests;
