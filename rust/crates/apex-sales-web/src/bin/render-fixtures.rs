//! `render-fixtures`: render every sales-workspace page and state into one
//! directory so the `console-a11y` gate can scan the real HTML.
//!
//! Usage: `render-fixtures [out-dir]`, default `rust/target/ui-fixtures-sales`.
//! The whole command lives in
//! [`apex_sales_web::fixture_render::run_cli`], which validates the written
//! page set against [`apex_sales_web::fixture_render::PAGE_NAMES`]; this
//! binary only forwards the arguments and prints the summary.

#![forbid(unsafe_code)]

/// The fixture page names, in render order. Declared as an explicit array
/// because the static dogfood parity gate parses this declaration; the test
/// below pins it equal to [`apex_sales_web::fixture_render::PAGE_NAMES`],
/// which `run_cli` additionally enforces against the written page set.
#[rustfmt::skip]
pub const PAGES: [&str; 38] = [
    "domains-empty.html",
    "domains-multi.html",
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
    "plan-multi.html",
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let run = apex_sales_web::fixture_render::run_cli(&args)?;
    println!(
        "rendered {} sales-workspace pages into {}",
        run.files.len(),
        run.out_dir.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_declared_pages_match_the_renderer() {
        assert_eq!(
            super::PAGES,
            apex_sales_web::fixture_render::PAGE_NAMES,
            "the bin's PAGES and PAGE_NAMES drifted; the parity gate reads the former"
        );
    }
}
