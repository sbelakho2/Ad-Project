//! Cross-cutting ledger tests: per-byte tamper classes, fail-closed appends,
//! rollback rules, file mode and file/in-memory equivalence.

use std::path::Path;

use apex_core::platform::ExperimentMetric;
use apex_core::{
    ExperimentResult, ExperimentSpec, ExperimentState, ExperimentVariantResult, PlatformId,
};
use ed25519_dalek::Signer;

use crate::errors::LedgerError;
use crate::record::{ChangeKind, LedgerRecord};
use crate::testutil::{keypair, seal_record, signed_line, TempDir};
use crate::{build_billing_proof, InMemoryLedger, Ledger};

fn change(op: u64) -> LedgerRecord {
    let op = i64::try_from(op).expect("test op fits i64");
    LedgerRecord::change_applied(
        1_700_000_000_000 + op * 1_000,
        PlatformId::Admob,
        "u1",
        ChangeKind::EcpmSet {
            mediation_group_id: "g1".into(),
            ad_unit_id: "u1".into(),
            ecpm: 1_750_000 + op,
        },
        &format!("admob-ecpm:g1:u1:{op}"),
        "prev-value",
    )
}

fn spec() -> ExperimentSpec {
    ExperimentSpec {
        name: "floors-ee".into(),
        ad_unit_id: "u1".into(),
        metric: ExperimentMetric::Ecpm,
        variants: 2,
        test_group_allocation: 50,
        change_description: "raise EE floor to 2.50".into(),
    }
}

fn result() -> ExperimentResult {
    ExperimentResult {
        experiment_id: "exp-1".into(),
        state: ExperimentState::Completed,
        results: vec![ExperimentVariantResult {
            variant_id: "control".into(),
            impressions: 10_000,
            revenue: 10_000_000,
            ecpm: 1_000_000,
        }],
        lift_percent_bp: Some(2_500),
        confidence: Some(9_800),
        winner_variant_id: Some("variant-a".into()),
    }
}

fn read_lines(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).expect("read ledger as utf-8");
    let body = text.strip_suffix('\n').expect("ledger ends with newline");
    if body.is_empty() {
        Vec::new()
    } else {
        body.split('\n').map(str::to_string).collect()
    }
}

fn write_lines(path: &Path, lines: &[String]) {
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::write(path, text).expect("write ledger");
}

/// Flip the first hex character of a `"field":"<hex>"` JSON string.
fn flip_hex_field(line: &str, field: &str) -> String {
    let marker = format!("\"{field}\":\"");
    let start = line.find(&marker).expect("field present") + marker.len();
    let mut bytes = line.as_bytes().to_vec();
    bytes[start] = if bytes[start] == b'0' { b'1' } else { b'0' };
    String::from_utf8(bytes).expect("line is utf-8")
}

fn append_sample(ledger: &mut Ledger) {
    ledger.append(change(0)).expect("append sample 0");
    ledger.append(change(1)).expect("append sample 1");
}

#[test]
fn empty_ledger_file_verifies_and_missing_file_is_an_io_error() {
    let dir = TempDir::new("empty");
    let path = dir.file("ledger.jsonl");
    std::fs::write(&path, b"").expect("write empty file");
    let keys = keypair(1);
    Ledger::verify(&path, &keys.verifying_key()).expect("empty chain verifies");
    let err = Ledger::verify(dir.file("missing.jsonl"), &keys.verifying_key())
        .expect_err("missing file must not verify");
    assert!(matches!(err, LedgerError::Io(_)), "{err}");
}

#[test]
fn read_verified_returns_the_records_from_the_verifying_read() {
    let dir = TempDir::new("read-verified");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(2);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);

    let records = Ledger::read_verified(&path, &keys.verifying_key()).expect("verified read");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].seq(), 0);
    assert_eq!(records[1].seq(), 1);
    assert_eq!(read_lines(&path).len(), 2);
}

