//! The authentication flows: password step, authenticator-code step and the
//! recent-authentication (step-up) rule.
//!
//! Both steps are evaluated here, not in the route handlers, so the whole
//! credential path is unit tested without an HTTP round trip. The rules:
//!
//! - both factors are required; a correct password alone mints only a
//!   five-minute pending login, never a session;
//! - an unknown operator id still spends one argon2id verification against
//!   [`crate::passwords::DUMMY_PHC`], so a known and an unknown id take the
//!   same path and the same work;
//! - rate limits are checked before verification (per operator and per
//!   hashed client key) and failures are counted on both keys;
//! - the failure copy is [`crate::copy::LOGIN_FAILED`] and only the safe
//!   fields (operator name, return path) are preserved;
//! - an operator without an authenticator secret can never complete step
//!   two: the page explains the configuration requirement and no session is
//!   issued.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::capabilities::Role;
use crate::config::Operators;
use crate::login::{safe_next, PendingLogins};
use crate::rate_limit::{client_key, operator_key, AttemptLimiter, LimitVerdict};

/// The result of the password step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordOutcome {
    /// The deployment has no operator configuration; nothing can be minted.
    Unconfigured,
    /// The operator or client key is locked out until `until_ms`.
    RateLimited {
        /// Unix milliseconds when the lockout ends.
        until_ms: i64,
    },
    /// Verification failed (wrong operator or password); only safe fields
    /// are preserved.
    Rejected {
        /// The operator name exactly as the operator typed it (safe field).
        preserved_operator: String,
        /// The validated return path (safe field).
        preserved_next: Option<String>,
        /// Whether the attempt was a step-up re-authentication.
        reauth: bool,
    },
    /// The password verified; the authenticator-code step follows.
    CodeRequired {
        /// Hidden token binding the two steps.
        pending_token: String,
        /// Operator display name for the code step.
        operator_name: String,
    },
    /// The password verified but the operator cannot pass a second factor.
    NoSecondFactor {
        /// Operator display name.
        operator_name: String,
    },
}

/// The highest authenticator time step each operator has already spent.
/// A code is single-use even inside the accepted drift window: replaying it
/// (or an older step) is refused exactly like a wrong code.
///
/// When built with [`UsedSteps::persistent`] the whole map is durably
/// recorded: the file is read once at startup and rewritten atomically (mode
/// `0600`, a sibling temporary file renamed over the target) on every claim,
/// so a restart cannot replay a spent step. An unreadable or malformed state
/// file fails closed ([`UsedSteps::load_error`] is set and every claim is
/// refused) until it is fixed or removed. A write that cannot be committed
/// also refuses the claim, so an accepted step is always durable.
#[derive(Debug)]
pub struct UsedSteps {
    inner: Mutex<HashMap<String, u64>>,
    path: Option<PathBuf>,
    load_error: Option<String>,
}

impl Default for UsedSteps {
    fn default() -> Self {
        Self::in_memory()
    }
}

impl UsedSteps {
    /// An in-memory store with no durable file (tests and callers that do
    /// not need restart protection).
    pub fn in_memory() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            path: None,
            load_error: None,
        }
    }

    /// A store backed by `path`. The file is read once here; a missing file
    /// is an empty store (a fresh deployment), while an unreadable or
    /// malformed file fails closed: every later claim is refused.
    pub fn persistent(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        match load_steps(&path) {
            Ok(steps) => Self {
                inner: Mutex::new(steps),
                path: Some(path),
                load_error: None,
            },
            Err(reason) => Self {
                inner: Mutex::new(HashMap::new()),
                path: Some(path),
                load_error: Some(reason),
            },
        }
    }

    /// Why the persisted state could not be trusted at startup, when it
    /// could not. Present means every claim is refused (fail closed).
    pub fn load_error(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    /// Claim `step` for `operator_id`; false when it is not newer than the
    /// last accepted step, when the persisted state could not be trusted, or
    /// when the new step could not be durably recorded.
    pub fn claim(&self, operator_id: &str, step: u64) -> bool {
        if self.load_error.is_some() {
            return false;
        }
        let path = self.path.as_deref();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match inner.get(operator_id).copied() {
            Some(last) if step <= last => false,
            _ => {
                let previous = inner.insert(operator_id.to_string(), step);
                if let Some(path) = path {
                    if write_steps(path, &inner).is_err() {
                        // A step that cannot be committed is refused and the
                        // in-memory map is restored, so a crash cannot expose
                        // an accepted step that was never durably recorded.
                        match previous {
                            Some(last) => {
                                inner.insert(operator_id.to_string(), last);
                            }
                            None => {
                                inner.remove(operator_id);
                            }
                        }
                        return false;
                    }
                }
                true
            }
        }
    }
}

