//! The sales workspace pages: one module per route, all built from the
//! frozen `apex_ui` components and tokens.
//!
//! Every page renders from [`crate::data::RequestData`] (the engine's own
//! snapshot and learned models) and shows its scope, its as-of time in UTC
//! and where the data lives. Missing data is `Not available (reason)`,
//! never a fabricated zero; rates are `n/N` with the denominator.

pub mod domains;
pub mod errors;
pub mod leads;
pub mod login;
pub mod models;
pub mod overview;
pub mod plan;
pub mod publisher;
pub mod replies;
pub mod result;
pub mod sends;
pub mod shared;