#[test]
fn read_verified_of_an_empty_file_is_zero_records() {
    let dir = TempDir::new("read-verified-empty");
    let path = dir.file("ledger.jsonl");
    std::fs::write(&path, b"").expect("write empty file");
    let keys = keypair(3);

    let records = Ledger::read_verified(&path, &keys.verifying_key()).expect("empty verifies");
    assert!(records.is_empty());
}

#[test]
fn read_verified_names_the_tampered_line() {
    let dir = TempDir::new("read-verified-tamper");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(4);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    for op in 0..3 {
        ledger.append(change(op)).expect("append");
    }

    let mut lines = read_lines(&path);
    lines[1] = lines[1].replacen("admob-ecpm:g1:u1:1", "admob-ecpm:g1:u1:9", 1);
    write_lines(&path, &lines);

    let err = Ledger::read_verified(&path, &keys.verifying_key()).expect_err("tampered");
    assert!(
        matches!(err, LedgerError::HashMismatch { line: 2 }),
        "{err}"
    );
}

#[test]
fn first_append_is_genesis_and_links_are_consistent() {
    let dir = TempDir::new("genesis");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(41);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    assert!(ledger.is_empty());

    let first = ledger.append(change(0)).expect("first append");
    assert_eq!(first.seq(), 0, "genesis sequence is 0");
    assert_eq!(first.prev_hash(), [0u8; 32], "genesis prevHash is zeros");
    assert_ne!(first.hash(), [0u8; 32]);

    let second = ledger.append(change(1)).expect("second append");
    assert_eq!(second.seq(), 1);
    assert_eq!(second.prev_hash(), first.hash());

    Ledger::verify(&path, &keys.verifying_key()).expect("verify");
    let lines = read_lines(&path);
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains(&format!("\"hash\":\"{}\"", hex::encode(first.hash()))));
    assert!(lines[1].contains(&format!("\"prevHash\":\"{}\"", hex::encode(first.hash()))));
}

#[test]
fn reopen_resumes_the_on_disk_chain() {
    let dir = TempDir::new("reopen");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(29);
    {
        let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
        append_sample(&mut ledger);
    }
    let mut ledger = Ledger::open(&path, keys.clone()).expect("reopen");
    assert_eq!(ledger.len(), 2);
    let third = ledger.append(change(2)).expect("third append");
    assert_eq!(third.seq(), 2);
    assert_eq!(third.prev_hash(), ledger.get(1).expect("second").hash());
    Ledger::verify(&path, &keys.verifying_key()).expect("verify");
}

#[test]
fn every_single_byte_flip_is_rejected() {
    let dir = TempDir::new("flip");
    let path = dir.file("ledger.jsonl");
    let scratch = dir.file("mutated.jsonl");
    let keys = keypair(11);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    for op in 0..3 {
        ledger.append(change(op)).expect("append");
    }
    let original = std::fs::read(&path).expect("read ledger");
    Ledger::verify(&path, &keys.verifying_key()).expect("baseline verifies");

    for (index, byte) in original.iter().enumerate() {
        let mut mutated = original.clone();
        mutated[index] = byte ^ 0x01;
        std::fs::write(&scratch, &mutated).expect("write mutated");
        let result = Ledger::verify(&scratch, &keys.verifying_key());
        assert!(result.is_err(), "flipping byte {index} was accepted");
    }
}

#[test]
fn tampered_payload_is_a_hash_mismatch() {
    let dir = TempDir::new("payload");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(12);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);

    let mut lines = read_lines(&path);
    lines[0] = lines[0].replace("admob-ecpm:g1:u1:0", "admob-ecpm:g1:u1:9");
    write_lines(&path, &lines);

    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("tampered payload");
    assert!(
        matches!(err, LedgerError::HashMismatch { line: 1 }),
        "{err}"
    );
}

#[test]
fn tampered_seq_is_a_hash_mismatch() {
    let dir = TempDir::new("seq");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(13);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);

    let mut lines = read_lines(&path);
    lines[0] = lines[0].replacen("\"seq\":0", "\"seq\":5", 1);
    write_lines(&path, &lines);

    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("tampered seq");
    assert!(
        matches!(err, LedgerError::HashMismatch { line: 1 }),
        "{err}"
    );
}

