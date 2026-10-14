//! The dogfood gate family, ported from dogfood-gates.mjs / yaml-policy-gate.mjs.

use std::collections::BTreeSet;

use regex::Regex;

use crate::classify::Manifest;
use crate::glob;
use crate::policy::{read_floors_txt, Floors, Policies, PolicyBaseline};
use crate::repo::Repo;
use crate::util::re;

pub fn orphan_gate(
    manifest: &Manifest,
    suites: &[Vec<String>],
    max_orphan_percent: f64,
) -> Result<(), String> {
    let code_files: Vec<&crate::classify::ManifestFile> = manifest
        .files
        .iter()
        .filter(|f| f.kind.as_deref() == Some("code"))
        .collect();
    let claimed = |path: &str| {
        suites
            .iter()
            .any(|covers| covers.iter().any(|g| glob::matches(g, path)))
    };
    let orphans: Vec<&str> = code_files
        .iter()
        .filter(|f| f.status == "planned" && !claimed(&f.path))
        .map(|f| f.path.as_str())
        .collect();
    let pct = if code_files.is_empty() {
        0.0
    } else {
        orphans.len() as f64 / code_files.len() as f64 * 100.0
    };
    if pct > max_orphan_percent {
        let first: Vec<&str> = orphans.iter().take(40).copied().collect();
        return Err(format!(
            "ORPHAN GATE FAILED: {}/{} code files ({pct:.2}%) unclaimed (max {max_orphan_percent}%). First: {}",
            orphans.len(),
            code_files.len(),
            first.join(", ")
        ));
    }
    println!(
        "orphan gate PASS: {}/{} orphans ({pct:.2}% <= {max_orphan_percent}%)",
        orphans.len(),
        code_files.len()
    );
    Ok(())
}

pub fn floor_ratchet(floors: &Floors) -> Result<(), String> {
    for pair in floors.history.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if b.orphan_floor_percent > a.orphan_floor_percent {
            return Err("floor-ratchet FAILED: orphan ceiling increased in history".into());
        }
        if b.mutation_kill_floor_percent < a.mutation_kill_floor_percent {
            return Err("floor-ratchet FAILED: mutation floor decreased in history".into());
        }
        for (d, v) in &b.domains {
            if a.domains.get(d).copied().unwrap_or(0.0) > *v {
                return Err(format!(
                    "floor-ratchet FAILED: domain {d} floor decreased in history"
                ));
            }
        }
    }
    if let Some(last) = floors.history.last() {
        if floors.orphan_floor_percent > last.orphan_floor_percent && !floors.history.is_empty() {
            return Err(
                "floor-ratchet FAILED: current orphan ceiling exceeds last recorded".into(),
            );
        }
        if floors.mutation_kill_floor_percent < last.mutation_kill_floor_percent {
            return Err("floor-ratchet FAILED: current mutation floor below last recorded".into());
        }
        // A domain recorded in history may never disappear or drop below its
        // recorded floor in the live table; otherwise a domain gate can be
        // removed or weakened after being pinned.
        for (d, v) in &last.domains {
            match floors.domains.get(d) {
                None => {
                    return Err(format!(
                        "floor-ratchet FAILED: domain {d} (floor {v}) disappeared from the live domain table"
                    ));
                }
                Some(current) if *current < *v => {
                    return Err(format!(
                        "floor-ratchet FAILED: domain {d} floor {current} below last recorded {v}"
                    ));
                }
                _ => {}
            }
        }
    }
    println!("floor-ratchet PASS: orphan ceiling non-increasing, domain and mutation floors non-decreasing");
    Ok(())
}

