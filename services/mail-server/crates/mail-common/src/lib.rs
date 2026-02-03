//! Common types and utilities for the mail server.

pub mod config;
pub mod error;
pub mod types;

pub use config::Config;
pub use error::{MailError, Result};
