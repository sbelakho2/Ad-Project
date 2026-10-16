//! The sales web origin's own tokens and comparisons.
//!
//! Session ids, CSRF tokens and idempotency keys are random 128-bit values
//! rendered as 32 lowercase hex characters. The owner token and the CSRF
//! token are compared in constant time, so comparison time depends only on
//! the length, never on the position of the first differing byte.

use rand::RngCore;

/// 32 lowercase hex characters from 16 random bytes.
pub fn random_hex_32() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Constant-time byte comparison: the comparison time depends only on the
/// length, never on the position of the first differing byte.
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

/// A well-formed 32-character lowercase hex token.
pub fn is_random_hex_32(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(is_lower_hex)
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

/// An operation id: `op_` plus 12 lowercase hex characters.
pub fn operation_id() -> String {
    format!("op_{}", &random_hex_32()[..12])
}

/// Whether `value` is a displayed operation id (`op_` + 12 lowercase hex).
pub fn is_operation_id(value: &str) -> bool {
    value
        .strip_prefix("op_")
        .is_some_and(|rest| rest.len() == 12 && rest.bytes().all(is_lower_hex))
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
    fn constant_time_eq_compares_bytes_exactly() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn operation_ids_have_the_documented_shape() {
        for _ in 0..16 {
            let id = operation_id();
            assert!(is_operation_id(&id), "{id}");
        }
        assert!(!is_operation_id("op_zzzzzzzzzzzz"));
        assert!(!is_operation_id("op_ABC"));
        assert!(!is_operation_id("op_abc"));
        assert!(!is_operation_id(""));
    }
}
