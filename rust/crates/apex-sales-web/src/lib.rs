//! # apex-sales-web
//!
//! The owner-only web UI for the [`apex_sales`] outbound engine. One Axum
//! process, server-rendered HTML, zero JavaScript: every page is built from
//! the frozen [`apex_ui`] design system and every number comes from the
//! engine's own public functions ([`apex_sales::engine::learn`],
//! [`apex_sales::engine::plan`], [`apex_sales::ops`] and the
//! scoring/bandit/funnel/cadence/health/holdout/stats modules). Nothing here
//! re-computes a statistic; the pages only select, label and lay out what
//! the engine returns.
//!
//! ## The owner gate
//!
//! `APEX_SALES_WEB_TOKEN` is required. With the variable unset the service
//! fails closed: every page renders the not-configured state and no session
//! can be minted. A wrong token answers `401` with a safe re-render of the
//! sign-in form; the right token (compared in constant time) mints a
//! short-TTL session cookie (`apex_sales_session`; host-only, `HttpOnly`,
//! `SameSite=Lax`, `Secure` when the request arrived via
//! `X-Forwarded-Proto: https`, 8-hour absolute lifetime) and redirects.
//! Every mutating POST requires the per-session CSRF token; actions that
//! write durable engine events carry a hidden idempotency key, so a
//! replayed form never duplicates its event.
//!
//! ## Assets
//!
//! Exactly one stylesheet is served: `GET /assets/apex.css` answers
//! [`apex_ui::tokens::console_css`] bytes. Pages link it and contain no
//! `<style>` block and no `style="` attribute, so the sales surface cannot
//! drift from the design tokens. The shell wrapper in [`render`] performs
//! the one documented head transformation: the frozen shell's single global
//! `<style>` block is replaced with the stylesheet link.
//!
//! ## Metrics
//!
//! The bounded `apex-metrics` registry records `apex_http_requests_total`,
//! `apex_http_request_duration_seconds` and `apex_http_requests_in_flight`
//! per route template (`service="apex-sales-web"`), exported to
//! `APEX_METRICS_TEXTFILE` when configured.

#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod actions;
pub mod assets;
pub mod config;
pub mod data;
pub mod favicon;
pub mod fixture_render;
pub mod pages;
pub mod render;
pub mod routes;
pub mod session;
pub mod signing;
pub mod state;
pub mod telemetry;
