//! Cross-cutting ledger tests: per-byte tamper classes, fail-closed appends,
//! rollback rules, file mode and file/in-memory equivalence.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use apex_core::platform::ExperimentMetric;
use apex_core::{
    ExperimentResult, ExperimentSpec, ExperimentState, ExperimentVariantResult, PlatformId,
};
use ed25519_dalek::{Signature, Signer, VerifyingKey};

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
fn append_refuses_a_malformed_external_tail() {
    let dir = TempDir::new("malformed-append-tail");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(66);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);

    let mut bytes = std::fs::read(&path).expect("read");
    bytes.extend_from_slice(b"not json\n");
    std::fs::write(&path, &bytes).expect("write malformed tail");
    let err = ledger
        .append(change(2))
        .expect_err("append must refuse a malformed external tail");
    assert!(
        matches!(err, LedgerError::Malformed { line: 3, .. }),
        "{err}"
    );
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
    let cut = bytes.len() - 5;
    assert_ne!(bytes[cut - 1], b'\n', "the cut is inside the second line");
    std::fs::write(&path, &bytes[..cut]).expect("write truncated");
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
fn append_refuses_on_same_length_prefix_tamper_with_restored_mtime() {
    // The length-equality fast path once let this through: rewrite the payload
    // in place with identical byte count and restore the mtime, so only the
    // identity's ctime (or the content digest) can catch it. Then append on the
    // open handle.
    let dir = TempDir::new("samelen-append");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(60);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    append_sample(&mut ledger);
    append_sample(&mut ledger);

    let modified = std::fs::metadata(&path)
        .expect("stat")
        .modified()
        .expect("mtime");
    let mut lines = read_lines(&path);
    lines[0] = lines[0].replacen("admob-ecpm:g1:u1:0", "admob-ecpm:g1:u1:9", 1);
    let before = lines[0].len();
    write_lines(&path, &lines);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .expect("open for mtime")
        .set_modified(modified)
        .expect("restore mtime");
    let after = read_lines(&path)[0].len();
    assert_eq!(
        before, after,
        "tamper must be same-length for this regression"
    );
    assert_eq!(
        std::fs::metadata(&path)
            .expect("stat")
            .modified()
            .expect("mtime"),
        modified,
        "the mtime is restored; only ctime and the content digest remain"
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
    assert!(
        matches!(err, LedgerError::StateDiverged { line: 2 }),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn append_reverifies_only_the_tail_when_the_prefix_is_unchanged() {
    let dir = TempDir::new("tail-fast-path");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(62);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    for index in 0..40 {
        ledger.append(change(index)).expect("append");
    }
    crate::ledger::reset_verified_lines();
    crate::ledger::reset_append_prefix_metrics();
    ledger.append(change(40)).expect("append");
    ledger.append(change(41)).expect("append");
    assert_eq!(
        crate::ledger::verified_lines(),
        2,
        "only the two appended lines are parsed and checked"
    );
    assert_eq!(
        crate::ledger::append_prefix_reads(),
        0,
        "the unchanged prefix is never re-read on consecutive self-appends"
    );
    assert_eq!(
        crate::ledger::append_prefix_hashed(),
        0,
        "the unchanged prefix is never re-hashed on consecutive self-appends"
    );

    // Control: a fresh open re-verifies every line of the same file.
    crate::ledger::reset_verified_lines();
    let reopened = Ledger::open(&path, keys).expect("reopen");
    assert_eq!(reopened.len(), 42);
    assert_eq!(
        crate::ledger::verified_lines(),
        42,
        "a fresh open verifies the whole file"
    );
}

#[cfg(unix)]
#[test]
fn the_first_append_after_open_reads_no_prefix() {
    let dir = TempDir::new("open-first-append");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(64);
    let mut ledger = Ledger::open(&path, keys).expect("open");
    // The identity captured by open is stable across the verifying read, so
    // even the first append needs no prefix read or line verification.
    crate::ledger::reset_verified_lines();
    crate::ledger::reset_append_prefix_metrics();
    ledger.append(change(0)).expect("append");
    assert_eq!(crate::ledger::append_prefix_reads(), 0);
    assert_eq!(crate::ledger::append_prefix_hashed(), 0);
    assert_eq!(crate::ledger::verified_lines(), 0);
}

#[test]
fn every_append_is_one_durable_write_and_one_chain_link() {
    let dir = TempDir::new("append-durability");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(66);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");

    for index in 0..8u64 {
        crate::ledger::reset_write_metrics();
        let record = ledger.append(change(index)).expect("append");
        assert_eq!(record.seq(), index);
        assert_eq!(
            crate::ledger::write_calls(),
            1,
            "record {index}: one durable write"
        );
        assert_eq!(crate::ledger::sync_calls(), 1, "record {index}: one sync");
    }

    let verified = Ledger::read_verified(&path, &keys.verifying_key()).expect("verify");
    assert_eq!(verified.len(), 8);
    for (index, record) in verified.iter().enumerate() {
        assert_eq!(record.seq(), index as u64);
        if index > 0 {
            assert_eq!(record.prev_hash(), verified[index - 1].hash());
        }
    }
}

#[test]
fn per_record_appends_produce_the_byte_identical_chain() {
    let dir = TempDir::new("append-bytes");
    let first_path = dir.file("first.jsonl");
    let second_path = dir.file("second.jsonl");
    let keys = keypair(67);
    let mut first = Ledger::open(&first_path, keys.clone()).expect("open first");
    let mut second = Ledger::open(&second_path, keys.clone()).expect("open second");

    // The same drafts appended one at a time on two independently opened
    // handles must produce exactly the same bytes: sealing, signing and the
    // canonical line format are deterministic.
    for index in 0..8u64 {
        first.append(change(index)).expect("first append");
        let sealed = second.append(change(index)).expect("second append");
        assert_eq!(sealed.seq(), index);
    }
    let first_bytes = std::fs::read(&first_path).expect("read first");
    let second_bytes = std::fs::read(&second_path).expect("read second");
    assert_eq!(
        first_bytes, second_bytes,
        "one record per append, same bytes"
    );
}

#[test]
fn a_refused_append_writes_nothing() {
    let dir = TempDir::new("append-refused");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(68);
    let mut ledger = Ledger::open(&path, keys).expect("open");
    ledger.append(change(0)).expect("prefix");
    let before = std::fs::read(&path).expect("bytes");

    // A BillingProof with non-positive lift is refused by the chain validator
    // before any byte is written.
    let bad = LedgerRecord::billing_proof(1_700_000_000_000, "p", "e", 0, 2_000, 0);
    crate::ledger::reset_write_metrics();
    let error = ledger.append(bad).expect_err("invalid append");
    assert!(
        matches!(error, LedgerError::InvalidBillingProof { line: 2, .. }),
        "{error}"
    );
    assert_eq!(std::fs::read(&path).expect("bytes"), before);
    assert_eq!(ledger.len(), 1);
    assert_eq!(crate::ledger::write_calls(), 0);
    assert_eq!(crate::ledger::sync_calls(), 0);
}

#[test]
fn every_complete_line_prefix_of_a_per_record_ledger_verifies() {
    let dir = TempDir::new("append-crash");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(69);
    let mut ledger = Ledger::open(&path, keys.clone()).expect("open");
    let count = 24usize;
    for index in 0..count as u64 {
        ledger.append(change(index)).expect("append");
    }
    let bytes = std::fs::read(&path).expect("bytes");
    let line_ends: Vec<usize> = bytes
        .iter()
        .enumerate()
        .filter_map(|(index, byte)| (*byte == b'\n').then_some(index + 1))
        .collect();
    assert_eq!(line_ends.len(), count);

    // A crash at a line boundary leaves exactly the completed prefix, which
    // is a fully valid chain because every record was synced on its own.
    for (index, end) in line_ends.iter().enumerate() {
        std::fs::write(&path, &bytes[..*end]).expect("prefix");
        let records = Ledger::read_verified(&path, &keys.verifying_key()).expect("prefix verifies");
        assert_eq!(records.len(), index + 1);
        assert_eq!(records[index].seq(), index as u64);
    }

    // A crash inside the next line is detected as a truncated final line,
    // never accepted as a shortened record.
    let partial = line_ends[10] + (line_ends[11] - line_ends[10]) / 2;
    std::fs::write(&path, &bytes[..partial]).expect("torn");
    let error = Ledger::read_verified(&path, &keys.verifying_key()).expect_err("torn line");
    assert!(
        matches!(error, LedgerError::Truncated { line: 12 }),
        "{error}"
    );
}

#[test]
fn an_external_append_is_tail_verified_and_picked_up() {
    let dir = TempDir::new("tail-external");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(63);
    let mut first = Ledger::open(&path, keys.clone()).expect("first handle");
    let mut second = Ledger::open(&path, keys.clone()).expect("second handle");
    first.append(change(0)).expect("record 0");
    first.append(change(1)).expect("record 1");
    // The second handle's identity is the empty file, so its append detects
    // the external writes. Its verified offset is 0: refresh re-reads the file,
    // the content digest still matches the empty verified prefix, and the two
    // external lines are verified in order (signature, canonical encoding,
    // chain, seq) before its own record with seq 2 is linked to that tail.
    crate::ledger::reset_verified_lines();
    crate::ledger::reset_append_prefix_metrics();
    second.append(change(2)).expect("record 2");
    assert_eq!(second.len(), 3);
    assert_eq!(second.get(2).expect("third").seq(), 2);
    assert_eq!(
        second.get(2).expect("third").prev_hash(),
        second.get(1).expect("second").hash()
    );
    assert_eq!(
        crate::ledger::append_prefix_reads(),
        1,
        "an identity mismatch takes the documented full-read fallback"
    );
    assert_eq!(
        crate::ledger::verified_lines(),
        2,
        "the fallback verifies only the two lines past the verified prefix"
    );

    // The first handle now detects the second handle's write. Its running
    // prefix digest still matches, so only the tail it had not seen is
    // re-verified before it appends seq 3: its own lazily-unverified seq 1
    // line plus the externally written seq 2 line.
    crate::ledger::reset_verified_lines();
    crate::ledger::reset_append_prefix_metrics();
    first.append(change(3)).expect("record 3");
    assert_eq!(
        crate::ledger::append_prefix_reads(),
        1,
        "the identity mismatch is detected by the first handle"
    );
    assert_eq!(
        crate::ledger::verified_lines(),
        2,
        "the content-digest fallback verifies only the two unseen tail lines"
    );
    assert_eq!(first.len(), 4);

    let records = Ledger::read_verified(&path, &keys.verifying_key()).expect("verify");
    assert_eq!(records.len(), 4);
    assert_eq!(records[3].prev_hash(), records[2].hash());
}

#[test]
fn two_handles_interleave_by_detecting_each_others_identity_change() {
    // The ledger is single-writer by contract, but the identity gate makes an
    // out-of-contract interleaving safe rather than corrupting: each writer's
    // next append detects the other's ctime/len change, re-verifies the valid
    // chain (tail-only when the digest still matches) and appends the next
    // sequence linked to the externally written tip. A shorter valid chain
    // would instead be refused with StateDiverged.
    let dir = TempDir::new("two-handle-interleave");
    let path = dir.file("ledger.jsonl");
    let keys = keypair(65);
    let mut a = Ledger::open(&path, keys.clone()).expect("handle a");
    let mut b = Ledger::open(&path, keys.clone()).expect("handle b");

    a.append(change(0)).expect("a seq 0");
    a.append(change(1)).expect("a seq 1");

    // B's identity is still the empty file: it detects A's two appends.
    crate::ledger::reset_append_prefix_metrics();
    b.append(change(2)).expect("b seq 2");
    assert_eq!(
        crate::ledger::append_prefix_reads(),
        1,
        "handle B detected handle A through the file identity"
    );
    assert_eq!(b.len(), 3);

    // A's identity is the two-line file it left: it detects B's append.
    crate::ledger::reset_append_prefix_metrics();
    a.append(change(3)).expect("a seq 3");
    assert_eq!(
        crate::ledger::append_prefix_reads(),
        1,
        "handle A detected handle B through the file identity"
    );
    assert_eq!(a.len(), 4);

    let records = Ledger::read_verified(&path, &keys.verifying_key()).expect("verify");
    assert_eq!(records.len(), 4);
    for (index, record) in records.iter().enumerate() {
        assert_eq!(
            record.seq(),
            u64::try_from(index).expect("index"),
            "seq order"
        );
    }
    for pair in records.windows(2) {
        assert_eq!(pair[1].prev_hash(), pair[0].hash(), "link chain");
    }
}

#[test]
fn the_ledger_log_trait_covers_both_backings() {
    use crate::LedgerLog;

    // In-memory backend.
    let mut memory = InMemoryLedger::new(keypair(7));
    let appended = LedgerLog::append(
        &mut memory,
        LedgerRecord::change_applied(
            1,
            PlatformId::Admob,
            "u1",
            ChangeKind::EcpmSet {
                mediation_group_id: "g".into(),
                ad_unit_id: "u1".into(),
                ecpm: 1,
            },
            "key-1",
            "prev",
        ),
    )
    .expect("append");
    assert_eq!(appended.seq(), 0);
    assert!(LedgerLog::get(&memory, 0).is_some());
    assert_eq!(LedgerLog::len(&memory), 1);
    assert!(!LedgerLog::is_empty(&memory));
    assert_eq!(LedgerLog::records(&memory).len(), 1);
    let rollback = LedgerLog::append_rollback(&mut memory, 2, PlatformId::Admob, "u1", 0, "why")
        .expect("memory rollback");
    assert_eq!(rollback.seq(), 1);
    assert_eq!(LedgerLog::len(&memory), 2);

    // File backend over the same records.
    let dir = TempDir::new("ledger-log-file");
    let path = dir.file("ledger.jsonl");
    let mut file = Ledger::open(&path, keypair(7)).expect("open");
    LedgerLog::append(
        &mut file,
        LedgerRecord::change_applied(
            1,
            PlatformId::Admob,
            "u1",
            ChangeKind::EcpmSet {
                mediation_group_id: "g".into(),
                ad_unit_id: "u1".into(),
                ecpm: 1,
            },
            "key-1",
            "prev",
        ),
    )
    .expect("append");
    let rollback = LedgerLog::append_rollback(&mut file, 2, PlatformId::Admob, "u1", 0, "why")
        .expect("rollback");
    assert_eq!(rollback.seq(), 1);
    assert_eq!(LedgerLog::len(&file), 2);
    assert!(LedgerLog::get(&file, 0).is_some());
    assert!(!LedgerLog::is_empty(&file));
    assert_eq!(LedgerLog::records(&file).len(), 2);
}

#[test]
fn record_constructors_and_accessors_are_exercised() {
    let lift = LedgerRecord::lift_record(123, "sha", "{\"record\":{}}");
    assert_eq!(lift.at_unix_ms(), 123);
    assert_eq!(lift.platform(), None, "a lift record has no platform");
    assert_eq!(lift.seq(), 0);
    assert!(matches!(lift, LedgerRecord::LiftRecord { .. }));

    let started = LedgerRecord::experiment_started(
        9,
        PlatformId::AppLovinMax,
        "exp",
        apex_core::ExperimentSpec {
            name: "n".into(),
            ad_unit_id: "u".into(),
            metric: apex_core::ExperimentMetric::Revenue,
            variants: 2,
            change_description: "c".into(),
            test_group_allocation: 25,
        },
    );
    assert_eq!(started.platform(), Some(PlatformId::AppLovinMax));
    assert_eq!(started.at_unix_ms(), 9);
}

#[test]
fn billing_proof_builds_errors_and_verifies_memory() {
    let proof = crate::build_billing_proof(
        "pub",
        "exp",
        50_000 * 1_000_000,
        55_000 * 1_000_000,
        2_000,
        7,
    )
    .expect("proof");
    assert!(matches!(proof, LedgerRecord::BillingProof { .. }));
    assert!(crate::build_billing_proof("pub", "exp", 0, 1, 2_000, 7).is_err());

    let mut memory = InMemoryLedger::new(keypair(3));
    memory.append(proof).expect("append");
    memory.verify().expect("verify");
}

#[test]
fn the_file_ledger_opens_from_owned_and_borrowed_paths() {
    let dir = TempDir::new("proof-open-paths");
    let owned = dir.file("owned.jsonl");
    let borrowed = dir.file("borrowed.jsonl");
    let mut a = Ledger::open(owned.clone(), keypair(4)).expect("owned");
    a.append(LedgerRecord::lift_record(1, "s", "{}"))
        .expect("append");
    assert_eq!(a.len(), 1);
    // A second open sees the same on-disk history (refresh path).
    let b = Ledger::open(&borrowed, keypair(4)).expect("borrowed");
    assert_eq!(b.len(), 0);
    let c = Ledger::open(&owned, keypair(4)).expect("reopen");
    assert_eq!(c.len(), 1);
}

// ---------------------------------------------------------------------------
// Parallel signature verification: sequential equivalence and thread seam
// ---------------------------------------------------------------------------

/// A valid chain of `count` sealed and signed lines.
fn chained_lines(count: usize, keys: &crate::Keypair) -> Vec<String> {
    let mut lines = Vec::with_capacity(count);
    let mut prev_hash = [0u8; 32];
    for index in 0..count {
        let seq = u64::try_from(index).expect("test index fits u64");
        let record = seal_record(change(seq), seq, prev_hash);
        prev_hash = record.hash();
        lines.push(signed_line(&record, keys));
    }
    lines
}

fn chain_bytes(lines: &[String]) -> Vec<u8> {
    let mut text = lines.join("\n");
    text.push('\n');
    text.into_bytes()
}

#[derive(Debug, serde::Deserialize)]
struct ReferenceLine {
    record: LedgerRecord,
    signature: String,
    pubkey: String,
}

/// The pre-parallel verification algorithm, kept as the test oracle: parse,
/// public key, signature, canonical bytes and chain rules, strictly in line
/// order, exactly as the sequential implementation did.
fn sequential_reference(
    bytes: &[u8],
    expected_pubkey: &VerifyingKey,
) -> Result<Vec<LedgerRecord>, LedgerError> {
    let mut records = Vec::new();
    let mut validator = crate::chain::ChainValidator::new();
    if !bytes.is_empty() {
        if bytes.last() != Some(&b'\n') {
            let complete_lines = bytes.iter().filter(|byte| **byte == b'\n').count();
            return Err(LedgerError::Truncated {
                line: complete_lines + 1,
            });
        }
        for (index, chunk) in bytes[..bytes.len() - 1]
            .split(|byte| *byte == b'\n')
            .enumerate()
        {
            let line = index + 1;
            let stored: ReferenceLine =
                serde_json::from_slice(chunk).map_err(|error| LedgerError::Malformed {
                    line,
                    detail: error.to_string(),
                })?;
            let pubkey: [u8; 32] = hex::decode(&stored.pubkey)
                .ok()
                .and_then(|bytes| bytes.try_into().ok())
                .ok_or(LedgerError::WrongPubkey { line })?;
            if pubkey != expected_pubkey.to_bytes() {
                return Err(LedgerError::WrongPubkey { line });
            }
            let signature_bytes: [u8; 64] = hex::decode(&stored.signature)
                .ok()
                .and_then(|bytes| bytes.try_into().ok())
                .ok_or(LedgerError::BadSignature { line })?;
            let signature = Signature::from_bytes(&signature_bytes);
            expected_pubkey
                .verify_strict(&stored.record.hash(), &signature)
                .map_err(|_| LedgerError::BadSignature { line })?;
            let canonical =
                crate::ledger::canonical_line(&stored.record, &signature, expected_pubkey)?;
            if canonical[..canonical.len() - 1] != *chunk {
                return Err(LedgerError::Malformed {
                    line,
                    detail: "line bytes are not the canonical serialization of the signed record"
                        .into(),
                });
            }
            validator.check(&stored.record, line)?;
            records.push(stored.record);
        }
    }
    Ok(records)
}

fn assert_same_ledger_error(actual: &LedgerError, expected: &LedgerError) {
    assert_eq!(
        std::mem::discriminant(actual),
        std::mem::discriminant(expected),
        "error variant differs: {actual:?} vs {expected:?}"
    );
    assert_eq!(
        actual.to_string(),
        expected.to_string(),
        "error message differs"
    );
}

#[derive(Debug, Clone, Copy)]
enum Expected {
    Valid,
    BadSignature(usize),
    HashMismatch(usize),
    Malformed(usize),
}

fn assert_expected(
    result: &Result<Vec<LedgerRecord>, LedgerError>,
    expected: Expected,
    case: &str,
) {
    match (result, expected) {
        (Ok(_), Expected::Valid) => {}
        (Err(error), Expected::BadSignature(line)) => {
            assert!(
                matches!(error, LedgerError::BadSignature { line: actual } if *actual == line),
                "{case}: expected BadSignature at line {line}, got {error:?}"
            );
        }
        (Err(error), Expected::HashMismatch(line)) => {
            assert!(
                matches!(error, LedgerError::HashMismatch { line: actual } if *actual == line),
                "{case}: expected HashMismatch at line {line}, got {error:?}"
            );
        }
        (Err(error), Expected::Malformed(line)) => {
            assert!(
                matches!(error, LedgerError::Malformed { line: actual, .. } if *actual == line),
                "{case}: expected Malformed at line {line}, got {error:?}"
            );
        }
        (other, expected) => {
            panic!("{case}: expected {expected:?}, got {other:?}");
        }
    }
}

/// Equivalence matrix: every tampered ledger must produce the exact same
/// typed error (variant and message) through the sequential reference, the
/// forced-single-worker path and the parallel path.
#[test]
fn parallel_verification_matches_the_sequential_reference() {
    let keys = keypair(70);
    let expected_key = keys.verifying_key();
    let pristine = chained_lines(12, &keys);
    let last_seq = pristine.len() - 1;
    let middle_seq = pristine.len() / 2;

    let mut cases: Vec<(String, Vec<String>, Expected)> = Vec::new();
    cases.push(("pristine".into(), pristine.clone(), Expected::Valid));

    for seq in [1, 2, middle_seq, last_seq] {
        let mut lines = pristine.clone();
        lines[seq] = flip_hex_field(&lines[seq], "signature");
        cases.push((
            format!("bad signature at seq {seq}"),
            lines,
            Expected::BadSignature(seq + 1),
        ));
    }

    // Two bad signatures: the smallest sequence must be reported, regardless
    // of which worker finishes first.
    let mut two_bad = pristine.clone();
    two_bad[9] = flip_hex_field(&two_bad[9], "signature");
    two_bad[5] = flip_hex_field(&two_bad[5], "signature");
    cases.push((
        "bad signatures at seq 5 and seq 9".into(),
        two_bad,
        Expected::BadSignature(6),
    ));

    // The same two failures listed in the opposite tampering order.
    let mut two_bad_reversed = pristine.clone();
    two_bad_reversed[5] = flip_hex_field(&two_bad_reversed[5], "signature");
    two_bad_reversed[9] = flip_hex_field(&two_bad_reversed[9], "signature");
    cases.push((
        "bad signatures at seq 5 and seq 9 (reverse order)".into(),
        two_bad_reversed,
        Expected::BadSignature(6),
    ));

    // Structural tamper at line 3 unchanged even when a later signature is bad.
    let mut structural = pristine.clone();
    structural[2] = structural[2].replacen("admob-ecpm:g1:u1:2", "admob-ecpm:g1:u1:9", 1);
    structural[8] = flip_hex_field(&structural[8], "signature");
    cases.push((
        "payload tamper at seq 2 plus bad signature at seq 8".into(),
        structural,
        Expected::HashMismatch(3),
    ));

    // An earlier bad signature wins over a later structural tamper.
    let mut earlier_signature = pristine.clone();
    earlier_signature[1] = flip_hex_field(&earlier_signature[1], "signature");
    earlier_signature[7] =
        earlier_signature[7].replacen("admob-ecpm:g1:u1:7", "admob-ecpm:g1:u1:9", 1);
    cases.push((
        "bad signature at seq 1 before payload tamper at seq 7".into(),
        earlier_signature,
        Expected::BadSignature(2),
    ));

    // Canonical-encoding tamper (injected annotation) is still Malformed.
    let mut injected = pristine.clone();
    let mut value: serde_json::Value =
        serde_json::from_str(&injected[3]).expect("parse persisted line");
    value["record"]["annotation"] = serde_json::json!("injected");
    injected[3] = serde_json::to_string(&value).expect("serialize injected line");
    cases.push((
        "injected field at seq 3".into(),
        injected,
        Expected::Malformed(4),
    ));

    for (case, lines, expected) in cases {
        let bytes = chain_bytes(&lines);
        let reference = sequential_reference(&bytes, &expected_key);
        let single =
            crate::ledger::read_verified_bytes_with_threads(&bytes, &expected_key, 1, None);
        let parallel =
            crate::ledger::read_verified_bytes_with_threads(&bytes, &expected_key, 4, None);

        assert_expected(&reference, expected, &format!("{case}: reference"));
        assert_expected(&single, expected, &format!("{case}: one worker"));
        assert_expected(&parallel, expected, &format!("{case}: four workers"));

        match (reference, parallel) {
            (Ok(reference), Ok(parallel)) => assert_eq!(reference, parallel, "{case}"),
            (Err(reference), Err(parallel)) => {
                assert_same_ledger_error(&parallel, &reference);
            }
            (reference, parallel) => {
                panic!("{case}: reference {reference:?}, parallel {parallel:?}");
            }
        }
    }
}

/// The concurrency seam: a large ledger uses more than one verification
/// worker, the pool stays bounded by the requested count, and forcing a single
/// thread runs exactly one worker (so the >1 assertion above is load-bearing).
#[test]
fn large_ledgers_verify_signatures_on_multiple_workers() {
    let keys = keypair(71);
    let expected_key = keys.verifying_key();
    let lines = chained_lines(64, &keys);
    let bytes = chain_bytes(&lines);

    let parallel_probe = AtomicU64::new(0);
    let records = crate::ledger::read_verified_bytes_with_threads(
        &bytes,
        &expected_key,
        4,
        Some(&parallel_probe),
    )
    .expect("parallel verify");
    assert_eq!(records.len(), 64);
    let workers = parallel_probe.load(Ordering::Relaxed);
    assert!(
        workers > 1,
        "a 64-record ledger must use more than one signature-verification worker, used {workers}"
    );
    assert!(
        workers <= 4,
        "the worker pool must stay bounded by the requested threads, used {workers}"
    );

    let single_probe = AtomicU64::new(0);
    crate::ledger::read_verified_bytes_with_threads(&bytes, &expected_key, 1, Some(&single_probe))
        .expect("single-worker verify");
    assert_eq!(
        single_probe.load(Ordering::Relaxed),
        1,
        "forcing one thread must run exactly one verification worker"
    );
}

#[test]
fn verification_uses_the_calling_thread_as_one_worker() {
    let keys = keypair(72);
    let expected_key = keys.verifying_key();
    let lines = chained_lines(64, &keys);
    let bytes = chain_bytes(&lines);

    // Four chunks: the caller verifies the first, so three threads are
    // spawned instead of four.
    crate::ledger::reset_worker_spawns();
    let probe = AtomicU64::new(0);
    crate::ledger::read_verified_bytes_with_threads(&bytes, &expected_key, 4, Some(&probe))
        .expect("parallel verify");
    assert_eq!(
        probe.load(Ordering::Relaxed),
        4,
        "every chunk, including the caller's, is one worker"
    );
    assert_eq!(
        crate::ledger::worker_spawns(),
        3,
        "the calling thread participates, so four workers spawn three threads"
    );

    // Below the parallel threshold the caller is the only worker and no
    // thread is created at all.
    let small = chain_bytes(&chained_lines(7, &keys));
    crate::ledger::reset_worker_spawns();
    let small_probe = AtomicU64::new(0);
    crate::ledger::read_verified_bytes_with_threads(&small, &expected_key, 8, Some(&small_probe))
        .expect("sequential verify");
    assert_eq!(small_probe.load(Ordering::Relaxed), 1);
    assert_eq!(crate::ledger::worker_spawns(), 0);
}

#[test]
fn verify_thread_override_is_parsed_and_bounded() {
    assert_eq!(crate::ledger::parse_verify_threads("1"), Some(1));
    assert_eq!(crate::ledger::parse_verify_threads(" 4 "), Some(4));
    assert_eq!(
        crate::ledger::parse_verify_threads("0"),
        Some(1),
        "zero workers is clamped to one"
    );
    assert_eq!(
        crate::ledger::parse_verify_threads("1000"),
        Some(crate::ledger::MAX_VERIFY_THREADS),
        "the override cannot exceed the sane cap"
    );
    assert_eq!(crate::ledger::parse_verify_threads("nope"), None);
    assert_eq!(crate::ledger::parse_verify_threads(""), None);
}