#[test]
fn tampered_prev_hash_is_a_hash_mismatch() {
    let dir = TempDir::new("prev");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(14);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);

    let mut lines = read_lines(&path);
    lines[0] = lines[0].replace(
        &format!("\"prevHash\":\"{}\"", "00".repeat(32)),
        &format!("\"prevHash\":\"{}\"", "11".repeat(32)),
    );
    write_lines(&path, &lines);

    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("tampered prevHash");
    assert!(
        matches!(err, LedgerError::HashMismatch { line: 1 }),
        "{err}"
    );
}

#[test]
fn tampered_hash_is_a_bad_signature() {
    let dir = TempDir::new("hash");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(15);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);

    let mut lines = read_lines(&path);
    lines[0] = flip_hex_field(&lines[0], "hash");
    write_lines(&path, &lines);

    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("tampered hash");
    assert!(
        matches!(err, LedgerError::BadSignature { line: 1 }),
        "{err}"
    );
}

#[test]
fn tampered_signature_is_a_bad_signature() {
    let dir = TempDir::new("sig");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(16);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);

    let mut lines = read_lines(&path);
    lines[1] = flip_hex_field(&lines[1], "signature");
    write_lines(&path, &lines);

    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("tampered signature");
    assert!(
        matches!(err, LedgerError::BadSignature { line: 2 }),
        "{err}"
    );
}

#[test]
fn truncated_final_line_is_rejected() {
    let dir = TempDir::new("truncated");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(17);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);
    let bytes = std::fs::read(&path).expect("read");

    // Missing trailing newline only.
    std::fs::write(&path, &bytes[..bytes.len() - 1]).expect("write");
    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("no newline");
    assert!(matches!(err, LedgerError::Truncated { line: 2 }), "{err}");

    // Cut mid-line.
    let cut = bytes.len() - 10;
    std::fs::write(&path, &bytes[..cut]).expect("write");
    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("mid-line cut");
    assert!(matches!(err, LedgerError::Truncated { line: 2 }), "{err}");
}

#[test]
fn swapped_and_reordered_lines_are_rejected() {
    let dir = TempDir::new("swapped");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(18);
    let r0 = seal_record(change(0), 0, [0u8; 32]);
    let r1 = seal_record(change(1), 1, r0.hash());
    let r2 = seal_record(change(2), 2, r1.hash());

    // Reordered (swap the last two lines): line 2 no longer links to line 1.
    write_lines(
        &path,
        &[
            signed_line(&r0, &keys),
            signed_line(&r2, &keys),
            signed_line(&r1, &keys),
        ],
    );
    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("reordered");
    assert!(matches!(err, LedgerError::ChainBreak { line: 2 }), "{err}");

    // Swapped (later record first): its prevHash is not the genesis zeros.
    write_lines(&path, &[signed_line(&r1, &keys), signed_line(&r0, &keys)]);
    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("swapped");
    assert!(matches!(err, LedgerError::Genesis { line: 1 }), "{err}");
}

#[test]
fn sequence_gap_is_rejected() {
    let dir = TempDir::new("gap");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(19);
    let r0 = seal_record(change(0), 0, [0u8; 32]);
    // Valid chain link, but seq jumps from 0 to 2 (signed with the real key, so
    // signature and hash are fine; only the sequence rule fails).
    let r2 = seal_record(change(2), 2, r0.hash());
    write_lines(&path, &[signed_line(&r0, &keys), signed_line(&r2, &keys)]);

    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("gap");
    assert!(
        matches!(
            err,
            LedgerError::SequenceGap {
                line: 2,
                expected: 1,
                found: 2
            }
        ),
        "{err}"
    );
}

