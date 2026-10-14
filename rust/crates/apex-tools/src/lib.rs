//! apex-tools — the all-Rust replacement for the JS gate stack. One binary,
//! subcommands mirroring the retired scripts (see `--help`). Data tables that
//! used to live in JS are read from tools/coverage/*.json; behaviour parity is
//! pinned by unit tests ported from the JS suites.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod a11y;
pub mod claims;
pub mod classify;
pub mod contract;
pub mod coverage;
pub mod csslint;
pub mod docs_lint;
pub mod docslinks;
pub mod dogfood;
pub mod glob;
pub mod i18n;
pub mod mutation;
pub mod perfbudget;
pub mod policy;
pub mod repo;
pub mod util;

pub use repo::Repo;
