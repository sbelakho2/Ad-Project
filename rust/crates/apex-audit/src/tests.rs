//! Cross-cutting adversarial tests: deterministic ordering under permuted input
//! vectors, digest stability and sensitivity, total recovery accounting, and
//! message/evidence number parity.
//!
//! ## Message-number parity rule
//!
//! Every standalone numeric token in a finding's `message` must also appear as
//! a number somewhere in the finding's `evidence`. Digit runs glued to letters
//! or underscores (identifiers such as `u1`, `30d`) are treated as identifiers,
//! not numbers; the crafted fixtures below use digit-free ids so the check is
//! strict for every actual data number.

use std::collections::BTreeSet;

use crate::model::{Finding, RuleId, Severity, Snapshot};
use crate::testkit::{ads_txt, bidder, empty_snapshot, floor, row, totals, unit, waterfall_entry};
use crate::{run, snapshot_digest, GENERATED_BY};

/// A snapshot crafted to trigger every rule at least once (all ids digit-free
/// so number parity is strict).
fn rich_snapshot() -> Snapshot {
    let mut snapshot = empty_snapshot();
    snapshot.ad_units = vec![
        unit("uA", true, vec![floor("EE", 2_500_000)]),
        unit("uB", true, Vec::new()),
    ];
    snapshot.country_revenue = vec![
        // stale floor: zero revenue over 100_000 impressions on a floored pair
        row("EE", "uA", 0, 100_000),
        // eCPM basis for unit uA
        row("US", "uA", 10_000_000, 100_000),
        // missing country floor: 60% of platform revenue, no floor on uB
        row("DE", "uB", 30_000_000, 300_000),
    ];
    snapshot.waterfall = vec![waterfall_entry("e1", "uA", "metaAds")];
    snapshot.bidder_accounts = vec![
        bidder("metaAds", true, false),  // integrated -> Critical
        bidder("unityAds", true, false), // not integrated -> Info
    ];
    snapshot.app_ads_txt = ads_txt(
        true,
        &[
            "admob.com, pubA, DIRECT",
            "broken line",
            "METAADS.COM, pubB, DIRECT",
        ],
        &["admob.com", "metaads.com", "unityads.com"],
    );
    snapshot.platform_totals_30d = totals(50_000_000, 1_000_000);
    snapshot.mediation_totals_30d = totals(45_000_000, 1_000_000);
    snapshot
}

/// Numeric tokens that the message/evidence parity contract covers.
///
/// Contract (documented in lib.rs): parity applies to every numeric token
/// OUTSIDE backticked spans. Backticked spans carry identifiers verbatim
/// (country codes, networks, domains) which may themselves contain digit runs;
/// those are data, not claims. The scanner is:
/// - backtick-aware (spans between ` pairs are skipped entirely),
/// - sign-aware ("-42" is the token -42, not 42),
/// - strict: a standalone digit run that does not parse as i128 PANICS, so an
///   unparseable run can never be silently ignored (verifier finding A4).
fn message_numbers(message: &str) -> Vec<i128> {
    let mut outside = String::new();
    for (index, chunk) in message.split('`').enumerate() {
        if index % 2 == 0 {
            outside.push_str(chunk);
        }
    }
    let chars: Vec<char> = outside.chars().collect();
    let mut numbers = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let negative = chars[index] == '-'
            && index + 1 < chars.len()
            && chars[index + 1].is_ascii_digit()
            && (index == 0 || !chars[index - 1].is_ascii_alphanumeric());
        let start = if negative { index + 1 } else { index };
        if chars[start].is_ascii_digit() {
            let mut end = start;
            while end < chars.len() && chars[end].is_ascii_digit() {
                end += 1;
            }
            let prev_is_alnum = start > 0 && chars[start - 1].is_ascii_alphanumeric();
            let next_is_alnum = end < chars.len() && chars[end].is_ascii_alphanumeric();
            if !prev_is_alnum && !next_is_alnum {
                let token: String = chars[index..end].iter().collect();
                let value = token
                    .parse::<i128>()
                    .unwrap_or_else(|_| panic!("message token {token:?} is not parseable as i128"));
                numbers.push(value);
            }
            index = end;
        } else {
            index += 1;
        }
    }
    numbers
}

