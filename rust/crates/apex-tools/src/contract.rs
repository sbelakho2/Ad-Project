//! The contract family: CI reference integrity, OpenAPI parity, the content
//! guard adversarial matrix, sample shape checks, docs script references and
//! pipeline path integrity. Ported from contract-checks.mjs; the generated
//! artifact pattern list is now single-sourced here.

use std::collections::BTreeSet;

use regex::Regex;

use crate::policy::CiRefBaseline;
use crate::repo::Repo;
use crate::util::{re, walk_files};

/// Generated-at-build paths that are legitimately absent in a fresh checkout
/// (single source: this table; contract checks only).
pub const CI_GENERATED_PATH_PATTERNS: &[&str] = &[
    r"(^|/)(coverage|dist|build|builds|artifacts|playwright-report|test-results|\.next|\.build|node_modules|Library|logs|public-repo|private-repo|docs|typedoc)(/|$)",
    r"\.(log|profdata|xcresult|sarif|gz|zip|tgz|tar|lcov|info)$",
    r"\.lighthouse",
    r"(^|/)(compatibility-matrix\.txt|compatibility-report\.md|unity-package-manifest\.json|k6-results\.json|eslint-report\.json|api-diff\.txt|validation_results\.json|unity-upm-size-report\.json)$",
    r"Documentation~",
    r"quality/perf/reports",
    r"website/analyze",
];

pub fn contract(repo: &Repo, check: &str, update_baseline: bool) -> Result<(), String> {
    match check {
        "ci-reference-integrity" => ci_reference_integrity(repo),
        "content-guard-adversarial" => content_guard_adversarial(repo),
        "docs-references" => docs_references(repo, update_baseline),
        "pipeline-paths" => pipeline_paths(repo),
        "github-metadata" => github_metadata(repo),
        other => Err(format!(
            "unknown contract check '{other}' (use ci-reference-integrity|content-guard-adversarial|docs-references|pipeline-paths)"
        )),
    }
}

fn generated_artifact_re() -> Result<Regex, String> {
    let joined = CI_GENERATED_PATH_PATTERNS.join("|");
    re(&joined)
}

/// Extract path refs from workflow yaml: only structured fields, never
/// free-text tokens. Mirrors extractPathRefs in the JS original.
pub fn extract_path_refs(text: &str, generated: &Regex) -> Result<Vec<String>, String> {
    let mut refs = Vec::new();
    let field_to = re(r"^\s*-\s*(dir|file|source|target):\s*(\S+)\s*$")?;
    let working = re(r"^\s*working-directory:\s*(\S+)\s*$")?;
    let paths = re(r"^\s*(paths?|files):\s*(\S+)\s*$")?;
    let quoted_glob = re(r"^\s*-\s*'([A-Za-z0-9_.\-/]+\*\*[*A-Za-z0-9_.\-/]*)'\s*$")?;
    let bare_path = re(r"^\s*path:\s*(\S+)\s*$")?;
    let block_scalar = re(r"^[|>][+-]?$")?;
    let add = |refs: &mut Vec<String>, raw: &str| {
        let trimmed = raw.trim_matches(|c| c == '"' || c == '\'');
        let cut = trimmed.split("${").next().unwrap_or(trimmed);
        let cut = cut.split("{{").next().unwrap_or(cut).trim();
        if cut.is_empty() || cut.starts_with('$') || cut == "." {
            return;
        }
        if block_scalar.is_match(cut) || generated.is_match(cut) {
            return;
        }
        refs.push(cut.to_string());
    };
    for line in text.lines() {
        if let Some(c) = working.captures(line) {
            add(&mut refs, &c[1]);
        }
        if let Some(c) = field_to.captures(line) {
            add(&mut refs, &c[2]);
        }
        if let Some(c) = paths.captures(line) {
            add(&mut refs, &c[2]);
        }
        if let Some(c) = quoted_glob.captures(line) {
            let star_marker = format!("/{}", "**");
            let p = c[1].trim_end_matches(star_marker.as_str());
            let p = p.split("**").next().unwrap_or(p);
            if !p.is_empty() && !generated.is_match(p) {
                refs.push(p.to_string());
            }
        }
        if let Some(c) = bare_path.captures(line) {
            add(&mut refs, &c[1]);
        }
    }
    Ok(refs)
}

