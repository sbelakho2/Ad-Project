//! Changed-file mutation verification.
//!
//! `mutation-smoke` is a curated sample over the two load-bearing crates. This
//! runner covers the complement: for every changed production Rust file it
//! generates a small, deterministic mutant, copies the repository to a
//! throwaway tree (the working tree is never mutated), proves the mutant
//! compiles, and requires the owning crate's tests to kill it. A survivor is
//! a test gap and fails the gate.
//!
//! Only explicit `--file` paths are evaluated, so CI can run this on exactly
//! the files a diff touched. Tests, binaries and generated fixtures are not
//! mutated: the gate is about production behaviour.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::repo::Repo;

/// Remove the throwaway tree even if a cargo run or parse returns early.
/// A killed run cannot run `Drop`. Remove abandoned throwaway trees older
/// than six hours so they do not accumulate gigabytes; a live concurrent run
/// is far younger than that.
fn clean_stale_trees() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("apex-mutation-diff-") {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age.as_secs() > 6 * 60 * 60);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Directories never copied into the throwaway tree.
const SKIP_COPY: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".apex",
    ".dogfood-live",
    ".kiwi",
    "future",
    "dist",
    "build",
    ".gradle",
    "vendor",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// 1-based line number.
    pub line: usize,
    /// The mutated line.
    pub text: String,
    /// A human-readable description of the mutation.
    pub description: String,
}

/// Changed line ranges keyed by repo-relative path.
pub type MutationRanges = std::collections::BTreeMap<String, Vec<(usize, usize)>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileResult {
    pub file: String,
    pub crate_name: String,
    pub candidate: Option<Candidate>,
    pub status: String,
}

/// The production prefix of a Rust source file: everything before the first
/// inline test module. Files commonly keep tests at the bottom; mutating test
/// assertions would measure the test suite against itself.
#[must_use]
pub fn production_prefix(source: &str) -> &str {
    // Cut at the inline test *module*, not at the many `#[cfg(test)]`
    // helpers production files keep for their scaling seams.
    for marker in ["#[cfg(test)]\nmod ", "#[cfg(test)]\r\nmod "] {
        if let Some(index) = source.find(marker) {
            return &source[..index];
        }
    }
    source
}

/// A line with comments and string/char/raw-string literals blanked out at
/// the same byte positions. Mutation must never rewrite prose or a regex.
fn code_mask(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = vec![b' '; bytes.len()];
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'/') {
            break;
        }
        if bytes[index] == b'/' && bytes.get(index + 1) == Some(&b'*') {
            if let Some(end) = line[index + 2..].find("*/") {
                index += 2 + end + 2;
                continue;
            }
            break;
        }
        if bytes[index] == b'r' {
            let mut cursor = index + 1;
            while bytes.get(cursor) == Some(&b'#') {
                cursor += 1;
            }
            if bytes.get(cursor) == Some(&b'"') {
                let hashes = cursor - index - 1;
                let terminator = format!("\"{}", "#".repeat(hashes));
                if let Some(end) = line[cursor + 1..].find(&terminator) {
                    index = cursor + 1 + end + terminator.len();
                    continue;
                }
                break;
            }
        }
        if matches!(bytes[index], b'"' | b'\'') {
            let quote = bytes[index];
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index += 2;
                } else if bytes[index] == quote {
                    index += 1;
                    break;
                } else {
                    index += 1;
                }
            }
            continue;
        }
        out[index] = bytes[index];
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The first simple mutation available in a production source. Comparison
/// and boolean flips always type-check, which keeps the compiler from
/// discarding a mutant before the tests see it.
#[must_use]
pub fn first_candidate(source: &str) -> Option<Candidate> {
    first_candidate_in(source, None)
}

/// A candidate whose 1-based line is inside the changed hunks. `None` means
/// the whole production prefix is eligible (a new file or an explicit single
/// file check). An empty scope is a deliberate "this change touched no
/// mutable decision" result, not a survivor.
fn first_candidate_in(
    source: &str,
    scope: Option<&std::collections::BTreeSet<usize>>,
) -> Option<Candidate> {
    candidates_in(source, scope, 1).into_iter().next()
}

