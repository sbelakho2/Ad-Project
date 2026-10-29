//! apex-audit — the pure, deterministic "revenue leak" audit engine.
//!
//! The engine takes an immutable [`Snapshot`] of a publisher's mediation state
//! (as read from MAX / LevelPlay / AdMob management APIs by `apex-connectors`)
//! and produces [`Finding`]s with exact, machine-checkable evidence.
//!
//! ## Purity contract
//!
//! - No I/O, no clocks, no randomness: every rule is a pure function over a
//!   [`Snapshot`]. `fetched_at_unix_ms` is data, never read from a clock here.
//! - [`engine::run`] is infallible **by design**: connectors are responsible for
//!   fail-closed parsing and for building a `Snapshot` only from well-formed
//!   platform responses. The snapshot types make every malformed-but-real input
//!   (zero impressions, negative revenue, empty files) representable, so the
//!   engine never has to reject anything — it reports over whatever it is given.
//!
//! ## Determinism
//!
//! Findings are sorted by severity (Critical, then Warning, then Info), then by
//! the stable [`RuleId`] wire string, then by `subject`. Each rule emits at most
//! one finding per subject, so the sort is total and the report is identical
//! for permuted input vectors (only [`AuditReport::snapshot_digest`] is
//! order-sensitive: it digests the snapshot value as given, including vector
//! order).
//!
//! `snapshot_digest` is the lowercase hex SHA-256 of the canonical JSON of the
//! snapshot: compact `serde_json` output of the typed structs. Struct fields
//! serialize in declaration order, there are no maps and no floating-point
//! values anywhere in a snapshot, so the byte stream is fully determined by the
//! field values and vector order. (An impossible serialization failure — only
//! reachable if a future field type is added that cannot be serialized at all —
//! falls back to the derived `Debug` representation instead of panicking.)
//!
//! ## Recovery model
//!
//! `estimated_monthly_recovery` is always an explicit arithmetic model, never a
//! marketing number. `ecpm_micros` follows [`apex_core::ecpm_micros`]: revenue
//! micros **per 1000 impressions**. Recovery for `n` affected impressions is
//! therefore `ecpm * n / 1000`, not `ecpm * n`; the shorthand "ecpm ×
//! impressions" from the product spec assumed per-impression units and would
//! overstate the claim by 1000×. Each model is documented on the rule and
//! pinned by a golden test with hand-computed micros.
//!
//! When a rule cannot even *estimate* (no impressions, zero or negative eCPM —
//! i.e. no realistic basis to claim money), recovery is 0 and the message says
//! so. Rules that are purely informational (reporting gap, app-ads.txt) always
//! have recovery 0 and their messages must not claim money.
//!
//! ## Message / evidence parity
//!
//! Every number that appears in a [`Finding::message`] as a standalone numeric
//! token is also present as a number in [`Finding::evidence`]. Prose that
//! contains no numbers (network names, countries) is free-form. This is
//! enforced by an adversarial test over crafted snapshots covering all five
//! rules (identifiers such as `u1` or `30d`-style shorthand are deliberately
//! avoided in messages; the parity scanner treats digit runs glued to letters
//! as identifiers, and the crafted fixtures keep ids digit-free).

#![forbid(unsafe_code)]
// Production code is held to the strict workspace lints; tests may unwrap/expect/panic
// because a panicking test IS a failing test.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used, clippy::panic))]

pub mod digest;
pub mod engine;
pub mod errors;
pub mod model;
pub mod rules;
#[cfg(test)]
pub(crate) mod testkit;
#[cfg(test)]
mod tests;

pub use digest::snapshot_digest;
pub use engine::{report_json, run};
pub use errors::AuditError;
pub use model::{
    AppAdsTxt, AuditReport, BidderAccount, CountryRevenueRow, DailyRevenueRow, Finding,
    RevenueTotals, RuleId, Severity, Snapshot, GENERATED_BY,
};
