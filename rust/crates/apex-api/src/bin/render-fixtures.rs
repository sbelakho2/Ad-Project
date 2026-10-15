//! render-fixtures — renders the console pages to an output directory from
//! checked-in sample fixtures, so the UI/UX accessibility lane scans the
//! rendered HTML (never a hand-written approximation).
//!
//! Usage: cargo run -p apex-api --bin render-fixtures -- <fixtures-dir> <out-dir>
//! Fixture files: admob.json (a real audit Snapshot named by platform) and
//! pub-1.jsonl (a real signed ledger written with the published test key).
use std::collections::HashMap;
use std::path::PathBuf;

use apex_api::html;
use apex_api::sources::{AuditSource, FileLedgerSource, JsonFileAuditSource, LedgerSource};
use apex_core::PlatformId;
use apex_proof::{ChangeKind, Keypair, Ledger, LedgerRecord, VerifyingKey};

const TEST_KEY_SEED: [u8; 32] = [7u8; 32];

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let fixtures = PathBuf::from(
        args.next()
            .ok_or("usage: render-fixtures <fixtures-dir> <out-dir>")?,
    );
    let out = PathBuf::from(
        args.next()
            .ok_or("usage: render-fixtures <fixtures-dir> <out-dir>")?,
    );
    run(&fixtures, &out).await
}

/// Render the fixtures into `out`, generating the ledger fixture when absent.
async fn run(
    fixtures: &std::path::Path,
    out: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(out)?;

    // The ledger fixture is generated once with the published test key so the
    // file verifies offline; regeneration happens only when the fixture format
    // changes.
    let ledger_path = fixtures.join("pub-1.jsonl");
    if !ledger_path.exists() {
        let mut ledger = Ledger::open(&ledger_path, Keypair::from_bytes(&TEST_KEY_SEED))?;
        ledger.append(LedgerRecord::change_applied(
            1_700_000_000_000,
            PlatformId::Admob,
            "u1",
            ChangeKind::EcpmSet {
                mediation_group_id: "g1".into(),
                ad_unit_id: "u1".into(),
                ecpm: 1_750_000,
            },
            "admob-ecpm:g1:u1:1.75",
            "1000000",
        ))?;
    }

    let keypair = Keypair::from_bytes(&TEST_KEY_SEED);
    let mut keys: HashMap<String, VerifyingKey> = HashMap::new();
    keys.insert("pub-1".to_string(), keypair.verifying_key());
    let ledger_source = FileLedgerSource::new(fixtures.to_path_buf(), keys);
    let audit_source = JsonFileAuditSource::new(fixtures.to_path_buf());

    let snapshot = audit_source
        .snapshot(PlatformId::Admob)
        .await
        .map_err(|error| std::io::Error::other(format!("admob snapshot read failed: {error}")))?;
    let report = apex_audit::run(&snapshot);
    let records = ledger_source
        .records("pub-1")
        .await
        .map_err(|error| std::io::Error::other(format!("pub-1 ledger read failed: {error}")))?;

    std::fs::write(out.join("overview.html"), html::render_overview())?;
    std::fs::write(
        out.join("audit.html"),
        html::render_audit_page(PlatformId::Admob, &report),
    )?;
    std::fs::write(
        out.join("ledger.html"),
        html::render_ledger_records("pub-1", &records),
    )?;
    std::fs::write(
        out.join("error.html"),
        html::render_error_page("Not found", "no such page"),
    )?;

    println!("rendered 4 console pages to {}", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::path::PathBuf;

    #[tokio::test]
    async fn renders_every_console_page_from_the_committed_fixtures() {
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        let out = std::env::temp_dir().join(format!("apex-fixtures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        super::run(&fixtures, &out).await.expect("render");
        for page in ["overview.html", "audit.html", "ledger.html", "error.html"] {
            assert!(out.join(page).exists(), "{page} was not rendered");
        }
        // Rendering generates the ledger fixture when it is absent.
        let generated = fixtures.join("pub-1.jsonl");
        assert!(generated.exists());
        let _ = std::fs::remove_dir_all(&out);
    }

    #[tokio::test]
    async fn the_ledger_fixture_is_generated_when_missing() {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        let dir = std::env::temp_dir().join(format!("apex-gen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(source.join("admob.json"), dir.join("admob.json")).unwrap();
        assert!(!dir.join("pub-1.jsonl").exists());
        let out = dir.join("out");
        super::run(&dir, &out).await.expect("render");
        assert!(dir.join("pub-1.jsonl").exists(), "the ledger is generated");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
