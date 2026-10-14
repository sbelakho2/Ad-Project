//! Coverage artifact parsers and gates, ported from coverage-gate.mjs and
//! lcov-gate.mjs (fail-closed semantics preserved).

use std::path::Path;

use crate::policy::Floors;
use crate::repo::Repo;
use crate::util::re;

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

pub fn parse_jest_summary(json: &str) -> Result<Metrics, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let t = v
        .get("total")
        .ok_or("coverage-summary.json missing total")?;
    let pct = |key: &str| {
        t.get(key)
            .and_then(|x| x.get("pct"))
            .and_then(serde_json::Value::as_f64)
    };
    Ok(Metrics {
        lines: pct("lines").ok_or("missing lines")?,
        branches: pct("branches").ok_or("missing branches")?,
        functions: pct("functions").ok_or("missing functions")?,
        statements: pct("statements").ok_or("missing statements")?,
    })
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

#[derive(Debug, Clone)]
pub struct GoRow {
    pub func: String,
    pub pct: f64,
}

pub fn parse_go_func(text: &str) -> (Vec<GoRow>, Option<f64>) {
    let mut rows = Vec::new();
    let mut total = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("total:") {
            if let Some(p) = rest
                .rsplit('\t')
                .next()
                .and_then(|s| s.trim_end_matches('%').parse::<f64>().ok())
            {
                total = Some(p);
            }
            continue;
        }
        let mut parts = line.rsplitn(3, '\t');
        if let (Some(pct_s), Some(name), Some(_loc)) = (parts.next(), parts.next(), parts.next()) {
            if let Ok(pct) = pct_s.trim_end_matches('%').trim().parse::<f64>() {
                rows.push(GoRow {
                    func: name.to_string(),
                    pct,
                });
            }
        }
    }
    (rows, total)
}

