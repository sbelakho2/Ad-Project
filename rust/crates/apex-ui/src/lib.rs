//! The Apex design system: one crate that renders every HTML surface
//! (publisher console, operator control plane, marketing pages) from the
//! frozen values of `Apex_Identity_UI_UX_Complete.pdf`.
//!
//! The public surface is frozen so parallel workstreams can code against it:
//!
//! - [`escape`]: the single escaping path for element text and quoted
//!   attributes.
//! - [`tokens`]: the exact color, spacing, radius, font and layout constants
//!   plus [`tokens::console_css`], the one stylesheet every component shares.
//! - [`format`]: money in integer micros, UTC timestamps, percentages and CSV
//!   fields, all without locale guessing.
//! - [`status`]: the status vocabulary (read-only, verified, needs attention,
//!   failure, not available, in progress, completed, queued).
//! - [`components`]: escaped HTML fragments for KPIs, notices, panels,
//!   tables, forms and navigation.
//! - [`charts`]: server-rendered SVG with `<title>`, `<desc>` and an adjacent
//!   data table; missing points render as gaps, never as zero.
//! - [`shell`]: the full document shell (dark rail, topbar, main region,
//!   footer) for the three areas.
//! - [`logo`]: the comparison mark and lockup from the identity master
//!   geometry.
//! - [`theme`]: the light/auto/dark cookie contract and its settings form.
//!
//! Invariants the tests enforce: no `<script>` in any fragment, every dynamic
//! string escaped, every color a frozen lowercase hex, and every foreground
//! and background pairing the stylesheet uses at WCAG AA contrast.
#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod charts;
pub mod components;
pub mod escape;
pub mod format;
pub mod logo;
pub mod shell;
pub mod status;
pub mod theme;
pub mod tokens;