fn evidence_numbers(value: &serde_json::Value, out: &mut Vec<i128>) {
    match value {
        serde_json::Value::Number(number) => {
            if let Some(value) = number.as_i64() {
                out.push(i128::from(value));
            } else if let Some(value) = number.as_u64() {
                out.push(i128::from(value));
            } else if let Some(value) = number.as_f64() {
                // Floats in evidence are compared by their exact decimal text;
                // any float in evidence must also be representable for the
                // message scanner (integers only), so surface it.
                out.push(i128::from(value as i64));
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                evidence_numbers(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for nested in map.values() {
                evidence_numbers(nested, out);
            }
        }
        _ => {}
    }
}

fn sort_key(finding: &Finding) -> (std::cmp::Reverse<Severity>, &'static str, &str) {
    (
        std::cmp::Reverse(finding.severity),
        finding.rule.as_str(),
        finding.subject.as_str(),
    )
}

#[test]
fn empty_snapshot_invariants() {
    let snapshot = empty_snapshot();
    let report = run(&snapshot);
    assert!(
        report.findings.is_empty(),
        "present-but-empty file is clean"
    );
    assert_eq!(report.total_estimated_monthly_recovery, 0);
    assert_eq!(report.snapshot_digest, snapshot_digest(&snapshot));

    let mut absent = empty_snapshot();
    absent.app_ads_txt = ads_txt(false, &[], &[]);
    let report = run(&absent);
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].rule, RuleId::AppAdsTxt);
    assert_eq!(report.findings[0].severity, Severity::Critical);
    assert_eq!(report.total_estimated_monthly_recovery, 0);
}

#[test]
fn findings_are_sorted_severity_then_rule_then_subject() {
    let report = run(&rich_snapshot());
    assert!(
        report.findings.len() >= 5,
        "fixture must exercise all rules"
    );
    // Every rule emits at most one finding per subject, so the sort key is
    // strictly increasing over the report.
    for pair in report.findings.windows(2) {
        assert!(
            sort_key(&pair[0]) < sort_key(&pair[1]),
            "unsorted: {:?} vs {:?}",
            sort_key(&pair[0]),
            sort_key(&pair[1])
        );
    }
    assert_eq!(
        report.findings.first().map(|f| f.severity),
        Some(Severity::Critical)
    );
    assert_eq!(
        report.findings.last().map(|f| f.severity),
        Some(Severity::Info)
    );
}

#[test]
fn permuted_input_vectors_produce_identical_findings() {
    let base = rich_snapshot();
    let mut permuted = base.clone();
    permuted.ad_units.reverse();
    permuted.waterfall.reverse();
    permuted.mediation_groups.reverse();
    permuted.bidder_accounts.reverse();
    permuted.country_revenue.reverse();

    let base_report = run(&base);
    let permuted_report = run(&permuted);
    assert_eq!(base_report.findings, permuted_report.findings);
    assert_eq!(
        base_report.total_estimated_monthly_recovery,
        permuted_report.total_estimated_monthly_recovery
    );
    // The digest is deliberately order-sensitive: it identifies the snapshot
    // value as given, not its set-semantics. Pinned so the contract is explicit.
    assert_ne!(base_report.snapshot_digest, permuted_report.snapshot_digest);
}

#[test]
fn digest_is_stable_and_sensitive() {
    let snapshot = rich_snapshot();
    let first = run(&snapshot).snapshot_digest;
    let second = run(&snapshot.clone()).snapshot_digest;
    assert_eq!(first, second);

    let mut tweaked = snapshot.clone();
    tweaked.platform_totals_30d.revenue_30d += 1;
    assert_ne!(first, run(&tweaked).snapshot_digest);
}

#[test]
fn message_numbers_are_backed_by_evidence_for_every_rule() {
    let report = run(&rich_snapshot());
    let mut covered: BTreeSet<RuleId> = BTreeSet::new();
    for finding in &report.findings {
        covered.insert(finding.rule);
        assert!(
            finding.evidence.is_object(),
            "evidence must be a JSON object: {}",
            finding.subject
        );
        let mut backed = Vec::new();
        evidence_numbers(&finding.evidence, &mut backed);
        for number in message_numbers(&finding.message) {
            assert!(
                backed.contains(&number),
                "message number {number} is not backed by evidence \
                 (rule {:?}, subject {}): {}",
                finding.rule,
                finding.subject,
                finding.message
            );
        }
    }
    let expected: BTreeSet<RuleId> = RuleId::ALL.into_iter().collect();
    assert_eq!(covered, expected, "the fixture must trigger every rule");
}

#[test]
fn total_is_the_exact_sum_of_findings() {
    let report = run(&rich_snapshot());
    let sum: i128 = report
        .findings
        .iter()
        .map(|finding| finding.estimated_monthly_recovery as i128)
        .sum();
    assert_eq!(report.total_estimated_monthly_recovery as i128, sum);
    assert!(report.total_estimated_monthly_recovery > 0);
    assert_eq!(report.generated_by, GENERATED_BY);
}

