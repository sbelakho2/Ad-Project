//! The control plane's own signing and hashing.
//!
//! The operator service owns its keys and digests end to end: session and
//! CSRF tokens are random 128-bit values, object versions are SHA-256
//! digests, journal before/after digests are SHA-256 over canonical JSON, and
//! the authentication code uses HMAC-SHA1 through [`crate::totp`]. Nothing
//! here derives from the publisher console, so a leaked publisher credential
//! yields no operator authority.

use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// 32 lowercase hex characters from 16 random bytes. Session ids, CSRF
/// tokens, pending-login tokens and idempotency keys are all this shape.
pub fn random_hex_32() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Lowercase hex SHA-256 of `input`.
pub fn sha256_hex(input: &[u8]) -> String {
    hex::encode(Sha256::digest(input))
}

/// Lowercase hex HMAC-SHA256 of `message` under `key`.
pub fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    let mut mac = match Hmac::<Sha256>::new_from_slice(key) {
        Ok(mac) => mac,
        // HMAC-SHA256 accepts any key length; an unusable key cannot arise
        // from this crate's callers, and returning a digest-shaped error
        // string is safer than an unreachable panic in a credential path.
        Err(_) => return "invalid-hmac-key".to_string(),
    };
    mac.update(message);
    hex::encode(mac.finalize().into_bytes())
}

/// Constant-time byte comparison, mirroring the publisher console's rule:
/// the comparison time depends only on the length, never on the position of
/// the first differing byte.
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// The canonical digest of one value for before/after comparisons and
/// idempotency fingerprints: SHA-256 over the compact JSON serialization.
pub fn digest_json(value: &serde_json::Value) -> String {
    sha256_hex(value.to_string().as_bytes())
}

/// The journal/analytics fingerprint of an idempotency key: the key itself is
/// a replayable handle, so only its digest is retained outside the request.
pub fn idempotency_fingerprint(key: &str) -> String {
    format!("sha256:{}", sha256_hex(key.as_bytes()))
}

/// The fingerprint binding an idempotency key to what it submitted: SHA-256
/// over the capability/kind name, the target and the canonical payload JSON.
/// A remembered result or durable job is a duplicate only when the key *and*
/// this fingerprint match; the same key with a different fingerprint is a
/// conflict.
pub fn payload_fingerprint(kind: &str, target: &str, payload: &serde_json::Value) -> String {
    format!(
        "sha256:{}",
        sha256_hex(format!("{kind}\n{target}\n{payload}").as_bytes())
    )
}

/// A well-formed opaque token: exactly 32 lowercase hex characters.
pub fn is_random_hex_32(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_tokens_are_hex32_and_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..64 {
            let token = random_hex_32();
            assert!(is_random_hex_32(&token), "{token}");
            assert!(seen.insert(token), "duplicate token");
        }
        assert!(!is_random_hex_32(&"A".repeat(32)));
        assert!(!is_random_hex_32(&"a".repeat(31)));
        assert!(!is_random_hex_32(""));
    }

    #[test]
    fn sha256_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hmac_sha256_matches_the_rfc4231_vector() {
        // RFC 4231 test case 1.
        let key = [0x0b; 20];
        let data = b"Hi There";
        assert_eq!(
            hmac_sha256_hex(&key, data),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn constant_time_eq_compares_bytes_exactly() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn digests_are_stable_and_distinct() {
        let a = serde_json::json!({"state": "open", "owner": "maya"});
        let b = serde_json::json!({"owner": "maya", "state": "open"});
        let c = serde_json::json!({"state": "open", "owner": "sam"});
        assert_eq!(
            digest_json(&a),
            digest_json(&b),
            "key order is canonicalized"
        );
        assert_ne!(digest_json(&a), digest_json(&c));
        assert_eq!(digest_json(&a).len(), 64);
    }

    #[test]
    fn payload_fingerprint_binds_kind_target_and_payload() {
        let payload = serde_json::json!({"platform": "applovin_max", "account": "acct_1"});
        let fingerprint = payload_fingerprint("retry_read", "acct_1", &payload);
        assert!(fingerprint.starts_with("sha256:"));
        assert_eq!(fingerprint.len(), "sha256:".len() + 64);
        // The same action in any order of payload keys is the same payload.
        assert_eq!(
            fingerprint,
            payload_fingerprint(
                "retry_read",
                "acct_1",
                &serde_json::json!({"account": "acct_1", "platform": "applovin_max"})
            )
        );
        // A different kind, target or payload is a different fingerprint.
        assert_ne!(
            fingerprint,
            payload_fingerprint("job_retry", "acct_1", &payload)
        );
        assert_ne!(
            fingerprint,
            payload_fingerprint("retry_read", "acct_2", &payload)
        );
        assert_ne!(
            fingerprint,
            payload_fingerprint(
                "retry_read",
                "acct_1",
                &serde_json::json!({"account": "acct_2"})
            )
        );
    }

    #[test]
    fn idempotency_fingerprint_is_prefixed_and_opaque() {
        let key = "0123456789abcdef0123456789abcdef";
        let fingerprint = idempotency_fingerprint(key);
        assert!(fingerprint.starts_with("sha256:"));
        assert_eq!(fingerprint.len(), "sha256:".len() + 64);
        assert!(!fingerprint.contains(key));
    }
}