#[test]
fn duplicate_sequence_is_rejected() {
    let dir = TempDir::new("duplicate");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(20);
    let r0 = seal_record(change(0), 0, [0u8; 32]);
    // Links correctly but repeats seq 0.
    let duplicate = seal_record(change(1), 0, r0.hash());
    write_lines(
        &path,
        &[signed_line(&r0, &keys), signed_line(&duplicate, &keys)],
    );

    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("duplicate");
    assert!(
        matches!(err, LedgerError::Duplicate { line: 2, seq: 0 }),
        "{err}"
    );
}

#[test]
fn genesis_break_is_rejected() {
    let dir = TempDir::new("genesis-break");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(21);
    // A perfectly signed first record that does not start from zeros.
    let orphan = seal_record(change(0), 0, [9u8; 32]);
    write_lines(&path, &[signed_line(&orphan, &keys)]);

    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("genesis break");
    assert!(matches!(err, LedgerError::Genesis { line: 1 }), "{err}");
}

#[test]
fn wrong_public_key_is_rejected() {
    let dir = TempDir::new("wrong-key");
    let path = dir.file("ledger.jsonl");
    let owner = keypair(22);
    let mut ledger = Ledger::open(&path, owner.clone()).expect("open");
    append_sample(&mut ledger);

    // Whole file signed by another key.
    let err = Ledger::verify(&path, &keypair(23).verifying_key()).expect_err("foreign key");
    assert!(matches!(err, LedgerError::WrongPubkey { line: 1 }), "{err}");

    // Mixed file: second line signed by a different key.
    let second = ledger.get(1).expect("second record").clone();
    let mut lines = read_lines(&path);
    lines[1] = signed_line(&second, &keypair(24));
    write_lines(&path, &lines);
    let err = Ledger::verify(&path, &owner.verifying_key()).expect_err("mixed keys");
    assert!(matches!(err, LedgerError::WrongPubkey { line: 2 }), "{err}");
}

#[test]
fn malformed_lines_are_rejected_with_line_numbers() {
    let dir = TempDir::new("malformed");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(25);

    std::fs::write(&path, b"not json\n").expect("write");
    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("bad json");
    assert!(
        matches!(err, LedgerError::Malformed { line: 1, .. }),
        "{err}"
    );

    std::fs::write(&path, b"").expect("reset");
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);
    let lines = read_lines(&path);
    write_lines(&path, &[lines[0].clone(), String::new(), lines[1].clone()]);
    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("blank line");
    assert!(
        matches!(err, LedgerError::Malformed { line: 2, .. }),
        "{err}"
    );
}

#[test]
fn append_refuses_a_broken_chain_and_leaves_the_file_untouched() {
    let dir = TempDir::new("broken-append");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(26);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);

    // Forge a canonical line whose payload differs from the stored hash: the
    // clone keeps the original `hash` (and we sign it), so the byte-exact
    // canonical check passes and the hash recomputation MUST catch the tamper.
    let original = ledger.get(0).expect("record 0").clone();
    let mut forged = original;
    match &mut forged {
        LedgerRecord::ChangeApplied {
            idempotency_key, ..
        } => {
            *idempotency_key = format!("{idempotency_key}-tampered");
        }
        other => panic!("unexpected record {other:?}"),
    }
    let signature = keys.sign(&forged.hash());
    let mut forged_line =
        crate::ledger::canonical_line(&forged, &signature, &keys.verifying_key()).expect("forge");
    forged_line.pop();
    let mut lines = read_lines(&path);
    lines[0] = String::from_utf8(forged_line).expect("utf8");
    write_lines(&path, &lines);
    let broken = std::fs::read(&path).expect("read broken file");

    let err = ledger
        .append(change(2))
        .expect_err("append must fail closed");
    assert!(
        matches!(err, LedgerError::HashMismatch { line: 1 }),
        "{err}"
    );
    assert_eq!(std::fs::read(&path).expect("read"), broken);
    assert_eq!(ledger.len(), 2, "in-memory state is unchanged");
}