pub fn ci_reference_integrity(repo: &Repo) -> Result<(), String> {
    let baseline = CiRefBaseline::load(repo)?;
    let known: BTreeSet<String> = baseline.known_missing.keys().cloned().collect();
    let generated = generated_artifact_re()?;
    let yaml_files: Vec<String> = repo
        .walk()
        .into_iter()
        .filter(|p| {
            (p.ends_with(".yml") || p.ends_with(".yaml"))
                && (p.starts_with(".github/workflows/") || p.starts_with(".woodpecker/"))
        })
        .collect();
    if yaml_files.is_empty() {
        println!(
            "ci-reference-integrity: no workflow files in this checkout; {} baselined entries are unreachable and skipped (PASS)",
            known.len()
        );
        return Ok(());
    }
    let mut missing: Vec<String> = Vec::new();
    for rel in &yaml_files {
        let text = repo.read(rel)?;
        for raw in extract_path_refs(&text, &generated)? {
            let no_slash = raw.trim_end_matches('/');
            if !repo.exists(&raw) && !repo.exists(no_slash) {
                missing.push(format!("{rel} -> {raw}"));
            }
        }
    }
    let missing_set: BTreeSet<String> = missing.iter().cloned().collect();
    let unexpected: Vec<&String> = missing.iter().filter(|m| !known.contains(*m)).collect();
    let stale: Vec<&String> = known.iter().filter(|k| !missing_set.contains(*k)).collect();
    if !unexpected.is_empty() || !stale.is_empty() {
        let mut lines: Vec<String> = unexpected.iter().map(|m| format!("  - {m}")).collect();
        for s in &stale {
            lines.push(format!("  - [stale] {s}"));
        }
        return Err(format!(
            "contract gate failed [ci-reference-integrity] missing refs: {} new, {} stale baseline entries (remove fixed entries from ci-ref-baseline.json)\n{}",
            unexpected.len(),
            stale.len(),
            lines.join("\n")
        ));
    }
    println!(
        "ci-reference-integrity PASS ({} workflow files; {} known-missing baselined for phase-1 remediation)",
        yaml_files.len(),
        known.len()
    );
    Ok(())
}

/// OpenAPI parity: every documented path must resolve in route/controller
/// sources. The TypeScript backend is absent from an all-Rust checkout, so an
/// absent backend tree is reported rather than failed.
/// Try suffixes of the documented path (routers mount sub-routers, so the
/// mount prefix may be absent from the registrar call).
/// Forbidden provider list, ported from tools/content-guard.js. Word-boundary
/// rules; split-token text is intentionally not fuzzy-matched.
pub const FORBIDDEN_PATTERNS: &[&str] = &[
    r"(?i)\bheroku\b",
    r"(?i)vercel",
    r"(?i)render\.com",
    r"(?i)\brailway\b",
    r"(?i)upstash",
    r"(?i)backblaze",
    r"(?i)backblazeb2",
    r"(?i)\bB2\b",
];

pub const CONTENT_GUARD_CASES: &[(&str, bool)] = &[
    ("we use HEROKU for this", true),
    ("V e r c e l", false),
    ("deployment is on vercel.com", true),
    ("nothing to see here", false),
    ("upstash-redis://xyz", true),
    ("backblaze b2 bucket", true),
];

