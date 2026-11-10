#![no_std]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]
//! A caller-buffered, protocol-independent AI call kernel.
//!
//! [`v1`] is the versioned semantic contract. The caller owns transport,
//! credentials, retries, clocks, storage, tool execution and agent decisions.
//! No library operation allocates or performs I/O.

mod capability;
mod common;
mod document;
mod session;

/// Version one of the stable request, response, event and lifecycle contract.
/// Rust source compatibility and semantics are versioned; binary layout is not.
pub mod v1 {
    pub use crate::capability::*;
    pub use crate::common::*;
    pub use crate::document::*;
    pub use crate::session::*;
}
