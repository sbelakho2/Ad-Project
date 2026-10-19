//! LIVE proof verification against the real file-backed ledger.
//!
//! Every case here uses a real ed25519 keypair generated from `/dev/urandom`
//! and the real `apex-proof` append/verify API on real files. Nothing is
//! mocked: the tamper cases rewrite the on-disk JSONL bytes exactly as an
//! attacker with file access could.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use apex_core::{ExperimentMetric, ExperimentResult, ExperimentSpec, ExperimentState, PlatformId};
use apex_proof::{
    build_billing_proof, ChangeKind, Keypair, Ledger, LedgerError, LedgerRecord, VerifyingKey,
};
use sha2::Digest;

fn live_dir(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!(
        "apex-proof-live-{}-{nanos}-{label}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("live dir");
    dir
}

/// A real keypair: 32 random bytes from the OS, consumed by the crate's real
/// ed25519 signing key type.
fn real_keypair() -> Keypair {
    let mut seed = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .expect("urandom")
        .read_exact(&mut seed)
        .expect("random seed");
    // Guard against an all-zero seed (astronomically unlikely) so the test
    // always exercises a real key rather than the degenerate one.
    if seed == [0u8; 32] {
        seed[0] = 1;
    }
    Keypair::from_bytes(&seed)
}

fn live_spec(name: &str, unit: &str) -> ExperimentSpec {
    ExperimentSpec {
        name: name.to_string(),
        ad_unit_id: unit.to_string(),
        metric: ExperimentMetric::Revenue,
        variants: 2,
        change_description: "live multi-record ledger".to_string(),
        test_group_allocation: 50,
    }
}

fn observed(id: &str) -> ExperimentResult {
    ExperimentResult {
        experiment_id: id.to_string(),
        state: ExperimentState::Completed,
        results: Vec::new(),
        lift_percent_bp: Some(250),
        confidence: Some(9_800),
        winner_variant_id: Some("test".to_string()),
    }
}

/// Append a deterministic multi-record ledger: change, experiment start,
/// observation, a signed lift payload, a rollback, a billing proof and more
/// changes. Returns the sealed records in order.
fn build_ledger(path: &Path, key: &Keypair, records: usize) -> Ledger {
    let mut ledger = Ledger::open(path, key.clone()).expect("open ledger");
    let mut appended = Vec::new();
    let mut index = 0usize;
    while appended.len() < records {
        match index % 6 {
            0 => {
                appended.push(
                    ledger
                        .append(LedgerRecord::change_applied(
                            1_700_000_000_000 + index as i64,
                            PlatformId::AppLovinMax,
                            &format!("unit-{index}"),
                            ChangeKind::CountryFloorSet {
                                country: "EE".to_string(),
                                floor_ecpm: 250 + index as i64,
                            },
                            &format!("live-key-{index}"),
                            "{\"floor\":200}",
                        ))
                        .expect("append change"),
                );
            }
            1 => {
                appended.push(
                    ledger
                        .append(LedgerRecord::experiment_started(
                            1_700_000_000_001 + index as i64,
                            PlatformId::AppLovinMax,
                            &format!("exp-{index}"),
                            live_spec(&format!("exp-{index}"), &format!("unit-{index}")),
                        ))
                        .expect("append start"),
                );
            }
            2 => {
                appended.push(
                    ledger
                        .append(LedgerRecord::experiment_observed(
                            1_700_000_000_002 + index as i64,
                            PlatformId::AppLovinMax,
                            &format!("exp-{}", index.saturating_sub(1)),
                            observed(&format!("exp-{}", index.saturating_sub(1))),
                        ))
                        .expect("append observation"),
                );
            }
            3 => {
                let payload = format!("{{\"live\":\"lift-{index}\"}}");
                let digest = hex::encode(sha2::Sha256::digest(payload.as_bytes()));
                appended.push(
                    ledger
                        .append(LedgerRecord::lift_record(
                            1_700_000_000_003 + index as i64,
                            &digest,
                            &payload,
                        ))
                        .expect("append lift"),
                );
            }
            4 => {
                // Roll back the most recent change on the same unit so the
                // rollback rules are exercised with a real target.
                let target = appended
                    .iter()
                    .rev()
                    .find(|record| matches!(record, LedgerRecord::ChangeApplied { .. }))
                    .map(|record| record.seq())
                    .unwrap_or(0);
                let unit = format!("unit-{}", target as usize);
                appended.push(
                    ledger
                        .append_rollback(
                            1_700_000_000_004 + index as i64,
                            PlatformId::AppLovinMax,
                            &unit,
                            target,
                            "live rollback",
                        )
                        .expect("append rollback"),
                );
            }
            _ => {
                let proof = build_billing_proof(
                    "live-publisher",
                    &format!("exp-billing-{index}"),
                    10_000_000,
                    12_500_000,
                    2_000,
                    1_700_000_000_005 + index as i64,
                )
                .expect("billing proof");
                appended.push(ledger.append(proof).expect("append billing proof"));
            }
        }
        index += 1;
    }
    assert_eq!(appended.len(), records);
    ledger
}

fn read_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .expect("read ledger")
        .lines()
        .map(str::to_string)
        .collect()
}

