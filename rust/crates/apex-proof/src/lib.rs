//! apex-proof — tamper-evident ledger for the autopilot and billing proofs.
//!
//! The ledger is an append-only chain of [`LedgerRecord`]s. Each record is
//! sealed with a SHA-256 `hash` over its canonical JSON (with the `hash` field
//! removed) and linked to its predecessor's hash via `prevHash`; each persisted
//! line additionally carries an ed25519 `signature` over the record hash and
//! the signer's `pubkey`:
//!
//! ```text
//! {"record":{...},"signature":"<128 hex>","pubkey":"<64 hex>"}\n
//! ```
//!
//! ## Guarantees
//!
//! - **Fail closed:** [`Ledger::open`] and every [`Ledger::append`] verify the
//!   whole on-disk chain first. A tampered, truncated or foreign-key ledger is
//!   never appended to, and the refusal names the first offending line.
//! - **Tamper evidence:** [`Ledger::verify`] rejects (typed errors naming the
//!   first offending line) payload/seq/prevHash/hash tampering, bad
//!   signatures, truncated final lines, swapped or reordered lines, foreign
//!   public keys, sequence gaps/duplicates and genesis breaks.
//! - **Determinism:** the canonical JSON form is pinned by a byte-level test
//!   and a hex digest snapshot, so the same record always hashes to the same
//!   bytes on any machine.
//! - **Money safety:** billing proofs are refused unless they are provable, and
//!   the fee is recomputed from `apex-core` arithmetic, never trusted from the
//!   caller (see [`build_billing_proof`]).
//!
//! ## Scope
//!
//! Timestamps and sequence numbers are data; this crate never reads a clock.
//! Rollbacks are records, not deletions: the ledger is append-only by
//! construction ([`Ledger::append_rollback`] refuses unknown targets,
//! rollback-of-rollback and double rollbacks).

#![forbid(unsafe_code)]
// Production code is held to the strict workspace lints; tests may unwrap/expect/panic
// because a panicking test IS a failing test.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used, clippy::panic))]

mod billing;
mod chain;
mod errors;
mod ledger;
mod memory;
mod record;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod testutil;

pub use billing::build_billing_proof;
pub use errors::LedgerError;
pub use ledger::Ledger;
pub use memory::InMemoryLedger;
pub use record::{ChangeKind, LedgerRecord};

pub use ed25519_dalek::{Signature, SigningKey, VerifyingKey};

/// The ed25519 signing key the ledgers use.
///
/// ed25519-dalek v2 names this type `SigningKey`; the alias keeps the product
/// spec's `Keypair` vocabulary (and its deterministic
/// `Keypair::from_bytes(&[u8; 32])` constructor) working.
pub type Keypair = SigningKey;