pub fn scan_text_forbidden(text: &str) -> Result<bool, String> {
    for pattern in FORBIDDEN_PATTERNS {
        if re(pattern)?.is_match(text) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn content_guard_adversarial(repo: &Repo) -> Result<(), String> {
    let mut failures = Vec::new();
    for (text, expected) in CONTENT_GUARD_CASES {
        let got = scan_text_forbidden(text)?;
        if got != *expected {
            failures.push(format!(
                "scanText({:?}) -> {got}, expected {expected}",
                text
            ));
        }
    }
    // The matrix alone would pass no matter what the site says; scan the
    // published site sources so the guard has a subject.
    let scannable = re(r"\.(md|html|toml|yml|yaml|txt)$")?;
    let mut scanned = 0usize;
    for rel in repo.walk() {
        if !rel.starts_with("site/") || !scannable.is_match(&rel) {
            continue;
        }
        // Fail closed: an unreadable file cannot be declared clean.
        let text = repo.read(&rel)?;
        scanned += 1;
        if scan_text_forbidden(&text)? {
            failures.push(format!(
                "{rel}: forbidden hosting-provider reference on the site"
            ));
        }
    }
    if !failures.is_empty() {
        return Err(format!(
            "contract gate failed [content-guard-adversarial]:\n  - {}",
            failures.join("\n  - ")
        ));
    }
    println!(
        "content-guard-adversarial PASS ({} payload cases, {scanned} site files scanned)",
        CONTENT_GUARD_CASES.len()
    );
    Ok(())
}

/// Pure: structural failures in the GitHub metadata files.
pub fn github_metadata_failures(
    required_checks: &str,
    dependabot: &str,
) -> Result<Vec<String>, String> {
    let mut failures = Vec::new();
    match serde_json::from_str::<serde_json::Value>(required_checks) {
        Ok(value) => match value.get("required").and_then(|v| v.as_array()) {
            None => failures.push("required-checks.json needs a 'required' array".to_string()),
            Some(list) => {
                if list.is_empty() {
                    failures.push("required-checks.json lists no checks".to_string());
                }
                let mut seen: Vec<String> = Vec::new();
                for item in list {
                    match item.as_str() {
                        Some(name) if !name.trim().is_empty() => {
                            if seen.contains(&name.to_string()) {
                                failures.push(format!("required-checks.json repeats '{name}'"));
                            }
                            seen.push(name.to_string());
                        }
                        _ => {
                            failures.push("required-checks.json has a blank check name".to_string())
                        }
                    }
                }
            }
        },
        Err(error) => failures.push(format!("required-checks.json is not JSON: {error}")),
    }
    if !dependabot.contains("version: 2") {
        failures.push("dependabot.yml is missing 'version: 2'".to_string());
    }
    let ecosystems = dependabot.matches("package-ecosystem:").count();
    if ecosystems == 0 {
        failures.push("dependabot.yml declares no package-ecosystem updates".to_string());
    }
    for (needle, label) in [
        ("directory:", "directory"),
        ("interval:", "schedule interval"),
    ] {
        if dependabot.matches(needle).count() < ecosystems {
            failures.push(format!("dependabot.yml has updates without a {label}"));
        }
    }
    Ok(failures)
}

/// Validate the GitHub metadata files in the repository.
pub fn github_metadata(repo: &Repo) -> Result<(), String> {
    let required = repo
        .read(".github/required-checks.json")
        .map_err(|e| format!(".github/required-checks.json: {e}"))?;
    let dependabot = repo
        .read(".github/dependabot.yml")
        .map_err(|e| format!(".github/dependabot.yml: {e}"))?;
    let failures = github_metadata_failures(&required, &dependabot)?;
    if !failures.is_empty() {
        return Err(format!(
            "contract gate failed [github-metadata]:\n  - {}",
            failures.join("\n  - ")
        ));
    }
    println!("github-metadata contract PASS");
    Ok(())
}

/// Pure: npm-run and make references in markdown that resolve to nothing that
/// exists. Returns one message per violation.
pub fn docs_reference_violations(
    md_texts: &[(String, String)],
    scripts: &BTreeSet<String>,
    make_targets: &BTreeSet<String>,
    enforce_npm: bool,
) -> Result<Vec<String>, String> {
    let npm_re = re(r"`npm run ([a-z0-9:_-]+)`")?;
    let make_re = re(r"`make ([a-z0-9][a-z0-9_.-]*)`")?;
    let mut bad = Vec::new();
    for (rel, text) in md_texts {
        for c in npm_re.captures_iter(text).filter(|_| enforce_npm) {
            if !scripts.contains(&c[1]) {
                bad.push(format!(
                    "{rel}: references npm script '{}' which exists nowhere",
                    &c[1]
                ));
            }
        }
        for c in make_re.captures_iter(text) {
            if !make_targets.contains(&c[1]) {
                bad.push(format!(
                    "{rel}: references make target '{}' which exists nowhere",
                    &c[1]
                ));
            }
        }
    }
    Ok(bad)
}

pub const PKG_JSON_LIST: &[&str] = &["package.json"];

pub const DOCS_REFERENCES_BASELINE: &str = "tools/coverage/docs-references-baseline.txt";

/// Names every doc may reference: package.json scripts plus Makefile targets.
pub fn known_scripts(repo: &Repo) -> Result<BTreeSet<String>, String> {
    let mut scripts: BTreeSet<String> = BTreeSet::new();
    for p in PKG_JSON_LIST {
        if !repo.exists(p) {
            continue;
        }
        let text = repo.read(p)?;
        let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{p}: {e}"))?;
        if let Some(objs) = v.get("scripts").and_then(|s| s.as_object()) {
            for k in objs.keys() {
                scripts.insert(k.clone());
            }
        }
    }
    Ok(scripts)
}

/// Makefile target names (lines `name:` that are not variable assignments).
pub fn make_targets(repo: &Repo) -> Result<BTreeSet<String>, String> {
    let mut targets = BTreeSet::new();
    let Ok(text) = repo.read("Makefile") else {
        return Ok(targets);
    };
    let target = re(r"^([A-Za-z0-9][A-Za-z0-9_.-]*):")?;
    for line in text.lines() {
        if let Some(c) = target.captures(line) {
            targets.insert(c[1].to_string());
        }
    }
    Ok(targets)
}

pub fn docs_references(repo: &Repo, update_baseline: bool) -> Result<(), String> {
    let scripts = known_scripts(repo)?;
    let targets = make_targets(repo)?;
    // With no npm project anywhere in the tree, npm mentions are historical
    // prose, not tool references; the check says so instead of flagging them.
    let enforce_npm = PKG_JSON_LIST.iter().any(|path| repo.exists(path));
    if !enforce_npm {
        println!("docs-references: no package.json exists in this checkout; npm-run mentions are not checked");
    }
    // Fail closed: an unreadable document might hide a stale reference.
    let mut md_texts: Vec<(String, String)> = Vec::new();
    for p in repo.walk().into_iter().filter(|p| p.ends_with(".md")) {
        let text = repo.read(&p)?;
        md_texts.push((p, text));
    }
    let bad = docs_reference_violations(&md_texts, &scripts, &targets, enforce_npm)?;
    let mut per_file: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for message in &bad {
        let file = message
            .split_once(':')
            .map(|(f, _)| f.to_string())
            .unwrap_or_else(|| "<unknown>".to_string());
        *per_file.entry(file).or_insert(0) += 1;
    }
    let total = bad.len();

    if update_baseline {
        let mut content = String::from(
            "# docs-references ratchet baseline: stale script/target references per file (rows may only shrink)\n",
        );
        for (file, count) in &per_file {
            content.push_str(&format!("{count} {file}\n"));
        }
        content.push_str(&format!("TOTAL {total}\n"));
        std::fs::write(repo.path(DOCS_REFERENCES_BASELINE), content)
            .map_err(|e| format!("{DOCS_REFERENCES_BASELINE}: {e}"))?;
        println!("docs-references baseline written: {DOCS_REFERENCES_BASELINE} (TOTAL {total})");
        return Ok(());
    }

    let path = repo.path(DOCS_REFERENCES_BASELINE);
    if !path.exists() {
        return Err(format!(
            "docs-references: baseline not found: {DOCS_REFERENCES_BASELINE} (generate with --update-baseline)"
        ));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let (allowed, allowed_total) = crate::baseline::parse_count_rows(&text);
    let (failures, _notes) = crate::baseline::ratchet(
        &per_file,
        &allowed,
        total,
        allowed_total,
        "stale references",
        crate::baseline::Stale::Failure,
    );
    if !failures.is_empty() {
        let mut detail: Vec<String> = failures.clone();
        for message in bad.iter().take(40) {
            detail.push(format!("    {message}"));
        }
        return Err(format!(
            "contract gate failed [docs-references]:\n  - {}",
            detail.join("\n  - ")
        ));
    }
    println!(
        "docs-reference-contract PASS: {total} stale references at or below baseline ({allowed_total}) across {} files",
        md_texts.len()
    );
    Ok(())
}

/// One pipeline path reference: relative path plus the file and line it came
/// from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineRef {
    pub file: String,
    pub line: usize,
    pub path: String,
}

/// Extract repo-relative file-like path candidates from a pipeline file.
pub fn pipeline_path_refs(file: &str, text: &str) -> Result<Vec<PipelineRef>, String> {
    let token_re = re(r"([A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9_./-]*\.[A-Za-z0-9]+)")?;
    let mut refs = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        for m in token_re.find_iter(line) {
            if let Some(before) = line[..m.start()].chars().next_back() {
                if before.is_alphanumeric() || before == '/' {
                    continue;
                }
            }
            let raw = m.as_str();
            let candidate = raw.trim_end_matches(|c: char| {
                c == '.' || c == ',' || c == ';' || c == ':' || c == '\'' || c == '"' || c == ')'
            });
            if candidate.is_empty() {
                continue;
            }
            if candidate.contains(['$', '*', '?', '{', '}', '(', ')', '<', '>', '|', '`', '\\']) {
                continue;
            }
            let segments: Vec<&str> = candidate.split('/').collect();
            if segments
                .iter()
                .any(|s| s.is_empty() || *s == ".." || *s == ".")
            {
                continue;
            }
            if segments[0].contains('.') {
                continue;
            }
            refs.push(PipelineRef {
                file: file.to_string(),
                line: idx + 1,
                path: candidate.to_string(),
            });
        }
    }
    Ok(refs)
}

pub fn pipeline_paths(repo: &Repo) -> Result<(), String> {
    let dir = repo.path(".woodpecker");
    if !dir.is_dir() {
        println!("pipeline-paths: no .woodpecker pipelines in this checkout (PASS)");
        return Ok(());
    }
    let pages = walk_files(&dir, &[]);
    let pipeline_files: Vec<String> = pages
        .iter()
        .filter(|p| {
            p.extension()
                .map(|e| e == "yml" || e == "yaml")
                .unwrap_or(false)
        })
        .map(|p| {
            p.strip_prefix(&repo.root)
                .map(crate::util::slash)
                .unwrap_or_else(|_| crate::util::slash(p))
        })
        .collect();
    let generated = generated_artifact_re()?;
    let tracked: BTreeSet<String> = repo
        .tracked_files_with_modes()?
        .into_iter()
        .map(|(_, p)| p)
        .collect();
    let mut missing: Vec<String> = Vec::new();
    let mut checked = 0usize;
    let mut skipped = 0usize;
    for file in &pipeline_files {
        let Ok(text) = repo.read(file) else { continue };
        for r in pipeline_path_refs(file, &text)? {
            let root_seg = r.path.split('/').next().unwrap_or("");
            if !repo.root.join(root_seg).is_dir() {
                skipped += 1; // reference into a tree absent from this checkout
                continue;
            }
            // The generated-artifact list treats any `coverage` segment as
            // build output; the gate stack's own tools/ tree is first-party,
            // so its references are still checked.
            if generated.is_match(&r.path) && !r.path.starts_with("tools/") {
                skipped += 1;
                continue;
            }
            let parent = match r.path.rsplit_once('/') {
                Some((p, _)) => p,
                None => continue,
            };
            if !repo.root.join(parent).is_dir() {
                skipped += 1;
                continue;
            }
            if repo.exists(&r.path) {
                checked += 1;
                continue;
            }
            if tracked.contains(&r.path) {
                missing.push(format!(
                    "{}:{}: referenced path missing: {}",
                    r.file, r.line, r.path
                ));
            } else {
                skipped += 1; // never-tracked path: foreign pipeline debt
            }
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "contract gate failed [pipeline-paths] missing referenced paths:\n  - {}",
            missing.join("\n  - ")
        ));
    }
    println!(
        "pipeline-paths PASS ({} pipeline files, {checked} references checked, {skipped} foreign/generated refs skipped)",
        pipeline_files.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_patterns_cover_build_outputs() {
        let g = generated_artifact_re().unwrap();
        assert!(g.is_match("services/autopilot/coverage/coverage-summary.json"));
        assert!(g.is_match("ML/coverage.lcov"));
        assert!(g.is_match("services/mail-server/lcov.info"));
        // Parity quirk, kept on purpose: the JS pattern list treats any
        // `coverage` path segment as generated, including tools/coverage.
        assert!(g.is_match("tools/coverage/floors.txt"));
        assert!(!g.is_match("site/config.toml"));
    }

    #[test]
    fn ci_ref_extraction_uses_structured_fields_only() {
        let g = generated_artifact_re().unwrap();
        let stars = "**";
        let text = format!("working-directory: backend\n  - file: infra/x.yml\n  paths: 'sdk/{stars}'\n  - 'backend/{stars}'\n  path: coverage/out.xml\nrun: hello world\n");
        let refs = extract_path_refs(&text, &g).unwrap();
        assert!(refs.contains(&"backend".to_string()));
        assert!(refs.contains(&"infra/x.yml".to_string()));
        assert!(refs.contains(&format!("sdk/{stars}")));
        // The generated pattern prunes coverage/out.xml and the glob prefix
        // 'backend' from the quoted list stays.
        assert!(!refs.iter().any(|r| r.contains("coverage")));
    }

    #[test]
    fn content_guard_matrix_matches_js() {
        assert!(scan_text_forbidden("we use HEROKU for this").unwrap());
        assert!(!scan_text_forbidden("V e r c e l").unwrap());
        assert!(scan_text_forbidden("deployment is on vercel.com").unwrap());
        assert!(!scan_text_forbidden("nothing to see here").unwrap());
        assert!(scan_text_forbidden("upstash-redis://xyz").unwrap());
        assert!(scan_text_forbidden("backblaze b2 bucket").unwrap());
        let repo = Repo::discover().expect("repo");
        assert!(content_guard_adversarial(&repo).is_ok());
    }

    #[test]
    fn docs_references_flag_unknown_scripts() {
        let scripts: BTreeSet<String> = ["build".to_string()].into_iter().collect();
        let md = vec![(
            "docs/x.md".to_string(),
            "run `npm run build` then `npm run do:tls`".to_string(),
        )];
        let targets: BTreeSet<String> = ["test".to_string()].into_iter().collect();
        let bad = docs_reference_violations(&md, &scripts, &targets, true).unwrap();
        assert_eq!(bad.len(), 1);
        assert!(bad[0].contains("do:tls"));
    }

    #[test]
    fn github_metadata_refuses_broken_files() {
        let good_checks = r#"{"required": ["coverage-gate", "dogfood-audit"]}"#;
        let good_dependabot = "version: 2\nupdates:\n  - package-ecosystem: cargo\n    directory: \"/\"\n    schedule:\n      interval: weekly\n";
        assert!(github_metadata_failures(good_checks, good_dependabot)
            .unwrap()
            .is_empty());
        for (checks, dependabot) in [
            (r#"{"required": []}"#, good_dependabot),
            (r#"{"required": ["a", "a"]}"#, good_dependabot),
            (r#"{"required": ["a", ""]}"#, good_dependabot),
            (r#"{"required": "a"}"#, good_dependabot),
            (good_checks, "version: 2\n"),
            (good_checks, "version: 1\nupdates:\n"),
            (
                good_checks,
                "version: 2\nupdates:\n  - package-ecosystem: cargo\n",
            ),
        ] {
            assert!(
                !github_metadata_failures(checks, dependabot)
                    .unwrap()
                    .is_empty(),
                "must refuse: {checks} / {dependabot}"
            );
        }
    }

    #[test]
    fn docs_references_flag_stale_make_targets() {
        let scripts: BTreeSet<String> = BTreeSet::new();
        let targets: BTreeSet<String> = ["test".to_string()].into_iter().collect();
        let md = vec![(
            "docs/y.md".to_string(),
            "run `make test` before `make deploy`".to_string(),
        )];
        let bad = docs_reference_violations(&md, &scripts, &targets, true).unwrap();
        assert_eq!(bad.len(), 1);
        assert!(bad[0].contains("make target 'deploy'"), "{:?}", bad);
    }

    #[test]
    fn pipeline_refs_skip_shell_syntax_and_urls() {
        let text = "image: node:20\n- node tools/coverage/lcov-gate.mjs ML/coverage.lcov $FLOOR && rm -rf ${X}\n- curl https://example.com/x.js\n";
        let refs = pipeline_path_refs("p.yml", text).unwrap();
        let paths: Vec<&str> = refs.iter().map(|r| r.path.as_str()).collect();
        assert!(paths.contains(&"tools/coverage/lcov-gate.mjs"));
        assert!(paths.contains(&"ML/coverage.lcov"));
        assert!(!paths.iter().any(|p| p.contains("FLOOR")));
        assert!(!paths.iter().any(|p| p.starts_with("example.com")));
    }

    #[test]
    fn pipeline_refs_require_tracked_missing_files() {
        // Pure extraction test doubles for the repo-dependent part.
        let text = "- sh tools/coverage/go-coverage-gate.sh cover.func 90 # note\n";
        let refs = pipeline_path_refs("p.yml", text).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].path, "tools/coverage/go-coverage-gate.sh");
        assert_eq!(refs[0].line, 1);
    }
}
