//! Sample-based mutation smoke for the load-bearing logic, replacing the
//! retired rust-mutation-smoke.sh: hand-picked mutants (apex-core money and
//! validation, apex-lift decision, statistics, billing and windows) are applied to a throwaway
//! copy of the crate (the repo is never mutated), the crate's own tests must
//! kill each one, and the kill rate is written to mutation-results.json for
//! mutation-floor to enforce.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::repo::Repo;

/// The generated minimal workspace, byte-identical to the retired shell.
pub const WORKSPACE_TOML: &str = r#"[workspace]
resolver = "2"
members = ["crates/@STAR@"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "proprietary"
publish = false

[workspace.dependencies]
apex-core = { path = "crates/apex-core" }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "2.0"
sha2 = "0.10"

[workspace.lints.rust]
unsafe_code = "forbid"
missing_debug_implementations = "deny"

[workspace.lints.clippy]
all = "deny"
"#;

pub const RESULTS_COMMENT: &str = "Sample-based mutation smoke for the load-bearing logic: apex-core (money, validation) and apex-lift (decision, stats, billing, window). Phase 3: the mutant set and the floor ratchet upward together; a survived mutant must be investigated, not silently dropped from the sample.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MutantFile {
    pub crate_name: &'static str,
    pub file_name: &'static str,
}

pub const CORE_MONEY: MutantFile = MutantFile {
    crate_name: "apex-core",
    file_name: "money.rs",
};
pub const CORE_VALIDATION: MutantFile = MutantFile {
    crate_name: "apex-core",
    file_name: "validation.rs",
};
pub const LIFT_DECISION: MutantFile = MutantFile {
    crate_name: "apex-lift",
    file_name: "decision.rs",
};
pub const LIFT_STATS: MutantFile = MutantFile {
    crate_name: "apex-lift",
    file_name: "stats.rs",
};
pub const LIFT_BILLING: MutantFile = MutantFile {
    crate_name: "apex-lift",
    file_name: "billing.rs",
};
pub const LIFT_RUN: MutantFile = MutantFile {
    crate_name: "apex-lift",
    file_name: "run.rs",
};
pub const LIFT_WINDOW: MutantFile = MutantFile {
    crate_name: "apex-lift",
    file_name: "window.rs",
};

#[derive(Debug, Clone, Copy)]
pub struct Mutant {
    pub file: MutantFile,
    pub find: &'static str,
    pub replace: &'static str,
    pub description: &'static str,
}

pub const MUTANTS: &[Mutant] = &[
    Mutant {
        file: CORE_MONEY,
        find: "MIN_LIFT_RATE_BP: u16 = 1_500",
        replace: "MIN_LIFT_RATE_BP: u16 = 1_400",
        description: "min lift rate 1500 -> 1400",
    },
    Mutant {
        file: CORE_MONEY,
        find: "MAX_LIFT_RATE_BP: u16 = 2_000",
        replace: "MAX_LIFT_RATE_BP: u16 = 2_100",
        description: "max lift rate 2000 -> 2100",
    },
    Mutant {
        file: CORE_MONEY,
        find: "checked_mul(10_000)",
        replace: "checked_mul(100)",
        description: "lift basis-points scale 10_000 -> 100",
    },
    Mutant {
        file: CORE_MONEY,
        find: "if impressions == 0 {",
        replace: "if impressions == 1 {",
        description: "zero-impressions guard == 0 -> == 1",
    },
    Mutant {
        file: CORE_VALIDATION,
        find: "!(1..=MAX_ECPM_MICROS).contains(&micros)",
        replace: "!(0..=MAX_ECPM_MICROS).contains(&micros)",
        description: "ecpm lower bound 1 -> 0",
    },
    Mutant {
        file: CORE_VALIDATION,
        find: "trimmed.len() == 2",
        replace: "trimmed.len() != 2",
        description: "country shape check inverted",
    },
    Mutant {
        file: LIFT_DECISION,
        find: "lambda: current.lower_90.max(0.0),",
        replace: "lambda: current.upper_90.max(0.0),",
        description: "bill the upper bound instead of the lower bound",
    },
    Mutant {
        file: LIFT_STATS,
        find: "(iterations as f64 - 1.0) * 0.10",
        replace: "(iterations as f64 - 1.0) * 0.50",
        description: "bootstrap 10th percentile -> median",
    },
    Mutant {
        file: LIFT_RUN,
        find: "range.last.saturating_add(1).min(month_days + 1)",
        replace: "range.last.min(month_days + 1)",
        description: "interval end boundary off by one",
    },
    Mutant {
        file: LIFT_WINDOW,
        find: "if index < burn_in_days {",
        replace: "if index < burn_in_days + 1 {",
        description: "burn-in boundary drops one analysis day",
    },
];

