//! Static parity gate for the adversarial-live dogfood surface catalog.
//!
//! `apex-tools dogfood-plan` proves — without booting anything — that the
//! declared live-probe catalog (`tools/dogfood/surfaces.json`) and the real
//! services agree:
//!
//! - **route parity**: every `.route("<path>", get(..) / post(..))`
//!   registration in the three `routes.rs` files must be declared, and every
//!   declaration must still be registered;
//! - **fixture parity**: the `pub const PAGES: [&str; N] = [` length in each
//!   `render-fixtures.rs` bin must equal the declared `fixture_pages` count;
//! - **corpus parity**: `probe_classes` must equal the frozen
//!   [`REQUIRED_PROBE_CLASSES`] registry, and every per-service `required`
//!   cell may reference only declared classes and declared route surfaces;
//! - **baseline ratchet**: every cell in `tools/dogfood/live-baseline.json`
//!   must still be declared required, or be covered by a `removals` entry
//!   that names a recorded baseline cell and carries a reason.
//!
//! The extraction is deliberately bounded: the regex is allowlisted to the
//! three route files and the three fixture bins, and a file that does not
//! yield robustly (zero `.route(` registrations, a `.route(` form the regex
//! cannot classify, a missing `PAGES` array) is a hard error, never an empty
//! set. `{param}` path segments are captured verbatim, in the spelling the
//! router registers.
//!
//! Frozen schema, `tools/dogfood/surfaces.json`:
//!
//! ```json
//! {
//!   "probe_classes": ["surface-parity", "response-contract", "..."],
//!   "services": {
//!     "apex-api": {
//!       "routes": [ { "path": "/healthz", "methods": ["GET"] } ],
//!       "fixture_pages": 62,
//!       "required": [
//!         { "surface": "GET /healthz", "classes": ["surface-parity"] }
//!       ]
//!     }
//!   }
//! }
//! ```
//!
//! A publisher-console GET that refuses anonymous reads is declared with
//! `"session_required": true` on its route entry (the live harness asserts
//! the anonymous `303 /console/ledger` and the documented public exceptions
//! from this marker). The marker is validated here: it may appear only on a
//! declared `apex-api` console `GET`, only as `true` (a `false` value is a
//! schema drift, not a way to spell "public"), and never on one of the
//! documented public console paths in [`PUBLIC_CONSOLE_GETS`]; a marker on a
//! public route is a hard failure because it would declare a gate the product
//! does not have.
//!
//! `tools/dogfood/live-baseline.json` records the measured requirement
//! baseline and an append-only removal ledger:
//!
//! ```json
//! {
//!   "cells": ["apex-api|GET /healthz|surface-parity"],
//!   "removals": [ { "cell": "...", "reason": "..." } ]
//! }
//! ```
//!
//! Cell ids are `"<service>|<surface>|<class>"`; a route surface is
//! `"<METHOD> <path>"`.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::repo::Repo;
use crate::util::re;

/// The closed probe corpus registry. The declared `probe_classes` list must
/// equal this table exactly (order included); removing or renaming a class
/// requires editing this constant and the pinning test below, which is the
/// point of freezing it in code.
///
/// The first twelve are the original corpus. The last four are the
/// ApAudit-review classes: `tenant-isolation` (one publisher's session never
/// sees another's snapshot, ledger totals or audit rows), `auth-surface`
/// (`/v1/audit/{platform}` refuses the anonymous read and the console GETs
/// refuse a session-less read), `signin-ownership` (the free-audit sign-in
/// link is delivered only to a workspace's recorded contacts and a repeat
/// single-use link is refused) and `payment-activation` (a setup-watch
/// sign-up lands pending, the webhook activates it, the tier follows evidence
/// and the monthly bill charges the stored card).
pub const REQUIRED_PROBE_CLASSES: [&str; 16] = [
    "surface-parity",
    "response-contract",
    "no-inline",
    "assets-fonts",
    "auth-authority",
    "input-corpus",
    "csrf-idempotency",
    "traversal",
    "escaping",
    "concurrency",
    "failure-injection",
    "differential-truth",
    "tenant-isolation",
    "auth-surface",
    "signin-ownership",
    "payment-activation",
];

/// The always-run core set the relevance filter adds to every selection: the
/// published `/healthz` contract (which also runs the static parity gate), the
/// console session gate (`auth-surface`), and the two-publisher tenant
/// isolation of the overview and the ledger. A filtered run always proves
/// these before it proves the changed subset, so a change never removes the
/// workspace-isolation and liveness guarantees.
pub const CORE_CELLS: [&str; 5] = [
    "apex-api|GET /healthz|surface-parity",
    "apex-api|GET /healthz|response-contract",
    "apex-api|GET /console/audit|auth-surface",
    "apex-api|GET /|tenant-isolation",
    "apex-api|GET /console/ledger|tenant-isolation",
];

/// The declared surface catalog owned by the live harness.
pub const SURFACES_REL: &str = "tools/dogfood/surfaces.json";
/// The recorded requirement baseline and removal ledger.
pub const BASELINE_REL: &str = "tools/dogfood/live-baseline.json";

/// The service ids the catalog must declare, with their allowlisted sources:
/// `(service, routes.rs, render-fixtures.rs)`.
const SOURCES: [(&str, &str, &str); 3] = [
    (
        "apex-api",
        "rust/crates/apex-api/src/routes.rs",
        "rust/crates/apex-api/src/bin/render-fixtures.rs",
    ),
    (
        "apex-control",
        "rust/crates/apex-control/src/routes.rs",
        "rust/crates/apex-control/src/bin/render-fixtures.rs",
    ),
    (
        "apex-sales-web",
        "rust/crates/apex-sales-web/src/routes.rs",
        "rust/crates/apex-sales-web/src/bin/render-fixtures.rs",
    ),
];

/// One registered route: a path template and the methods registered on it.
/// Methods are normalized to upper case and sorted; the path is captured
/// verbatim (`{param}` segments are kept as the router spells them).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RegisteredRoute {
    /// The registered path template.
    pub path: String,
    /// The registered methods, upper-cased and sorted.
    pub methods: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SurfacesFile {
    probe_classes: Vec<String>,
    services: BTreeMap<String, DeclaredService>,
}