/// Read the used-step map; a missing file is an empty store.
fn load_steps(path: &Path) -> Result<HashMap<String, u64>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => {
            return Err(format!(
                "the TOTP state file {} could not be read: {error}",
                path.display()
            ))
        }
    };
    if raw.trim().is_empty() {
        return Ok(HashMap::new());
    }
    serde_json::from_str(&raw).map_err(|error| {
        format!(
            "the TOTP state file {} did not parse: {error}",
            path.display()
        )
    })
}

/// Write the whole used-step map atomically: a sibling temporary file with
/// mode `0600`, fsynced and renamed over the target, so a reader (or a
/// crash) never observes a half-written map.
fn write_steps(path: &Path, steps: &HashMap<String, u64>) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let body = serde_json::to_vec(steps)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let temporary = temporary_sibling(path);
    let write = || -> std::io::Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.create(true).write(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&body)?;
        file.sync_all()
    };
    if let Err(error) = write() {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

/// A unique sibling path for an atomic rename.
fn temporary_sibling(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "totp-steps.json".to_string());
    let unique = crate::signing::random_hex_32();
    let temporary = format!(".{name}.tmp.{}.{}", std::process::id(), &unique[..12]);
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(temporary),
        _ => PathBuf::from(temporary),
    }
}

/// The result of the authenticator-code step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeOutcome {
    /// The deployment has no operator configuration.
    Unconfigured,
    /// The pending login is unknown or expired.
    StepExpired,
    /// The operator or client key is locked out until `until_ms`.
    RateLimited {
        /// Unix milliseconds when the lockout ends.
        until_ms: i64,
    },
    /// The code did not verify; the pending login stays valid for a retry.
    Rejected {
        /// Hidden token to retry the code step.
        pending_token: String,
        /// Operator display name.
        operator_name: String,
    },
    /// Both factors verified.
    Verified {
        /// Operator identity.
        operator_id: String,
        /// Operator display name.
        operator_name: String,
        /// Effective role.
        role: Role,
        /// Validated return path, when the flow carried one.
        next: Option<String>,
        /// Whether this was a step-up re-authentication of a live session.
        reauth: bool,
    },
}

/// Run the password step.
#[allow(clippy::too_many_arguments)]
pub fn password_login(
    operators: &Operators,
    limiter: &AttemptLimiter,
    pending: &PendingLogins,
    operator_input: &str,
    password: &str,
    client: &str,
    next: Option<&str>,
    reauth: bool,
    now_ms: i64,
) -> PasswordOutcome {
    if operators.is_empty() {
        return PasswordOutcome::Unconfigured;
    }
    let operator_input = operator_input.trim();
    let preserved_next = next.and_then(safe_next).map(str::to_string);
    let op_key = operator_key(operator_input);
    let client = client_key(client);
    if let Some(until_ms) = locked(
        &limiter.check(&op_key, now_ms),
        &limiter.check(&client, now_ms),
    ) {
        return PasswordOutcome::RateLimited { until_ms };
    }
    // The lookup happens first, but an unknown id still pays one argon2id
    // verification against the fixed dummy hash, so an unknown id and a
    // wrong password for a known id take the same path and cost the same
    // work. Only a match on a real operator entry can verify.
    let operator = operators.find(operator_input);
    let phc = operator
        .map(|operator| operator.config.password_argon2id_phc.as_str())
        .unwrap_or(crate::passwords::DUMMY_PHC);
    let verified = crate::passwords::verify(password, phc) && operator.is_some();
    if !verified {
        let _ = limiter.record_failure(&op_key, now_ms);
        let _ = limiter.record_failure(&client, now_ms);
        return PasswordOutcome::Rejected {
            preserved_operator: operator_input.chars().take(80).collect(),
            preserved_next,
            reauth,
        };
    }
    // The password matched; clear the operator's failure count. The client
    // count clears too: an authenticated operator on a shared address should
    // not inherit another client's failures.
    limiter.record_success(&op_key);
    limiter.record_success(&client);
    let operator = match operator {
        Some(operator) => operator,
        None => return PasswordOutcome::Unconfigured,
    };
    if operator.totp_secret.is_none() {
        return PasswordOutcome::NoSecondFactor {
            operator_name: operator.config.name.clone(),
        };
    }
    let pending_token = pending.open_with_reason(
        &operator.config.id,
        preserved_next.as_deref(),
        reauth,
        now_ms,
    );
    PasswordOutcome::CodeRequired {
        pending_token,
        operator_name: operator.config.name.clone(),
    }
}

