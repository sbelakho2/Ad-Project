//! Settings read models: account facts, policy summary, monitoring health.
//!
//! Nothing here invents configuration. Account facts come from explicit
//! deployment variables; the policy summary comes from the newest signed
//! pre-registration record (thresholds, engine hash, time, sequence), so the
//! page can say who last changed the stop conditions and when; reporting
//! health comes from the recorded snapshots, connection observations and
//! signed records. Every value a deployment has not recorded renders as
//! "Not available" with the reason, never as a plausible default.

use apex_core::PlatformId;
use apex_lift::records::LiftRecord;
use apex_proof::LedgerRecord;

use crate::connections::CredentialState;

/// Contact shown on the account settings page (unset is honest: the account
/// team decides where contact details live).
pub const CONTACT_ENV: &str = "APEX_CONSOLE_CONTACT";
/// Workspace timezone label (defaults to `UTC`).
pub const TIMEZONE_ENV: &str = "APEX_CONSOLE_TIMEZONE";
/// Default workspace timezone.
pub const DEFAULT_TIMEZONE: &str = "UTC";
/// Retention window in months for signed records (unset: not recorded).
pub const RETENTION_MONTHS_ENV: &str = "APEX_RETENTION_MONTHS";
/// The runner's recorded policy file (`AutopilotPolicy` JSON, read-only).
pub const POLICY_FILE_ENV: &str = "APEX_POLICY_FILE";

/// The account settings view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountView {
    /// The workspace label.
    pub workspace: String,
    /// The support contact, when the deployment records one.
    pub contact: Option<String>,
    /// The workspace timezone.
    pub timezone: String,
    /// The display currency.
    pub currency: String,
    /// Signed-record retention in months, when recorded.
    pub retention_months: Option<u32>,
    /// Whether platform sessions end when credentials stop working.
    pub sessions_bound_to_credentials: bool,
}

/// Read the account view. `sessions_bound_to_credentials` is the product
/// truth: revoking a connection ends the platform sessions the runner holds,
/// because every call re-reads the stored credential.
pub fn account_view(
    lookup: &dyn Fn(&str) -> Option<String>,
    workspace: &str,
    currency: &str,
) -> AccountView {
    AccountView {
        workspace: workspace.to_string(),
        contact: lookup(CONTACT_ENV).filter(|value| !value.trim().is_empty()),
        timezone: lookup(TIMEZONE_ENV)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_TIMEZONE.to_string()),
        currency: currency.to_string(),
        retention_months: lookup(RETENTION_MONTHS_ENV)
            .and_then(|raw| raw.trim().parse::<u32>().ok())
            .filter(|months| *months > 0),
        sessions_bound_to_credentials: true,
    }
}

/// The policy page view: the newest signed pre-registration's thresholds.
#[derive(Debug, Clone, PartialEq)]
pub struct PolicyView {
    /// The newest signed pre-registration record: experiment, policy hash,
    /// thresholds, time and sequence.
    pub latest_registration: Option<RegisteredPolicy>,
}

/// One signed pre-registration that shaped the current policy.
#[derive(Debug, Clone, PartialEq)]
pub struct RegisteredPolicy {
    /// The experiment the thresholds were registered for.
    pub experiment_id: String,
    /// The engine version hash the registration pinned.
    pub code_version_hash: String,
    /// Contract rate in basis points.
    pub rate_bp: u16,
    /// Registered burn-in days.
    pub burn_in_days: u32,
    /// Registered analysis window days.
    pub window_days: u32,
    /// The signed thresholds (the record stores them as JSON).
    pub thresholds: serde_json::Value,
    /// Ledger sequence of the signed registration.
    pub seq: u64,
    /// When it was recorded.
    pub at_unix_ms: i64,
}

/// Project the policy view from verified records: the newest registration
/// wins (ledger order is the authority).
pub fn policy_view(records: &[LedgerRecord]) -> PolicyView {
    let mut latest: Option<RegisteredPolicy> = None;
    for record in records {
        if let LedgerRecord::LiftRecord {
            seq,
            at_unix_ms,
            payload,
            ..
        } = record
        {
            let Ok(LiftRecord::PreRegistration {
                experiment_id,
                code_version_hash,
                rate_bp,
                burn_in_days,
                window_days,
                guardrail_thresholds,
                ..
            }) = serde_json::from_str::<LiftRecord>(payload)
            else {
                continue;
            };
            let candidate = RegisteredPolicy {
                experiment_id,
                code_version_hash,
                rate_bp,
                burn_in_days,
                window_days,
                thresholds: guardrail_thresholds,
                seq: *seq,
                at_unix_ms: *at_unix_ms,
            };
            if latest
                .as_ref()
                .is_none_or(|current| candidate.seq >= current.seq)
            {
                latest = Some(candidate);
            }
        }
    }
    PolicyView {
        latest_registration: latest,
    }
}

/// The frozen signed-order note of the policy page.
pub const SIGNED_ORDER_NOTE: &str =
    "The signed order is the authority: a UI preference cannot widen a stop condition or bypass \
     an approval.";
/// The frozen approval-mode statement (the product's gate).
pub const APPROVAL_MODE_STATEMENT: &str =
    "Approval required before the first write of each test. Approving never grants unlimited \
     write authority: the test still runs under the stop conditions below.";

/// One platform's reporting health on the monitoring settings page.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitoringRow {
    /// Console label.
    pub label: String,
    /// Wire name.
    pub wire: &'static str,
    /// Whether the newest snapshot exists for this platform.
    pub last_snapshot_unix_ms: Option<i64>,
    /// Recorded credential state.
    pub credential_state: CredentialState,
    /// Whether reporting reads were confirmed (`None`: never observed).
    pub reporting: Option<bool>,
}