fn write_lines(path: &Path, lines: &[String]) {
    let mut body = lines.join("\n");
    body.push('\n');
    std::fs::write(path, body).expect("write ledger");
}

fn fail_line(error: &LedgerError) -> usize {
    match error {
        LedgerError::HashMismatch { line }
        | LedgerError::ChainBreak { line }
        | LedgerError::BadSignature { line }
        | LedgerError::WrongPubkey { line }
        | LedgerError::SequenceGap { line, .. }
        | LedgerError::Duplicate { line, .. }
        | LedgerError::Genesis { line }
        | LedgerError::Truncated { line }
        | LedgerError::Malformed { line, .. }
        | LedgerError::InvalidBillingProof { line, .. } => *line,
        other => panic!("error names no line: {other}"),
    }
}

#[test]
fn multi_record_ledger_lineage_latest_counts_and_reload_are_real() {
    let dir = live_dir("lineage");
    let key = real_keypair();
    let path = dir.join("ledger.jsonl");
    let ledger = build_ledger(&path, &key, 30);
    assert_eq!(ledger.len(), 30);

    let records = ledger.records();
    assert_eq!(records.len(), 30);
    assert_eq!(records.first().map(|record| record.seq()), Some(0));
    assert_eq!(records.last().map(|record| record.seq()), Some(29));
    assert_eq!(ledger.get(0).map(|record| record.seq()), Some(0));
    assert_eq!(ledger.get(29).map(|record| record.seq()), Some(29));
    assert!(ledger.get(30).is_none());

    // Lineage: the genesis link is zero and every later link names the
    // previous record's sealed hash.
    assert_eq!(records[0].prev_hash(), [0u8; 32]);
    for pair in records.windows(2) {
        assert_eq!(
            pair[1].prev_hash(),
            pair[0].hash(),
            "lineage break at seq {}",
            pair[1].seq()
        );
        assert_eq!(pair[1].seq(), pair[0].seq() + 1);
    }

    // The public verifier reads the same records from the same verifying read.
    let verified_records = Ledger::read_verified(&path, &key.verifying_key()).expect("verify");
    assert_eq!(verified_records.len(), 30);
    assert_eq!(verified_records.last().map(|record| record.seq()), Some(29));
    assert_eq!(
        verified_records.last().map(|record| record.hash()),
        Some(records[29].hash())
    );
    Ledger::verify(&path, &key.verifying_key()).expect("verify only");

    // Restart/reload: a fresh handle verifies the whole file and continues
    // the chain at the next sequence.
    drop(ledger);
    let mut reopened = Ledger::open(&path, key.clone()).expect("reopen");
    assert_eq!(reopened.len(), 30);
    let next = reopened
        .append(LedgerRecord::change_applied(
            1_700_000_001_000,
            PlatformId::AppLovinMax,
            "unit-30",
            ChangeKind::CountryFloorSet {
                country: "EE".to_string(),
                floor_ecpm: 999,
            },
            "live-key-30",
            "{}",
        ))
        .expect("append after reload");
    assert_eq!(next.seq(), 30);
    assert_eq!(next.prev_hash(), verified_records[29].hash());

    // File mode is 0600 for a freshly created ledger.
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "ledger mode {mode:o}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tampering_is_detected_at_first_middle_and_last_record_and_in_the_chain() {
    let dir = live_dir("tamper");
    let key = real_keypair();
    let path = dir.join("ledger.jsonl");
    build_ledger(&path, &key, 30);
    let original = read_lines(&path);
    assert_eq!(original.len(), 30);
    let verify = |path: &Path| Ledger::verify(path, &key.verifying_key());

    // First record payload.
    let mut lines = original.clone();
    lines[0] = lines[0].replace("\"live-key-0\"", "\"live-key-X\"");
    write_lines(&path, &lines);
    let error = verify(&path).expect_err("first tamper");
    assert!(
        matches!(error, LedgerError::HashMismatch { line: 1 }),
        "{error:?}"
    );
    assert_eq!(fail_line(&error), 1);

    // Middle record payload.
    let mut lines = original.clone();
    lines[14] = lines[14].replace(
        "\"experimentId\":\"exp-13\"",
        "\"experimentId\":\"exp-X13\"",
    );
    write_lines(&path, &lines);
    let error = verify(&path).expect_err("middle tamper");
    assert!(
        matches!(error, LedgerError::HashMismatch { line: 15 }),
        "{error:?}"
    );
    assert_eq!(fail_line(&error), 15);

    // Last record payload.
    let mut lines = original.clone();
    lines[29] = lines[29].replace("live-publisher", "LIVE-publisher");
    write_lines(&path, &lines);
    let error = verify(&path).expect_err("last tamper");
    assert_eq!(fail_line(&error), 30, "{error:?}");

    // Chain tamper: reorder two lines. The parser sees a sequence gap / chain
    // break at the first reordered line, never a silent pass.
    let mut lines = original.clone();
    lines.swap(9, 10);
    write_lines(&path, &lines);
    let error = verify(&path).expect_err("reordered chain");
    let line = fail_line(&error);
    assert!(
        line == 10 || line == 11,
        "chain tamper reported line {line}: {error:?}"
    );
    assert!(
        matches!(
            error,
            LedgerError::SequenceGap { .. }
                | LedgerError::ChainBreak { .. }
                | LedgerError::HashMismatch { .. }
        ),
        "{error:?}"
    );

    // Deleting a middle line is a chain failure at the deletion point.
    let mut lines = original.clone();
    lines.remove(19);
    write_lines(&path, &lines);
    let error = verify(&path).expect_err("deleted line");
    assert_eq!(fail_line(&error), 20, "{error:?}");

    // Truncating the final newline is a typed truncation, not a pass.
    let body = std::fs::read_to_string(&path).expect("read");
    std::fs::write(&path, body.trim_end_matches('\n')).expect("truncate");
    let error = verify(&path).expect_err("truncated final line");
    assert!(matches!(error, LedgerError::Truncated { .. }), "{error:?}");

    // A foreign key refuses a valid ledger it did not sign.
    write_lines(&path, &original);
    let foreign = real_keypair();
    let error = verify_with(&path, &foreign.verifying_key()).expect_err("foreign key");
    assert!(
        matches!(error, LedgerError::WrongPubkey { line: 1 }),
        "{error:?}"
    );

    // Restoring the exact bytes restores verification: every check above was
    // about the bytes, not the handle.
    verify(&path).expect("restored ledger verifies");
    let _ = std::fs::remove_dir_all(&dir);
}

