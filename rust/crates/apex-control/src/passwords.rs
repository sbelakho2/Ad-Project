//! Argon2id password verification against PHC strings.
//!
//! Operator passwords never exist in plaintext on the control plane: the
//! `APEX_CONTROL_OPERATORS` file carries an argon2id PHC string and the
//! service only ever verifies a candidate against it. There is no default
//! password and no code path that mints a session without a verified PHC
//! match.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;

/// A well-formed argon2id PHC hash used as the verification target for an
/// unknown operator id. No password verifies against it (the hash bytes are
/// fixed), but parsing and verifying it costs exactly one argon2id
/// verification at the argon2 crate's default parameters, the same cost the
/// stored operator hashes use, so a sign-in attempt for an unknown id takes
/// the same path as a wrong password for a known id.
pub const DUMMY_PHC: &str = "$argon2id$v=19$m=19456,t=2,p=1$YXBleC1jb250cm9sLWR1bW15$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

/// Why a PHC string was refused at configuration load time.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PasswordError {
    /// The stored string is not a parseable PHC hash.
    #[error("password hash is not a PHC string: {reason}")]
    Malformed {
        /// Parser message (never contains the password).
        reason: String,
    },
    /// The PHC string uses an algorithm other than argon2id.
    #[error("password hash must use argon2id, found algorithm '{algorithm}'")]
    WrongAlgorithm {
        /// The algorithm named by the PHC string.
        algorithm: String,
    },
    /// Hashing failed while building a fixture or test hash.
    #[error("hashing failed: {reason}")]
    Hashing {
        /// Underlying reason.
        reason: String,
    },
}

/// Whether `phc` is a well-formed argon2id PHC string.
pub fn phc_is_argon2id(phc: &str) -> Result<(), PasswordError> {
    let parsed = PasswordHash::new(phc.trim()).map_err(|error| PasswordError::Malformed {
        reason: error.to_string(),
    })?;
    let algorithm = parsed.algorithm.as_str().to_string();
    if algorithm != "argon2id" {
        return Err(PasswordError::WrongAlgorithm { algorithm });
    }
    Ok(())
}

/// Verify `password` against the argon2id PHC string `phc`. A malformed or
/// non-argon2id hash fails closed (`false`), never open.
pub fn verify(password: &str, phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc.trim()) else {
        return false;
    };
    if parsed.algorithm.as_str() != "argon2id" {
        return false;
    }
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Hash `password` with argon2id into a PHC string using `salt` as the
/// salt bytes. Used by the fixture dataset and tests to mint non-secret
/// operator records; deployments generate their own hashes out of band.
pub fn hash_with_salt(password: &str, salt: &[u8]) -> Result<String, PasswordError> {
    let salt = SaltString::encode_b64(salt).map_err(|error| PasswordError::Hashing {
        reason: error.to_string(),
    })?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| PasswordError::Hashing {
            reason: error.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_roundtrips_and_refuses_wrong_passwords() {
        let phc = hash_with_salt("correct horse battery", b"apex-control-fixture").expect("hash");
        assert!(phc_is_argon2id(&phc).is_ok());
        assert!(verify("correct horse battery", &phc));
        assert!(!verify("wrong password", &phc));
        assert!(!verify("", &phc));
    }

    #[test]
    fn malformed_hashes_fail_closed() {
        assert!(!verify("anything", "not a phc string"));
        assert!(!verify("anything", ""));
        assert!(matches!(
            phc_is_argon2id("nonsense"),
            Err(PasswordError::Malformed { .. })
        ));
    }

    #[test]
    fn a_non_argon2id_hash_is_refused() {
        // A bcrypt-shaped modular crypt string is not an argon2id PHC hash;
        // whichever check rejects it, verification must fail closed.
        let bcrypt = "$2b$12$abcdefghijklmnopqrstuvwxyz012345678901234567890123456";
        assert!(phc_is_argon2id(bcrypt).is_err());
        assert!(!verify("anything", bcrypt));
        // A scrypt PHC string names another algorithm explicitly.
        let scrypt = "$scrypt$ln=16,r=8,p=1$c29tZXNhbHQ$0102030405060708090a0b0c";
        assert!(phc_is_argon2id(scrypt).is_err());
        assert!(!verify("anything", scrypt));
    }

    #[test]
    fn verification_is_not_confused_by_whitespace_around_the_hash() {
        let phc = hash_with_salt("pw", b"apex-control-fixture").expect("hash");
        assert!(verify("pw", &format!("  {phc}  ")));
    }

    #[test]
    fn the_dummy_hash_is_argon2id_at_the_stored_hash_cost_and_fails_closed() {
        assert!(phc_is_argon2id(DUMMY_PHC).is_ok());
        // The same default parameter set the stored operator hashes use
        // (m=19456 KiB, t=2, p=1): an unknown id pays one full verification.
        let stored = hash_with_salt("pw", b"apex-control-fixture").expect("hash");
        let stored_params = stored.split('$').nth(3).expect("stored params");
        let dummy_params = DUMMY_PHC.split('$').nth(3).expect("dummy params");
        assert_eq!(stored_params, dummy_params);
        for candidate in ["", "pw", "correct horse", "fixture-password"] {
            assert!(!verify(candidate, DUMMY_PHC), "{candidate}");
        }
    }
}