#[test]
fn append_refuses_a_truncated_file() {
    let dir = TempDir::new("truncated-append");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(27);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);

    let bytes = std::fs::read(&path).expect("read");
    std::fs::write(&path, &bytes[..bytes.len() - 1]).expect("write truncated");
    let err = ledger
        .append(change(2))
        .expect_err("append must fail closed");
    assert!(matches!(err, LedgerError::Truncated { line: 2 }), "{err}");
    assert_eq!(ledger.len(), 2);
}

#[test]
fn rollback_rules_are_enforced() {
    let dir = TempDir::new("rollback");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(28);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    ledger.append(change(0)).expect("change");

    let rollback = ledger
        .append_rollback(1_700_000_100_000, PlatformId::Admob, "u1", 0, "regression")
        .expect("rollback");
    assert_eq!(rollback.seq(), 1);

    let err = ledger
        .append_rollback(1_700_000_200_000, PlatformId::Admob, "u1", 0, "again")
        .expect_err("double rollback");
    assert!(
        matches!(err, LedgerError::DuplicateRollback { seq: 0 }),
        "{err}"
    );

    let err = ledger
        .append_rollback(1_700_000_300_000, PlatformId::Admob, "u1", 1, "meta")
        .expect_err("rollback of rollback");
    assert!(
        matches!(err, LedgerError::RollbackOfRollback { seq: 1 }),
        "{err}"
    );

    let err = ledger
        .append_rollback(1_700_000_400_000, PlatformId::Admob, "u1", 99, "ghost")
        .expect_err("unknown target");
    assert!(
        matches!(err, LedgerError::UnknownRollbackTarget { seq: 99 }),
        "{err}"
    );

    assert_eq!(ledger.len(), 2, "refused rollbacks are not appended");
    Ledger::verify(&path, &keys.verifying_key()).expect("chain still verifies");
}

#[test]
fn in_memory_rollback_rules_match_the_file_ledger() {
    let mut memory = InMemoryLedger::new(keypair(30));
    memory.append(change(0)).expect("change");
    memory
        .append_rollback(1_700_000_100_000, PlatformId::Admob, "u1", 0, "regression")
        .expect("rollback");

    let err = memory
        .append_rollback(1_700_000_200_000, PlatformId::Admob, "u1", 0, "again")
        .expect_err("double rollback");
    assert!(
        matches!(err, LedgerError::DuplicateRollback { seq: 0 }),
        "{err}"
    );
    let err = memory
        .append_rollback(1_700_000_300_000, PlatformId::Admob, "u1", 1, "meta")
        .expect_err("rollback of rollback");
    assert!(
        matches!(err, LedgerError::RollbackOfRollback { seq: 1 }),
        "{err}"
    );
    let err = memory
        .append_rollback(1_700_000_400_000, PlatformId::Admob, "u1", 7, "ghost")
        .expect_err("unknown target");
    assert!(
        matches!(err, LedgerError::UnknownRollbackTarget { seq: 7 }),
        "{err}"
    );
    assert_eq!(memory.len(), 2);
    memory.verify().expect("chain verifies");
}

