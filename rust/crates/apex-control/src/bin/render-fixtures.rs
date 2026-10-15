//! `render-fixtures`: render every control-plane page and state into one
//! directory so the `console-a11y` gate can scan the real HTML.
//!
//! Usage: `render-fixtures [out-dir]`, default `rust/target/ui-fixtures-control`.

#![forbid(unsafe_code)]

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("rust/target/ui-fixtures-control"));
    let files = apex_control::fixture_render::render_all(&out)?;
    println!(
        "rendered {} control-plane pages into {}",
        files.len(),
        out.display()
    );
    Ok(())
}