pub fn floors_sync(repo: &Repo) -> Result<(), String> {
    let floors = Floors::load(repo)?;
    let txt = read_floors_txt(repo)?;
    let mut problems = Vec::new();
    for (d, v) in &floors.domains {
        match txt.get(d) {
            None => problems.push(format!("floors.txt missing domain '{d}'")),
            Some(t) if t != v => problems.push(format!("floors.txt {d}={t} != json {v}")),
            _ => {}
        }
    }
    for d in txt.keys() {
        if !floors.domains.contains_key(d) {
            problems.push(format!("floors.txt has unknown domain '{d}'"));
        }
    }
    // Every code domain the classifier knows must carry a floor entry, so a
    // new domain cannot slip in unmeasured.
    let policies = Policies::load(repo)?;
    let missing: Vec<&str> = policies
        .domains
        .iter()
        .filter(|domain| domain.kind == "code")
        .map(|domain| domain.id.as_str())
        .filter(|id| !floors.domains.contains_key(*id))
        .collect();
    for id in missing {
        problems.push(format!("no floor entry for code domain '{id}'"));
    }
    if !problems.is_empty() {
        return Err(format!(
            "FLOORS-SYNC FAILED (run sync-floors):\n  - {}",
            problems.join("\n  - ")
        ));
    }
    println!("floors-sync PASS: {} domains in lockstep", txt.len());
    Ok(())
}

pub fn sync_floors_write(repo: &Repo) -> Result<(), String> {
    let floors = Floors::load(repo)?;
    let mut lines = vec![
        "# GENERATED by apex-tools sync-floors — DO NOT EDIT.".to_string(),
        "# Source of truth: tools/coverage/coverage-floor.json (ratchet-only).".to_string(),
        "# Format: <domain> <minimum-percent>.".to_string(),
    ];
    for (d, v) in &floors.domains {
        lines.push(format!(
            "{d} {}",
            if v.fract() == 0.0 {
                format!("{}", *v as i64)
            } else {
                format!("{v}")
            }
        ));
    }
    std::fs::write(
        repo.path("tools/coverage/floors.txt"),
        lines.join("\n") + "\n",
    )
    .map_err(|e| e.to_string())?;
    println!("floors.txt written ({} domains)", floors.domains.len());
    Ok(())
}

