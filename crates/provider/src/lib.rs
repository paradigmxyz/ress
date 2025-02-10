//! Ress provider implementation.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

/// Main provider for retrieving data.
mod provider;
pub use provider::RessProvider;

/// Error types.
pub mod errors;

/// Data storage backends.
pub mod backends;
