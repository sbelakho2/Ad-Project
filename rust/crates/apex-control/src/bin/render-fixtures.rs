//! `render-fixtures`: render every control-plane page and state into one
//! directory so the `console-a11y` gate can scan the real HTML.
//!
//! Usage: `render-fixtures [out-dir]`, default `rust/target/ui-fixtures-control`.

#![forbid(unsafe_code)]

use std::path::PathBuf;

/// Every fixture page this binary renders.
pub const PAGES: [&str; 56] = [
    "access-denied.html",
    "access.html",
    "audits-no-inputs.html",
    "audits.html",
    "conflict.html",
    "customer-activity.html",
    "customer-audits.html",
    "customer-connections.html",
    "customer-experiments.html",
    "customer-finance.html",
    "customer-summary.html",
    "customers-filtered.html",
    "customers-no-matches.html",
    "customers.html",
    "denied.html",
    "error-bad-request.html",
    "error-internal.html",
    "error-method-not-allowed.html",
    "error-not-configured.html",
    "error-not-found.html",
    "experiment-detail.html",
    "experiments-awaiting.html",
    "experiments-effectiveness.html",
    "experiments.html",
    "finance-analytics.html",
    "finance.html",
    "growth-incomplete.html",
    "growth-no-site-requests.html",
    "growth.html",
    "incident-detail-stepup.html",
    "incident-detail.html",
    "incidents-empty.html",
    "incidents.html",
    "ingestion-delayed.html",
    "ingestion.html",
    "invoice.html",
    "login-code-error.html",
    "login-code.html",
    "login-no-second-factor.html",
    "login-password-error.html",
    "login-password.html",
    "login-rate-limited.html",
    "login-session-expired.html",
    "login-step-expired.html",
    "login-unconfigured.html",
    "overview-empty.html",
    "overview-unavailable.html",
    "overview.html",
    "result-confirmed.html",
    "result-failed.html",
    "result-requested.html",
    "result-running.html",
    "result-state-unknown.html",
    "system-policies-no-evidence.html",
    "system-policies.html",
    "system.html",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("rust/target/ui-fixtures-control"));
    let files = apex_control::fixture_render::render_all(&out)?;
    if files.len() != PAGES.len() {
        return Err(format!(
            "the renderer produced {} pages but PAGES lists {}",
            files.len(),
            PAGES.len()
        )
        .into());
    }
    for name in &files {
        if !PAGES.contains(&name.as_str()) {
            return Err(format!("page {name} is not listed in PAGES").into());
        }
    }
    println!(
        "rendered {} control-plane pages into {}",
        files.len(),
        out.display()
    );
    Ok(())
}
