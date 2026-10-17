//! Corpus parity test for the `app_ads_txt` audit rule.
//!
//! The shared corpus at `tests/fixtures/adstxt/` is the single source of truth
//! for the app-ads.txt line classification: each `<stem>.txt` file has a
//! `<stem>.expected.json` sidecar with the lines the shared classification
//! reports as malformed (with their kind), the accepted record and variable
//! counts, and the sorted domains of accepted records. `apex-sales` asserts its
//! parser's error set against the same sidecars (its test reads this directory
//! through `CARGO_MANIFEST_DIR/../apex-audit`), so "the audit and the sales
//! engine report the same errors for the same file" is enforced at build time
//! in both crates.
//!
//! The audit rule reports line numbers only (its public finding shape is
//! unchanged), so this test asserts line-level equality with the sidecar's
//! `errors[].line` values; kind-level equality is enforced by the sales test.
//! It additionally proves the reported-networks fix: every domain of an
//! accepted record — including records that only appear on four-field lines —
//! satisfies presence, so when the expected networks are exactly those domains
//! plus one sentinel, only the sentinel is missing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use apex_audit::model::{AppAdsTxt, RevenueTotals, RuleId, Severity, Snapshot};
use apex_audit::rules::app_ads_txt::check;
use apex_core::PlatformId;

/// Every corpus file stem (both `<stem>.txt` and `<stem>.expected.json` must
/// exist, and nothing else may live in the corpus directory).
const CORPUS: [&str; 10] = [
    "clean-standard",
    "commented",
    "variables-only",
    "crlf-bom",
    "reseller",
    "malformed-two-field",
    "malformed-five-field",
    "mixed-duplicates",
    "google-cert",
    "comments-only",
];

/// A domain no fixture can contain: with the expected networks set to the
/// sidecar domains plus this sentinel, exactly the sentinel may be missing.
const SENTINEL: &str = "not-listed-anywhere.example";

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adstxt")
}

fn snapshot_for(lines: Vec<String>, expected_networks: Vec<String>) -> Snapshot {
    Snapshot {
        platform: PlatformId::Admob,
        fetched_at_unix_ms: 1_700_000_000_000,
        ad_units: Vec::new(),
        waterfall: Vec::new(),
        mediation_groups: Vec::new(),
        bidder_accounts: Vec::new(),
        country_revenue: Vec::new(),
        app_ads_txt: AppAdsTxt {
            present: true,
            lines,
            expected_networks,
        },
        platform_totals_30d: RevenueTotals {
            revenue_30d: 0,
            impressions_30d: 0,
        },
        mediation_totals_30d: RevenueTotals {
            revenue_30d: 0,
            impressions_30d: 0,
        },
    }
}

#[test]
fn corpus_directory_is_complete_and_paired() {
    let found: BTreeSet<String> = std::fs::read_dir(corpus_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    let mut expected: BTreeSet<String> = BTreeSet::new();
    for stem in CORPUS {
        expected.insert(format!("{stem}.txt"));
        expected.insert(format!("{stem}.expected.json"));
    }
    assert_eq!(
        found, expected,
        "corpus files drifted from the hardcoded list"
    );
    assert!(CORPUS.len() >= 8, "the corpus must hold at least 8 files");
}

#[test]
fn rule_matches_the_expected_errors_for_every_corpus_file() {
    let dir = corpus_dir();
    for stem in CORPUS {
        let file_name = format!("{stem}.txt");
        let text = std::fs::read_to_string(dir.join(&file_name)).unwrap();
        let expected_text =
            std::fs::read_to_string(dir.join(format!("{stem}.expected.json"))).unwrap();
        let expected: serde_json::Value = serde_json::from_str(&expected_text).unwrap();
        assert_eq!(expected["file"].as_str(), Some(file_name.as_str()));

        let expected_lines: Vec<u64> = expected["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|error| error["line"].as_u64().unwrap())
            .collect();
        assert!(
            expected_lines.windows(2).all(|pair| pair[0] < pair[1]),
            "{stem}: the sidecar errors must be sorted by line"
        );
        let domains: Vec<String> = expected["domains"]
            .as_array()
            .unwrap()
            .iter()
            .map(|domain| domain.as_str().unwrap().to_string())
            .collect();

        // Split on '\n' only, so CRLF bytes reach the rule and must be
        // tolerated there (the sales parser receives the same text through
        // `str::lines()`).
        let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        let mut expected_networks = domains.clone();
        expected_networks.push(SENTINEL.to_string());
        let snapshot = snapshot_for(lines, expected_networks);
        let findings = check(&snapshot);

        let lines_finding = findings
            .iter()
            .find(|finding| finding.subject == "app_ads_txt:lines");
        let networks_finding = findings
            .iter()
            .find(|finding| finding.subject == "app_ads_txt:networks");
        for finding in &findings {
            assert_eq!(finding.rule, RuleId::AppAdsTxt, "{stem}");
        }

        if expected_lines.is_empty() {
            assert!(
                lines_finding.is_none(),
                "{stem}: legal lines must not be reported as malformed"
            );
            assert_eq!(
                findings.len(),
                1,
                "{stem}: only the sentinel may be missing"
            );
        } else {
            assert_eq!(findings.len(), 2, "{stem}");
            let finding = lines_finding.unwrap();
            assert_eq!(finding.severity, Severity::Warning, "{stem}");
            assert_eq!(finding.evidence["malformed_count"], expected_lines.len());
            assert_eq!(
                finding.evidence["malformed_line_numbers"],
                serde_json::json!(expected_lines),
                "{stem}: malformed lines drifted from the sidecar"
            );
            assert!(
                finding
                    .message
                    .contains(&format!("{} malformed line(s)", expected_lines.len())),
                "{stem}: finding message shape changed: {}",
                finding.message
            );
        }

        // Every accepted-record domain satisfies presence (four-field records
        // and the google.com wrong-certification-record case included), so
        // only the sentinel is missing.
        let networks = networks_finding.unwrap();
        assert_eq!(networks.severity, Severity::Warning, "{stem}");
        assert_eq!(
            networks.evidence["expected_networks_count"],
            domains.len() + 1,
            "{stem}"
        );
        assert_eq!(networks.evidence["missing_count"], 1, "{stem}");
        assert_eq!(
            networks.evidence["missing_networks"],
            serde_json::json!([SENTINEL]),
            "{stem}: a listed domain was reported missing"
        );
    }
}