/// Apply one mutant with an in-memory replacement; the pattern must be found.
pub fn apply_mutant(source: &str, find: &str, replace: &str) -> Result<String, String> {
    if !source.contains(find) {
        return Err(format!("mutant pattern not found: {find}"));
    }
    Ok(source.replacen(find, replace, 1))
}

/// UTC ISO-8601 seconds precision, no chrono dependency.
pub fn utc_now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant's civil-from-days algorithm.
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn unique_workdir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "apex-mutation-smoke-{}-{nanos}",
        std::process::id()
    ))
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", crate::util::slash(to)))?;
    let entries =
        std::fs::read_dir(from).map_err(|e| format!("{}: {e}", crate::util::slash(from)))?;
    for entry in entries.flatten() {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        let ft = entry.file_type().map_err(|e| e.to_string())?;
        if ft.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", crate::util::slash(&src)))?;
        }
    }
    Ok(())
}

fn run_tests(workdir: &Path, crate_name: &str) -> Result<bool, String> {
    let output = Command::new("cargo")
        .args(["test", "-p", crate_name, "--offline", "--quiet"])
        .current_dir(workdir)
        .output()
        .map_err(|e| format!("cargo test failed to spawn: {e}"))?;
    Ok(output.status.success())
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{}\"", value.replace('"', "")))
}

