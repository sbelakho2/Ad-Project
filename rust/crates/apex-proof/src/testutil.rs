//! Test-only helpers: deterministic keypairs, crafted ledger lines and a RAII
//! temp directory (no external crates).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ed25519_dalek::Signer;

use crate::record::LedgerRecord;
use crate::Keypair;

/// Deterministic keypair from a fixed seed byte.
pub(crate) fn keypair(seed: u8) -> Keypair {
    Keypair::from_bytes(&[seed; 32])
}

/// Seal a record outside a ledger (for crafting valid and deliberately
/// invalid chains in verifier tests).
pub(crate) fn seal_record(mut record: LedgerRecord, seq: u64, prev_hash: [u8; 32]) -> LedgerRecord {
    record.seal(seq, prev_hash).expect("seal test record");
    record
}

/// Serialize one persisted line (no trailing newline) through the same canonical
/// writer the ledger uses, signed with `keypair`. Byte-identical to what
/// [`crate::Ledger::append`] would write for this record.
pub(crate) fn signed_line(record: &LedgerRecord, keypair: &Keypair) -> String {
    let signature = keypair.sign(&record.hash());
    let mut bytes = crate::ledger::canonical_line(record, &signature, &keypair.verifying_key())
        .expect("serialize ledger line");
    bytes.pop(); // strip the trailing newline; write_lines re-adds it
    String::from_utf8(bytes).expect("ledger line is utf-8")
}

/// Unique temporary directory removed on drop (RAII cleanup).
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub(crate) fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let unique = format!(
            "apex-proof-{tag}-{}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    /// Path of a file inside the temp dir (not created).
    pub(crate) fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
