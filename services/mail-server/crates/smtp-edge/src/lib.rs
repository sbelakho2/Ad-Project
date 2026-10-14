//! SMTP edge library surface: the session handler and the command parser.
//!
//! The `smtp-edge` binary wires these together; the parser is exercised by its
//! own test module as well.

pub mod parser;
pub mod session;