/// Run the authenticator-code step.
#[allow(clippy::too_many_arguments)]
pub fn code_login(
    operators: &Operators,
    limiter: &AttemptLimiter,
    pending: &PendingLogins,
    used_steps: &UsedSteps,
    pending_token: &str,
    code: &str,
    client: &str,
    now_ms: i64,
) -> CodeOutcome {
    if operators.is_empty() {
        return CodeOutcome::Unconfigured;
    }
    let Some(view) = pending.get(pending_token, now_ms) else {
        return CodeOutcome::StepExpired;
    };
    let Some(operator) = operators.find(&view.operator_id) else {
        // The operator was removed from the configuration between steps.
        let _ = pending.consume(pending_token, now_ms);
        return CodeOutcome::StepExpired;
    };
    let Some(secret) = operator.totp_secret.as_deref() else {
        // No second factor configured: the step cannot succeed, and the
        // pending login is dropped so no half-authenticated state lingers.
        let _ = pending.consume(pending_token, now_ms);
        return CodeOutcome::StepExpired;
    };
    let op_key = operator_key(&view.operator_id);
    let client = client_key(client);
    if let Some(until_ms) = locked(
        &limiter.check(&op_key, now_ms),
        &limiter.check(&client, now_ms),
    ) {
        return CodeOutcome::RateLimited { until_ms };
    }
    let seconds = now_ms.div_euclid(1_000);
    let Some(step) = crate::totp::verify_step(secret, code, seconds) else {
        let _ = limiter.record_failure(&op_key, now_ms);
        let _ = limiter.record_failure(&client, now_ms);
        return CodeOutcome::Rejected {
            pending_token: pending_token.to_string(),
            operator_name: operator.config.name.clone(),
        };
    };
    if !used_steps.claim(&operator.config.id, step) {
        // A replayed (or older) code is not a second factor.
        let _ = limiter.record_failure(&op_key, now_ms);
        let _ = limiter.record_failure(&client, now_ms);
        return CodeOutcome::Rejected {
            pending_token: pending_token.to_string(),
            operator_name: operator.config.name.clone(),
        };
    }
    limiter.record_success(&op_key);
    limiter.record_success(&client);
    let Some(consumed) = pending.consume(pending_token, now_ms) else {
        return CodeOutcome::StepExpired;
    };
    CodeOutcome::Verified {
        operator_id: operator.config.id.clone(),
        operator_name: operator.config.name.clone(),
        role: operator.role,
        next: consumed.next,
        reauth: consumed.reauth,
    }
}

fn locked(operator: &LimitVerdict, client: &LimitVerdict) -> Option<i64> {
    match (operator, client) {
        (LimitVerdict::Locked { until_ms }, _) | (_, LimitVerdict::Locked { until_ms }) => {
            Some(*until_ms)
        }
        _ => None,
    }
}

/// A bounded device label for the session record: the user-agent reduced to
/// printable characters and at most 48 of them, or `Unknown device`.
pub fn device_label(headers: &axum::http::HeaderMap) -> String {
    let raw = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let cleaned: String = raw
        .chars()
        .filter(|ch| !ch.is_control() && *ch != '<' && *ch != '>')
        .take(48)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "Unknown device".to_string()
    } else {
        cleaned.to_string()
    }
}

