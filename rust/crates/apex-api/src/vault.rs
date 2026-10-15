//! Credential vault: AES-256-GCM at rest for publisher platform keys.
//!
//! The marketing claim ("Network credentials from publishers are kept encrypted
//! under AES-256-GCM") is anchored to this module and its tests. The key comes
//! from a deployment secret (`APEX_VAULT_KEY`, 64 lowercase hex characters) and
//! never from the repository; ciphertext format is `nonce hex + "." + ct hex`
//! so a stored blob is self-contained and versioned by construction.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("vault key must be 64 lowercase hex characters (32 bytes)")]
    BadKey,
    #[error("ciphertext is not in nonce.ciphertext hex format")]
    BadFormat,
    #[error("decryption failed (wrong key or tampered ciphertext)")]
    Decrypt,
}

pub struct Vault {
    cipher: Aes256Gcm,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never render key material: a derived Debug on the cipher could leak it.
        f.debug_struct("Vault").finish_non_exhaustive()
    }
}

impl Vault {
    /// Parse the deployment key from hex; any deviation is refused.
    pub fn from_hex_key(key_hex: &str) -> Result<Self, VaultError> {
        if key_hex.len() != 64
            || !key_hex
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err(VaultError::BadKey);
        }
        let bytes = hex::decode(key_hex).map_err(|_| VaultError::BadKey)?;
        let key = aes_gcm::Key::<Aes256Gcm>::from_slice(&bytes);
        Ok(Self {
            cipher: Aes256Gcm::new(key),
        })
    }

    /// Encrypt to `nonce-hex "." ciphertext-hex`. A fresh random nonce is drawn
    /// for every call; nonce reuse is the one fatal AES-GCM misuse.
    pub fn encrypt(&self, plaintext: &str) -> String {
        use aes_gcm::aead::rand_core::RngCore;
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext.as_bytes())
            .unwrap_or_else(|_| Vec::new());
        format!("{}.{}", hex::encode(nonce_bytes), hex::encode(ciphertext))
    }

    pub fn decrypt(&self, blob: &str) -> Result<String, VaultError> {
        let (nonce_hex, ct_hex) = blob.split_once('.').ok_or(VaultError::BadFormat)?;
        let nonce_bytes = hex::decode(nonce_hex).map_err(|_| VaultError::BadFormat)?;
        if nonce_bytes.len() != 12 {
            return Err(VaultError::BadFormat);
        }
        let ciphertext = hex::decode(ct_hex).map_err(|_| VaultError::BadFormat)?;
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext.as_ref())
            .map_err(|_| VaultError::Decrypt)?;
        String::from_utf8(plaintext).map_err(|_| VaultError::Decrypt)
    }
}

/// Evidence record the console can surface without revealing the secret.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct VaultEntryMeta {
    pub ciphertext_len: usize,
    pub scheme: &'static str,
}

pub fn entry_meta(blob: &str) -> VaultEntryMeta {
    VaultEntryMeta {
        ciphertext_len: blob.len(),
        scheme: "aes-256-gcm",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    const KEY_B: &str = "1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100";

    #[test]
    fn vault_roundtrip_and_tamper_detection() {
        let vault = Vault::from_hex_key(KEY_A).expect("key");
        let blob = vault.encrypt("publisher-secret-key-1");
        assert_eq!(
            vault.decrypt(&blob).expect("roundtrip"),
            "publisher-secret-key-1"
        );

        // Tamper the ciphertext body: decryption must fail, never garble.
        let (nonce, ct) = blob.split_once('.').expect("format");
        let mut ct_bytes = hex::decode(ct).expect("hex");
        ct_bytes[0] ^= 0x01;
        let tampered = format!("{nonce}.{}", hex::encode(&ct_bytes));
        assert!(matches!(vault.decrypt(&tampered), Err(VaultError::Decrypt)));

        // Tamper the nonce: also refused.
        let mut nonce_bytes = hex::decode(nonce).expect("hex");
        nonce_bytes[0] ^= 0x01;
        let tampered_nonce = format!("{}.{ct}", hex::encode(&nonce_bytes));
        assert!(matches!(
            vault.decrypt(&tampered_nonce),
            Err(VaultError::Decrypt)
        ));

        // Wrong key: refused.
        let other = Vault::from_hex_key(KEY_B).expect("key");
        assert!(matches!(other.decrypt(&blob), Err(VaultError::Decrypt)));
    }

    #[test]
    fn nonces_are_unique_per_encryption() {
        let vault = Vault::from_hex_key(KEY_A).expect("key");
        let a = vault.encrypt("same-plaintext");
        let b = vault.encrypt("same-plaintext");
        assert_ne!(a, b, "nonce reuse would be fatal for AES-GCM");
    }

    #[test]
    fn key_and_format_validation() {
        assert!(matches!(
            Vault::from_hex_key("short"),
            Err(VaultError::BadKey)
        ));
        assert!(matches!(
            Vault::from_hex_key(&KEY_A.to_uppercase()),
            Err(VaultError::BadKey)
        ));
        let vault = Vault::from_hex_key(KEY_A).expect("key");
        assert!(matches!(
            vault.decrypt("not-a-blob"),
            Err(VaultError::BadFormat)
        ));
        assert!(matches!(vault.decrypt("00.z"), Err(VaultError::BadFormat)));
        assert!(matches!(
            vault.decrypt("000102030405060708090a0b.zz"),
            Err(VaultError::BadFormat)
        ));
    }

    #[test]
    fn unicode_and_empty_plaintexts_roundtrip() {
        let vault = Vault::from_hex_key(KEY_A).expect("key");
        for text in ["", "võti-🔑-ключ", &"x".repeat(4096)] {
            let blob = vault.encrypt(text);
            assert_eq!(vault.decrypt(&blob).expect("roundtrip"), text);
        }
    }

    #[test]
    fn entry_meta_never_reveals_plaintext() {
        let vault = Vault::from_hex_key(KEY_A).expect("key");
        let blob = vault.encrypt("secret");
        let meta = entry_meta(&blob);
        assert_eq!(meta.scheme, "aes-256-gcm");
        assert!(meta.ciphertext_len > 0);
    }
}