#[cfg(unix)]
#[test]
fn ledger_file_is_created_with_mode_0600() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new("mode");
    let path = dir.file("ledger.jsonl");
    let _ledger = Ledger::open(&path, keypair(31)).expect("open");
    let mode = std::fs::metadata(&path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn billing_proof_chains_end_to_end() {
    let dir = TempDir::new("billing");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(32);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");

    let proof = build_billing_proof(
        "pub-1",
        "exp-1",
        10_000_000,
        12_500_000,
        2_000,
        1_700_000_000_000,
    )
    .expect("proof");
    let sealed = ledger.append(proof).expect("append");

    match sealed {
        LedgerRecord::BillingProof {
            seq,
            publisher_id,
            experiment_id,
            proven_lift,
            rate_bp,
            fee_micros,
            ..
        } => {
            assert_eq!(seq, 0);
            assert_eq!(publisher_id, "pub-1");
            assert_eq!(experiment_id, "exp-1");
            assert_eq!(proven_lift, 2_500_000);
            assert_eq!(rate_bp, 2_000);
            assert_eq!(fee_micros, 500_000);
        }
        other => panic!("unexpected record {other:?}"),
    }
    Ledger::verify(&path, &keys.verifying_key()).expect("verify");
}

/// The shared append/rollback/verify surface, so one script can drive both
/// ledger implementations.
trait Chain {
    fn append_record(&mut self, record: LedgerRecord) -> Result<LedgerRecord, LedgerError>;
    fn append_rollback_record(
        &mut self,
        at_unix_ms: i64,
        platform: PlatformId,
        ad_unit_id: &str,
        rolled_back_seq: u64,
        reason: &str,
    ) -> Result<LedgerRecord, LedgerError>;
    fn verify_chain(&self) -> Result<(), LedgerError>;
}

impl Chain for Ledger {
    fn append_record(&mut self, record: LedgerRecord) -> Result<LedgerRecord, LedgerError> {
        self.append(record)
    }

    fn append_rollback_record(
        &mut self,
        at_unix_ms: i64,
        platform: PlatformId,
        ad_unit_id: &str,
        rolled_back_seq: u64,
        reason: &str,
    ) -> Result<LedgerRecord, LedgerError> {
        self.append_rollback(at_unix_ms, platform, ad_unit_id, rolled_back_seq, reason)
    }

    fn verify_chain(&self) -> Result<(), LedgerError> {
        Ledger::verify(self.path(), self.public_key())
    }
}

impl Chain for InMemoryLedger {
    fn append_record(&mut self, record: LedgerRecord) -> Result<LedgerRecord, LedgerError> {
        self.append(record)
    }

    fn append_rollback_record(
        &mut self,
        at_unix_ms: i64,
        platform: PlatformId,
        ad_unit_id: &str,
        rolled_back_seq: u64,
        reason: &str,
    ) -> Result<LedgerRecord, LedgerError> {
        self.append_rollback(at_unix_ms, platform, ad_unit_id, rolled_back_seq, reason)
    }

    fn verify_chain(&self) -> Result<(), LedgerError> {
        self.verify()
    }
}

fn scripted_sequence(chain: &mut impl Chain) {
    chain.append_record(change(0)).expect("append change");
    chain
        .append_record(LedgerRecord::experiment_started(
            1_700_000_010_000,
            PlatformId::AppLovinMax,
            "exp-1",
            spec(),
        ))
        .expect("append experiment");
    chain
        .append_record(LedgerRecord::experiment_observed(
            1_700_000_020_000,
            PlatformId::AppLovinMax,
            "exp-1",
            result(),
        ))
        .expect("append observation");
    let proof = build_billing_proof(
        "pub-1",
        "exp-1",
        10_000_000,
        12_500_000,
        2_000,
        1_700_000_030_000,
    )
    .expect("proof");
    chain.append_record(proof).expect("append proof");
    chain
        .append_rollback_record(1_700_000_040_000, PlatformId::Admob, "u1", 0, "regression")
        .expect("append rollback");
    chain.verify_chain().expect("scripted chain verifies");
}

#[test]
fn in_memory_ledger_matches_the_file_ledger_on_a_scripted_sequence() {
    let keys = keypair(33);
    let dir = TempDir::new("equivalence");
    let path = dir.file("ledger.jsonl");

    let mut file_ledger = Ledger::open(&path, keys.clone()).expect("open");
    let mut memory = InMemoryLedger::new(keys.clone());
    scripted_sequence(&mut file_ledger);
    scripted_sequence(&mut memory);

    assert_eq!(file_ledger.len(), 5);
    assert_eq!(memory.len(), 5);
    for index in 0..file_ledger.len() {
        assert_eq!(
            file_ledger.get(index),
            memory.get(index),
            "record {index} differs"
        );
    }

    file_ledger.verify_chain().expect("file chain verifies");
    memory.verify().expect("memory chain verifies");
    Ledger::verify(&path, memory.public_key()).expect("static verifier agrees");
}

#[test]
fn in_memory_verify_detects_tampering() {
    let mut memory = InMemoryLedger::new(keypair(34));
    memory.append(change(0)).expect("append");
    memory.append(change(1)).expect("append");

    let entry = memory.entry_mut(0).expect("entry 0");
    // Mutate a payload field in place: the stored hash stays the same, so the
    // signature still verifies and the tamper must be caught by hashing.
    match &mut entry.record {
        LedgerRecord::ChangeApplied { change, .. } => {
            *change = ChangeKind::CountryFloorSet {
                country: "DE".into(),
                floor_ecpm: 1,
            };
        }
        other => panic!("unexpected record {other:?}"),
    }
    let err = memory.verify().expect_err("tampered memory chain");
    assert!(
        matches!(err, LedgerError::HashMismatch { line: 1 }),
        "{err}"
    );
    let err = memory.append(change(2)).expect_err("append must refuse");
    assert!(
        matches!(err, LedgerError::HashMismatch { line: 1 }),
        "{err}"
    );
}

// ---------------- verifier-requested regressions (adversarial re-attack) ----------------

#[test]
fn injected_unknown_fields_are_rejected() {
    let dir = TempDir::new("injected-field");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(50);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);

    // Attacker adds fields to the signed record (top level and nested).
    let mut lines = read_lines(&path);
    let mut value: serde_json::Value = serde_json::from_str(&lines[0]).expect("parse");
    value["record"]["evil"] = serde_json::json!("injected-by-attacker");
    value["record"]["change"]["approvedBy"] = serde_json::json!("CFO");
    lines[0] = serde_json::to_string(&value).expect("serialize");
    write_lines(&path, &lines);

    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("injection must fail");
    assert!(
        matches!(err, LedgerError::Malformed { line: 1, ref detail } if detail.contains("canonical")),
        "{err}"
    );
}

