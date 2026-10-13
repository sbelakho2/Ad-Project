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

pub mod keys;
pub mod model;
pub mod ops;
pub mod plan;
pub mod settle;

pub use keys::{experiment_name, idempotency_key, platform_of};
pub use model::{
    Action, ActionOutcome, AutopilotPolicy, CycleOutcome, PlanError, Proposal, SettleOutcome,
};
pub use ops::{PlatformOps, RecordingOps};
pub use plan::run_cycle;
pub use settle::settle_experiment;

#[cfg(test)]
mod tests;
