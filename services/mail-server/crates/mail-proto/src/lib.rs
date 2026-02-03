//! Mail Protocol Definitions
//!
//! gRPC/Protobuf definitions for inter-service communication.

pub mod generated {
    tonic::include_proto!("apex.mail.v1");
}

pub use generated::*;
