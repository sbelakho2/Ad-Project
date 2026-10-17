//! The autopilot cycle boundary and this deployment's read-only runner.
//!
//! The `apex-autopilot` crate owns the write path (policy evaluation, platform
//! mutations, experiment bookkeeping) and is deployed separately. The API
//! talks to it through [`CycleRunner`]: a small, typed seam whose contract is
//! "given a publisher, a platform and validated policy inputs, produce the
//! cycle summary or a typed refusal". Wiring a write-capable deployment means
//! implementing this trait over the autopilot; nothing else in the API changes.
//!
//! This crate ships [`LedgerOnlyRunner`], the real runner for the *read-only
//! console* deployment mode: it re-audits the named platform, reads (and
//! re-verifies) the publisher's ledger, and reports the current state. It never
//! mutates anything — it holds no platform credentials, and the response says
//! so with `"mode":"read_only"` plus an explicit note, rather than styling the
//! absence of a write path as an error.

use std::sync::Arc;

use apex_audit::Severity;
use apex_core::{PlatformId, MAX_LIFT_RATE_BP, MIN_LIFT_RATE_BP};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::sources::{AuditSource, LedgerSource};

/// The mode string every cycle response carries in this deployment.
pub const READ_ONLY_MODE: &str = "read_only";

/// The operator-facing explanation of what read-only mode means.
pub const READ_ONLY_NOTE: &str =
    "read-only deployment: the cycle audits the platform snapshot and \
     reports the verified ledger state without mutating anything; applying write cycles requires \
     platform credentials that this deployment does not hold";

/// Policy inputs a caller may attach to a cycle request.
///
/// They are validated before any work happens: an impossible lift rate or a
/// negative fee ceiling is a `400 invalid_policy`, never a silent fallback.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CyclePolicy {
    /// Requested minimum proven-lift rate in basis points; must lie in the
    /// business band `[1500, 2000]` (15.00%..=20.00%) when present.
    #[serde(default)]
    pub min_lift_rate_bp: Option<u16>,
    /// Requested fee ceiling in micros; must not be negative when present.
    #[serde(default)]
    pub max_fee_micros: Option<i64>,
}

/// The optional JSON body of `POST /v1/cycle/{publisher}`. An empty body is
/// the default request; unknown fields are refused so typos cannot silently
/// drop policy inputs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CycleRequest {
    /// Validated-before-use policy inputs.
    #[serde(default)]
    pub policy: Option<CyclePolicy>,
}

/// The typed result of one cycle, as served by the API. Field names are the
/// pinned snake_case wire contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CycleSummary {
    /// Always [`READ_ONLY_MODE`] for [`LedgerOnlyRunner`].
    pub mode: &'static str,
    /// The publisher the cycle ran for.
    pub publisher: String,
    /// Number of records in the publisher's verified ledger.
    pub records: usize,
    /// Critical findings in the platform audit that accompanied the cycle.
    pub critical_findings: usize,
    /// Human-readable statement of what this mode can and cannot do.
    pub note: &'static str,
}

/// A runner that can produce a cycle summary for one publisher/platform.
#[async_trait]
pub trait CycleRunner: Send + Sync + std::fmt::Debug {
    /// Validate the request policy, then run the cycle.
    async fn run(
        &self,
        publisher: &str,
        platform: PlatformId,
        request: CycleRequest,
    ) -> Result<CycleSummary, ApiError>;
}

/// The read-only runner: audit + verified ledger state, no mutations.
#[derive(Debug)]
pub struct LedgerOnlyRunner {
    audit: Arc<dyn AuditSource>,
    ledger: Arc<dyn LedgerSource>,
}

impl LedgerOnlyRunner {
    /// Build a runner over the shared sources.
    pub fn new(audit: Arc<dyn AuditSource>, ledger: Arc<dyn LedgerSource>) -> Self {
        Self { audit, ledger }
    }

    /// Validate policy inputs, returning the typed refusal for anything
    /// outside the business contract.
    pub fn validate_policy(policy: &CyclePolicy) -> Result<(), ApiError> {
        if let Some(rate_bp) = policy.min_lift_rate_bp {
            if !(MIN_LIFT_RATE_BP..=MAX_LIFT_RATE_BP).contains(&rate_bp) {
                return Err(ApiError::BadRequest("invalid_policy"));
            }
        }
        if policy.max_fee_micros.is_some_and(|fee| fee < 0) {
            return Err(ApiError::BadRequest("invalid_policy"));
        }
        Ok(())
    }
}

#[async_trait]
impl CycleRunner for LedgerOnlyRunner {
    async fn run(
        &self,
        publisher: &str,
        platform: PlatformId,
        request: CycleRequest,
    ) -> Result<CycleSummary, ApiError> {
        if let Some(policy) = &request.policy {
            Self::validate_policy(policy)?;
        }
        let snapshot = self.audit.snapshot(publisher, platform).await?;
        let report = apex_audit::run(&snapshot);
        let critical_findings = report
            .findings
            .iter()
            .filter(|finding| finding.severity == Severity::Critical)
            .count();
        let records = self.ledger.records(publisher).await?.len();
        Ok(CycleSummary {
            mode: READ_ONLY_MODE,
            publisher: publisher.to_string(),
            records,
            critical_findings,
            note: READ_ONLY_NOTE,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_validation_pins_the_business_band() {
        for rate in [MIN_LIFT_RATE_BP, 1_750, MAX_LIFT_RATE_BP] {
            let policy = CyclePolicy {
                min_lift_rate_bp: Some(rate),
                max_fee_micros: Some(0),
            };
            assert!(LedgerOnlyRunner::validate_policy(&policy).is_ok(), "{rate}");
        }
        for rate in [0, 1_499, 2_001, u16::MAX] {
            let policy = CyclePolicy {
                min_lift_rate_bp: Some(rate),
                max_fee_micros: None,
            };
            assert!(
                matches!(
                    LedgerOnlyRunner::validate_policy(&policy),
                    Err(ApiError::BadRequest("invalid_policy"))
                ),
                "{rate}"
            );
        }
        assert!(LedgerOnlyRunner::validate_policy(&CyclePolicy::default()).is_ok());
        let negative_fee = CyclePolicy {
            min_lift_rate_bp: None,
            max_fee_micros: Some(-1),
        };
        assert!(matches!(
            LedgerOnlyRunner::validate_policy(&negative_fee),
            Err(ApiError::BadRequest("invalid_policy"))
        ));
    }

    #[test]
    fn request_body_is_strict_about_unknown_fields() {
        let parsed: CycleRequest =
            serde_json::from_str(r#"{"policy":{"minLiftRateBp":1800}}"#).expect("parse");
        assert_eq!(parsed.policy.expect("policy").min_lift_rate_bp, Some(1_800));
        assert!(serde_json::from_str::<CycleRequest>(r#"{"policies":{}}"#).is_err());
        assert!(serde_json::from_str::<CycleRequest>(r#"{"policy":{"min_lift":1}}"#).is_err());
        assert!(serde_json::from_str::<CycleRequest>("{}").is_ok());
    }
}
