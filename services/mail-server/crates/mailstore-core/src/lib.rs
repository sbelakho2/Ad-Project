//! Mailstore core library surface: database, blob storage, and the gRPC
//! service implementation consumed by the `mailstore-core` binary.

pub mod db;
pub mod service;
pub mod storage;