#[derive(Debug, Deserialize)]
struct DeclaredService {
    routes: Vec<DeclaredRoute>,
    fixture_pages: usize,
    required: Vec<RequiredCell>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeclaredRoute {
    path: String,
    methods: Vec<String>,
    #[serde(default)]
    session_required: Option<bool>,
    /// The repo paths that implement the surface. The relevance filter
    /// (`--changed` / `--git-diff`) selects only the cells whose surface
    /// sources intersect the changed paths, so every route must name at least
    /// one real path. An empty list here is a schema drift, never a way to opt
    /// a surface out of relevance.
    #[serde(default)]
    sources: Vec<String>,
}

/// The documented publisher-console GET paths that deliberately stay public:
/// the access panel and its redirects, the marketing flows, the one-time
/// sign-in link, the setup-watch form and its confirmation. A
/// `session_required` marker on one of these contradicts the product's own
/// documentation and is refused.
pub const PUBLIC_CONSOLE_GETS: [&str; 10] = [
    "/console",
    "/console/",
    "/console/ledger",
    "/console/signin",
    "/free-audit",
    "/free-audit/",
    "/audit-requested/",
    "/watch",
    "/watch/",
    "/watch/requested",
];

#[derive(Debug, Clone, Deserialize)]
struct RequiredCell {
    surface: String,
    classes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BaselineFile {
    #[serde(default)]
    cells: Vec<String>,
    #[serde(default)]
    removals: Vec<BaselineRemoval>,
}

#[derive(Debug, Deserialize)]
struct BaselineRemoval {
    cell: String,
    #[serde(default)]
    reason: String,
}

/// The allowlisted `.route("<path>", <method-chain>)` matcher. The handler
/// expression is bounded to a Rust path (`crate::favicon::ico`); chained
/// methods (`get(a).post(b)`) and the multi-line dangling-comma argument form
/// are both recognized.
fn route_re() -> Result<regex::Regex, String> {
    let handler = r"[A-Za-z_][A-Za-z0-9_]*(?:\s*::\s*[A-Za-z_][A-Za-z0-9_]*)*";
    let method = format!(r"(?:get|post)\(\s*{handler}\s*\)");
    re(&format!(
        r#"\.route\(\s*"(?P<path>[^"]*)"\s*,\s*(?P<spec>{method}(?:\s*\.\s*{method})*)\s*,?\s*\)"#
    ))
}

/// The allowlisted `pub const PAGES: [&str; N] = [` matcher.
fn fixture_pages_re() -> Result<regex::Regex, String> {
    re(r"pub const PAGES: \[&str; (?P<n>[0-9]+)\] = \[")
}

/// Drop `//` line comments so a commented-out registration cannot be read as
/// real. The allowlisted files carry no `//` inside a route string literal.
fn strip_line_comments(text: &str) -> String {
    text.lines()
        .map(|line| match line.find("//") {
            Some(idx) => &line[..idx],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract every `.route(...)` registration from `text`, in path order.
///
/// Hard errors (never an empty set): no `.route(` call at all, a `.route(`
/// call the bounded regex cannot classify (count mismatch), a duplicate
/// method registration on one path, or a method chain with no extractable
/// method.
pub fn extract_routes(text: &str, rel: &str) -> Result<Vec<RegisteredRoute>, String> {
    let stripped = strip_line_comments(text);
    let occurrences = stripped.matches(".route(").count();
    if occurrences == 0 {
        return Err(format!(
            "{rel}: no .route( registrations found; refusing an empty extraction"
        ));
    }
    let route_re = route_re()?;
    let method_re = re(r"\b(get|post)\(")?;
    let mut routes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut matched = 0usize;
    for caps in route_re.captures_iter(&stripped) {
        matched += 1;
        let path = caps
            .name("path")
            .ok_or_else(|| format!("{rel}: route match without a path"))?
            .as_str()
            .to_string();
        let spec = caps
            .name("spec")
            .ok_or_else(|| format!("{rel}: route '{path}' match without a method spec"))?
            .as_str();
        let methods: BTreeSet<String> = method_re
            .captures_iter(spec)
            .map(|m| m[1].to_ascii_uppercase())
            .collect();
        if methods.is_empty() {
            return Err(format!(
                "{rel}: route '{path}' carries no extractable method"
            ));
        }
        match routes.get_mut(&path) {
            Some(existing) => {
                let duplicates: Vec<&str> = methods
                    .iter()
                    .filter(|method| existing.contains(*method))
                    .map(String::as_str)
                    .collect();
                if !duplicates.is_empty() {
                    return Err(format!(
                        "{rel}: route '{path}' registers method(s) {} twice",
                        duplicates.join(", ")
                    ));
                }
                existing.extend(methods);
            }
            None => {
                routes.insert(path, methods);
            }
        }
    }
    if matched != occurrences {
        return Err(format!(
            "{rel}: found {occurrences} .route( call(s) but the bounded regex classified {matched}; an unrecognized registration form would be skipped"
        ));
    }
    Ok(routes
        .into_iter()
        .map(|(path, methods)| RegisteredRoute {
            path,
            methods: methods.into_iter().collect(),
        })
        .collect())
}

/// Extract the declared fixture page count `N` from
/// `pub const PAGES: [&str; N] = [`. Exactly one such array must exist; a
/// missing array is a hard error (the fixture list is part of the frozen
/// contract, not an inferred count).
pub fn extract_fixture_page_count(text: &str, rel: &str) -> Result<usize, String> {
    let pages_re = fixture_pages_re()?;
    let counts: Vec<usize> = pages_re
        .captures_iter(&strip_line_comments(text))
        .filter_map(|caps| {
            caps.name("n")
                .and_then(|n| n.as_str().parse::<usize>().ok())
        })
        .collect();
    match counts.len() {
        1 => Ok(counts[0]),
        0 => Err(format!(
            "{rel}: no `pub const PAGES: [&str; N] = [` array found; the fixture page list must be declared explicitly"
        )),
        n => Err(format!(
            "{rel}: expected exactly one PAGES array, found {n}"
        )),
    }
}

/// Parse a `"<service>|<surface>|<class>"` cell id.
fn parse_cell_id(cell: &str) -> Option<(&str, &str, &str)> {
    let mut parts = cell.split('|');
    let service = parts.next()?;
    let surface = parts.next()?;
    let class = parts.next()?;
    if parts.next().is_some() || service.is_empty() || surface.is_empty() || class.is_empty() {
        return None;
    }
    Some((service, surface, class))
}

fn check_probe_classes(surfaces: &SurfacesFile, problems: &mut Vec<String>) {
    let declared: Vec<&str> = surfaces.probe_classes.iter().map(String::as_str).collect();
    let unique: BTreeSet<&str> = declared.iter().copied().collect();
    if unique.len() != declared.len() {
        problems.push(format!("{SURFACES_REL}: probe_classes lists a class twice"));
    }
    if declared != REQUIRED_PROBE_CLASSES.to_vec() {
        let missing: Vec<&str> = REQUIRED_PROBE_CLASSES
            .iter()
            .copied()
            .filter(|c| !unique.contains(c))
            .collect();
        let unexpected: Vec<&str> = declared
            .iter()
            .copied()
            .filter(|c| !REQUIRED_PROBE_CLASSES.contains(c))
            .collect();
        problems.push(format!(
            "{SURFACES_REL}: probe_classes != the frozen REQUIRED_PROBE_CLASSES (missing: {missing:?}; unexpected: {unexpected:?}; order is part of the equality)"
        ));
    }
}

fn declared_route_surfaces(
    service: &str,
    routes: &[DeclaredRoute],
    problems: &mut Vec<String>,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for route in routes {
        if !route.path.starts_with('/') {
            problems.push(format!(
                "{SURFACES_REL}: {service}: route path '{}' is not absolute",
                route.path
            ));
            continue;
        }
        if route.methods.is_empty() {
            problems.push(format!(
                "{SURFACES_REL}: {service}: route '{}' declares no methods",
                route.path
            ));
            continue;
        }
        for method in &route.methods {
            let normalized = method.trim().to_ascii_uppercase();
            if normalized != "GET" && normalized != "POST" {
                problems.push(format!(
                    "{SURFACES_REL}: {service}: route '{}' declares unsupported method '{}'",
                    route.path, method
                ));
                continue;
            }
            let surface = format!("{normalized} {}", route.path);
            if !out.insert(surface.clone()) {
                problems.push(format!(
                    "{SURFACES_REL}: {service}: route surface '{surface}' is declared twice"
                ));
            }
        }
    }
    out
}

/// The relevance selection over the required cells.
///
/// `changed` empty means the unfiltered catalog (`--changed` absent): every
/// cell is selected and no class is skipped. Otherwise every cell whose
/// surface `sources` intersect a changed path is selected, the always-run
/// [`CORE_CELLS`] are added, and the classes with no selected cell are
/// reported as skipped. A changed path that matches no declared source is
/// returned in `unmatched`, so the caller can say so instead of silently
/// dropping it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relevance {
    /// The selected cells, sorted.
    pub selected: BTreeSet<String>,
    /// The classes with no selected cell (reported as skipped).
    pub skipped_classes: BTreeSet<String>,
    /// The changed paths that matched no declared source.
    pub unmatched: Vec<String>,
}

/// Whether a changed repo path matches one declared source. A source naming a
/// directory (trailing `/`) matches it and any path under it; otherwise the
/// match is exact. Byte-exact, so the selection is deterministic.
fn source_matches(changed: &str, source: &str) -> bool {
    match source.strip_suffix('/') {
        Some(dir) => changed == dir || changed.starts_with(source),
        None => changed == source,
    }
}

/// The per-surface `sources` map (`"<service>|<path>"` to the declared paths).
/// Every declared route must name at least one repo-relative path: the
/// relevance filter depends on it, so a missing or absolute source is a hard
/// schema failure, never a silent opt-out.
fn declared_route_sources(
    surfaces: &SurfacesFile,
    problems: &mut Vec<String>,
) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for (service, declared) in &surfaces.services {
        if !SOURCES.iter().any(|(id, _, _)| id == service) {
            continue;
        }
        for route in &declared.routes {
            if route.sources.is_empty() {
                problems.push(format!(
                    "{SURFACES_REL}: {service} {}: declares no `sources`; the relevance filter needs the implementing repo paths",
                    route.path
                ));
                continue;
            }
            for source in &route.sources {
                if source.trim().is_empty() || source.starts_with('/') || source.contains("..") {
                    problems.push(format!(
                        "{SURFACES_REL}: {service} {}: source '{source}' must be a repo-relative path",
                        route.path
                    ));
                }
            }
            out.insert(format!("{service}|{}", route.path), route.sources.clone());
        }
    }
    out
}

/// Select the required cells relevant to `changed`. The cell key is
/// `"<service>|<METHOD path>|<class>"`; its surface key is `"<service>|<path>"`.
pub fn select_cells(
    all_cells: &BTreeSet<String>,
    surface_sources: &BTreeMap<String, Vec<String>>,
    changed: &[String],
) -> Relevance {
    if changed.is_empty() {
        return Relevance {
            selected: all_cells.clone(),
            skipped_classes: BTreeSet::new(),
            unmatched: Vec::new(),
        };
    }
    let mut selected = BTreeSet::new();
    for cell in all_cells {
        let mut parts = cell.split('|');
        let (Some(service), Some(surface)) = (parts.next(), parts.next()) else {
            continue;
        };
        let path = surface.split_once(' ').map_or(surface, |(_, path)| path);
        let key = format!("{service}|{path}");
        if let Some(sources) = surface_sources.get(&key) {
            if changed
                .iter()
                .any(|change| sources.iter().any(|source| source_matches(change, source)))
            {
                selected.insert(cell.clone());
            }
        }
    }
    let unmatched: Vec<String> = changed
        .iter()
        .filter(|change| {
            !surface_sources
                .values()
                .flatten()
                .any(|source| source_matches(change, source))
        })
        .cloned()
        .collect();
    for cell in CORE_CELLS {
        if all_cells.contains(cell) {
            selected.insert(cell.to_string());
        }
    }
    let skipped_classes: BTreeSet<String> = all_cells
        .iter()
        .map(|cell| cell.split('|').nth(2).unwrap_or("").to_string())
        .filter(|class| {
            !selected
                .iter()
                .any(|cell| cell.split('|').nth(2) == Some(class.as_str()))
        })
        .collect();
    Relevance {
        selected,
        skipped_classes,
        unmatched,
    }
}

/// The services whose declared route sources intersect `changed`, sorted.
/// Used by the static gate to run route/session/fixture parity for the
/// changed subset only.
fn services_for_changes(
    surface_sources: &BTreeMap<String, Vec<String>>,
    changed: &[String],
) -> BTreeSet<String> {
    let mut services = BTreeSet::new();
    for (key, sources) in surface_sources {
        let service = key.split('|').next().unwrap_or("");
        if changed
            .iter()
            .any(|change| sources.iter().any(|source| source_matches(change, source)))
        {
            services.insert(service.to_string());
        }
    }
    services
}

/// Resolve the changed repo paths from CLI arguments: `--changed <path>`
/// (repeatable) and `--git-diff <base>..<head>`. The paths are sorted and
/// de-duplicated so the selection is stable.
fn changed_paths_from_args(args: &[String]) -> Result<Vec<String>, String> {
    let mut changed: Vec<String> = Vec::new();
    let mut git_diff: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--changed" => {
                let value = args
                    .get(index + 1)
                    .ok_or("--changed needs a repo-relative path")?;
                changed.push(value.clone());
                index += 2;
            }
            "--git-diff" => {
                let value = args
                    .get(index + 1)
                    .ok_or("--git-diff needs a <base>..<head> range")?;
                git_diff = Some(value.clone());
                index += 2;
            }
            _ => index += 1,
        }
    }
    if let Some(spec) = git_diff {
        changed.extend(git_diff_paths(&spec)?);
    }
    changed.sort();
    changed.dedup();
    Ok(changed)
}

