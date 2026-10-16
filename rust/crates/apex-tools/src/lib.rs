//! apex-tools — the all-Rust replacement for the JS gate stack. One binary,
//! subcommands mirroring the retired scripts (see `--help`). Data tables that
//! used to live in JS are read from tools/coverage/*.json; behaviour parity is
//! pinned by unit tests ported from the JS suites.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

/// Every subcommand the CLI accepts, in usage order. The pipeline inventory
/// check asserts that the Woodpecker lanes and the Makefile never reference
/// anything else.
pub const SUBCOMMANDS: &[&str] = &[
    "manifest",
    "walker-parity",
    "orphan-gate",
    "floor-ratchet",
    "floors-sync",
    "sync-floors",
    "assertion-presence",
    "stub-ban",
    "mutation-smoke",
    "mutation-floor",
    "yaml-policy",
    "aggregate-verdict",
    "lcov-gate",
    "contract",
    "docs-links",
    "css-lint",
    "claims",
    "perf-budget",
    "console-a11y",
    "docs-lint",
    "dogfood-plan",
];

pub mod a11y;
pub mod baseline;
pub mod claims;
pub mod classify;
pub mod contract;
pub mod coverage;
pub mod csslint;
pub mod docs_lint;
pub mod docslinks;
pub mod dogfood;
pub mod dogfood_plan;
pub mod glob;
pub mod inventory;
pub mod mutation;
pub mod perfbudget;
pub mod policy;
pub mod repo;
pub mod util;

pub use repo::Repo;