/// Scaffolding markers and their allowlist. `placeholder` only counts when it
/// labels deferred work (`placeholder implementation`, `placeholder values`):
/// the SDKs legitimately render test-mode placeholder creatives, and blanket
/// matching would ban real product behavior.
/// One scaffolding marker and its human label.
pub type StubPattern = (Regex, &'static str);

pub fn stub_patterns() -> Result<(Vec<StubPattern>, Regex), String> {
    let patterns: Vec<StubPattern> = vec![
        (re(r"todo!\(")?, "rust todo!()"),
        (re(r"unimplemented!\(")?, "rust unimplemented!()"),
        (re(r"\bTODO\b")?, "TODO marker"),
        (re(r"\bFIXME\b")?, "FIXME marker"),
        (re(r"\bTBD\b")?, "TBD marker"),
        (re(r"(?i)lorem ipsum")?, "lorem ipsum"),
        (re(r"(?i)coming soon")?, "coming soon"),
        (re(r"(?i)not implemented")?, "not-implemented prose"),
        (
            re(
                r"(?i)placeholder[-\s](implementation|impl|value|values|configuration|config|data)\b",
            )?,
            "placeholder marker",
        ),
        (re(r"\bWIP\b")?, "work-in-progress marker"),
    ];
    // `Status::unimplemented` is a gRPC status for an unimplemented RPC. The
    // UIKit `init(coder:)` idiom uses `fatalError` with the same words, which
    // is also not scaffolding; both are allowed.
    let allow = re(
        r"placeholder\s*=|::placeholder|placeholder-shown|Status::unimplemented|init\(coder:\)",
    )?;
    Ok((patterns, allow))
}

/// 1-based line numbers carrying a Rust `#[ignore]` marker.
pub fn maybe_ignore_marker(text: &str, marker: &Regex) -> Result<Vec<usize>, String> {
    Ok(text
        .lines()
        .enumerate()
        .filter(|(_, line)| marker.is_match(line))
        .map(|(i, _)| i + 1)
        .collect())
}

/// Does this file define test cases? Markers are language-aware: the JS `it`
/// and test forms, the Kotlin test-function prefix, and the C# `[Test]`
/// attribute would otherwise fire on production names like `setTestMode` or
/// `TestModeAdLoader` in other languages.
pub fn defines_test_cases(rel: &str, text: &str) -> Result<bool, String> {
    let ext = rel.rsplit('.').next().unwrap_or("");
    let hit = |pat: &str| re(pat).map(|r| r.is_match(text));
    match ext {
        "rs" => Ok(hit(
            r"#\[((tokio::)?test|test_case|rstest|test_log::test|parameterized)\]",
        )?),
        "kt" | "java" => Ok(hit(r"@Test\b")?
            || hit(r"fun\s+test[A-Z0-9_]\w*\s*\(")?
            || hit(r"class\s+\w+Tests?\s*[:{]")?),
        // XCTest files subclass XCTestCase. A plain `func testX()` can also be
        // a production harness (the adapter dev kit ships one), so the function
        // name alone is not a marker for Swift.
        "swift" => hit(r"XCTestCase"),
        "cs" => Ok(
            hit(r"\[(Test|TestCase|Fact|Theory|UnityTest)(\([^]]*\))?\]")?
                || hit(r"class\s+\w+Tests?\s*[:{]")?,
        ),
        "dart" => hit(r"(^|[^.\w])(test|testWidgets|group)\("),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => {
            // No lookbehind in the regex crate: an explicit preceding-char
            // class carries the JS negative-lookbehind semantics.
            hit(r"(^|[^.\w])(it|test)\(")
        }
        "py" => hit(r"def test_"),
        _ => Ok(false),
    }
}

fn assert_re() -> Result<Regex, String> {
    // `\bassert\b` alone misses the Rust macros (`assert_eq!`, `assert_ne!`,
    // `matches!`) because `_` and `!` are not word boundaries.
    re("(#expect|XCTAssert|XCTFail|expect\\(|assert\\.|\\bassert(_eq|_ne)?!|\\bdebug_assert!|\\bassert\\b|Assert\\.|require\\.Equal|t\\.Error|t\\.Fatal|\\[Fact\\]|\\[ \"\\$|assertTrue|assertFalse|assertEquals|assertNotNull|assertNull|assertThrows|assertFailsWith|@Test\\(expected|ViewAssertions|\\bmatches!?\\()")
}

pub fn assertion_presence(repo: &Repo) -> Result<(), String> {
    let ext_re = re(r"\.(py|ts|tsx|js|jsx|mjs|cjs|dart|go|rs|kt|java|swift|cs|bats)$")?;
    let assertion = assert_re()?;
    let mut bad = Vec::new();
    let mut checked = 0usize;
    // Content-based: any code file that defines test cases must carry at least
    // one assertion, independent of how the classifier labels the path. Inline
    // Rust `#[cfg(test)]` modules live inside `src/` files, so a
    // classification-gated scan would silently skip the entire all-Rust test
    // surface.
    for rel in repo.walk() {
        if !ext_re.is_match(&rel) {
            continue;
        }
        let Ok(text) = repo.read(&rel) else { continue };
        if !defines_test_cases(&rel, &text)? {
            continue;
        }
        checked += 1;
        if !assertion.is_match(&text) {
            bad.push(format!(
                "{rel} (defines test cases but contains no assertion)"
            ));
        }
    }
    if !bad.is_empty() {
        return Err(format!(
            "ASSERTION-PRESENCE GATE FAILED:\n  - {}",
            bad.join("\n  - ")
        ));
    }
    println!("assertion-presence PASS ({checked} files define test cases)");
    Ok(())
}

pub fn stub_ban(repo: &Repo) -> Result<(), String> {
    let roots = [
        "rust/",
        "tools/coverage/",
        "site/",
        ".woodpecker/",
        "infrastructure/",
        "monitoring/",
        ".github/",
    ];
    let (patterns, allow) = stub_patterns()?;
    let mut bad = Vec::new();
    for rel in repo.walk() {
        if !roots.iter().any(|r| rel.starts_with(r)) {
            continue;
        }
        if rel == "tools/coverage/dogfood.rs" || rel.contains("/src/dogfood.rs") {
            continue; // the pattern table itself is data, not scaffolding
        }
        let Ok(text) = repo.read(&rel) else { continue };
        for (i, line) in text.lines().enumerate() {
            if allow.is_match(line) {
                continue;
            }
            for (re, name) in &patterns {
                if re.is_match(line) {
                    bad.push(format!("{rel}:{}: [{name}] {}", i + 1, line.trim()));
                    if bad.len() > 60 {
                        break;
                    }
                }
            }
        }
    }
    if !bad.is_empty() {
        return Err(format!(
            "STUB-BAN FAILED ({} markers):\n  - {}",
            bad.len(),
            bad.join("\n  - ")
        ));
    }
    println!("stub-ban PASS: no scaffolding markers in {:?}", roots);
    Ok(())
}

pub fn walker_parity(repo: &Repo, manifest_path: &str) -> Result<(), String> {
    let text = repo.read(manifest_path)?;
    let manifest: Manifest = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut in_manifest: BTreeSet<String> = BTreeSet::new();
    let mut folded: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for f in &manifest.files {
        in_manifest.insert(f.path.clone());
        folded.insert(f.path.to_lowercase(), f.path.clone());
    }
    let tracked = repo.tracked_files_with_modes()?;
    let mut invisible = Vec::new();
    let mut missing_on_disk = Vec::new();
    let mut case_folded = Vec::new();
    let mut gitlinks = 0usize;
    for (mode, path) in tracked {
        if mode == "160000" {
            gitlinks += 1;
            continue;
        }
        if in_manifest.contains(&path) {
            continue;
        }
        // A tracked file that is absent from disk cannot be classified: the
        // index and the tree disagree, and a fresh CI checkout would resurrect
        // the file. Fail and force either `git add -A` or a restore.
        if !repo.path(&path).exists() {
            missing_on_disk.push(path);
            continue;
        }
        if let Some(found) = folded.get(&path.to_lowercase()) {
            case_folded.push(format!("{path} ~ {found}"));
            continue;
        }
        invisible.push(format!("{path} (mode {mode})"));
    }
    if !missing_on_disk.is_empty() {
        let first: Vec<&str> = missing_on_disk
            .iter()
            .take(10)
            .map(String::as_str)
            .collect();
        return Err(format!(
            "WALKER-PARITY FAILED: {} tracked paths are missing from disk (index/tree divergence). Stage the deletions (git add -A) or restore them. First: {}",
            missing_on_disk.len(),
            first.join(", ")
        ));
    }
    if !invisible.is_empty() {
        return Err(format!(
            "WALKER-PARITY FAILED: {} tracked paths invisible to the manifest:\n  - {}",
            invisible.len(),
            invisible.join("\n  - ")
        ));
    }
    for note in &case_folded {
        println!("walker-parity note: case-only name difference on this filesystem: {note}");
    }
    println!(
        "walker-parity PASS: all tracked regular files and symlinks classified ({} manifest paths, {gitlinks} gitlinks accounted, {} case-folded, 0 missing from disk)",
        in_manifest.len(),
        case_folded.len()
    );
    Ok(())
}

pub fn mutation_floor(repo: &Repo, floor_override: f64) -> Result<(), String> {
    let floors = Floors::load(repo)?;
    let required = floor_override.max(floors.mutation_kill_floor_percent);
    let path = "tools/coverage/mutation-results.json";
    if !repo.exists(path) {
        // Fail closed: a deleted results file must not read as a pass.
        return Err(format!(
            "MUTATION FLOOR FAILED: {path} is missing; run mutation-smoke first (floor={required}%)"
        ));
    }
    let v: serde_json::Value =
        serde_json::from_str(&repo.read(path)?).map_err(|e| e.to_string())?;
    let total = v
        .get("total")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if total == 0 {
        return Err(format!(
            "MUTATION FLOOR FAILED: {path} records zero mutants; the sample must not be empty"
        ));
    }
    let score = v
        .get("killRatePercent")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    if score < required {
        return Err(format!(
            "MUTATION FLOOR FAILED: kill rate {score}% < {required}% ({total} mutants sampled)"
        ));
    }
    println!("mutation-floor PASS: kill rate {score}% >= {required}% ({total} mutants sampled)");
    Ok(())
}

pub fn yaml_policy(repo: &Repo, full: bool, shell: bool) -> Result<(), String> {
    let baseline = PolicyBaseline::load(repo)?;
    let shell_allow: Vec<&str> = Vec::new();
    let pragma = re(r"(pragma:\s*no cover|istanbul ignore|c8 ignore-next|coverage-ignore)")?;
    let skip = re(r"(describe|it|test)\.(skip|todo)\(")?;
    // Matches both `#[ignore]` and the attributed `#[ignore = "reason"]` form.
    let ignore_re = re(r"#\[ignore\b")?;
    let destructive = re(
        r"(?i)\b(deploy|ssh|kubectl|docker|pm2|systemctl|helm|terraform|migrate|drop|truncate)\b",
    )?;
    let set_ok_re = re(r"set\s+-eu(o\s+pipefail)?")?;
    let code_ext = re(r"\.(ts|tsx|js|jsx|mjs|cjs|py|kt|swift|rs|cs|go)$")?;
    let test_ext = re(r"\.(ts|tsx|js|jsx)$")?;
    let or_true_re = re(r"\|\|\s*true")?;
    let mut actual: std::collections::BTreeMap<&str, Vec<String>> =
        std::collections::BTreeMap::new();
    for k in ["shell", "pragma", "skip", "orTrue"] {
        actual.insert(k, Vec::new());
    }
    let in_test_tree = |rel: &str| {
        rel.contains("/tests/")
            || rel.contains("__tests__")
            || rel.contains(".test.")
            || rel.contains(".spec.")
    };
    for rel in repo.walk() {
        let Ok(text) = repo.read(&rel) else { continue };
        let is_shell = rel.ends_with(".sh") || rel == ".husky/pre-commit";
        let mut shell_hits: Vec<String> = Vec::new();
        if (shell || full) && is_shell {
            if rel.ends_with(".sh") {
                let head: String = text.chars().take(2000).collect();
                if !set_ok_re.is_match(&head) && !shell_allow.iter().any(|p| rel.starts_with(p)) {
                    shell_hits.push(format!("{rel}: set -eu missing"));
                }
            }
            if destructive.is_match(&text) && !text.contains("DOGFOOD_DRY_RUN") {
                shell_hits.push(format!("{rel}: destructive without DOGFOOD_DRY_RUN guard"));
            }
        }
        if !shell_hits.is_empty() {
            actual
                .get_mut("shell")
                .ok_or("shell bucket missing")?
                .append(&mut shell_hits);
        }
        if full {
            if code_ext.is_match(&rel)
                && pragma.is_match(&text)
                // Only the gate file that defines the pragma regex may contain
                // the pattern text; every other file (including the rest of
                // apex-tools) is scanned. A whole-crate exclusion hid markers.
                && rel != "rust/crates/apex-tools/src/dogfood.rs"
            {
                actual
                    .get_mut("pragma")
                    .ok_or("pragma bucket missing")?
                    .push(rel.clone());
            }
            if in_test_tree(&rel)
                && test_ext.is_match(&rel)
                && skip.is_match(&text)
                && !text.contains("dogfood:skip:")
            {
                actual
                    .get_mut("skip")
                    .ok_or("skip bucket missing")?
                    .push(rel.clone());
            }
            // Rust's skip marker is #[ignore]; the JS describe/it form does
            // not exist in this tree, and src/tests.rs style modules are not
            // under a /tests/ path. The gate's own source contains the
            // `dogfood:skip:` literal, so it is excluded explicitly rather
            // than through the marker check, which would exempt it always.
            if rel.ends_with(".rs")
                && rel != "rust/crates/apex-tools/src/dogfood.rs"
                && !text.contains("dogfood:skip:")
            {
                maybe_ignore_marker(&text, &ignore_re)?
                    .into_iter()
                    .for_each(|line| {
                        if let Some(bucket) = actual.get_mut("skip") {
                            bucket.push(format!("{rel}:{line}"));
                        }
                    });
            }
            if (rel.ends_with(".yml") || rel.ends_with(".yaml"))
                && (rel.starts_with(".github/workflows/") || rel.starts_with(".woodpecker/"))
            {
                for (i, line) in text.lines().enumerate() {
                    if line.trim_start().starts_with('#') {
                        continue;
                    }
                    if or_true_re.is_match(line) {
                        actual
                            .get_mut("orTrue")
                            .ok_or("orTrue bucket missing")?
                            .push(format!("{rel}:{}", i + 1));
                    }
                }
            }
        }
    }
    let scopes: Vec<&str> = if full {
        vec!["shell", "pragma", "skip", "orTrue"]
    } else {
        vec!["shell"]
    };
    let mut violations: Vec<String> = Vec::new();
    let mut stale: Vec<String> = Vec::new();
    for kind in scopes {
        let base: BTreeSet<String> = match kind {
            "shell" => baseline.shell.iter().cloned().collect(),
            "pragma" => baseline.pragma.iter().cloned().collect(),
            "skip" => baseline.skip.iter().cloned().collect(),
            _ => baseline.or_true.iter().cloned().collect(),
        };
        let items = actual
            .get(kind)
            .cloned()
            .ok_or_else(|| format!("bucket {kind} missing"))?;
        for v in &items {
            let key = if kind == "orTrue" {
                v.split(':').next().unwrap_or(v).to_string()
            } else {
                v.split(": ").next().unwrap_or(v).to_string()
            };
            if base.contains(v) || base.contains(&key) {
                continue;
            }
            violations.push(format!("[{kind}] {v}"));
        }
        for b in &base {
            let present = items
                .iter()
                .any(|v| v == b || v.starts_with(&format!("{b}:")));
            if !present {
                stale.push(format!(
                    "[{kind}] '{b}' is no longer a violation; remove it from policy-baseline.json"
                ));
            }
        }
    }
    if !violations.is_empty() || !stale.is_empty() {
        return Err(format!(
            "POLICY GATE FAILED ({} new, {} stale):\n  - {}",
            violations.len(),
            stale.len(),
            violations
                .iter()
                .chain(stale.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n  - ")
        ));
    }
    let counts: Vec<String> = ["shell", "pragma", "skip", "orTrue"]
        .iter()
        .map(|k| {
            format!(
                "{k}={}",
                match *k {
                    "shell" => baseline.shell.len(),
                    "pragma" => baseline.pragma.len(),
                    "skip" => baseline.skip.len(),
                    _ => baseline.or_true.len(),
                }
            )
        })
        .collect();
    println!(
        "policy gate PASS ({}, baseline: {})",
        if full { "full" } else { "shell" },
        counts.join(" ")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assertion_regex_matches_rust_macros() {
        let assertion = assert_re().unwrap();
        for text in [
            "assert_eq!(a, b);",
            "assert_ne!(a, b);",
            "assert!(x.is_ok());",
            "debug_assert!(x);",
            "matches!(x, Some(_));",
        ] {
            assert!(assertion.is_match(text), "missing assertion match: {text}");
        }
    }

    #[test]
    fn ignore_markers_are_line_numbered_in_both_forms() {
        let marker = re(r"#\[ignore\b").unwrap();
        let text = "#[test]\n#[ignore]\nfn slow() {}\n#[ignore = \"slow\"]\nfn slower() {}\n";
        assert_eq!(maybe_ignore_marker(text, &marker).unwrap(), vec![2, 4]);
        assert_eq!(
            maybe_ignore_marker("#[test]\n", &marker).unwrap(),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn floor_ratchet_rejects_a_domain_removed_after_being_pinned() {
        let floors = Floors {
            orphan_floor_percent: 0.0,
            mutation_kill_floor_percent: 60.0,
            domains: std::collections::BTreeMap::new(),
            history: vec![crate::policy::HistoryEntry {
                orphan_floor_percent: 0.0,
                mutation_kill_floor_percent: 60.0,
                domains: [("apex-core".to_string(), 40.0)].into_iter().collect(),
                at: "test".into(),
            }],
        };
        let err = floor_ratchet(&floors).unwrap_err();
        assert!(err.contains("apex-core"), "{err}");
    }
}

#[cfg(test)]
mod marker_tests {
    use super::*;

    #[test]
    fn test_case_markers_are_language_aware() {
        assert!(!defines_test_cases("a/BelAds.kt", "fun setTestMode(enabled: Boolean)").unwrap());
        assert!(!defines_test_cases("a/AuctionClient.kt", "fun isTestRuntime()").unwrap());
        assert!(!defines_test_cases(
            "a/TestModeAdLoader.swift",
            "class TestModeAdLoader: Sendable"
        )
        .unwrap());
        assert!(defines_test_cases("a/BelAdsTest.kt", "fun testFoo() {}").unwrap());
        assert!(defines_test_cases("a/BelAdsTest.kt", "class BelAdsTest {").unwrap());
        assert!(defines_test_cases("a/BelAdsTest.kt", "@Test fun x() {}").unwrap());
        assert!(defines_test_cases("a/XTests.swift", "class XTests: XCTestCase {}").unwrap());
        assert!(!defines_test_cases("a/Harness.swift", "func testInterstitial() {}").unwrap());
        assert!(defines_test_cases("a/XTests.cs", "[Test] public void x() {}").unwrap());
        assert!(defines_test_cases("a/x.test.ts", "it('works', () => {});").unwrap());
        assert!(defines_test_cases("a/x.test.ts", "test('works', () => {});").unwrap());
        assert!(!defines_test_cases("a/legit.ts", "await commit('works');").unwrap());
        assert!(defines_test_cases("a/x_test.dart", "test('works', () {});").unwrap());
        assert!(defines_test_cases("a/x_test.py", "def test_works():").unwrap());
        assert!(defines_test_cases("a/x.rs", "#[tokio::test] async fn x() {}").unwrap());
        assert!(defines_test_cases("a/x.rs", "#[test_case] fn x() {}").unwrap());
        assert!(defines_test_cases("a/x.rs", "#[rstest] fn x() {}").unwrap());
        assert!(defines_test_cases("a/x.rs", "#[parameterized] fn x() {}").unwrap());
        assert!(!defines_test_cases("a/readme.md", "def test_works()").unwrap());
    }
}

#[cfg(test)]
mod stub_pattern_tests {
    use super::*;

    #[test]
    fn placeholder_only_flags_deferred_work() {
        let (patterns, allow) = stub_patterns().unwrap();
        let flagged = |line: &str| {
            patterns
                .iter()
                .any(|(p, _)| p.is_match(line) && !allow.is_match(line))
        };
        assert!(flagged("// Placeholder implementation - adjust"));
        assert!(flagged("// For now, return placeholder values"));
        assert!(flagged("# Placeholder configuration for now"));
        assert!(!flagged("# Placeholder ProGuard rules"));
        assert!(!flagged("renders a safe placeholder in test mode"));
        assert!(!flagged("placeholder.dismiss(animated: true)"));
        assert!(!flagged("/// Provides a placeholder tint"));
        assert!(!flagged("android:text=\"Ad placeholder\""));
        assert!(!flagged("fatalError(\"init(coder:) not implemented\")"));
        assert!(flagged(concat!("// TO", "DO: wire this")));
    }
}
