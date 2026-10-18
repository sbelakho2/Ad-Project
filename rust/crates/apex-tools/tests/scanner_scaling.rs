//! Scanner scaling evidence at the requested 13k / 26k file sizes.
//!
//! The ignored test builds two throwaway repositories (13,000 and 26,000
//! data files plus the policy file) and drives the real gates over them:
//! `Repo::walk`, `build_manifest` (classifier + walk), `orphan_gate` (every
//! code file against every suite glob), `assertion_presence` (walk + read +
//! per-file regex), and the docs-lint prose parse/check (the per-file work;
//! its walk is the same `walk_files` measured by `Repo::walk`).
//!
//! Correctness is asserted at both sizes (every file is classified, the
//! manifest count is exact, the orphan and assertion gates pass, and every
//! synthetic document is parsed). The millisecond table is printed evidence,
//! never asserted, so a loaded machine cannot flake the suite.
//!
//! Run explicitly:
//!
//! ```text
//! cd rust && cargo test -p apex-tools --test scanner_scaling -- --ignored --nocapture
//! ```

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

// dogfood:skip: the 13k/26k evidence benchmark is deliberately ignored in the
// default suite and is run explicitly with `--ignored --nocapture`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use apex_tools::classify::build_manifest;
use apex_tools::docs_lint::{md_prose, run_checks};
use apex_tools::dogfood::{assertion_presence, orphan_gate};
use apex_tools::repo::Repo;

const CRATES: [&str; 18] = [
    "apex-analytics",
    "apex-api",
    "apex-audit",
    "apex-autopilot",
    "apex-billing",
    "apex-connectors",
    "apex-control",
    "apex-core",
    "apex-lift",
    "apex-metrics",
    "apex-notify",
    "apex-proof",
    "apex-sales",
    "apex-sales-web",
    "apex-tools",
    "apex-ui",
    "apex-vault",
    "apex-watch",
];

fn repo_root() -> PathBuf {
    // rust/crates/apex-tools -> rust/crates -> rust -> repository root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repository root")
        .to_path_buf()
}

fn temp_root(tag: &str, n: usize) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "apex-tools-scaling-{tag}-{n}-{}-{stamp}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    root
}

fn write_tree(root: &Path, n: usize) {
    fs::create_dir_all(root.join("tools/coverage")).expect("policy dir");
    fs::copy(
        repo_root().join("tools/coverage/policies.json"),
        root.join("tools/coverage/policies.json"),
    )
    .expect("copy policies");
    for krate in CRATES {
        fs::create_dir_all(root.join("rust/crates").join(krate).join("src")).expect("crate dir");
    }
    fs::create_dir_all(root.join("docs")).expect("docs dir");
    fs::create_dir_all(root.join("site/public")).expect("site dir");
    for index in 0..n {
        let (path, body) = match index % 10 {
            // Code: a test module with one real assertion, so the assertion
            // gate has to read and match it.
            0..=5 => {
                let krate = CRATES[index % CRATES.len()];
                (
                    root.join("rust/crates")
                        .join(krate)
                        .join("src")
                        .join(format!("file_{index:06}.rs")),
                    "#[test]\nfn synthetic_case() {\n    assert!(true);\n}\n".to_string(),
                )
            }
            // Markdown: one short paragraph per file.
            6..=7 => (
                root.join("docs").join(format!("page_{index:06}.md")),
                "The scanner reads this short paragraph.\n\nAnother sentence here.\n".to_string(),
            ),
            // A generated site artifact (excluded, but walked).
            8 => (
                root.join("site/public")
                    .join(format!("asset_{index:06}.bin")),
                String::new(),
            ),
            // The gate stack itself (contract-build domain).
            _ => (
                root.join("tools/coverage")
                    .join(format!("gate_{index:06}.rs")),
                "pub fn synthetic_gate() {}\n".to_string(),
            ),
        };
        fs::write(path, body).expect("write synthetic file");
    }
}

fn suites() -> Vec<Vec<String>> {
    CRATES
        .iter()
        .map(|krate| vec![format!("rust/crates/{krate}/**/*.rs")])
        .collect()
}

#[test]
#[ignore = "13k/26k scanner scaling evidence; run with --ignored --nocapture"]
fn scanners_stay_linear_at_13k_and_26k_files() {
    let mut rows = Vec::new();
    for n in [13_000usize, 26_000] {
        let root = temp_root("scan", n);
        write_tree(&root, n);
        let repo = Repo { root: root.clone() };
        let suites = suites();

        let start = Instant::now();
        let walked = repo.walk();
        let walk_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(
            walked.len(),
            n + 1,
            "the walker must see every data file plus policies.json"
        );

        let start = Instant::now();
        let manifest = build_manifest(&repo).expect("manifest");
        let manifest_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(manifest.files.len(), n + 1);
        assert!(
            manifest
                .files
                .iter()
                .all(|file| file.status != "unclassified"),
            "every synthetic path must classify"
        );

        let start = Instant::now();
        orphan_gate(&manifest, &suites, 0.0).expect("orphan gate");
        let orphan_ms = start.elapsed().as_secs_f64() * 1000.0;

        let start = Instant::now();
        assertion_presence(&repo).expect("assertion gate");
        let assertion_ms = start.elapsed().as_secs_f64() * 1000.0;

        // The docs-lint per-file work: parse prose and run every check over
        // n synthetic documents (the walk is the same walk measured above).
        let document = "The scanner reads this short paragraph.\n\nAnother sentence here.\n";
        let start = Instant::now();
        for _ in 0..n {
            let lines = md_prose(document).expect("parse");
            run_checks(&lines, "page.md").expect("check");
        }
        let docs_ms = start.elapsed().as_secs_f64() * 1000.0;

        rows.push((n, walk_ms, manifest_ms, orphan_ms, assertion_ms, docs_ms));
        eprintln!(
            "SCAN n={n:>6} walk={walk_ms:8.1}ms manifest={manifest_ms:8.1}ms \
             orphan={orphan_ms:8.1}ms assertion={assertion_ms:8.1}ms docs_parse={docs_ms:8.1}ms"
        );
        let _ = fs::remove_dir_all(&root);
    }
    // n/2n shape: the second size is exactly twice the first, and every
    // correctness assertion above ran at both sizes.
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].0, 2 * rows[0].0);
    for (n, walk, manifest, orphan, assertion, docs) in rows {
        assert!(
            walk > 0.0 && manifest > 0.0 && orphan > 0.0 && assertion > 0.0 && docs > 0.0,
            "timing row for {n} must be measured"
        );
    }
}
