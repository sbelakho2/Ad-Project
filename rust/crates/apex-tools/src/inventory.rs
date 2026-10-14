//! Pipeline inventory parity: every `apex-tools` invocation in the Woodpecker
//! lanes and the Makefile must name a subcommand the CLI actually accepts.
//! This pins the drift between the hand-maintained command lists and the
//! binary. Only shell command positions count: prose mentions ("the apex-tools
//! binary"), step names and comments are ignored.

use crate::repo::Repo;
use crate::util::re;

/// Subcommands referenced by actual `apex-tools` invocations: `cargo run … -p
/// apex-tools -- <sub>` and direct `.../apex-tools <sub>` calls.
pub fn referenced_subcommands(text: &str) -> Result<Vec<String>, String> {
    let invocation = re(r"^\s*(?:--\s+)?([a-z][a-z0-9-]*)")?;
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        // Comments in the pipeline files and the Makefile are prose.
        if trimmed.starts_with('#') {
            continue;
        }
        for (idx, _) in line.match_indices("apex-tools") {
            let before = &line[..idx];
            let after = &line[idx + "apex-tools".len()..];
            let command_position = before.trim().is_empty()
                || before.ends_with("&& ")
                || before.ends_with("; ")
                || before.contains("cargo run")
                || before.ends_with("target/debug/")
                || before.ends_with("target/release/");
            let boundary = after
                .chars()
                .next()
                .map(|c| !c.is_ascii_alphanumeric() && c != '-')
                .unwrap_or(false);
            if !command_position || !boundary {
                continue;
            }
            if let Some(c) = invocation.captures(after) {
                out.push(c[1].to_string());
            }
        }
    }
    Ok(out)
}

pub fn check_pipeline_inventory(repo: &Repo) -> Result<(), String> {
    let mut unknown: Vec<String> = Vec::new();
    let mut checked = 0usize;
    let mut sources: Vec<String> = repo
        .walk()
        .into_iter()
        .filter(|p| p.starts_with(".woodpecker/") && p.ends_with(".yml"))
        .collect();
    sources.push("Makefile".to_string());
    for rel in &sources {
        let Ok(text) = repo.read(rel) else { continue };
        for sub in referenced_subcommands(&text)? {
            checked += 1;
            if !crate::SUBCOMMANDS.contains(&sub.as_str()) {
                unknown.push(format!("{rel}: references unknown subcommand '{sub}'"));
            }
        }
    }
    if !unknown.is_empty() {
        return Err(format!(
            "PIPELINE INVENTORY FAILED: lanes or Makefile reference subcommands that do not exist:\n  - {}",
            unknown.join("\n  - ")
        ));
    }
    // The reverse direction matters too: a subcommand with no caller in any
    // lane or the Makefile is dead scaffolding and must be removed or wired.
    let referenced: std::collections::BTreeSet<String> = {
        let mut set = std::collections::BTreeSet::new();
        for rel in &sources {
            if let Ok(text) = repo.read(rel) {
                for sub in referenced_subcommands(&text)? {
                    set.insert(sub);
                }
            }
        }
        set
    };
    let dead: Vec<&str> = crate::SUBCOMMANDS
        .iter()
        .copied()
        .filter(|sub| !referenced.contains(*sub))
        .collect();
    if !dead.is_empty() {
        return Err(format!(
            "PIPELINE INVENTORY FAILED: subcommands with no lane or Makefile caller: {}",
            dead.join(", ")
        ));
    }
    // The coverage lane's matrix must name every workspace crate, so a new
    // crate cannot exist without its gate and a deleted crate cannot linger.
    let lane = repo
        .read(".woodpecker/apex-rs-coverage.yml")
        .unwrap_or_default();
    let matrix = re(r"CRATE:\s*([a-z0-9-]+)")?;
    let in_matrix: std::collections::BTreeSet<String> = matrix
        .captures_iter(&lane)
        .map(|c| c[1].to_string())
        .collect();
    let workspace: std::collections::BTreeSet<String> = repo
        .walk()
        .into_iter()
        .filter(|p| {
            p.starts_with("rust/crates/")
                && p.ends_with("/Cargo.toml")
                && p.matches('/').count() == 3
        })
        .filter_map(|p| p.split('/').nth(2).map(str::to_string))
        .collect();
    let missing: Vec<String> = workspace.difference(&in_matrix).cloned().collect();
    if !missing.is_empty() {
        return Err(format!(
            "PIPELINE INVENTORY FAILED: workspace crates missing from the coverage lane matrix: {}",
            missing.join(", ")
        ));
    }
    let stale: Vec<String> = in_matrix.difference(&workspace).cloned().collect();
    if !stale.is_empty() {
        return Err(format!(
            "PIPELINE INVENTORY FAILED: lane matrix names crates that do not exist: {}",
            stale.join(", ")
        ));
    }
    println!(
        "pipeline inventory PASS: {checked} apex-tools invocations across lanes and Makefile; all {} subcommands referenced; {} workspace crates in the coverage matrix",
        crate::SUBCOMMANDS.len(),
        workspace.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_both_invocation_forms_and_ignores_prose() {
        let text = concat!(
            "cd rust && cargo run --release -q -p apex-tools -- docs-lint --source\n",
            "rust/target/release/apex-tools lcov-gate x.lcov 90\n",
            "# a comment about the apex-tools binary\n",
            "# subcommand of the apex-tools binary: nothing here\n",
            "install the apex-tools binary somewhere\n",
        );
        assert_eq!(
            referenced_subcommands(text).unwrap(),
            vec!["docs-lint".to_string(), "lcov-gate".to_string()]
        );
    }
}
