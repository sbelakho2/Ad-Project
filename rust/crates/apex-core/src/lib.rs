//! apex-core — the domain contract layer for the no-SDK autopilot.
//!
//! Everything above this crate (connectors, audit, autopilot, proof, billing, api)
//! depends on these types and invariants. The TypeScript layer in
//! `backend/src/services/platforms` is the behavioral reference; fixtures and
//! idempotency-key formats are ported 1:1 and pinned by tests here.
//!
//! Money rule (stricter than the TS reference, on purpose): all currency amounts are
//! integer micros (1e-6 of a currency unit). There is no floating-point money anywhere
//! in this workspace; float parsing from platform payloads goes through explicit
//! decimal conversion in the connectors crate.

#![forbid(unsafe_code)]
// Production code is held to the strict workspace lints; tests may unwrap/expect/panic
// because a panicking test IS a failing test.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used, clippy::panic))]

pub mod endpoint;
pub mod errors;
pub mod money;
pub mod platform;
pub mod validation;

pub use endpoint::{EndpointMeta, SourceVerification};
pub use errors::PlatformError;
pub use money::{
    ecpm_micros, lift_basis_points, lift_fee_micros, Micros, MAX_LIFT_RATE_BP, MICROS_PER_UNIT,
    MIN_LIFT_RATE_BP,
};
pub use platform::{
    AdFormat, AdUnit, CountryFloor, ExperimentMetric, ExperimentResult, ExperimentSpec,
    ExperimentState, ExperimentVariantResult, Mapping, MediationGroup, MediationMapping,
    PlatformId, WaterfallEntry,
};
pub use validation::{
    admob_ecpm_key, format_micros_short, levelplay_create_key, levelplay_price_key,
    levelplay_remove_key, max_experiment_key, max_floor_key, normalize_country, validate_ecpm,
    validate_floor,
};
