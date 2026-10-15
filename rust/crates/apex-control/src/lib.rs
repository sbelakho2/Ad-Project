//! apex-control: the isolated operator control plane.
//!
//! This crate is the separate Axum service the design document (section 04,
//! pages 53 to 72, plus the page 96 sign-in and page 97 failure copy) asks
//! for: server-rendered HTML with zero JavaScript, its own origin, its own
//! `APEX_CONTROL_*` environment namespace, its own session cookie and
//! in-memory session store, its own CSRF tokens and signing, and an
//! authorization model that denies by default.
//!
//! Isolation from the publisher console is structural, not a policy:
//!
//! - the session cookie is [`session::SESSION_COOKIE`] scoped to
//!   `/control`, a different name from the console's cookie, so a console
//!   cookie grants nothing here and vice versa;
//! - operator credentials live only in the `APEX_CONTROL_OPERATORS` file
//!   (argon2id PHC strings and optional TOTP secrets); no customer
//!   credential is ever read, stored or rendered;
//! - reads come from the shared analytics read models
//!   ([`apex_analytics::AnalyticsReader`]); platform mutations are never
//!   executed here, they become durable operator jobs in the frozen file
//!   format for the autopilot workstream.
//!
//! The module map: [`auth`] (password + TOTP checks), [`base32`], [`totp`],
//! [`passwords`], [`rate_limit`], [`session`], [`login`], [`signing`],
//! [`redact`], [`capabilities`], [`copy`], [`journal`], [`jobs`],
//! [`versions`], [`data`] (read models and durable files), [`events`]
//! (analytics emission), [`actions`] (the action engine), [`telemetry`]
//! (the bounded metrics registry and the request middleware), [`state`],
//! [`routes`] and [`pages`].
#![forbid(unsafe_code)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod actions;
pub mod auth;
pub mod base32;
pub mod capabilities;
pub mod config;
pub mod copy;
pub mod data;
pub mod events;
pub mod favicon;
pub mod fixture_render;
pub mod fixtures;
pub mod governance;
pub mod jobs;
pub mod journal;
pub mod login;
pub mod pages;
pub mod passwords;
pub mod rate_limit;
pub mod redact;
pub mod routes;
pub mod session;
pub mod signing;
pub mod state;
pub mod telemetry;
pub mod totp;
pub mod versions;