/// Standalone lcov gate: same rules as the retired lcov-gate.mjs, including
/// the worst-files listing and the pass summary line format.
pub fn lcov_gate(path: &str, floor: f64, require_branches: bool) -> Result<(), String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("lcov not found: {path} ({e})"))?;
    let files = parse_lcov(&text);
    if files.is_empty() {
        return Err("lcov contains no records".into());
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

/// Standalone Go function-coverage gate: every row that ends in `<d>.<d>%`
/// (including the total line) must be at or above the floor.
pub fn go_func_gate(path: &str, floor: f64) -> Result<(), String> {
    if !Path::new(path).is_file() {
        return Err(format!("go coverage file not found: {path}"));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let row_re = re(r"([0-9]+\.[0-9]+)%\s*$")?;
    let mut offenders: Vec<String> = Vec::new();
    let mut rows = 0usize;
    for line in text.lines() {
        let Some(caps) = row_re.captures(line) else {
            continue;
        };
        rows += 1;
        let Ok(pct) = caps[1].parse::<f64>() else {
            continue;
        };
        if pct < floor {
            offenders.push(format!("  - {line}"));
        }
    }
    if !offenders.is_empty() {
        return Err(format!(
            "GO COVERAGE GATE FAILED (floor={floor}%) — offenders:\n{}",
            offenders.join("\n")
        ));
    }
    println!("go coverage gate PASS (floor={floor}%): {rows} rows checked incl. total");
    Ok(())
}

pub fn coverage_gate(
    repo: &Repo,
    domain: &str,
    artifact: &str,
    tool: &str,
    require_branches: bool,
) -> Result<(), String> {
    let floors = Floors::load(repo)?;
    if !floors.domains.contains_key(domain) {
        return Err(format!(
            "unknown domain '{domain}' (not present in coverage-floor.json)"
        ));
    }
    let floor = floors.floor_for(domain);
    let text = repo
        .read(artifact)
        .map_err(|e| format!("artifact not found: {artifact}: {e}"))?;
    let mut violations = Vec::new();
    match tool {
        "jest" | "c8" => {
            let m = parse_jest_summary(&text)?;
            for (name, value) in [
                ("lines", m.lines),
                ("branches", m.branches),
                ("functions", m.functions),
                ("statements", m.statements),
            ] {
                if value < floor {
                    violations.push(format!("{name}: {value}% < floor {floor}%"));
                }
            }
        }
        "lcov" => {
            let files = parse_lcov(&text);
            if files.is_empty() {
                return Err("lcov contains no records".into());
            }
            let t = lcov_totals(&files, require_branches);
            violations.extend(t.violations.iter().cloned());
            for (name, value) in [
                ("lines", t.metrics.lines),
                ("branches", t.metrics.branches),
                ("functions", t.metrics.functions),
            ] {
                if value < floor {
                    violations.push(format!("{name}: {value:.2}% < floor {floor}%"));
                }
            }
            if floor > 0.0 && !t.zero_line_files.is_empty() {
                violations.push(format!(
                    "files with zero executable lines in record: {}",
                    t.zero_line_files.join(", ")
                ));
            }
        }
        "go-func" => {
            let (rows, total) = parse_go_func(&text);
            let total = total.ok_or("cover.func has no total line")?;
            if total < floor {
                violations.push(format!("total: {total}% < floor {floor}%"));
            }
            for r in rows.iter().filter(|r| r.pct < floor) {
                violations.push(format!("{}: {}%", r.func, r.pct));
            }
        }
        other => return Err(format!("unknown tool '{other}' (use jest|c8|lcov|go-func)")),
    }
    if violations.is_empty() {
        println!("gate PASS [{domain}] floor={floor} ({tool})");
        Ok(())
    } else {
        Err(format!(
            "COVERAGE GATE FAILED [{domain}] {}:\n  - {}",
            tool,
            violations.join("\n  - ")
        ))
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
    fn jest_summary_reads_totals() {
        let m = parse_jest_summary(
            r#"{"total":{"lines":{"pct":100},"branches":{"pct":99.1},"functions":{"pct":100},"statements":{"pct":100}}}"#,
        )
        .unwrap();
        assert_eq!(m.lines, 100.0);
        assert_eq!(m.branches, 99.1);
    }

    #[test]
    fn lcov_totals_and_fail_closed_branches() {
        let files = parse_lcov("SF:a.ts\nLF:10\nLH:10\nFNF:1\nFNH:1\nend_of_record\n");
        assert_eq!(files.len(), 1);
        let t = lcov_totals(&files, true);
        assert!(!t.violations.is_empty());
        assert_eq!(lcov_totals(&files, false).metrics.lines, 100.0);
    }

    #[test]
    fn go_func_parses_tabs() {
        let (rows, total) =
            parse_go_func("github.com/x/a.go:10:\tDo\t85.7%\ntotal:\t(statements)\t92.3%");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].pct, 85.7);
        assert_eq!(total, Some(92.3));
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
    fn go_func_gate_counts_rows_and_total() {
        let dir = std::env::temp_dir().join(format!("apex-gofunc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cover.func");
        std::fs::write(
            &path,
            "github.com/x/a.go:10:\tDo\t85.7%\ntotal:\t(statements)\t92.3%\n",
        )
        .unwrap();
        let ok = go_func_gate(path.to_str().unwrap(), 85.0);
        assert!(ok.is_ok());
        let err = go_func_gate(path.to_str().unwrap(), 90.0).unwrap_err();
        assert!(
            err.contains("GO COVERAGE GATE FAILED (floor=90%) — offenders:"),
            "{err}"
        );
        assert!(err.contains("85.7%"), "{err}");
        assert!(
            !err.contains("92.3%"),
            "a row at or above the floor is not an offender: {err}"
        );
        let missing = go_func_gate(path.join("nope").to_str().unwrap(), 1.0).unwrap_err();
        assert!(missing.contains("go coverage file not found"), "{missing}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn go_func_gate_row_regex_requires_trailing_percent() {
        let dir = std::env::temp_dir().join(format!("apex-gofunc2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cover.func");
        std::fs::write(&path, "total:\t(statements)\t100%\nnot a row 62.5%.\n").unwrap();
        let ok = go_func_gate(path.to_str().unwrap(), 100.0);
        assert!(
            ok.is_ok(),
            "integer-only percents and trailing dots are not rows"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