fn verify_with(path: &Path, key: &VerifyingKey) -> Result<(), LedgerError> {
    Ledger::verify(path, key)
}

#[test]
fn concurrent_verification_and_large_ledger_timing_are_real() {
    let dir = live_dir("concurrent");
    let key = real_keypair();
    let path = dir.join("ledger.jsonl");
    let started = Instant::now();
    let ledger = build_ledger(&path, &key, 500);
    let append_elapsed = started.elapsed();
    assert_eq!(ledger.len(), 500);
    drop(ledger);

    // Concurrent verification: every worker reads and verifies the same real
    // file against the real public key.
    let started = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..8 {
        let path = path.clone();
        let key = key.verifying_key();
        handles.push(std::thread::spawn(move || Ledger::verify(&path, &key)));
    }
    for handle in handles {
        handle.join().expect("verify thread").expect("verified");
    }
    let concurrent_elapsed = started.elapsed();

    let started = Instant::now();
    Ledger::verify(&path, &key.verifying_key()).expect("single verify");
    let single_elapsed = started.elapsed();

    println!(
        "LIVE proof timing: append 500 records {append_elapsed:?}; 8 concurrent verifies {concurrent_elapsed:?}; single verify {single_elapsed:?}"
    );

    // A large ledger verifies in a bounded, real time on this host. The bound
    // is intentionally generous (the point is no pathological blow-up).
    assert!(
        single_elapsed < std::time::Duration::from_secs(30),
        "large verify took {single_elapsed:?}"
    );

    // A tampered large ledger is refused by the same real verifier.
    let raw = std::fs::read_to_string(&path).expect("read");
    let mut lines: Vec<String> = raw.lines().map(str::to_string).collect();
    lines[250] = lines[250].replace("live rollback", "LIVE rollback");
    write_lines(&path, &lines);
    let error = Ledger::verify(&path, &key.verifying_key()).expect_err("large tamper");
    assert_eq!(fail_line(&error), 251, "{error:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
