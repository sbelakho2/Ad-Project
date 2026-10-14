//! Coverage artifact parsers and gates, ported from coverage-gate.mjs and
//! lcov-gate.mjs (fail-closed semantics preserved).

use crate::policy::Floors;
use crate::repo::Repo;

#[derive(Debug, Clone, Copy, Default)]
pub struct Metrics {
    pub lines: f64,
    pub branches: f64,
    pub functions: f64,
    pub statements: f64,
}

#[derive(Debug, Clone, Default)]
pub struct LcovFile {
    pub file: String,
    pub lh: u64,
    pub lf: u64,
    pub brh: u64,
    pub brf: u64,
    pub fnh: u64,
    pub fnf: u64,
}

pub fn parse_lcov(text: &str) -> Vec<LcovFile> {
    let mut out = Vec::new();
    let mut cur: Option<LcovFile> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("SF:") {
            cur = Some(LcovFile {
                file: rest.trim().to_string(),
                ..Default::default()
            });
        } else if let Some(f) = cur.as_mut() {
            if let Some(v) = line.strip_prefix("LF:") {
                f.lf = v.parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("LH:") {
                f.lh = v.parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("BRF:") {
                f.brf = v.parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("BRH:") {
                f.brh = v.parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("FNF:") {
                f.fnf = v.parse().unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("FNH:") {
                f.fnh = v.parse().unwrap_or(0);
            } else if line.starts_with("end_of_record") {
                if let Some(f) = cur.take() {
                    out.push(f);
                }
            }
        }
    }
    out
}

#[derive(Debug)]
pub struct LcovTotals {
    pub metrics: Metrics,
    pub branch_records: u64,
    pub zero_line_files: Vec<String>,
    pub violations: Vec<String>,
}

pub fn lcov_totals(files: &[LcovFile], require_branches: bool) -> LcovTotals {
    let sum = |f: fn(&LcovFile) -> u64| files.iter().map(f).sum::<u64>();
    let (lf, lh, brf, brh, fnf, fnh) = (
        sum(|f| f.lf),
        sum(|f| f.lh),
        sum(|f| f.brf),
        sum(|f| f.brh),
        sum(|f| f.fnf),
        sum(|f| f.fnh),
    );
    let pct = |h: u64, t: u64| {
        if t == 0 {
            100.0
        } else {
            (h as f64 / t as f64) * 100.0
        }
    };
    let mut violations = Vec::new();
    if require_branches && brf == 0 {
        violations.push("no branch data recorded although branches are required".to_string());
    }
    if !files.is_empty() && files.iter().all(|f| f.lf == 0) {
        violations.push("lcov records contain no executable lines (all LF:0)".to_string());
    }
    LcovTotals {
        metrics: Metrics {
            lines: pct(lh, lf),
            branches: if require_branches || brf > 0 {
                pct(brh, brf)
            } else {
                100.0
            },
            functions: pct(fnh, fnf),
            statements: 0.0,
        },
        branch_records: brf,
        zero_line_files: files
            .iter()
            .filter(|f| f.lf == 0 && !f.file.ends_with("__init__.py"))
            .map(|f| f.file.clone())
            .collect(),
        violations,
    }
}

/// Standalone lcov gate: same rules as the retired lcov-gate.mjs, including
/// the worst-files listing and the pass summary line format.
pub fn lcov_gate(
    path: &str,
    floor: f64,
    require_branches: bool,
    expect_crate: Option<&str>,
) -> Result<(), String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("lcov not found: {path} ({e})"))?;
    let files = parse_lcov(&text);
    if files.is_empty() {
        return Err("lcov contains no records".into());
    }
    // The artifact must cover the crate the matrix entry names: a wrong PKG
    // or a stale row must fail loudly instead of gating another crate's file.
    if let Some(crate_name) = expect_crate {
        let needle = format!("/crates/{crate_name}/");
        if !files.iter().any(|file| {
            file.file.contains(&needle) || file.file.starts_with(&format!("crates/{crate_name}/"))
        }) {
            return Err(format!(
                "LCOV GATE FAILED ({path}): no records under crates/{crate_name}; the artifact does not cover the named crate"
            ));
        }
    }
    let (pass, message) = evaluate_lcov_gate(&files, floor, require_branches);
    if pass {
        println!("lcov gate PASS ({path}): {message}");
        Ok(())
    } else {
        Err(format!("LCOV GATE FAILED ({path})\n{message}"))
    }
}

/// Pure evaluator for the standalone lcov gate: pass line when clean, or the
/// `  - violation` block plus the worst low-coverage files when not.
pub fn evaluate_lcov_gate(
    files: &[LcovFile],
    floor: f64,
    require_branches: bool,
) -> (bool, String) {
    let t = lcov_totals(files, require_branches);
    let mut violations = t.violations.clone();
    if t.metrics.lines < floor {
        violations.push(format!("lines: {:.2}% < {floor}%", t.metrics.lines));
    }
    if require_branches && t.branch_records > 0 && t.metrics.branches < floor {
        violations.push(format!("branches: {:.2}% < {floor}%", t.metrics.branches));
    }
    if t.metrics.functions < floor {
        violations.push(format!("functions: {:.2}% < {floor}%", t.metrics.functions));
    }
    if violations.is_empty() {
        return (
            true,
            format!(
                "lines={:.2}% branches={:.2}% functions={:.2}% over {} files",
                t.metrics.lines,
                t.metrics.branches,
                t.metrics.functions,
                files.len()
            ),
        );
    }
    let mut low: Vec<&LcovFile> = files
        .iter()
        .filter(|f| f.lf > 0 && pct(f.lh, f.lf) < floor)
        .collect();
    low.sort_by(|a, b| pct(a.lh, a.lf).total_cmp(&pct(b.lh, b.lf)));
    let mut lines: Vec<String> = violations.iter().map(|v| format!("  - {v}")).collect();
    for f in low.iter().take(20) {
        lines.push(format!("  low: {} {:.1}%", f.file, pct(f.lh, f.lf)));
    }
    (false, lines.join("\n"))
}

fn pct(h: u64, t: u64) -> f64 {
    if t == 0 {
        100.0
    } else {
        (h as f64 / t as f64) * 100.0
    }
}

/// Aggregate verdict: re-validates manifest completeness, orphan ceiling and the
/// floor ratchet, then writes the markdown report.
pub fn aggregate_verdict(repo: &Repo, report_rel: &str) -> Result<(), String> {
    let manifest = crate::classify::build_manifest(repo)?;
    let manifest_json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    std::fs::write(repo.path("coverage-manifest.json"), manifest_json)
        .map_err(|e| e.to_string())?;
    let floors = Floors::load(repo)?;
    let suites = crate::policy::read_suites(repo)?;
    crate::dogfood::orphan_gate(&manifest, &suites, floors.orphan_floor_percent)?;
    crate::dogfood::floor_ratchet(&floors)?;
    // The aggregate verdict replays the cheap invariants that guard the
    // manifest it just produced: floors mirror lockstep, the tracked tree is
    // fully visible, and the mutation sample still clears its floor.
    crate::dogfood::floors_sync(repo)?;
    crate::dogfood::walker_parity(repo, "coverage-manifest.json")?;
    crate::dogfood::mutation_floor(repo, 0.0)?;
    // Guards against lane and Makefile drift: every apex-tools invocation in
    // the pipelines must name a real subcommand.
    crate::inventory::check_pipeline_inventory(repo)?;

    let mut rows = String::new();
    for (d, v) in &floors.domains {
        rows.push_str(&format!(
            "| {d} | {v}% | {} |\n",
            if *v >= 100.0 { "DONE" } else { "ratcheting" }
        ));
    }
    let report = format!(
        "# Coverage gate report (apex-tools)\n\n- orphan floor: {}%\n- mutation kill floor: {}%\n\n| domain | floor | status |\n|---|---|---|\n{}",
        floors.orphan_floor_percent, floors.mutation_kill_floor_percent, rows
    );
    std::fs::write(repo.path(report_rel), report).map_err(|e| e.to_string())?;
    println!("AGGREGATE COVERAGE VERDICT: PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcov_totals_and_fail_closed_branches() {
        let files = parse_lcov("SF:a.ts\nLF:10\nLH:10\nFNF:1\nFNH:1\nend_of_record\n");
        assert_eq!(files.len(), 1);
        let t = lcov_totals(&files, true);
        assert!(!t.violations.is_empty());
        assert_eq!(lcov_totals(&files, false).metrics.lines, 100.0);
    }

    #[test]
    fn lcov_zero_line_files_exempt_init() {
        let files = parse_lcov(
            "SF:pkg/__init__.py\nLF:0\nLH:0\nend_of_record\nSF:x.py\nLF:0\nLH:0\nend_of_record\n",
        );
        let t = lcov_totals(&files, false);
        assert_eq!(t.zero_line_files, vec!["x.py".to_string()]);
    }

    #[test]
    fn lcov_gate_totals_and_pass_line() {
        let lcov = [
            "SF:a.ts",
            "LF:10",
            "LH:10",
            "BRF:4",
            "BRH:2",
            "FNF:3",
            "FNH:3",
            "end_of_record",
            "SF:b.ts",
            "LF:10",
            "LH:5",
            "BRF:2",
            "BRH:2",
            "FNF:1",
            "FNH:0",
            "end_of_record",
        ]
        .join("\n");
        let files = parse_lcov(&lcov);
        let (pass, msg) = evaluate_lcov_gate(&files, 60.0, true);
        assert!(pass);
        assert!(
            msg.starts_with("lines=75.00% branches=66.67% functions=75.00%"),
            "{msg}"
        );
    }

    #[test]
    fn lcov_gate_requires_the_named_crate() {
        let dir = std::env::temp_dir().join(format!("apex-lcov-crate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.lcov");
        std::fs::write(
            &path,
            "SF:/w/rust/crates/apex-lift/src/stats.rs\nLF:10\nLH:10\nend_of_record\n",
        )
        .unwrap();
        let file = path.to_str().unwrap();
        assert!(lcov_gate(file, 90.0, false, Some("apex-lift")).is_ok());
        let err = lcov_gate(file, 90.0, false, Some("apex-tools")).unwrap_err();
        assert!(err.contains("does not cover the named crate"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lcov_gate_fails_below_floor_and_lists_worst() {
        let lcov = [
            "SF:ok.ts",
            "LF:10",
            "LH:10",
            "FNF:1",
            "FNH:1",
            "end_of_record",
            "SF:bad.ts",
            "LF:10",
            "LH:1",
            "FNF:1",
            "FNH:1",
            "end_of_record",
        ]
        .join("\n");
        let files = parse_lcov(&lcov);
        let (pass, msg) = evaluate_lcov_gate(&files, 90.0, false);
        assert!(!pass);
        assert!(msg.contains("lines: 55.00% < 90%"), "{msg}");
        assert!(msg.contains("low: bad.ts 10.0%"), "{msg}");
    }

    #[test]
    fn lcov_gate_branch_rule_matches_js() {
        let branches_present =
            parse_lcov("SF:a.ts\nLF:10\nLH:10\nBRF:4\nBRH:1\nFNF:1\nFNH:1\nend_of_record");
        let (pass, msg) = evaluate_lcov_gate(&branches_present, 90.0, true);
        assert!(!pass);
        assert!(msg.contains("branches: 25.00% < 90%"), "{msg}");
        // Without branch records the branch comparison stays silent unless
        // the caller asked for branches, where the fail-closed violation fires.
        let no_branches = parse_lcov("SF:a.ts\nLF:10\nLH:10\nFNF:1\nFNH:1\nend_of_record");
        let (pass, msg) = evaluate_lcov_gate(&no_branches, 90.0, true);
        assert!(!pass);
        assert!(msg.contains("no branch data"), "{msg}");
        assert!(!msg.contains("branches: "), "{msg}");
    }

    #[test]
    fn lcov_all_zero_line_records_fail_closed() {
        let files = parse_lcov("SF:a.ts\nLF:0\nLH:0\nend_of_record\n");
        let totals = lcov_totals(&files, false);
        assert!(
            totals
                .violations
                .iter()
                .any(|v| v.contains("no executable lines")),
            "{:?}",
            totals.violations
        );
        let (pass, msg) = evaluate_lcov_gate(&files, 0.0, false);
        assert!(!pass, "{msg}");
    }
}