#[test]
fn hex_case_and_whitespace_edits_are_rejected() {
    let dir = TempDir::new("hex-case");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(51);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);
    let pristine = read_lines(&path);

    // Uppercase one signature hex letter XOR-style (byte 0x20): decodes to the
    // same signature bytes, but the canonical encoding has exactly one form.
    let mut upper = pristine.clone();
    let line = upper[0].clone();
    let sig_marker = "\"signature\":\"";
    let start = line.find(sig_marker).expect("signature present") + sig_marker.len();
    let mut bytes = line.into_bytes();
    // Flip a hex *letter*, not a digit: digits have no case, so uppercasing
    // one is a no-op that would make the tampering invisible.
    for byte in bytes[start..].iter_mut() {
        if byte.is_ascii_alphabetic() {
            *byte ^= 0x20;
            break;
        }
    }
    upper[0] = String::from_utf8(bytes).expect("utf8");
    write_lines(&path, &upper);
    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("uppercase hex");
    assert!(
        matches!(err, LedgerError::Malformed { line: 1, .. }),
        "{err}"
    );

    // Whitespace edit inside the record.
    let mut spaced = pristine.clone();
    spaced[0] = spaced[0].replacen("\"seq\":0", "\"seq\" : 0", 1);
    write_lines(&path, &spaced);
    let err = Ledger::verify(&path, &keys.verifying_key()).expect_err("whitespace");
    assert!(
        matches!(err, LedgerError::Malformed { line: 1, .. }),
        "{err}"
    );
}

