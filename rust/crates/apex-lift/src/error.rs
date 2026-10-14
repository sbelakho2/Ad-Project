//! Typed refusals for the lift engine. Every variant is a refusal; none
//! carries unbounded attacker data.

/// A lift-engine refusal.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LiftError {
    /// Malformed or impossible input (dates, identifiers, records).
    #[error("malformed input: {0}")]
    Malformed(String),

    /// A record or statistic carried a non-finite number and must not be
    /// hashed or billed.
    #[error("non-finite value in {context}")]
    NonFinite { context: String },
}

/// Convenience alias for engine results.
pub type LiftResult<T> = Result<T, LiftError>;