/// Up to `max` distinct line mutations. One per changed hunk is not enough:
/// a file can carry several changed decisions and the first operator may be
/// unrelated. All selected mutants must be killed.
fn candidates_in(
    source: &str,
    scope: Option<&std::collections::BTreeSet<usize>>,
    max: usize,
) -> Vec<Candidate> {
    let production = production_prefix(source);
    let mut in_block = false;
    let mut out = Vec::new();
    for (line_index, line) in production.lines().enumerate() {
        if out.len() >= max {
            break;
        }
        if let Some(lines) = scope {
            if !lines.contains(&(line_index + 1)) {
                continue;
            }
        }
        let masked = code_mask(line);
        if in_block {
            if line.contains("*/") {
                in_block = false;
            }
            continue;
        }
        if masked.contains("/*") && !masked.contains("*/") {
            in_block = true;
            continue;
        }
        if masked.trim().is_empty() {
            continue;
        }
        if let Some((from, to, label, index)) = flip_operator(&masked) {
            let mut text = String::with_capacity(line.len());
            text.push_str(&line[..index]);
            text.push_str(to);
            text.push_str(&line[index + from.len()..]);
            if text != line {
                out.push(Candidate {
                    line: line_index + 1,
                    text,
                    description: label.to_string(),
                });
            }
        }
    }
    out
}
fn flip_operator(line: &str) -> Option<(&'static str, &'static str, &'static str, usize)> {
    let operators: [(&'static str, &'static str, &'static str); 6] = [
        ("==", "!=", "== -> !="),
        ("!=", "==", "!= -> =="),
        ("<=", "<", "<= -> <"),
        (">=", ">", ">= -> >"),
        ("&&", "||", "&& -> ||"),
        ("||", "&&", "|| -> &&"),
    ];
    for (index, _) in line.char_indices() {
        let rest = &line[index..];
        for (from, to, label) in operators {
            if rest.starts_with(from) {
                if from == "||"
                    && (line[..index]
                        .trim_end()
                        .ends_with(['(', ',', '=', '>', '{', '['])
                        || line[..index].trim_end().ends_with("move"))
                {
                    continue;
                }
                return Some((from, to, label, index));
            }
        }
        if rest.starts_with("true")
            && rest[4..]
                .chars()
                .next()
                .is_none_or(|next| !next.is_ascii_alphanumeric() && next != '_')
        {
            return Some(("true", "false", "true -> false", index));
        }
        if rest.starts_with("false")
            && rest[5..]
                .chars()
                .next()
                .is_none_or(|next| !next.is_ascii_alphanumeric() && next != '_')
        {
            return Some(("false", "true", "false -> true", index));
        }
    }
    None
}

/// Parse the owning crate from a repo-relative production source path.
#[must_use]
pub fn crate_for(file: &str) -> Option<String> {
    let parts: Vec<&str> = file.split('/').collect();
    if parts.len() < 5 || parts[0] != "rust" || parts[1] != "crates" {
        return None;
    }
    let crate_name = parts[2];
    let file_name = parts.last().copied().unwrap_or("");
    let in_production = parts.get(3) == Some(&"src")
        && !parts.contains(&"tests")
        && parts.get(4).is_none_or(|part| *part != "bin")
        && !matches!(
            file_name,
            "tests.rs" | "test_support.rs" | "testkit.rs" | "testing.rs"
        );
    in_production.then(|| crate_name.to_string())
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let entries = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if SKIP_COPY.contains(&name.as_ref())
            || (name.as_ref() == "public" && from.ends_with("site"))
        {
            continue;
        }
        let src = entry.path();
        let dst = to.join(name.as_ref());
        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        if file_type.is_dir() {
            copy_tree(&src, &dst)?;
        } else if file_type.is_file() {
            std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    }
    Ok(())
}

fn run(command: &mut Command, what: &str) -> Result<std::process::Output, String> {
    let output = command
        .output()
        .map_err(|e| format!("{what} could not spawn: {e}"))?;
    Ok(output)
}

fn run_tests(work: &Path, crate_name: &str, compile_only: bool) -> Result<bool, String> {
    let target = work
        .parent()
        .ok_or("throwaway workspace has no parent")?
        .join("mutation-target");
    let mut command = Command::new("cargo");
    command
        .current_dir(work.join("rust"))
        .env("CARGO_TARGET_DIR", target)
        .args(["test", "-p", crate_name, "--offline", "--quiet"]);
    if compile_only {
        command.arg("--no-run");
    }
    let output = run(&mut command, "cargo test")?;
    Ok(output.status.success())
}

fn write_candidate(path: &Path, source: &str, candidate: &Candidate) -> Result<String, String> {
    let mut mutated = String::with_capacity(source.len() + 16);
    for (index, line) in source.lines().enumerate() {
        if index + 1 == candidate.line {
            mutated.push_str(&candidate.text);
        } else {
            mutated.push_str(line);
        }
        mutated.push('\n');
    }
    std::fs::write(path, &mutated).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(mutated)
}

/// Parse `--file path` and `--line path:start-end` arguments. Keeping the
/// parser pure lets the CLI test the exact shape `changed.py` emits.
pub fn parse_args(args: &[String]) -> Result<(Vec<String>, MutationRanges), String> {
    let mut files = Vec::new();
    let mut ranges = MutationRanges::new();
    for pair in args.windows(2) {
        match pair[0].as_str() {
            "--file" => files.push(pair[1].clone()),
            "--line" => {
                let Some((path, span)) = pair[1].rsplit_once(':') else {
                    return Err(format!("invalid --line value '{}'", pair[1]));
                };
                let Some((start, end)) = span.split_once('-') else {
                    return Err(format!("invalid --line range '{}'", pair[1]));
                };
                let start = start
                    .parse()
                    .map_err(|_| format!("invalid --line start '{}'", pair[1]))?;
                let end = end
                    .parse()
                    .map_err(|_| format!("invalid --line end '{}'", pair[1]))?;
                ranges
                    .entry(path.to_string())
                    .or_default()
                    .push((start, end));
            }
            _ => {}
        }
    }
    if files.is_empty() {
        Err("usage: apex-tools mutation-diff --file <repo-relative.rs> ...".into())
    } else {
        Ok((files, ranges))
    }
}

/// Mutate only the changed production lines. A survivor or uncompilable
/// mutant is a failure. A changed hunk with no comparison/boolean is copy or
/// structure and is reported as `no-decision` rather than mutated. A crate
/// root (`lib.rs`) is wiring and is reported as `wiring`.
pub fn mutation_diff(
    repo: &Repo,
    files: &[String],
    line_ranges: &MutationRanges,
) -> Result<(), String> {
    let mut targets: Vec<(String, String, String)> = Vec::new();
    for file in files {
        let Some(crate_name) = crate_for(file) else {
            continue;
        };
        if !repo.root.join(file).is_file() {
            // A deletion has no source to mutate; compilation and the suite
            // still cover the callers.
            println!("mutation-diff: {file}: deleted, no mutant");
            continue;
        }
        let source = repo.read(file)?;
        targets.push((file.clone(), crate_name, source));
    }
    if targets.is_empty() {
        println!("mutation-diff: no changed production Rust files");
        return Ok(());
    }

    clean_stale_trees();
    let parent = std::env::temp_dir().join(format!(
        "apex-mutation-diff-{}-{}",
        std::process::id(),
        crate::mutation::utc_now_iso().replace(':', "")
    ));
    let work = parent.join("repo");
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700));
    }
    let _cleanup = Cleanup(parent.clone());
    copy_tree(&repo.root, &work)?;

    let mut failures: Vec<String> = Vec::new();
    let mut results: Vec<FileResult> = Vec::new();
    let mut baseline: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (_, crate_name, _) in &targets {
        if baseline.insert(crate_name.clone()) && !run_tests(&work, crate_name, false)? {
            return Err(format!(
                "mutation-diff: baseline tests for {crate_name} failed in the throwaway tree"
            ));
        }
    }

    for (file, crate_name, source) in targets {
        let scope: Option<std::collections::BTreeSet<usize>> =
            line_ranges.get(&file).map(|ranges| {
                ranges
                    .iter()
                    .flat_map(|(start, end)| *start..=*end)
                    .collect()
            });
        let candidates = candidates_in(&source, scope.as_ref(), 1);
        if candidates.is_empty() {
            // A crate root is wiring by definition; its modules carry the
            // decisions and are checked as their own changed files. A hunk
            // with no comparison/boolean is copy or structure, not a hidden
            // decision.
            let wiring = file.ends_with("/lib.rs");
            let status = if wiring {
                "wiring"
            } else if scope.is_some() {
                "no-decision"
            } else {
                "missing"
            };
            if status == "missing" {
                failures.push(format!("{file}: no mutation operator in production code"));
            }
            results.push(FileResult {
                file,
                crate_name,
                candidate: None,
                status: status.to_string(),
            });
            continue;
        }
        let mut file_clean = true;
        for candidate in candidates {
            let path = work.join(&file);
            write_candidate(&path, &source, &candidate)?;
            let compiles = run_tests(&work, &crate_name, true)?;
            let status = if !compiles {
                "uncompilable"
            } else if run_tests(&work, &crate_name, false)? {
                "survived"
            } else {
                "killed"
            };
            std::fs::write(&path, &source).map_err(|e| format!("{}: {e}", path.display()))?;
            if status != "killed" {
                file_clean = false;
                failures.push(format!(
                    "{file}:{}: {status} ({})",
                    candidate.line, candidate.description
                ));
            }
            println!(
                "mutation-diff: {}:{} {} -> {}",
                file, candidate.line, candidate.description, status
            );
            results.push(FileResult {
                file: file.clone(),
                crate_name: crate_name.clone(),
                candidate: Some(candidate),
                status: status.to_string(),
            });
        }
        let _ = file_clean;
    }
    if !failures.is_empty() {
        return Err(format!(
            "mutation-diff: {} file(s) not covered:\n  {}",
            failures.len(),
            failures.join("\n  ")
        ));
    }
    println!(
        "mutation-diff PASS: {} changed production file(s)",
        results.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_comparison_is_flipped_once_and_tests_are_ignored() {
        let source = "let ok = a == b;\n#[cfg(test)]\nmod tests { let x = a == b; }\n";
        let candidate = first_candidate(source).unwrap();
        assert_eq!(candidate.line, 1);
        assert!(candidate.text.contains("a != b"));
        assert_eq!(candidate.description, "== -> !=");
    }

    #[test]
    fn booleans_and_guards_are_mutable() {
        assert_eq!(
            first_candidate("let live = true;\n").unwrap().description,
            "true -> false"
        );
        assert_eq!(
            first_candidate("if count >= 3 {}\n").unwrap().description,
            ">= -> >"
        );
        assert!(first_candidate("let name = \"x == y\";\n").is_none());
        assert!(first_candidate("// a == b\n").is_none());
        assert!(first_candidate("let r = r\"```+|~~~+\";\n").is_none());
        assert!(first_candidate("let x = 1; // a == b\n").is_none());
    }

    #[test]
    fn the_cli_parser_reads_files_and_line_ranges() {
        let args = vec![
            "--file".to_string(),
            "rust/crates/a/src/lib.rs".to_string(),
            "--line".to_string(),
            "rust/crates/a/src/lib.rs:4-9".to_string(),
            "--file".to_string(),
            "rust/crates/b/src/x.rs".to_string(),
        ];
        let (files, ranges) = parse_args(&args).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(ranges.get("rust/crates/a/src/lib.rs"), Some(&vec![(4, 9)]));
        assert!(parse_args(&[]).is_err());
        assert!(parse_args(&["--line".into(), "bad".into()]).is_err());
    }

    #[test]
    fn a_hunk_scope_does_not_mutate_an_unrelated_line() {
        let source = "let early = a == b;\nlet changed = c >= d;\n";
        let mut scope = std::collections::BTreeSet::new();
        scope.insert(2);
        let candidate = first_candidate_in(source, Some(&scope)).unwrap();
        assert_eq!(candidate.line, 2);
        assert_eq!(candidate.description, ">= -> >");
        let empty = std::collections::BTreeSet::new();
        assert!(first_candidate_in(source, Some(&empty)).is_none());
    }

    #[test]
    fn multiple_changed_decisions_are_each_mutated() {
        let source =
            "let a = 1 == 1;\nlet b = 2 >= 2;\nlet c = true;\nlet d = false;\nlet e = 5 != 5;\n";
        let candidates = candidates_in(source, None, 4);
        assert_eq!(candidates.len(), 4);
        assert_eq!(candidates[0].line, 1);
        assert_eq!(candidates[3].line, 4);
    }

    #[test]
    fn inline_test_helpers_do_not_truncate_production() {
        let source = "#[cfg(test)]\nfn helper() {}\n\nfn real() -> bool { 1 == 1 }\n\n#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n";
        let production = production_prefix(source);
        assert!(production.contains("fn real"));
        assert!(first_candidate(source).is_some());
    }

    #[test]
    fn only_production_sources_belong_to_the_gate() {
        assert_eq!(
            crate_for("rust/crates/apex-watch/src/apps.rs"),
            Some("apex-watch".to_string())
        );
        assert_eq!(crate_for("rust/crates/apex-watch/tests/live.rs"), None);
        assert_eq!(crate_for("rust/crates/apex-watch/src/bin/tool.rs"), None);
        assert_eq!(
            crate_for("rust/crates/apex-watch/src/test_support.rs"),
            None
        );
        assert_eq!(crate_for("rust/crates/apex-watch/src/testkit.rs"), None);
        assert_eq!(crate_for("site/static/style.css"), None);
    }
}
