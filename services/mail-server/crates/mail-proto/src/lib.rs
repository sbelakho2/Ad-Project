//! Mail Protocol Definitions
//!
//! gRPC/Protobuf definitions for inter-service communication.

pub mod generated {
    // Generated tonic/prost code: clippy lints that only make sense for
    // hand-written code (large generated Status errors, doc wrapping) stay off
    // inside this module, not across the crate.
    #![allow(clippy::result_large_err)]
    tonic::include_proto!("apex.mail.v1");
}

pub use generated::*;