/// Build the per-platform reporting health rows from the newest snapshots
/// and the connection observations.
pub fn monitoring_rows(
    snapshots: &[(PlatformId, i64)],
    meta: &crate::connections::ConnectionMeta,
) -> Vec<MonitoringRow> {
    crate::connections::CAPABILITIES
        .iter()
        .map(|capability| {
            let platform = parse_wire(capability.wire);
            let last_snapshot = platform.and_then(|platform| {
                snapshots
                    .iter()
                    .filter(|(candidate, _)| *candidate == platform)
                    .map(|(_, at)| *at)
                    .max()
            });
            let state = meta.platform(capability.wire);
            MonitoringRow {
                label: capability.label.to_string(),
                wire: capability.wire,
                last_snapshot_unix_ms: last_snapshot,
                credential_state: state.credential_state(),
                reporting: state.reporting,
            }
        })
        .collect()
}

fn parse_wire(wire: &str) -> Option<PlatformId> {
    match wire {
        "applovin_max" => Some(PlatformId::AppLovinMax),
        "levelplay" => Some(PlatformId::LevelPlay),
        "admob" => Some(PlatformId::Admob),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use sha2::{Digest, Sha256};

    use super::*;

    fn registration(experiment: &str, seq_rate: u16) -> LedgerRecord {
        let payload = format!(
            r#"{{"type":"preRegistration","experiment_id":"{experiment}","scope":"u","before_value":1,"after_value":2,"split":0.25,"metric":"RevenuePerUser","burn_in_days":3,"window_days":14,"looks":1,"extension_allowed":true,"guardrail_thresholds":{{"harmPoint":-0.04,"fillRewarded":0.02,"fillBanner":0.05,"splitChiSquare":10.83}},"code_version_hash":"apex-lift-{seq_rate}","rate_bp":{seq_rate}}}"#
        );
        let digest = hex::encode(Sha256::digest(payload.as_bytes()));
        LedgerRecord::lift_record(1_792_000_000_000 + i64::from(seq_rate), &digest, &payload)
    }

    fn sealed(records: Vec<LedgerRecord>) -> Vec<LedgerRecord> {
        let mut ledger =
            apex_proof::InMemoryLedger::new(apex_proof::Keypair::from_bytes(&[7u8; 32]));
        records
            .into_iter()
            .map(|draft| ledger.append(draft).expect("seal"))
            .collect()
    }

    #[test]
    fn account_view_reads_explicit_configuration_only() {
        let lookup = |key: &str| match key {
            CONTACT_ENV => Some("ops@northstar.example".to_string()),
            TIMEZONE_ENV => Some("Europe/Amsterdam".to_string()),
            RETENTION_MONTHS_ENV => Some("24".to_string()),
            _ => None,
        };
        let view = account_view(&lookup, "Northstar Studio", "USD");
        assert_eq!(view.workspace, "Northstar Studio");
        assert_eq!(view.contact.as_deref(), Some("ops@northstar.example"));
        assert_eq!(view.timezone, "Europe/Amsterdam");
        assert_eq!(view.retention_months, Some(24));
        assert!(view.sessions_bound_to_credentials);
        let fallback = account_view(&|_| None, "W", "EUR");
        assert_eq!(fallback.contact, None);
        assert_eq!(fallback.timezone, "UTC");
        assert_eq!(fallback.retention_months, None);
        assert_eq!(fallback.currency, "EUR");
        assert_eq!(
            account_view(&|_| Some("0".to_string()), "W", "USD").retention_months,
            None,
            "a zero retention window is not a window"
        );
    }

    #[test]
    fn policy_view_uses_the_newest_signed_registration() {
        let records = sealed(vec![
            registration("exp_a", 1500),
            registration("exp_b", 2000),
        ]);
        let view = policy_view(&records);
        let latest = view.latest_registration.expect("registration");
        assert_eq!(latest.experiment_id, "exp_b");
        assert_eq!(latest.rate_bp, 2000);
        assert_eq!(latest.thresholds["harmPoint"], serde_json::json!(-0.04));
        assert!(latest.seq >= 1);
        assert!(policy_view(&[]).latest_registration.is_none());
    }

    #[test]
    fn monitoring_rows_report_what_was_observed() {
        let snapshots = [(PlatformId::AppLovinMax, 1_792_000_000_000)];
        let mut meta = crate::connections::ConnectionMeta::default();
        meta.set_platform(
            "applovin_max",
            crate::connections::PlatformState {
                state: Some(CredentialState::Validated),
                reporting: Some(true),
                ..Default::default()
            },
        );
        meta.set_platform(
            "levelplay",
            crate::connections::PlatformState {
                state: Some(CredentialState::Expired),
                reporting: Some(false),
                ..Default::default()
            },
        );
        let rows = monitoring_rows(&snapshots, &meta);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].label, "AppLovin MAX");
        assert_eq!(rows[0].last_snapshot_unix_ms, Some(1_792_000_000_000));
        assert_eq!(rows[0].credential_state, CredentialState::Validated);
        assert_eq!(rows[0].reporting, Some(true));
        assert_eq!(rows[1].credential_state, CredentialState::Expired);
        assert_eq!(
            rows[1].last_snapshot_unix_ms, None,
            "no snapshot is observed"
        );
        assert_eq!(rows[2].credential_state, CredentialState::Unvalidated);
    }

    #[test]
    fn frozen_policy_copy_is_pinned() {
        assert!(SIGNED_ORDER_NOTE.contains("cannot widen a stop condition"));
        assert!(APPROVAL_MODE_STATEMENT.contains("Approval required before the first write"));
        assert!(APPROVAL_MODE_STATEMENT.contains("never grants unlimited write authority"));
    }
}
