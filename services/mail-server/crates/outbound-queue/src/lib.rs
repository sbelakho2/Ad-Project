//! Outbound queue library surface: the queue item model and the Resend client.
//!
//! The `outbound-queue` binary wires these together for operator delivery checks
//! and the service loop.

pub mod queue;
pub mod resend;
