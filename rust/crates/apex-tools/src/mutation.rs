//! Sample-based mutation smoke for apex-core, replacing the retired
//! rust-mutation-smoke.sh: six hand-picked mutants are applied to a throwaway
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
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "2.0"

[workspace.lints.rust]
unsafe_code = "forbid"
missing_debug_implementations = "deny"

[workspace.lints.clippy]
all = "deny"
"#;

pub const RESULTS_COMMENT: &str = "Sample-based mutation smoke for apex-core (money.rs + validation.rs). Phase 3: the mutant set and the floor ratchet upward together; a survived mutant must be investigated, not silently dropped from the sample.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutantFile {
    Money,
    Validation,
}

impl MutantFile {
    pub fn name(self) -> &'static str {
        match self {
            MutantFile::Money => "money.rs",
            MutantFile::Validation => "validation.rs",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Mutant {
    pub file: MutantFile,
    pub find: &'static str,
    pub replace: &'static str,
    pub description: &'static str,
}

pub const MUTANTS: &[Mutant] = &[
    Mutant {
        file: MutantFile::Money,
        find: "MIN_LIFT_RATE_BP: u16 = 1_500",
        replace: "MIN_LIFT_RATE_BP: u16 = 1_400",
        description: "min lift rate 1500 -> 1400",
    },
    Mutant {
        file: MutantFile::Money,
        find: "MAX_LIFT_RATE_BP: u16 = 2_000",
        replace: "MAX_LIFT_RATE_BP: u16 = 2_100",
        description: "max lift rate 2000 -> 2100",
    },
    Mutant {
        file: MutantFile::Money,
        find: "checked_mul(10_000)",
        replace: "checked_mul(100)",
        description: "lift basis-points scale 10_000 -> 100",
    },
    Mutant {
        file: MutantFile::Money,
        find: "if impressions == 0 {",
        replace: "if impressions == 1 {",
        description: "zero-impressions guard == 0 -> == 1",
    },
    Mutant {
        file: MutantFile::Validation,
        find: "!(1..=MAX_ECPM_MICROS).contains(&micros)",
        replace: "!(0..=MAX_ECPM_MICROS).contains(&micros)",
        description: "ecpm lower bound 1 -> 0",
    },
    Mutant {
        file: MutantFile::Validation,
        find: "trimmed.len() == 2",
        replace: "trimmed.len() != 2",
        description: "country shape check inverted",
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

fn run_tests(workdir: &Path) -> Result<bool, String> {
    let output = Command::new("cargo")
        .args(["test", "-p", "apex-core", "--offline", "--quiet"])
        .current_dir(workdir)
        .output()
        .map_err(|e| format!("cargo test failed to spawn: {e}"))?;
    Ok(output.status.success())
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| format!("\"{}\"", value.replace('"', "")))
}

pub fn mutation_smoke(repo: &Repo) -> Result<(), String> {
    let crate_src = repo.path("rust/crates/apex-core");
    if !crate_src.is_dir() {
        return Err("mutation-smoke: rust/crates/apex-core not found".into());
    }
    let work = unique_workdir();
    let crates_dir = work.join("crates");
    std::fs::create_dir_all(&crates_dir).map_err(|e| e.to_string())?;
    copy_dir(&crate_src, &crates_dir.join("apex-core"))?;
    let workspace = WORKSPACE_TOML.replace("@STAR@", "*");
    std::fs::write(work.join("Cargo.toml"), workspace).map_err(|e| e.to_string())?;

    let src_dir = crates_dir.join("apex-core/src");
    let original_money =
        std::fs::read_to_string(src_dir.join("money.rs")).map_err(|e| format!("money.rs: {e}"))?;
    let original_validation = std::fs::read_to_string(src_dir.join("validation.rs"))
        .map_err(|e| format!("validation.rs: {e}"))?;

    let restore = |file: MutantFile| -> Result<(), String> {
        let (name, content) = match file {
            MutantFile::Money => ("money.rs", original_money.as_str()),
            MutantFile::Validation => ("validation.rs", original_validation.as_str()),
        };
        std::fs::write(src_dir.join(name), content).map_err(|e| format!("{name}: {e}"))
    };

    if !run_tests(&work)? {
        let _ = std::fs::remove_dir_all(&work);
        return Err("mutation-smoke: baseline test run failed; aborting".into());
    }
    println!("mutation-smoke: baseline green");

    let mut killed = 0usize;
    let mut total = 0usize;
    let mut results: Vec<(String, &'static str)> = Vec::new();
    for mutant in MUTANTS {
        let pristine = match mutant.file {
            MutantFile::Money => original_money.as_str(),
            MutantFile::Validation => original_validation.as_str(),
        };
        let mutated = match apply_mutant(pristine, mutant.find, mutant.replace) {
            Ok(m) => m,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&work);
                return Err(format!("mutation-smoke: {e} in {}", mutant.file.name()));
            }
        };
        restore(mutant.file)?;
        std::fs::write(src_dir.join(mutant.file.name()), mutated)
            .map_err(|e| format!("{}: {e}", mutant.file.name()))?;
        total += 1;
        let status = if run_tests(&work)? {
            eprintln!("SURVIVED: {}", mutant.description);
            "survived"
        } else {
            killed += 1;
            "killed"
        };
        results.push((mutant.description.to_string(), status));
        restore(mutant.file)?;
    }
    restore(MutantFile::Money)?;
    restore(MutantFile::Validation)?;
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
        "{{\n  \"__comment\": {},\n  \"generatedAt\": {},\n  \"target\": {},\n  \"mutants\": [\n{}\n  ],\n  \"killed\": {killed},\n  \"total\": {total},\n  \"killRatePercent\": {score}\n}}\n",
        json_string(RESULTS_COMMENT),
        json_string(&utc_now_iso()),
        json_string("apex-core: money.rs, validation.rs"),
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
        let base =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/apex-core/src");
        let money = std::fs::read_to_string(base.join("money.rs")).unwrap();
        let validation = std::fs::read_to_string(base.join("validation.rs")).unwrap();
        for mutant in MUTANTS {
            let source = match mutant.file {
                MutantFile::Money => &money,
                MutantFile::Validation => &validation,
            };
            let mutated = apply_mutant(source, mutant.find, mutant.replace).unwrap();
            assert_ne!(
                &mutated, source,
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