#[test]
fn extreme_values_do_not_panic_and_stay_exact() {
    let mut snapshot = empty_snapshot();
    snapshot.ad_units = vec![unit("uA", true, vec![floor("EE", i64::MAX)])];
    snapshot.country_revenue = vec![row("EE", "uA", 0, u64::MAX), row("US", "uA", i64::MAX, 1)];
    snapshot.platform_totals_30d = totals(i64::MAX, u64::MAX);
    let report = run(&snapshot);
    assert!(!report.findings.is_empty());
    for finding in &report.findings {
        assert!(finding.estimated_monthly_recovery >= 0);
    }
    // Hand computation with saturating impression sums:
    //   unit impressions: u64::MAX + 1 saturates to u64::MAX
    //   unit eCPM       : i64::MAX * 1000 / u64::MAX = 499 (truncated)
    //   recovery        : 499 * u64::MAX / 1000 = 9204925292781066255
    let stale = report
        .findings
        .iter()
        .find(|finding| finding.rule == RuleId::StaleFloor)
        .expect("stale floor finding");
    assert_eq!(stale.evidence["ad_unit_ecpm_micros"], 499);
    assert_eq!(stale.estimated_monthly_recovery, 9_204_925_292_781_066_255);
    assert_eq!(
        report.total_estimated_monthly_recovery,
        9_204_925_292_781_066_255
    );
}

// ---------------- verifier-requested regressions (adversarial re-attack) ----------------

#[test]
fn duplicate_ad_unit_ids_are_order_insensitive() {
    let enabled_unit = unit("uA", true, vec![floor("EE", 1)]);
    let disabled_unit = unit("uA", false, Vec::new());

    let mut first = empty_snapshot();
    first.platform_totals_30d = totals(10_000_000, 1_000);
    first.mediation_totals_30d = totals(10_000_000, 1_000);
    first.ad_units = vec![enabled_unit.clone(), disabled_unit.clone()];
    first.country_revenue = vec![row("EE", "uA", 1_500_000, 10_000)];

    let mut second = first.clone();
    second.ad_units = vec![disabled_unit, enabled_unit];

    let key = |snapshot: &Snapshot| {
        run(snapshot)
            .findings
            .iter()
            .map(|finding| (finding.rule, finding.severity, finding.subject.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        key(&first),
        key(&second),
        "duplicate ids must not make findings depend on vector order"
    );
}

#[test]
fn identifiers_with_digit_runs_stay_out_of_the_parity_contract() {
    let mut snapshot = empty_snapshot();
    snapshot.platform_totals_30d = totals(10_000_000, 1_000);
    snapshot.mediation_totals_30d = totals(10_000_000, 1_000);
    snapshot.bidder_accounts = vec![bidder("app 2", true, false)];
    snapshot.app_ads_txt = ads_txt(true, &["admob.com, pubA, DIRECT"], &["ads-7.example.com"]);

    let report = run(&snapshot);
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.message.contains("`app 2`")),
        "network identifiers must be backticked verbatim"
    );
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.message.contains("`ads-7.example.com`")),
        "expected-network identifiers must be backticked verbatim"
    );

    // The full parity contract must still hold: digit runs inside backticks are
    // identifiers, everything else must be backed by evidence.
    for finding in &report.findings {
        let mut backed = Vec::new();
        evidence_numbers(&finding.evidence, &mut backed);
        for number in message_numbers(&finding.message) {
            assert!(
                backed.contains(&number),
                "digit-bearing identifier leaked into the parity contract: {} ({})",
                finding.message,
                finding.subject
            );
        }
    }
}

#[test]
fn negative_one_sided_totals_do_not_break_parity() {
    let mut snapshot = empty_snapshot();
    snapshot.platform_totals_30d = totals(-5, 1_000);
    snapshot.mediation_totals_30d = totals(1_000_000, 1_000);
    let report = run(&snapshot);
    for finding in &report.findings {
        let mut backed = Vec::new();
        evidence_numbers(&finding.evidence, &mut backed);
        for number in message_numbers(&finding.message) {
            assert!(
                backed.contains(&number),
                "sign handling lost a message number: {} ({})",
                finding.message,
                finding.subject
            );
        }
    }
}

#[test]
fn extreme_share_clamps_consistently_in_message_and_evidence() {
    let mut snapshot = empty_snapshot();
    snapshot.platform_totals_30d = totals(1, 1);
    snapshot.mediation_totals_30d = totals(1, 1);
    snapshot.country_revenue = vec![row("EE", "uA", i64::MAX, 10_000)];

    let report = run(&snapshot);
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.rule == RuleId::MissingCountryFloor)
        .expect("extreme share must fire");
    let mut backed = Vec::new();
    evidence_numbers(&finding.evidence, &mut backed);
    for number in message_numbers(&finding.message) {
        assert!(
            backed.contains(&number),
            "clamped share must match between message and evidence: {}",
            finding.message
        );
    }
}
