//! apex-tools CLI — the all-Rust gate stack entry point.

#![forbid(unsafe_code)]

use apex_tools::repo::Repo;
use apex_tools::{
    a11y, claims, contract, coverage, csslint, docs_lint, docslinks, dogfood, i18n, mutation,
    perfbudget, policy::Floors,
};

fn flag(args: &[String], name: &str) -> Option<String> {
    let idx = args.iter().position(|a| a == name)?;
    args.get(idx + 1).cloned()
}

fn positionals(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_next = false;
    for a in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if a.starts_with("--") {
            skip_next = matches!(
                a.as_str(),
                "--baseline"
                    | "--update-baseline"
                    | "--dist"
                    | "--min-pages"
                    | "--root"
                    | "--floor"
                    | "--domain"
                    | "--artifact"
                    | "--tool"
                    | "--manifest"
                    | "--max-orphan-percent"
                    | "--report"
                    | "--out"
            );
            continue;
        }
        out.push(a.clone());
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = || {
        eprintln!(
            "usage: apex-tools <{}> [flags]",
            apex_tools::SUBCOMMANDS.join("|")
        );
    };
    let Some(cmd) = args.first().map(String::as_str) else {
        usage();
        std::process::exit(2);
    };
    if !apex_tools::SUBCOMMANDS.contains(&cmd) {
        usage();
        std::process::exit(2);
    }
    let rest = &args[1..];
    let repo = match Repo::discover() {
        Ok(r) => r,
        Err(e) => fail(&e),
    };
    let result: Result<(), String> = match cmd {
        "manifest" => (|| {
            let out = flag(rest, "--out").unwrap_or_else(|| "coverage-manifest.json".into());
            let manifest = apex_tools::classify::build_manifest(&repo)?;
            let json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
            std::fs::write(repo.path(&out), json).map_err(|e| e.to_string())?;
            println!(
                "manifest written: {out} ({} files; counts={:?})",
                manifest.files.len(),
                manifest.counts
            );
            Ok(())
        })(),
        "walker-parity" => {
            let manifest =
                flag(rest, "--manifest").unwrap_or_else(|| "coverage-manifest.json".into());
            dogfood::walker_parity(&repo, &manifest)
        }
        "orphan-gate" => (|| {
            let manifest_path =
                flag(rest, "--manifest").unwrap_or_else(|| "coverage-manifest.json".into());
            let floor = Floors::load(&repo)?;
            let max = flag(rest, "--max-orphan-percent")
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(floor.orphan_floor_percent);
            let manifest: apex_tools::classify::Manifest =
                serde_json::from_str(&repo.read(&manifest_path)?).map_err(|e| e.to_string())?;
            let suites = apex_tools::policy::read_suites(&repo)?;
            dogfood::orphan_gate(&manifest, &suites, max)
        })(),
        "floor-ratchet" => Floors::load(&repo).and_then(|f| dogfood::floor_ratchet(&f)),
        "floors-sync" => dogfood::floors_sync(&repo),
        "sync-floors" => dogfood::sync_floors_write(&repo),
        "assertion-presence" => dogfood::assertion_presence(&repo),
        "stub-ban" => dogfood::stub_ban(&repo),
        "mutation-floor" => {
            let floor = flag(rest, "--floor")
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0);
            dogfood::mutation_floor(&repo, floor)
        }
        "yaml-policy" => {
            let full = rest.iter().any(|a| a == "--full");
            let shell = rest.iter().any(|a| a == "--shell");
            dogfood::yaml_policy(&repo, full, shell)
        }
        "coverage-gate" => (|| {
            let domain = flag(rest, "--domain").ok_or("--domain required")?;
            let artifact = flag(rest, "--artifact").ok_or("--artifact required")?;
            let tool = flag(rest, "--tool").unwrap_or_else(|| "jest".into());
            let require_branches = rest.iter().any(|a| a == "--require-branches");
            coverage::coverage_gate(&repo, &domain, &artifact, &tool, require_branches)
        })(),
        "aggregate-verdict" => {
            let report = flag(rest, "--report").unwrap_or_else(|| "coverage-report.md".into());
            coverage::aggregate_verdict(&repo, &report)
        }
        "lcov-gate" => (|| {
            let positional = positionals(rest);
            let file = positional.first().ok_or(
                "usage: apex-tools lcov-gate <lcov.info> <floorPercent> [--require-branches]",
            )?;
            let floor_arg = positional.get(1).ok_or(
                "usage: apex-tools lcov-gate <lcov.info> <floorPercent> [--require-branches]",
            )?;
            let floor = floor_arg
                .parse::<f64>()
                .map_err(|_| format!("invalid floor '{floor_arg}'"))?;
            let require_branches = rest.iter().any(|a| a == "--require-branches");
            coverage::lcov_gate(file, floor, require_branches)
        })(),
        "go-func-gate" => (|| {
            let positional = positionals(rest);
            let file = positional
                .first()
                .ok_or("usage: apex-tools go-func-gate <cover.func> <floor>")?;
            let floor_arg = positional
                .get(1)
                .ok_or("usage: apex-tools go-func-gate <cover.func> <floor>")?;
            let floor = floor_arg
                .parse::<f64>()
                .map_err(|_| format!("invalid floor '{floor_arg}'"))?;
            coverage::go_func_gate(file, floor)
        })(),
        "contract" => (|| {
            let check = positionals(rest).into_iter().next().ok_or(
                "usage: apex-tools contract <ci-reference-integrity|openapi-parity|content-guard-adversarial|samples|docs-references|pipeline-paths> [--update-baseline]",
            )?;
            let update_baseline = rest.iter().any(|a| a == "--update-baseline");
            contract::contract(&repo, &check, update_baseline)
        })(),
        "docs-links" => match docslinks::parse_mode(rest) {
            Ok(mode) => docslinks::docs_links(&repo, &mode),
            Err(e) => Err(e),
        },
        "i18n-parity" => {
            let roots: Vec<String> = rest
                .iter()
                .enumerate()
                .filter(|(_, a)| a.as_str() == "--root")
                .filter_map(|(i, _)| rest.get(i + 1).cloned())
                .collect();
            i18n::i18n_parity(&repo, &roots)
        }
        "css-lint" => csslint::css_lint(&repo),
        "claims" => claims::claims_contract(&repo),
        "perf-budget" => {
            let dist = flag(rest, "--dist").unwrap_or_else(|| "site/public".into());
            let min_pages = flag(rest, "--min-pages")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(6);
            perfbudget::perf_budget(&repo, &dist, min_pages)
        }
        "console-a11y" => {
            let dir = positionals(rest).into_iter().next();
            a11y::console_a11y(&repo, dir.as_deref())
        }
        "docs-lint" => (|| {
            let parsed = docs_lint::parse_args(rest)?;
            docs_lint::docs_lint(&repo, &parsed)
        })(),
        "mutation-smoke" => mutation::mutation_smoke(&repo),
        other => Err(format!("unknown subcommand '{other}'")),
    };
    match result {
        Ok(()) => {}
        Err(e) => fail(&e),
    }
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}
