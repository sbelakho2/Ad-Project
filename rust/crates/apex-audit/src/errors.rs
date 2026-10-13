//! Boundary errors. The audit engine itself is infallible
//! ([`crate::engine::run`]); this error exists only at the report-serialization
//! boundary used by the API layer.

#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error("serialize audit report: {0}")]
    Serialize(#[from] serde_json::Error),
}
