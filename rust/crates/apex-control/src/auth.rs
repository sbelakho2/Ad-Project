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
pub fn code_login(
    operators: &Operators,
    limiter: &AttemptLimiter,
    pending: &PendingLogins,
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
    if !crate::totp::verify(secret, code, seconds) {
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
/// first forwarded-for value when present, else the peer address placeholder.
/// The raw value is never stored or logged; [`client_key`] hashes it.
pub fn client_identifier(headers: &axum::http::HeaderMap, peer: Option<&str>) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
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
            code_login(&none, &limiter, &pending, "t", "123456", "c", NOW),
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
    fn client_identifier_prefers_the_forwarded_chain() {
        let mut headers = axum::http::HeaderMap::new();
        assert_eq!(
            client_identifier(&headers, Some("10.0.0.5:1234")),
            "10.0.0.5:1234"
        );
        assert_eq!(client_identifier(&headers, None), "unknown-client");
        headers.insert(
            "x-forwarded-for",
            "203.0.113.7, 10.0.0.1".parse().expect("value"),
        );
        assert_eq!(
            client_identifier(&headers, Some("10.0.0.5:1234")),
            "203.0.113.7"
        );
        headers.insert("x-forwarded-for", "  ".parse().expect("value"));
        assert_eq!(
            client_identifier(&headers, Some("10.0.0.5:1234")),
            "10.0.0.5:1234"
        );
    }
}