pub fn mutation_smoke(repo: &Repo) -> Result<(), String> {
    let core_src = repo.path("rust/crates/apex-core");
    let lift_src = repo.path("rust/crates/apex-lift");
    if !core_src.is_dir() || !lift_src.is_dir() {
        return Err("mutation-smoke: rust/crates/apex-core or apex-lift not found".into());
    }
    let work = unique_workdir();
    let crates_dir = work.join("crates");
    std::fs::create_dir_all(&crates_dir).map_err(|e| e.to_string())?;
    copy_dir(&core_src, &crates_dir.join("apex-core"))?;
    copy_dir(&lift_src, &crates_dir.join("apex-lift"))?;
    let workspace = WORKSPACE_TOML.replace("@STAR@", "*");
    std::fs::write(work.join("Cargo.toml"), workspace).map_err(|e| e.to_string())?;

    // Pristine contents of every mutable file.
    let mut originals: std::collections::BTreeMap<(&'static str, &'static str), String> =
        std::collections::BTreeMap::new();
    for mutant in MUTANTS {
        let key = (mutant.file.crate_name, mutant.file.file_name);
        if originals.contains_key(&key) {
            continue;
        }
        let path = crates_dir
            .join(mutant.file.crate_name)
            .join("src")
            .join(mutant.file.file_name);
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("{}/{}: {e}", mutant.file.crate_name, mutant.file.file_name))?;
        originals.insert(key, content);
    }

    let restore = |file: &MutantFile| -> Result<(), String> {
        let content = originals
            .get(&(file.crate_name, file.file_name))
            .ok_or_else(|| format!("no original for {}/{}", file.crate_name, file.file_name))?;
        let path = crates_dir
            .join(file.crate_name)
            .join("src")
            .join(file.file_name);
        std::fs::write(&path, content).map_err(|e| format!("{}: {e}", file.file_name))
    };

    if !run_tests(&work, "apex-core")? || !run_tests(&work, "apex-lift")? {
        let _ = std::fs::remove_dir_all(&work);
        return Err("mutation-smoke: baseline test run failed; aborting".into());
    }
    println!("mutation-smoke: baseline green");

    let mut killed = 0usize;
    let mut total = 0usize;
    let mut results: Vec<(String, &'static str)> = Vec::new();
    for mutant in MUTANTS {
        let pristine = originals
            .get(&(mutant.file.crate_name, mutant.file.file_name))
            .ok_or("missing original")?;
        let mutated = match apply_mutant(pristine, mutant.find, mutant.replace) {
            Ok(m) => m,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&work);
                return Err(format!(
                    "mutation-smoke: {e} in {}/{}",
                    mutant.file.crate_name, mutant.file.file_name
                ));
            }
        };
        restore(&mutant.file)?;
        let path = crates_dir
            .join(mutant.file.crate_name)
            .join("src")
            .join(mutant.file.file_name);
        std::fs::write(&path, mutated).map_err(|e| format!("{}: {e}", mutant.file.file_name))?;
        total += 1;
        let status = if run_tests(&work, mutant.file.crate_name)? {
            eprintln!("SURVIVED: {}", mutant.description);
            "survived"
        } else {
            killed += 1;
            "killed"
        };
        results.push((mutant.description.to_string(), status));
        restore(&mutant.file)?;
    }
    for mutant in MUTANTS {
        restore(&mutant.file)?;
    }
    let _ = std::fs::remove_dir_all(&work);

    let score = (killed * 100).checked_div(total).unwrap_or(0);
    let mut mutant_json = String::new();
    for (i, (description, status)) in results.iter().enumerate() {
        if i > 0 {
            mutant_json.push_str(",\n");
        }
        mutant_json.push_str(&format!(
            "    {{ \"description\": {}, \"status\": {} }}",
            json_string(description),
            json_string(status)
        ));
    }
    let json = format!(
        "{{\n  \"__comment\": {},\n  \"target\": {},\n  \"mutants\": [\n{}\n  ],\n  \"killed\": {killed},\n  \"total\": {total},\n  \"killRatePercent\": {score}\n}}\n",
        json_string(RESULTS_COMMENT),
        json_string("apex-core and apex-lift: money, validation, decision, stats, billing, window"),
        mutant_json
    );
    let out = repo.path("tools/coverage/mutation-results.json");
    std::fs::write(&out, json).map_err(|e| format!("{}: {e}", crate::util::slash(&out)))?;

    println!(
        "mutation-smoke: killed {killed}/{total} ({score}%) -> tools/coverage/mutation-results.json"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutant_application_replaces_exactly_once_and_reports_missing_patterns() {
        let fixture = "let a = MIN_LIFT_RATE_BP: u16 = 1_500;\nlet b = 1;\n";
        let mutated = apply_mutant(
            fixture,
            "MIN_LIFT_RATE_BP: u16 = 1_500",
            "MIN_LIFT_RATE_BP: u16 = 1_400",
        )
        .unwrap();
        assert!(mutated.contains("1_400"));
        assert!(!mutated.contains("1_500"));
        let err = apply_mutant(fixture, "not present anywhere", "x").unwrap_err();
        assert!(err.contains("mutant pattern not found"));
    }

    #[test]
    fn all_mutants_apply_to_the_real_sources() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates");
        let mut cache: std::collections::BTreeMap<(&str, &str), String> =
            std::collections::BTreeMap::new();
        for mutant in MUTANTS {
            let key = (mutant.file.crate_name, mutant.file.file_name);
            let source = cache
                .entry(key)
                .or_insert_with(|| {
                    std::fs::read_to_string(crates.join(key.0).join("src").join(key.1)).unwrap()
                })
                .clone();
            let mutated = apply_mutant(&source, mutant.find, mutant.replace).unwrap();
            assert_ne!(
                &mutated, &source,
                "mutant must change the source: {}",
                mutant.description
            );
            assert!(mutated.contains(mutant.replace));
        }
    }

    #[test]
    fn utc_now_iso_has_iso_shape() {
        let stamp = utc_now_iso();
        assert_eq!(stamp.len(), 20, "{stamp}");
        assert!(stamp.ends_with('Z'));
        assert_eq!(&stamp[4..5], "-");
        assert_eq!(&stamp[10..11], "T");
    }
}
