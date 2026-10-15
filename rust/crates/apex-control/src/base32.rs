//! RFC 4648 base32 decoding for authenticator secrets.
//!
//! Authenticator enrollment hands the operator a base32 secret (the classic
//! `JBSWY3DPEHPK3PXP` shape). This module implements the decoder by hand so
//! the exact acceptance rules are testable: the alphabet is `A` to `Z` then
//! `2` to `7`, matching is case-insensitive, padding (`=`) is accepted only
//! at the end, and leftover bits that do not form a byte must be zero, so a
//! truncated or mistyped secret is refused instead of silently shifting every
//! code.

/// Why a base32 string was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Base32Error {
    /// A character outside the RFC 4648 alphabet (or padding in the middle).
    #[error("invalid base32 character at position {position}")]
    InvalidCharacter {
        /// Zero-based position of the offending character.
        position: usize,
    },
    /// The encoded length cannot carry a whole number of bytes.
    #[error("base32 length {length} cannot decode to whole bytes")]
    InvalidLength {
        /// The number of significant (non-padding) characters.
        length: usize,
    },
    /// The final partial group carried non-zero bits.
    #[error("base32 trailing bits are non-zero")]
    NonZeroTrailingBits,
    /// The input is empty.
    #[error("base32 input is empty")]
    Empty,
}

/// Decode an RFC 4648 base32 string (case-insensitive, optional `=` padding).
///
/// Returns the decoded bytes or the precise reason the input was refused.
pub fn decode(input: &str) -> Result<Vec<u8>, Base32Error> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(Base32Error::Empty);
    }
    let mut bytes = Vec::with_capacity(trimmed.len() * 5 / 8);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    let mut significant = 0usize;
    let mut padding_started = false;
    for (position, ch) in trimmed.chars().enumerate() {
        if ch == '=' {
            padding_started = true;
            continue;
        }
        if padding_started {
            return Err(Base32Error::InvalidCharacter { position });
        }
        let value = match ch.to_ascii_uppercase() {
            'A'..='Z' => ch.to_ascii_uppercase() as u32 - 'A' as u32,
            '2'..='7' => ch as u32 - '2' as u32 + 26,
            _ => return Err(Base32Error::InvalidCharacter { position }),
        };
        buffer = (buffer << 5) | value;
        bits += 5;
        significant += 1;
        if bits >= 8 {
            bits -= 8;
            bytes.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    if significant == 0 {
        return Err(Base32Error::Empty);
    }
    // A valid base32 length is 2, 4, 5, 7 or 8 characters per 8-byte group;
    // lengths of 1, 3 or 6 characters leave an unusable partial group.
    if matches!(significant % 8, 1 | 3 | 6) {
        return Err(Base32Error::InvalidLength {
            length: significant,
        });
    }
    if bits > 0 && buffer & ((1 << bits) - 1) != 0 {
        return Err(Base32Error::NonZeroTrailingBits);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_rfc4648_vectors() {
        assert_eq!(decode("MY======").expect("vector"), b"f");
        assert_eq!(decode("MZXQ====").expect("vector"), b"fo");
        assert_eq!(decode("MZXW6===").expect("vector"), b"foo");
        assert_eq!(decode("MZXW6YQ=").expect("vector"), b"foob");
        assert_eq!(decode("MZXW6YTB").expect("vector"), b"fooba");
        assert_eq!(decode("MZXW6YTBOI======").expect("vector"), b"foobar");
    }

    #[test]
    fn decoding_is_case_insensitive_and_padding_is_optional() {
        assert_eq!(decode("mzxw6ytboi").expect("lower"), b"foobar");
        assert_eq!(decode("MZXW6YTBOI").expect("plain"), b"foobar");
        assert_eq!(decode(" mzxw6ytb ").expect("spaced"), b"fooba");
    }

    #[test]
    fn decodes_an_authenticator_shaped_secret() {
        // The RFC 6238 SHA-1 test secret "12345678901234567890".
        let secret = decode("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ").expect("secret");
        assert_eq!(secret, b"12345678901234567890");
    }

    #[test]
    fn refuses_invalid_characters_with_position() {
        assert_eq!(
            decode("MZXW6YT1").expect_err("digit 1"),
            Base32Error::InvalidCharacter { position: 7 }
        );
        assert_eq!(
            decode("MZ XW6YTB").expect_err("space inside"),
            Base32Error::InvalidCharacter { position: 2 }
        );
        assert_eq!(
            decode("MZXW=6YTB").expect_err("padding inside"),
            Base32Error::InvalidCharacter { position: 5 }
        );
        assert_eq!(
            decode("mzxw6ytb!").expect_err("punctuation"),
            Base32Error::InvalidCharacter { position: 8 }
        );
    }

    #[test]
    fn refuses_unusable_lengths() {
        assert_eq!(
            decode("M").expect_err("one char"),
            Base32Error::InvalidLength { length: 1 }
        );
        assert_eq!(
            decode("MZX").expect_err("three chars"),
            Base32Error::InvalidLength { length: 3 }
        );
        assert_eq!(
            decode("MZXW6Y").expect_err("six chars"),
            Base32Error::InvalidLength { length: 6 }
        );
    }

    #[test]
    fn refuses_non_zero_trailing_bits_and_empty_input() {
        // "MB" decodes "f" with a non-zero low bit in the second character.
        assert_eq!(
            decode("MB").expect_err("trailing bits"),
            Base32Error::NonZeroTrailingBits
        );
        assert_eq!(decode("").expect_err("empty"), Base32Error::Empty);
        assert_eq!(decode("  ").expect_err("blank"), Base32Error::Empty);
        assert_eq!(
            decode("====").expect_err("padding only"),
            Base32Error::Empty
        );
    }
}
