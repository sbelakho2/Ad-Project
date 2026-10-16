//! `render-fixtures`: render every sales-workspace page and state into one
//! directory so the `console-a11y` gate can scan the real HTML.
//!
//! Usage: `render-fixtures [out-dir]`, default `rust/target/ui-fixtures-sales`.

#![forbid(unsafe_code)]

use std::path::PathBuf;

/// Every fixture page this binary renders.
pub const PAGES: [&str; 36] = [
    "domains-empty.html",
    "domains-paused.html",
    "domains-quarantine.html",
    "domains.html",
    "error-bad-request.html",
    "error-conflict.html",
    "error-data-unavailable.html",
    "error-not-found.html",
    "leads-empty.html",
    "leads-filtered.html",
    "leads.html",
    "login-failed.html",
    "login-session-required.html",
    "login-signed-out.html",
    "login-unconfigured.html",
    "login.html",
    "models-empty.html",
    "models.html",
    "overview-empty.html",
    "overview-quarantine.html",
    "overview.html",
    "plan-approval.html",
    "plan-autonomous.html",
    "plan-empty.html",
    "plan-shadow.html",
    "publisher-suppressed.html",
    "publisher.html",
    "replies-empty.html",
    "replies.html",
    "result-confirmed.html",
    "result-failed.html",
    "sends-empty.html",
    "sends-preview-refused.html",
    "sends-preview.html",
    "sends-sendmail.html",
    "sends.html",
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("rust/target/ui-fixtures-sales"));
    let files = apex_sales_web::fixture_render::render_all(&out)?;
    if files.len() != PAGES.len() {
        return Err(format!(
            "the renderer produced {} pages but PAGES lists {}",
            files.len(),
            PAGES.len()
        )
        .into());
    }
    for path in &files {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("page {} is not a UTF-8 file name", path.display()))?;
        if !PAGES.contains(&name) {
            return Err(format!("page {name} is not listed in PAGES").into());
        }
    }
    println!(
        "rendered {} sales-workspace pages into {}",
        files.len(),
        out.display()
    );
    Ok(())
}