/// The client identifier used for the hashed per-client rate limit: the
/// **last** forwarded-for value (the hop the trusted proxy appended) when
/// present, else the peer address placeholder. Earlier entries are
/// client-supplied and never select a rate-limit bucket. The raw value is
/// never stored or logged; [`client_key`] hashes it.
pub fn client_identifier(headers: &axum::http::HeaderMap, peer: Option<&str>) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next_back())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| peer.map(str::to_string))
        .unwrap_or_else(|| "unknown-client".to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::config::OperatorConfig;
    use crate::rate_limit::AttemptLimiter;

    const NOW: i64 = 1_792_000_000_000;
    /// The RFC 6238 SHA-1 secret "12345678901234567890" in base32.
    const SECRET_B32: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    fn operators(with_secret: bool) -> Operators {
        let phc = crate::passwords::hash_with_salt("correct horse", b"apex-control-fixture")
            .expect("hash");
        Operators::from_entries(vec![OperatorConfig {
            id: "maya".to_string(),
            name: "Maya".to_string(),
            role: "operator".to_string(),
            password_argon2id_phc: phc,
            totp_secret_base32: with_secret.then(|| SECRET_B32.to_string()),
        }])
        .expect("operators")
    }

    fn limiter() -> AttemptLimiter {
        AttemptLimiter::new(16, 3, 60_000, 300_000)
    }

    fn code_for(now_ms: i64) -> String {
        crate::totp::code_at(b"12345678901234567890", now_ms.div_euclid(1_000))
    }

    #[test]
    fn a_spent_code_cannot_be_replayed_and_the_next_step_still_works() {
        let operators = operators(true);
        let limiter = limiter();
        let pending = PendingLogins::bounded();
        let used = UsedSteps::default();
        let token = |now_ms: i64| match password_login(
            &operators,
            &limiter,
            &pending,
            "maya",
            "correct horse",
            "203.0.113.7",
            None,
            false,
            now_ms,
        ) {
            PasswordOutcome::CodeRequired { pending_token, .. } => pending_token,
            other => panic!("expected the code step, got {other:?}"),
        };

        let first_token = token(NOW);
        assert!(matches!(
            code_login(
                &operators,
                &limiter,
                &pending,
                &used,
                &first_token,
                &code_for(NOW),
                "203.0.113.7",
                NOW,
            ),
            CodeOutcome::Verified { .. }
        ));

        // A fresh login with the same code inside the same step is refused.
        let second_token = token(NOW + 1_000);
        assert!(matches!(
            code_login(
                &operators,
                &limiter,
                &pending,
                &used,
                &second_token,
                &code_for(NOW),
                "203.0.113.7",
                NOW + 1_000,
            ),
            CodeOutcome::Rejected { .. }
        ));

        // The next step's code is a new single-use factor.
        let third_token = token(NOW + 31_000);
        assert!(matches!(
            code_login(
                &operators,
                &limiter,
                &pending,
                &used,
                &third_token,
                &code_for(NOW + 31_000),
                "203.0.113.7",
                NOW + 31_000,
            ),
            CodeOutcome::Verified { .. }
        ));
    }

    #[test]
    fn the_happy_path_requires_both_factors() {
        let operators = operators(true);
        let limiter = limiter();
        let pending = PendingLogins::bounded();
        let first = password_login(
            &operators,
            &limiter,
            &pending,
            "maya",
            "correct horse",
            "203.0.113.7",
            Some("/control/incidents/inc_018"),
            false,
            NOW,
        );
        let token = match first {
            PasswordOutcome::CodeRequired {
                pending_token,
                operator_name,
            } => {
                assert_eq!(operator_name, "Maya");
                pending_token
            }
            other => panic!("expected the code step, got {other:?}"),
        };
        let outcome = code_login(
            &operators,
            &limiter,
            &pending,
            &UsedSteps::default(),
            &token,
            &code_for(NOW),
            "203.0.113.7",
            NOW + 30_000,
        );
        match outcome {
            CodeOutcome::Verified {
                operator_id,
                role,
                next,
                reauth,
                ..
            } => {
                assert_eq!(operator_id, "maya");
                assert_eq!(role, Role::Operator);
                assert_eq!(next.as_deref(), Some("/control/incidents/inc_018"));
                assert!(!reauth);
            }
            other => panic!("expected verified, got {other:?}"),
        }
        // The pending login is one-shot.
        assert!(pending.is_empty());
    }

    #[test]
    fn a_wrong_password_or_code_uses_the_frozen_copy_state_and_preserves_safe_fields() {
        let operators = operators(true);
        let limiter = limiter();
        let pending = PendingLogins::bounded();
        let rejected = password_login(
            &operators,
            &limiter,
            &pending,
            "maya",
            "wrong",
            "203.0.113.7",
            Some("/control/"),
            false,
            NOW,
        );
        match rejected {
            PasswordOutcome::Rejected {
                preserved_operator,
                preserved_next,
                reauth,
            } => {
                assert_eq!(preserved_operator, "maya");
                assert_eq!(preserved_next.as_deref(), Some("/control/"));
                assert!(!reauth);
            }
            other => panic!("expected rejected, got {other:?}"),
        }
        // An unknown operator takes exactly the same path: one argon2id
        // verification, here against the fixed dummy hash.
        let unknown = password_login(
            &operators,
            &limiter,
            &pending,
            "nobody",
            "whatever",
            "203.0.113.7",
            None,
            false,
            NOW,
        );
        assert!(matches!(unknown, PasswordOutcome::Rejected { .. }));
        // The dummy is a well-formed argon2id hash whose verification is
        // really exercised on the unknown-id path and fails closed.
        assert!(crate::passwords::phc_is_argon2id(crate::passwords::DUMMY_PHC).is_ok());
        assert!(!crate::passwords::verify("", crate::passwords::DUMMY_PHC));
        assert!(!crate::passwords::verify(
            "fixture-password",
            crate::passwords::DUMMY_PHC
        ));
        // Wrong code: the pending login survives for a retry.
        let token = match password_login(
            &operators,
            &limiter,
            &pending,
            "maya",
            "correct horse",
            "203.0.113.7",
            None,
            false,
            NOW,
        ) {
            PasswordOutcome::CodeRequired { pending_token, .. } => pending_token,
            other => panic!("expected the code step, got {other:?}"),
        };
        let bad_code = code_login(
            &operators,
            &limiter,
            &pending,
            &UsedSteps::default(),
            &token,
            "000000",
            "c",
            NOW + 1,
        );
        match bad_code {
            CodeOutcome::Rejected {
                pending_token: returned,
                ..
            } => assert_eq!(returned, token),
            other => panic!("expected rejected, got {other:?}"),
        }
        assert!(pending.get(&token, NOW + 2).is_some());
    }

    #[test]
    fn rate_limit_locks_both_keys_and_returns_the_deadline() {
        let operators = operators(true);
        let limiter = AttemptLimiter::new(16, 2, 60_000, 300_000);
        let pending = PendingLogins::bounded();
        let attempt = |at| {
            password_login(
                &operators,
                &limiter,
                &pending,
                "maya",
                "wrong",
                "203.0.113.7",
                None,
                false,
                at,
            )
        };
        assert!(matches!(attempt(NOW), PasswordOutcome::Rejected { .. }));
        assert!(matches!(attempt(NOW + 1), PasswordOutcome::Rejected { .. }));
        // The second failure crossed the threshold: the next attempt is
        // rate limited, and the deadline is returned for the page copy.
        match attempt(NOW + 2) {
            PasswordOutcome::RateLimited { until_ms } => {
                assert_eq!(until_ms, NOW + 1 + 300_000);
            }
            other => panic!("expected rate limited, got {other:?}"),
        }
        // A different client is limited by the operator key too.
        let other_client = password_login(
            &operators,
            &limiter,
            &pending,
            "maya",
            "correct horse",
            "198.51.100.9",
            None,
            false,
            NOW + 4,
        );
        assert!(matches!(other_client, PasswordOutcome::RateLimited { .. }));
        // After the lockout ends, the correct password works again.
        let recovered = password_login(
            &operators,
            &limiter,
            &pending,
            "maya",
            "correct horse",
            "198.51.100.9",
            None,
            false,
            NOW + 1 + 300_000,
        );
        assert!(matches!(recovered, PasswordOutcome::CodeRequired { .. }));
    }

    #[test]
    fn an_operator_without_a_secret_can_never_reach_the_code_step() {
        let operators = operators(false);
        let limiter = limiter();
        let pending = PendingLogins::bounded();
        let outcome = password_login(
            &operators,
            &limiter,
            &pending,
            "maya",
            "correct horse",
            "c",
            None,
            false,
            NOW,
        );
        match outcome {
            PasswordOutcome::NoSecondFactor { operator_name } => {
                assert_eq!(operator_name, "Maya")
            }
            other => panic!("expected no-second-factor, got {other:?}"),
        }
        assert!(pending.is_empty(), "no pending login may exist");
    }

    #[test]
    fn step_up_flows_carry_the_reauth_flag_to_verification() {
        let operators = operators(true);
        let limiter = limiter();
        let pending = PendingLogins::bounded();
        let token = match password_login(
            &operators,
            &limiter,
            &pending,
            "maya",
            "correct horse",
            "c",
            Some("/control/finance"),
            true,
            NOW,
        ) {
            PasswordOutcome::CodeRequired { pending_token, .. } => pending_token,
            other => panic!("expected the code step, got {other:?}"),
        };
        match code_login(
            &operators,
            &limiter,
            &pending,
            &UsedSteps::default(),
            &token,
            &code_for(NOW),
            "c",
            NOW,
        ) {
            CodeOutcome::Verified { reauth, next, .. } => {
                assert!(reauth);
                assert_eq!(next.as_deref(), Some("/control/finance"));
            }
            other => panic!("expected verified, got {other:?}"),
        }
    }

    #[test]
    fn expired_pending_logins_and_unconfigured_deployments_are_distinct() {
        let operators = operators(true);
        let limiter = limiter();
        let pending = PendingLogins::new(4, 1_000);
        let token = match password_login(
            &operators,
            &limiter,
            &pending,
            "maya",
            "correct horse",
            "c",
            None,
            false,
            NOW,
        ) {
            PasswordOutcome::CodeRequired { pending_token, .. } => pending_token,
            other => panic!("expected the code step, got {other:?}"),
        };
        let late = code_login(
            &operators,
            &limiter,
            &pending,
            &UsedSteps::default(),
            &token,
            &code_for(NOW + 2_000),
            "c",
            NOW + 2_000,
        );
        assert_eq!(late, CodeOutcome::StepExpired);
        let none = Operators::empty();
        assert_eq!(
            password_login(&none, &limiter, &pending, "maya", "x", "c", None, false, NOW),
            PasswordOutcome::Unconfigured
        );
        assert_eq!(
            code_login(
                &none,
                &limiter,
                &pending,
                &UsedSteps::default(),
                "t",
                "123456",
                "c",
                NOW
            ),
            CodeOutcome::Unconfigured
        );
    }

    #[test]
    fn device_labels_are_bounded_and_sanitized() {
        let mut headers = axum::http::HeaderMap::new();
        assert_eq!(device_label(&headers), "Unknown device");
        headers.insert(
            axum::http::header::USER_AGENT,
            "Mozilla/5.0 (Macintosh) AppleWebKit/537.36 <script>"
                .parse()
                .expect("value"),
        );
        let label = device_label(&headers);
        assert!(label.len() <= 48);
        assert!(!label.contains('<'));
        assert!(!label.contains('>'));
        // The sanitizer never splits characters and bounds the label.
        let mut exact = axum::http::HeaderMap::new();
        exact.insert(
            axum::http::header::USER_AGENT,
            format!("x{}", "y".repeat(100)).parse().expect("value"),
        );
        let long_label = device_label(&exact);
        assert_eq!(long_label.chars().count(), 48);
        assert!(long_label.starts_with('x'));
    }

    #[test]
    fn client_identifier_uses_the_last_forwarded_hop() {
        let mut headers = axum::http::HeaderMap::new();
        assert_eq!(
            client_identifier(&headers, Some("10.0.0.5:1234")),
            "10.0.0.5:1234"
        );
        assert_eq!(client_identifier(&headers, None), "unknown-client");
        // The proxy-appended last hop wins; a spoofed first entry is ignored.
        headers.insert(
            "x-forwarded-for",
            "203.0.113.7, 10.0.0.1".parse().expect("value"),
        );
        assert_eq!(
            client_identifier(&headers, Some("10.0.0.5:1234")),
            "10.0.0.1"
        );
        headers.insert(
            "x-forwarded-for",
            "198.51.100.9, 203.0.113.7, 10.0.0.2"
                .parse()
                .expect("value"),
        );
        assert_eq!(
            client_identifier(&headers, Some("10.0.0.5:1234")),
            "10.0.0.2"
        );
        headers.insert("x-forwarded-for", "  ".parse().expect("value"));
        assert_eq!(
            client_identifier(&headers, Some("10.0.0.5:1234")),
            "10.0.0.5:1234"
        );
    }

    fn unique_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "apex-control-{label}-{}-{}",
            std::process::id(),
            crate::signing::random_hex_32()
        ))
    }

    #[test]
    fn a_spent_step_is_still_refused_after_a_restart() {
        let dir = unique_dir("totp-restart");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("totp-steps.json");
        let step = 1_111_111_111u64 / crate::totp::STEP_SECONDS;
        {
            let used = UsedSteps::persistent(&path);
            assert_eq!(used.load_error(), None);
            assert!(used.claim("maya", step));
            assert!(path.exists(), "the claim is written through");
        }
        // A restart: a fresh store loads the spent step and refuses it.
        let restarted = UsedSteps::persistent(&path);
        assert_eq!(restarted.load_error(), None);
        assert!(
            !restarted.claim("maya", step),
            "a spent step is refused after a restart"
        );
        assert!(
            !restarted.claim("maya", step - 1),
            "an older step is refused"
        );
        assert!(
            restarted.claim("maya", step + 1),
            "the next step is accepted"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_state_file_fails_closed_until_it_is_fixed() {
        let dir = unique_dir("totp-corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("totp-steps.json");
        std::fs::write(&path, "{ not json").expect("write");
        let used = UsedSteps::persistent(&path);
        assert!(used.load_error().is_some(), "a malformed file is refused");
        assert!(
            !used.claim("maya", 42),
            "every claim is refused while the file is unreadable"
        );
        // Fixing the file and rebuilding the store restores claims.
        std::fs::write(&path, "{\"maya\":7}").expect("write");
        let fixed = UsedSteps::persistent(&path);
        assert_eq!(fixed.load_error(), None);
        assert!(!fixed.claim("maya", 7), "the loaded step is remembered");
        assert!(fixed.claim("maya", 8));
        // A value that is not a step is malformed and fails closed too.
        std::fs::write(&path, "{\"maya\":\"nope\"}").expect("write");
        assert!(UsedSteps::persistent(&path).load_error().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_claims_write_one_entry_per_operator() {
        let dir = unique_dir("totp-concurrent");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("totp-steps.json");
        let used = std::sync::Arc::new(UsedSteps::persistent(&path));
        let operators = ["maya", "sam", "lina", "ravi", "ada", "noor"];
        let mut handles = Vec::new();
        for (index, operator) in operators.iter().enumerate() {
            let used = std::sync::Arc::clone(&used);
            let operator = (*operator).to_string();
            handles.push(std::thread::spawn(move || {
                assert!(used.claim(&operator, (index as u64) + 1));
            }));
        }
        for handle in handles {
            handle.join().expect("join");
        }
        let raw = std::fs::read_to_string(&path).expect("read");
        let stored: std::collections::HashMap<String, u64> =
            serde_json::from_str(&raw).expect("parse");
        assert_eq!(stored.len(), operators.len(), "one entry per operator");
        for (index, operator) in operators.iter().enumerate() {
            assert_eq!(stored.get(*operator), Some(&((index as u64) + 1)));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_state_file_is_written_mode_0600() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = unique_dir("totp-mode");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("totp-steps.json");
        assert!(UsedSteps::persistent(&path).claim("maya", 1));
        let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the state file is owner-only");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