/// The repo-relative paths changed between two git revisions.
fn git_diff_paths(spec: &str) -> Result<Vec<String>, String> {
    let output = std::process::Command::new("git")
        .args(["diff", "--name-only", spec])
        .output()
        .map_err(|error| format!("git diff --name-only {spec}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git diff --name-only {spec} exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn check_route_parity<'a>(
    surfaces: &'a SurfacesFile,
    registered: &BTreeMap<&str, Vec<RegisteredRoute>>,
    only: Option<&BTreeSet<String>>,
    problems: &mut Vec<String>,
) -> BTreeMap<&'a str, BTreeSet<String>> {
    let mut declared_by_service: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for (service, declared) in &surfaces.services {
        if !SOURCES.iter().any(|(id, _, _)| id == service) {
            continue; // the unknown-service problem is already recorded
        }
        let declared_surfaces = declared_route_surfaces(service, &declared.routes, problems);
        let Some(actual_routes) = registered.get(service.as_str()) else {
            continue;
        };
        let report = only.is_none_or(|set| set.contains(service));
        let actual: BTreeSet<String> = actual_routes
            .iter()
            .flat_map(|route| {
                route
                    .methods
                    .iter()
                    .map(move |method| format!("{method} {}", route.path))
            })
            .collect();
        if report {
            for surface in actual.difference(&declared_surfaces) {
                problems.push(format!(
                    "registered route not declared in {SURFACES_REL}: {service} {surface}"
                ));
            }
            for surface in declared_surfaces.difference(&actual) {
                problems.push(format!(
                    "declared route is no longer registered (stale declaration): {service} {surface}"
                ));
            }
        }
        declared_by_service.insert(service, declared_surfaces);
    }
    declared_by_service
}

fn check_fixture_parity(
    surfaces: &SurfacesFile,
    fixture_counts: &BTreeMap<&str, usize>,
    only: Option<&BTreeSet<String>>,
    problems: &mut Vec<String>,
) {
    for (service, declared) in &surfaces.services {
        if only.is_some_and(|set| !set.contains(service)) {
            continue;
        }
        let Some(actual) = fixture_counts.get(service.as_str()) else {
            continue;
        };
        if declared.fixture_pages != *actual {
            problems.push(format!(
                "{service}: fixture_pages declares {} but the render-fixtures bin declares PAGES len {actual}",
                declared.fixture_pages
            ));
        }
    }
}

/// Validate every `session_required` marker: present only as `true`, only on
/// a declared `apex-api` console `GET`, and never on a documented public
/// console path. The marker is the recorded expectation the live harness
/// asserts (anonymous `303 /console/ledger`, session `200`), so a marker the
/// product cannot honor — or one on a route that is documented public — must
/// fail here, before any live run repeats it.
fn check_session_required(
    surfaces: &SurfacesFile,
    only: Option<&BTreeSet<String>>,
    problems: &mut Vec<String>,
) {
    for (service, declared) in &surfaces.services {
        if only.is_some_and(|set| !set.contains(service)) {
            continue;
        }
        for route in &declared.routes {
            match route.session_required {
                None => continue,
                Some(false) => {
                    problems.push(format!(
                        "{SURFACES_REL}: {service} {}: session_required is present but false; the marker records a real gate, so it must be true when present",
                        route.path
                    ));
                    continue;
                }
                Some(true) => {}
            }
            if service != "apex-api" {
                problems.push(format!(
                    "{SURFACES_REL}: {service} {}: session_required marks the publisher console only; service '{service}' has no such gate",
                    route.path
                ));
                continue;
            }
            let is_console = route.path == "/" || route.path.starts_with("/console");
            if !is_console {
                problems.push(format!(
                    "{SURFACES_REL}: {service} {}: session_required may only mark a publisher-console route (the console serves '/' and '/console/...')",
                    route.path
                ));
            }
            if !route
                .methods
                .iter()
                .any(|method| method.eq_ignore_ascii_case("GET"))
            {
                problems.push(format!(
                    "{SURFACES_REL}: {service} {}: session_required must reference a declared GET route",
                    route.path
                ));
            }
            if PUBLIC_CONSOLE_GETS.contains(&route.path.as_str()) {
                problems.push(format!(
                    "{SURFACES_REL}: {service} {}: session_required marks a documented public route; the console exception list stays public",
                    route.path
                ));
            }
        }
    }
}

/// Expand the per-service `required` cells into
/// `"<service>|<surface>|<class>"` ids, validating every class and surface
/// reference on the way.
fn expand_required_cells(
    surfaces: &SurfacesFile,
    declared_surfaces: &BTreeMap<&str, BTreeSet<String>>,
    problems: &mut Vec<String>,
) -> BTreeSet<String> {
    let declared_classes: BTreeSet<&str> =
        surfaces.probe_classes.iter().map(String::as_str).collect();
    let mut cells = BTreeSet::new();
    for (service, declared) in &surfaces.services {
        if !SOURCES.iter().any(|(id, _, _)| id == service) {
            continue;
        }
        let valid_surfaces = declared_surfaces.get(service.as_str());
        for cell in &declared.required {
            if cell.classes.is_empty() {
                problems.push(format!(
                    "{SURFACES_REL}: {service}: required cell '{}' lists no classes",
                    cell.surface
                ));
            }
            let surface_is_valid = valid_surfaces.is_some_and(|set| set.contains(&cell.surface));
            if !surface_is_valid {
                problems.push(format!(
                    "{SURFACES_REL}: {service}: required cell references unknown surface '{}'",
                    cell.surface
                ));
            }
            let mut seen = BTreeSet::new();
            for class in &cell.classes {
                if !seen.insert(class.as_str()) {
                    problems.push(format!(
                        "{SURFACES_REL}: {service}: required cell '{}' repeats class '{class}'",
                        cell.surface
                    ));
                    continue;
                }
                if !declared_classes.contains(class.as_str()) {
                    problems.push(format!(
                        "{SURFACES_REL}: {service}: required cell '{}' references undeclared class '{class}'",
                        cell.surface
                    ));
                    continue;
                }
                if surface_is_valid {
                    cells.insert(format!("{service}|{}|{class}", cell.surface));
                }
            }
        }
    }
    cells
}

/// The baseline ratchet. `cells` is the recorded requirement set: every cell
/// must still be required by `surfaces.json`, unless a `removals` entry
/// names it with a reason. A removal entry must name a recorded baseline
/// cell (an entry for an unknown cell authorizes nothing and would otherwise
/// pre-approve a future silent shrink) and must not cover a cell that is
/// required again (it is stale until deleted).
fn check_baseline(
    baseline: &BaselineFile,
    required: &BTreeSet<String>,
    problems: &mut Vec<String>,
) {
    let mut cells: BTreeSet<&str> = BTreeSet::new();
    for cell in &baseline.cells {
        if !cells.insert(cell.as_str()) {
            problems.push(format!("{BASELINE_REL}: cell '{cell}' is listed twice"));
        }
        if parse_cell_id(cell).is_none() {
            problems.push(format!(
                "{BASELINE_REL}: malformed cell id '{cell}' (expected <service>|<surface>|<class>)"
            ));
        }
    }
    let mut removed: BTreeSet<&str> = BTreeSet::new();
    for removal in &baseline.removals {
        if removal.reason.trim().is_empty() {
            problems.push(format!(
                "{BASELINE_REL}: removal '{}' records no reason",
                removal.cell
            ));
        }
        if parse_cell_id(&removal.cell).is_none() {
            problems.push(format!(
                "{BASELINE_REL}: malformed removal cell id '{}'",
                removal.cell
            ));
            continue;
        }
        if !cells.contains(removal.cell.as_str()) {
            problems.push(format!(
                "{BASELINE_REL}: removal '{}' does not name a recorded baseline cell; an entry can only release a recorded requirement",
                removal.cell
            ));
        }
        if required.contains(&removal.cell) {
            problems.push(format!(
                "{BASELINE_REL}: removal '{}' is stale (the cell is declared required again); delete the entry",
                removal.cell
            ));
        }
        removed.insert(removal.cell.as_str());
    }
    for cell in &cells {
        if required.contains(*cell) || removed.contains(cell) {
            continue;
        }
        problems.push(format!(
            "{BASELINE_REL}: required baseline cell '{cell}' is no longer declared in {SURFACES_REL} and has no recorded removal with a reason"
        ));
    }
}

/// Run the static parity gate: `tools/dogfood/surfaces.json` against the real
/// route registrations, fixture page lists, the frozen probe corpus and the
/// recorded baseline.
///
/// Flags are read from the process arguments (`--changed <path>`,
/// `--git-diff <base>..<head>`), mirroring the live harness. With no flag the
/// whole catalog is checked; with a change set the route, session-marker and
/// fixture-page checks run for the services the change set touches only, and
/// the schema, corpus and baseline checks always run.
pub fn dogfood_plan(repo: &Repo) -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let changed = changed_paths_from_args(&args)?;
    dogfood_plan_with(repo, &changed)
}