#[test]
fn rollbacks_must_target_the_same_platform_and_ad_unit_change() {
    let dir = TempDir::new("rollback-target");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(52);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    ledger.append(change(0)).expect("change"); // Admob, u1, seq 0

    // Cross-platform rollback of an Admob change.
    let err = ledger
        .append_rollback(1, PlatformId::LevelPlay, "u9", 0, "cross-platform")
        .expect_err("cross-platform rollback");
    assert!(
        matches!(err, LedgerError::RollbackTargetMismatch { seq: 0 }),
        "{err}"
    );

    // Same platform but a different ad unit.
    let err = ledger
        .append_rollback(1, PlatformId::Admob, "u9", 0, "wrong unit")
        .expect_err("wrong unit");
    assert!(
        matches!(err, LedgerError::RollbackTargetMismatch { seq: 0 }),
        "{err}"
    );

    // A non-change record cannot be rolled back either.
    ledger
        .append(LedgerRecord::experiment_started(
            2,
            PlatformId::Admob,
            "exp-1",
            spec(),
        ))
        .expect("experiment");
    let err = ledger
        .append_rollback(3, PlatformId::Admob, "u1", 1, "not a change")
        .expect_err("non-change target");
    assert!(
        matches!(err, LedgerError::RollbackTargetMismatch { seq: 1 }),
        "{err}"
    );

    // The legitimate rollback still works.
    ledger
        .append_rollback(4, PlatformId::Admob, "u1", 0, "regression")
        .expect("legitimate rollback");
}

#[test]
fn forged_billing_records_are_refused_at_append() {
    let dir = TempDir::new("forged-billing");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(53);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");

    // Wrong fee for the lift/rate pair.
    let forged = LedgerRecord::billing_proof(1, "p", "exp-1", 2_500_000, 2_000, 999_999);
    let err = ledger.append(forged).expect_err("fee mismatch");
    assert!(
        matches!(err, LedgerError::InvalidBillingProof { line: 1, .. }),
        "{err}"
    );

    // Negative lift.
    let negative = LedgerRecord::billing_proof(1, "p", "exp-1", -1, 1_800, 0);
    let err = ledger.append(negative).expect_err("negative lift");
    assert!(
        matches!(err, LedgerError::InvalidBillingProof { line: 1, .. }),
        "{err}"
    );

    // Rate outside the business bounds.
    let bad_rate = LedgerRecord::billing_proof(1, "p", "exp-1", 2_500_000, 2_500, 500_000);
    let err = ledger.append(bad_rate).expect_err("bad rate");
    assert!(
        matches!(err, LedgerError::InvalidBillingProof { line: 1, .. }),
        "{err}"
    );

    assert!(ledger.is_empty(), "no forged money record was persisted");

    // The legitimate builder still lands.
    let good =
        build_billing_proof("p", "exp-1", 50_000_000, 52_500_000, 2_000, 10).expect("builder");
    ledger.append(good).expect("legitimate proof");
    assert_eq!(ledger.len(), 1);
}

// ---------------- verifier-round 2 regressions ----------------

#[test]
fn append_refuses_on_same_length_prefix_tamper() {
    // The length-equality fast path once let this through: rewrite the payload
    // in place with identical byte count, then append on the open handle.
    let dir = TempDir::new("samelen-append");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(60);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);
    append_sample(&mut ledger);

    let mut lines = read_lines(&path);
    lines[0] = lines[0].replacen("admob-ecpm:g1:u1:0", "admob-ecpm:g1:u1:9", 1);
    let before = lines[0].len();
    write_lines(&path, &lines);
    let after = read_lines(&path)[0].len();
    assert_eq!(
        before, after,
        "tamper must be same-length for this regression"
    );

    let err = ledger
        .append(change(9))
        .expect_err("append must refuse a same-length tampered prefix");
    assert!(
        matches!(err, LedgerError::HashMismatch { line: 1 }),
        "{err}"
    );
}

#[test]
fn append_refuses_when_the_file_was_swapped_for_a_shorter_valid_chain() {
    let dir = TempDir::new("shorter-swap");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(61);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);
    append_sample(&mut ledger);

    let only_first = {
        let mut short = Ledger::open(dir.file("short.jsonl"), keys.clone()).expect("short");
        short.append(change(0)).expect("one record");
        std::fs::read(dir.file("short.jsonl")).expect("bytes")
    };
    std::fs::write(&path, only_first).expect("swap file");
    let err = ledger.append(change(9)).expect_err("shorter swap refused");
    assert!(matches!(err, LedgerError::StateDiverged { .. }), "{err}");
}