/// [`dogfood_plan`] with an explicit change set (`[]` means the whole
/// catalog). Exposed so the relevance selection is testable without touching
/// the process arguments.
pub fn dogfood_plan_with(repo: &Repo, changed: &[String]) -> Result<(), String> {
    if !repo.exists(SURFACES_REL) {
        return Err("the live dogfood surface catalog is missing".into());
    }
    let surfaces: SurfacesFile = serde_json::from_str(&repo.read(SURFACES_REL)?)
        .map_err(|e| format!("{SURFACES_REL}: {e}"))?;
    let mut problems: Vec<String> = Vec::new();
    check_probe_classes(&surfaces, &mut problems);
    for (service, _, _) in SOURCES {
        if !surfaces.services.contains_key(service) {
            problems.push(format!(
                "{SURFACES_REL}: service '{service}' is not declared"
            ));
        }
    }
    for service in surfaces.services.keys() {
        if !SOURCES.iter().any(|(id, _, _)| id == service) {
            problems.push(format!(
                "{SURFACES_REL}: unknown service '{service}' (known: {})",
                SOURCES
                    .iter()
                    .map(|(id, _, _)| *id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let mut registered: BTreeMap<&str, Vec<RegisteredRoute>> = BTreeMap::new();
    let mut fixture_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for (service, route_rel, fixture_rel) in SOURCES {
        registered.insert(service, extract_routes(&repo.read(route_rel)?, route_rel)?);
        fixture_counts.insert(
            service,
            extract_fixture_page_count(&repo.read(fixture_rel)?, fixture_rel)?,
        );
    }
    // The per-surface source map is always built and validated (schema), and it
    // is what narrows the route/session/fixture checks to the changed subset.
    let surface_sources = declared_route_sources(&surfaces, &mut problems);
    let only: Option<BTreeSet<String>> = if changed.is_empty() {
        None
    } else {
        Some(services_for_changes(&surface_sources, changed))
    };
    let declared_surfaces =
        check_route_parity(&surfaces, &registered, only.as_ref(), &mut problems);
    check_fixture_parity(&surfaces, &fixture_counts, only.as_ref(), &mut problems);
    check_session_required(&surfaces, only.as_ref(), &mut problems);
    let required_cells = expand_required_cells(&surfaces, &declared_surfaces, &mut problems);
    let baseline: Option<BaselineFile> = if repo.exists(BASELINE_REL) {
        Some(
            serde_json::from_str(&repo.read(BASELINE_REL)?)
                .map_err(|e| format!("{BASELINE_REL}: {e}"))?,
        )
    } else {
        problems.push(format!(
            "{BASELINE_REL} is missing; the baseline ratchet cannot be verified"
        ));
        None
    };
    if let Some(baseline) = &baseline {
        check_baseline(baseline, &required_cells, &mut problems);
    }
    if !problems.is_empty() {
        return Err(format!(
            "DOGFOOD-PLAN FAILED ({} mismatch(es)):\n  - {}",
            problems.len(),
            problems.join("\n  - ")
        ));
    }
    let relevance = select_cells(&required_cells, &surface_sources, changed);
    println!(
        "dogfood-plan PASS: routes registered/declared {}; fixture pages declared/actual {}; probe classes {}/{}; cells required={} baseline={} removals={}",
        route_counts(&registered, &surfaces),
        fixture_summary(&fixture_counts, &surfaces),
        surfaces.probe_classes.len(),
        REQUIRED_PROBE_CLASSES.len(),
        required_cells.len(),
        baseline.as_ref().map_or(0, |b| b.cells.len()),
        baseline.as_ref().map_or(0, |b| b.removals.len())
    );
    if !changed.is_empty() {
        let services = only.unwrap_or_default();
        println!(
            "dogfood-plan relevance: selected {} of {} cells (skipped {} unrelated); changed services [{}]; skipped classes [{}]",
            relevance.selected.len(),
            required_cells.len(),
            required_cells.len().saturating_sub(relevance.selected.len()),
            services.iter().cloned().collect::<Vec<_>>().join(", "),
            relevance
                .skipped_classes
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        );
        if !relevance.unmatched.is_empty() {
            println!(
                "dogfood-plan relevance: {} changed path(s) matched no declared source (schema checks still ran): [{}]",
                relevance.unmatched.len(),
                relevance.unmatched.join(", ")
            );
        }
    }
    Ok(())
}

fn route_counts(
    registered: &BTreeMap<&str, Vec<RegisteredRoute>>,
    surfaces: &SurfacesFile,
) -> String {
    let mut parts = Vec::new();
    for (service, _, _) in SOURCES {
        let registered_count = registered.get(service).map_or(0, |list| list.len());
        let declared_count = surfaces
            .services
            .get(service)
            .map_or(0, |declared| declared.routes.len());
        parts.push(format!("{service}={registered_count}/{declared_count}"));
    }
    parts.join(" ")
}

fn fixture_summary(fixture_counts: &BTreeMap<&str, usize>, surfaces: &SurfacesFile) -> String {
    let mut parts = Vec::new();
    for (service, _, _) in SOURCES {
        let declared = surfaces
            .services
            .get(service)
            .map_or(0, |declared| declared.fixture_pages);
        let actual = fixture_counts.get(service).copied().unwrap_or(0);
        parts.push(format!("{service}={declared}/{actual}"));
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surfaces_json(probe: &[&str], services_json: &str) -> String {
        format!(
            "{{\"probe_classes\":[{}],\"services\":{{{services_json}}}}}",
            probe
                .iter()
                .map(|c| format!("\"{c}\""))
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    fn full_probe_list() -> Vec<&'static str> {
        REQUIRED_PROBE_CLASSES.to_vec()
    }

    #[test]
    fn probe_registry_is_pinned_to_the_frozen_sixteen() {
        assert_eq!(
            REQUIRED_PROBE_CLASSES.to_vec(),
            vec![
                "surface-parity",
                "response-contract",
                "no-inline",
                "assets-fonts",
                "auth-authority",
                "input-corpus",
                "csrf-idempotency",
                "traversal",
                "escaping",
                "concurrency",
                "failure-injection",
                "differential-truth",
                "tenant-isolation",
                "auth-surface",
                "signin-ownership",
                "payment-activation",
            ]
        );
    }

    #[test]
    fn core_cells_are_declared_requirements() {
        // The always-run relevance core names real cells; a typo would make the
        // core silently empty.
        for cell in CORE_CELLS {
            assert_eq!(cell.split('|').count(), 3, "{cell}");
            assert!(
                parse_cell_id(cell).is_some(),
                "core cell is malformed: {cell}"
            );
        }
        assert_eq!(CORE_CELLS.len(), 5);
    }

    fn relevance_fixture() -> (BTreeSet<String>, BTreeMap<String, Vec<String>>) {
        let cells: BTreeSet<String> = [
            "apex-api|GET /console/audit|response-contract",
            "apex-api|GET /console/audit|tenant-isolation",
            "apex-api|GET /console/audit|auth-surface",
            "apex-api|GET /|tenant-isolation",
            "apex-api|GET /console/ledger|tenant-isolation",
            "apex-api|GET /healthz|surface-parity",
            "apex-api|GET /healthz|response-contract",
            "apex-api|POST /watch|failure-injection",
            "apex-sales-web|GET /leads|response-contract",
        ]
        .iter()
        .map(|cell| cell.to_string())
        .collect();
        let sources: BTreeMap<String, Vec<String>> = [
            (
                "apex-api|/console/audit",
                vec![
                    "rust/crates/apex-api/src/routes.rs",
                    "rust/crates/apex-api/src/html/",
                ],
            ),
            (
                "apex-api|/",
                vec!["rust/crates/apex-api/src/html/overview.rs"],
            ),
            (
                "apex-api|/console/ledger",
                vec!["rust/crates/apex-api/src/html/ledger.rs"],
            ),
            (
                "apex-api|/healthz",
                vec!["rust/crates/apex-api/src/routes.rs"],
            ),
            (
                "apex-api|/watch",
                vec![
                    "rust/crates/apex-watch/src/fetch.rs",
                    "rust/crates/apex-watch/src/guard.rs",
                ],
            ),
            (
                "apex-sales-web|/leads",
                vec!["rust/crates/apex-sales-web/src/routes.rs"],
            ),
        ]
        .into_iter()
        .map(|(key, values)| {
            (
                key.to_string(),
                values.iter().map(|v| v.to_string()).collect::<Vec<_>>(),
            )
        })
        .collect();
        (cells, sources)
    }

    #[test]
    fn relevance_selects_a_changed_console_page_and_skips_sales() {
        let (cells, sources) = relevance_fixture();
        let changed = vec!["rust/crates/apex-api/src/html/audit.rs".to_string()];
        let relevance = select_cells(&cells, &sources, &changed);
        assert!(
            relevance
                .selected
                .contains("apex-api|GET /console/audit|response-contract"),
            "{:?}",
            relevance.selected
        );
        assert!(
            relevance
                .selected
                .contains("apex-api|GET /console/audit|tenant-isolation"),
            "{:?}",
            relevance.selected
        );
        assert!(
            relevance
                .selected
                .contains("apex-api|GET /|tenant-isolation"),
            "the always-run core tenant cell is selected"
        );
        assert!(
            !relevance
                .selected
                .contains("apex-sales-web|GET /leads|response-contract"),
            "the sales web is untouched"
        );
        assert!(
            relevance.skipped_classes.contains("failure-injection"),
            "{:?}",
            relevance.skipped_classes
        );
        assert!(relevance.unmatched.is_empty(), "{:?}", relevance.unmatched);
    }

    #[test]
    fn relevance_selects_the_ssrf_fetch_cell_by_its_source() {
        let (cells, sources) = relevance_fixture();
        let changed = vec!["rust/crates/apex-watch/src/fetch.rs".to_string()];
        let relevance = select_cells(&cells, &sources, &changed);
        assert!(
            relevance
                .selected
                .contains("apex-api|POST /watch|failure-injection"),
            "{:?}",
            relevance.selected
        );
        assert!(!relevance
            .selected
            .contains("apex-sales-web|GET /leads|response-contract"));
    }

    #[test]
    fn relevance_without_paths_selects_everything_and_an_unknown_path_selects_the_core() {
        let (cells, sources) = relevance_fixture();
        let all = select_cells(&cells, &sources, &[]);
        assert_eq!(all.selected, cells);
        assert!(all.skipped_classes.is_empty());
        assert!(all.unmatched.is_empty());

        let unknown = vec!["docs/nowhere.md".to_string()];
        let relevance = select_cells(&cells, &sources, &unknown);
        let core: BTreeSet<String> = CORE_CELLS
            .iter()
            .filter(|cell| cells.contains(**cell))
            .map(|cell| (*cell).to_string())
            .collect();
        assert_eq!(
            relevance.selected, core,
            "unknown path selects the core only"
        );
        assert_eq!(relevance.unmatched, unknown);
        assert!(relevance.selected.len() < cells.len());
        assert_eq!(
            select_cells(&cells, &sources, &unknown),
            relevance,
            "the selection is deterministic"
        );
    }

    #[test]
    fn relevance_source_matching_is_prefix_aware_and_exact() {
        assert!(source_matches(
            "rust/crates/apex-api/src/html/audit.rs",
            "rust/crates/apex-api/src/html/"
        ));
        assert!(source_matches(
            "rust/crates/apex-api/src/html/",
            "rust/crates/apex-api/src/html/"
        ));
        assert!(!source_matches(
            "rust/crates/apex-api/src/html_extra/x.rs",
            "rust/crates/apex-api/src/html/"
        ));
        assert!(source_matches(
            "rust/crates/apex-watch/src/fetch.rs",
            "rust/crates/apex-watch/src/fetch.rs"
        ));
        assert!(!source_matches(
            "rust/crates/apex-watch/src/fetch.rs.bak",
            "rust/crates/apex-watch/src/fetch.rs"
        ));
    }

    #[test]
    fn changed_paths_from_args_parses_repeatable_and_git_diff_flags() {
        let args: Vec<String> = ["--changed", "a/b.rs", "--changed", "c/d.rs"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let changed = changed_paths_from_args(&args).unwrap();
        assert_eq!(changed, vec!["a/b.rs".to_string(), "c/d.rs".to_string()]);
        let args: Vec<String> = ["--changed"].iter().map(|s| s.to_string()).collect();
        assert!(changed_paths_from_args(&args).is_err());
    }

    #[test]
    fn chained_methods_nested_paths_and_verbatim_params_extract() {
        let text = concat!(
            "Router::new()\n",
            "    .route(\"/\", get(root))\n",
            "    .route(\"/v1/audit/{platform}\", get(audit_report).post(live_audit))\n",
            "    .route(\n",
            "        \"/control/actions/{kind}/result/{id}\",\n",
            "        get(action_result),\n",
            "    )\n",
            "    .route(\"/console/ledger\", get(console_ledger).post(console_ledger_open))\n",
        );
        let routes = extract_routes(text, "t.rs").unwrap();
        assert_eq!(
            routes,
            vec![
                RegisteredRoute {
                    path: "/".into(),
                    methods: vec!["GET".into()],
                },
                RegisteredRoute {
                    path: "/console/ledger".into(),
                    methods: vec!["GET".into(), "POST".into()],
                },
                RegisteredRoute {
                    path: "/control/actions/{kind}/result/{id}".into(),
                    methods: vec!["GET".into()],
                },
                RegisteredRoute {
                    path: "/v1/audit/{platform}".into(),
                    methods: vec!["GET".into(), "POST".into()],
                },
            ]
        );
    }

    #[test]
    fn hostile_formatting_is_handled_but_comments_are_not_routes() {
        let text = concat!(
            "// .route(\"/commented\", get(nope))\n",
            "let x = Router::new()\n",
            "  .route( \"/spaced\" ,  get(a) . post( b ) )\n",
            "  .route(\"crate::nested\", get(crate::favicon::ico));\n",
        );
        let routes = extract_routes(text, "t.rs").unwrap();
        assert_eq!(
            routes,
            vec![
                RegisteredRoute {
                    path: "/spaced".into(),
                    methods: vec!["GET".into(), "POST".into()],
                },
                RegisteredRoute {
                    path: "crate::nested".into(),
                    methods: vec!["GET".into()],
                },
            ]
        );
    }

    #[test]
    fn unclassifiable_route_forms_are_hard_errors_not_silent_skips() {
        let err = extract_routes("Router::new().route(PATH, get(x))\n", "t.rs").unwrap_err();
        assert!(err.contains("unrecognized registration form"), "{err}");
        let err = extract_routes("let router = Router::new();\n", "t.rs").unwrap_err();
        assert!(err.contains("no .route( registrations"), "{err}");
        let err = extract_routes(
            "Router::new().route(\"/a\", get(x)).route(\"/a\", get(y))\n",
            "t.rs",
        )
        .unwrap_err();
        assert!(err.contains("twice"), "{err}");
    }

    #[test]
    fn fixture_page_count_parses_and_absence_is_a_hard_error() {
        let text = "/// Every fixture page this binary renders.\npub const PAGES: [&str; 60] = [\n    \"a.html\",\n];\n";
        assert_eq!(extract_fixture_page_count(text, "b.rs").unwrap(), 60);
        let err = extract_fixture_page_count("fn main() {}\n", "b.rs").unwrap_err();
        assert!(err.contains("no `pub const PAGES"), "{err}");
        let two = concat!(
            "pub const PAGES: [&str; 3] = [\"a\"];\n",
            "pub const PAGES: [&str; 4] = [\"b\"];\n",
        );
        let err = extract_fixture_page_count(two, "b.rs").unwrap_err();
        assert!(err.contains("exactly one PAGES array"), "{err}");
    }

    #[test]
    fn corpus_equality_fails_on_reordered_or_edited_lists() {
        let mut problems = Vec::new();
        let mut reordered = full_probe_list();
        reordered.swap(0, 1);
        check_probe_classes(
            &SurfacesFile {
                probe_classes: reordered.iter().map(|c| c.to_string()).collect(),
                services: BTreeMap::new(),
            },
            &mut problems,
        );
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("order is part of the equality"));
        let mut problems = Vec::new();
        let mut shorter = full_probe_list();
        shorter.remove(11);
        check_probe_classes(
            &SurfacesFile {
                probe_classes: shorter.iter().map(|c| c.to_string()).collect(),
                services: BTreeMap::new(),
            },
            &mut problems,
        );
        assert!(problems[0].contains("differential-truth"), "{problems:?}");
    }

    #[test]
    fn route_parity_fails_in_both_directions_with_the_exact_route() {
        let surfaces = SurfacesFile {
            probe_classes: full_probe_list().iter().map(|c| c.to_string()).collect(),
            services: [(
                "apex-api".to_string(),
                DeclaredService {
                    routes: vec![
                        DeclaredRoute {
                            path: "/a".into(),
                            methods: vec!["GET".into()],
                            session_required: None,
                            sources: vec!["rust/crates/apex-api/src/routes.rs".to_string()],
                        },
                        DeclaredRoute {
                            path: "/stale".into(),
                            methods: vec!["POST".into()],
                            session_required: None,
                            sources: vec!["rust/crates/apex-api/src/routes.rs".to_string()],
                        },
                    ],
                    fixture_pages: 0,
                    required: vec![],
                },
            )]
            .into_iter()
            .collect(),
        };
        let registered: BTreeMap<&str, Vec<RegisteredRoute>> = [(
            "apex-api",
            vec![
                RegisteredRoute {
                    path: "/a".into(),
                    methods: vec!["GET".into()],
                },
                RegisteredRoute {
                    path: "/undeclared".into(),
                    methods: vec!["GET".into()],
                },
            ],
        )]
        .into_iter()
        .collect();
        let mut problems = Vec::new();
        check_route_parity(&surfaces, &registered, None, &mut problems);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(
            problems
                .iter()
                .any(|p| p.contains("apex-api GET /undeclared")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("apex-api POST /stale")),
            "{problems:?}"
        );
    }

    #[test]
    fn required_cells_must_name_declared_classes_and_surfaces() {
        let surfaces = SurfacesFile {
            probe_classes: full_probe_list().iter().map(|c| c.to_string()).collect(),
            services: [(
                "apex-api".to_string(),
                DeclaredService {
                    routes: vec![DeclaredRoute {
                        path: "/a".into(),
                        methods: vec!["GET".into()],
                        session_required: None,
                        sources: vec!["rust/crates/apex-api/src/routes.rs".to_string()],
                    }],
                    fixture_pages: 0,
                    required: vec![
                        RequiredCell {
                            surface: "GET /a".into(),
                            classes: vec!["surface-parity".into(), "not-a-class".into()],
                        },
                        RequiredCell {
                            surface: "POST /a".into(),
                            classes: vec!["surface-parity".into()],
                        },
                    ],
                },
            )]
            .into_iter()
            .collect(),
        };
        let declared = [("apex-api", ["GET /a".to_string()].into_iter().collect())]
            .into_iter()
            .collect();
        let mut problems = Vec::new();
        let cells = expand_required_cells(&surfaces, &declared, &mut problems);
        assert_eq!(cells.len(), 1);
        assert!(cells.contains("apex-api|GET /a|surface-parity"));
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("not-a-class")));
        assert!(problems.iter().any(|p| p.contains("POST /a")));
    }

    #[test]
    fn baseline_ratchet_requires_a_reasoned_removal_to_shrink() {
        let required: BTreeSet<String> = ["apex-api|GET /a|surface-parity".to_string()]
            .into_iter()
            .collect();
        let shrunk = BaselineFile {
            cells: vec![
                "apex-api|GET /a|surface-parity".into(),
                "apex-api|GET /b|escaping".into(),
            ],
            removals: vec![],
        };
        let mut problems = Vec::new();
        check_baseline(&shrunk, &required, &mut problems);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("apex-api|GET /b|escaping"));

        let mut reasoned = BaselineFile {
            cells: vec![
                "apex-api|GET /a|surface-parity".into(),
                "apex-api|GET /b|escaping".into(),
            ],
            removals: vec![BaselineRemoval {
                cell: "apex-api|GET /b|escaping".into(),
                reason: "surface retired with the b endpoint".into(),
            }],
        };
        let mut problems = Vec::new();
        check_baseline(&reasoned, &required, &mut problems);
        assert!(problems.is_empty(), "{problems:?}");

        reasoned.removals[0].reason = "   ".into();
        let mut problems = Vec::new();
        check_baseline(&reasoned, &required, &mut problems);
        assert!(problems.iter().any(|p| p.contains("records no reason")));

        reasoned.removals[0].reason = "ok".into();
        reasoned.removals[0].cell = "apex-api|GET /b|other-class".into();
        let mut problems = Vec::new();
        check_baseline(&reasoned, &required, &mut problems);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("does not name a recorded baseline cell")),
            "{problems:?}"
        );

        let mut stale = BaselineFile {
            cells: vec!["apex-api|GET /a|surface-parity".into()],
            removals: vec![BaselineRemoval {
                cell: "apex-api|GET /a|surface-parity".into(),
                reason: "old shrink".into(),
            }],
        };
        let mut problems = Vec::new();
        check_baseline(&stale, &required, &mut problems);
        assert!(problems.iter().any(|p| p.contains("stale")), "{problems:?}");
        stale.removals.clear();
        let mut problems = Vec::new();
        check_baseline(&stale, &required, &mut problems);
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn session_required_markers_are_validated_against_the_product_gate() {
        let surfaces =
            |service: &str, path: &str, methods: &[&str], marker: Option<bool>| SurfacesFile {
                probe_classes: full_probe_list().iter().map(|c| c.to_string()).collect(),
                services: [(
                    service.to_string(),
                    DeclaredService {
                        routes: vec![DeclaredRoute {
                            path: path.into(),
                            methods: methods.iter().map(|m| (*m).into()).collect(),
                            session_required: marker,
                            sources: vec!["rust/crates/apex-api/src/routes.rs".to_string()],
                        }],
                        fixture_pages: 0,
                        required: vec![],
                    },
                )]
                .into_iter()
                .collect(),
            };

        // The valid case: an apex-api console GET records the gate.
        let mut problems = Vec::new();
        check_session_required(
            &surfaces("apex-api", "/console/audit", &["GET"], Some(true)),
            None,
            &mut problems,
        );
        assert!(problems.is_empty(), "{problems:?}");

        // A marker on a documented public route fails.
        let mut problems = Vec::new();
        check_session_required(
            &surfaces("apex-api", "/console/ledger", &["GET"], Some(true)),
            None,
            &mut problems,
        );
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("documented public route"),
            "{problems:?}"
        );

        // A marker must reference a declared GET route.
        let mut problems = Vec::new();
        check_session_required(
            &surfaces("apex-api", "/console/settings/theme", &["POST"], Some(true)),
            None,
            &mut problems,
        );
        assert!(
            problems.iter().any(|p| p.contains("declared GET route")),
            "{problems:?}"
        );

        // `session_required: false` is schema drift: the marker records a
        // real gate, so it is present only as true.
        let mut problems = Vec::new();
        check_session_required(
            &surfaces("apex-api", "/console/audit", &["GET"], Some(false)),
            None,
            &mut problems,
        );
        assert!(
            problems.iter().any(|p| p.contains("present but false")),
            "{problems:?}"
        );

        // The publisher console is the only service with this gate.
        let mut problems = Vec::new();
        check_session_required(
            &surfaces("apex-control", "/console/audit", &["GET"], Some(true)),
            None,
            &mut problems,
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("publisher console only")),
            "{problems:?}"
        );

        // A non-console path cannot carry the marker.
        let mut problems = Vec::new();
        check_session_required(
            &surfaces("apex-api", "/healthz", &["GET"], Some(true)),
            None,
            &mut problems,
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("publisher-console route")),
            "{problems:?}"
        );
    }

    #[test]
    fn unknown_route_fields_are_schema_drift_not_silent_ignores() {
        let err = serde_json::from_str::<SurfacesFile>(
            r#"{"probe_classes":[],"services":{"apex-api":{"routes":[{"path":"/a","methods":["GET"],"session_require":true}],"fixture_pages":0,"required":[]}}}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
    }

    #[test]
    fn session_required_marker_round_trips_through_the_end_to_end_gate() {
        let (repo, root) = temp_repo("session-marker");
        write_services(&root);
        write(
            &root.join(SOURCES[0].1),
            "pub fn router() {\n    Router::new().route(\"/console/audit\", get(root))\n}\n",
        );
        let mut declared: serde_json::Value = serde_json::from_str(&end_to_end_surfaces()).unwrap();
        declared["services"]["apex-api"]["routes"] = serde_json::json!([
            {"path": "/console/audit", "methods": ["GET"], "session_required": true, "sources": ["rust/crates/apex-api/src/routes.rs", "rust/crates/apex-api/src/html/"]}
        ]);
        declared["services"]["apex-api"]["required"] = serde_json::json!([
            {"surface": "GET /console/audit", "classes": ["surface-parity"]}
        ]);
        write(
            &root.join(SURFACES_REL),
            &serde_json::to_string(&declared).unwrap(),
        );
        write(
            &root.join(BASELINE_REL),
            r#"{"cells":["apex-api|GET /console/audit|surface-parity"],"removals":[]}"#,
        );
        assert_eq!(dogfood_plan(&repo), Ok(()));

        // The same marker on the documented public access panel fails the
        // whole gate.
        declared["services"]["apex-api"]["routes"] = serde_json::json!([
            {"path": "/console/ledger", "methods": ["GET"], "session_required": true, "sources": ["rust/crates/apex-api/src/routes.rs"]}
        ]);
        declared["services"]["apex-api"]["required"] = serde_json::json!([
            {"surface": "GET /console/ledger", "classes": ["surface-parity"]}
        ]);
        write(
            &root.join(SOURCES[0].1),
            "pub fn router() {\n    Router::new().route(\"/console/ledger\", get(root))\n}\n",
        );
        write(
            &root.join(SURFACES_REL),
            &serde_json::to_string(&declared).unwrap(),
        );
        write(
            &root.join(BASELINE_REL),
            r#"{"cells":["apex-api|GET /console/ledger|surface-parity"],"removals":[]}"#,
        );
        let err = dogfood_plan(&repo).unwrap_err();
        assert!(err.contains("documented public route"), "{err}");

        // A typo'd marker field is a parse failure, never a silently ignored
        // declaration.
        declared["services"]["apex-api"]["routes"] = serde_json::json!([
            {"path": "/console/audit", "methods": ["GET"], "session_require": true, "sources": ["rust/crates/apex-api/src/routes.rs"]}
        ]);
        write(
            &root.join(SURFACES_REL),
            &serde_json::to_string(&declared).unwrap(),
        );
        let err = dogfood_plan(&repo).unwrap_err();
        assert!(err.contains("unknown field"), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn write(path: &std::path::Path, text: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, text).unwrap();
    }

    fn temp_repo(name: &str) -> (Repo, std::path::PathBuf) {
        let root =
            std::env::temp_dir().join(format!("apex-dogfood-plan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        (Repo { root: root.clone() }, root)
    }

    fn write_services(root: &std::path::Path) {
        for (service, route_rel, fixture_rel) in SOURCES {
            write(
                &root.join(route_rel),
                &format!(
                    "pub fn router() {{\n    Router::new().route(\"/{service}\", get(root))\n}}\n"
                ),
            );
            write(
                &root.join(fixture_rel),
                &format!("pub const PAGES: [&str; 7] = [\n    \"{service}\",\n];\n"),
            );
        }
    }

    fn end_to_end_surfaces() -> String {
        let service = |path: &str| {
            format!(
                "{{\"routes\":[{{\"path\":\"/{path}\",\"methods\":[\"GET\"],\"sources\":[\"rust/crates/{path}/src/routes.rs\"]}}],\"fixture_pages\":7,\"required\":[{{\"surface\":\"GET /{path}\",\"classes\":[\"surface-parity\"]}}]}}"
            )
        };
        surfaces_json(
            &full_probe_list(),
            &format!(
                "\"apex-api\":{},\"apex-control\":{},\"apex-sales-web\":{}",
                service("apex-api"),
                service("apex-control"),
                service("apex-sales-web")
            ),
        )
    }

    fn valid_baseline() -> &'static str {
        r#"{"cells":["apex-api|GET /apex-api|surface-parity"],"removals":[]}"#
    }

    #[test]
    fn missing_surface_catalog_fails_with_the_pinned_message() {
        let (repo, root) = temp_repo("missing");
        let err = dogfood_plan(&repo).unwrap_err();
        assert_eq!(err, "the live dogfood surface catalog is missing");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn end_to_end_pass_and_every_drift_is_caught() {
        let (repo, root) = temp_repo("pass");
        write_services(&root);
        write(&root.join(SURFACES_REL), &end_to_end_surfaces());
        write(&root.join(BASELINE_REL), valid_baseline());
        assert_eq!(dogfood_plan(&repo), Ok(()));

        // Registered route missing from the declaration.
        let mut declared: serde_json::Value = serde_json::from_str(&end_to_end_surfaces()).unwrap();
        declared["services"]["apex-api"]["routes"] = serde_json::json!([]);
        write(
            &root.join(SURFACES_REL),
            &serde_json::to_string(&declared).unwrap(),
        );
        let err = dogfood_plan(&repo).unwrap_err();
        assert!(
            err.contains("registered route not declared in tools/dogfood/surfaces.json: apex-api GET /apex-api"),
            "{err}"
        );
        assert!(err.contains("DOGFOOD-PLAN FAILED"), "{err}");

        // Fixture count drift.
        declared["services"]["apex-api"]["routes"] = serde_json::json!([
            {"path": "/apex-api", "methods": ["GET"], "sources": ["rust/crates/apex-api/src/routes.rs"]}
        ]);
        declared["services"]["apex-control"]["fixture_pages"] = serde_json::json!(8);
        write(
            &root.join(SURFACES_REL),
            &serde_json::to_string(&declared).unwrap(),
        );
        let err = dogfood_plan(&repo).unwrap_err();
        assert!(
            err.contains("apex-control: fixture_pages declares 8 but the render-fixtures bin declares PAGES len 7"),
            "{err}"
        );

        // The baseline cell disappears from the declaration without a
        // recorded removal.
        declared["services"]["apex-api"]["required"] = serde_json::json!([]);
        declared["services"]["apex-control"]["fixture_pages"] = serde_json::json!(7);
        write(
            &root.join(SURFACES_REL),
            &serde_json::to_string(&declared).unwrap(),
        );
        let err = dogfood_plan(&repo).unwrap_err();
        assert!(
            err.contains("apex-api|GET /apex-api|surface-parity"),
            "{err}"
        );

        // A reasoned removal entry authorizes the shrink.
        write(
            &root.join(SURFACES_REL),
            &serde_json::to_string(&declared).unwrap(),
        );
        write(
            &root.join(BASELINE_REL),
            r#"{"cells":["apex-api|GET /apex-api|surface-parity"],"removals":[{"cell":"apex-api|GET /apex-api|surface-parity","reason":"surface coverage paused"}]}"#,
        );
        assert_eq!(dogfood_plan(&repo), Ok(()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unknown_service_and_missing_baseline_are_failures() {
        let (repo, root) = temp_repo("services");
        write_services(&root);
        let mut declared: serde_json::Value = serde_json::from_str(&end_to_end_surfaces()).unwrap();
        declared["services"]["apex-ghost"] =
            serde_json::json!({"routes": [], "fixture_pages": 0, "required": []});
        declared["services"]
            .as_object_mut()
            .unwrap()
            .remove("apex-sales-web");
        write(
            &root.join(SURFACES_REL),
            &serde_json::to_string(&declared).unwrap(),
        );
        write(&root.join(BASELINE_REL), valid_baseline());
        let err = dogfood_plan(&repo).unwrap_err();
        assert!(err.contains("unknown service 'apex-ghost'"), "{err}");
        assert!(
            err.contains("service 'apex-sales-web' is not declared"),
            "{err}"
        );

        std::fs::remove_file(root.join(BASELINE_REL)).unwrap();
        let err = dogfood_plan(&repo).unwrap_err();
        assert!(
            err.contains("tools/dogfood/live-baseline.json is missing"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn malformed_surfaces_json_is_a_parse_error() {
        let (repo, root) = temp_repo("malformed");
        write(&root.join(SURFACES_REL), "{not json");
        let err = dogfood_plan(&repo).unwrap_err();
        assert!(err.starts_with(SURFACES_REL), "{err}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
