//! Ress provider implementation.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

/// Main provider for retrieving data.
mod provider;
pub use provider::RessProvider;

/// Ress database.
mod database;
pub use database::RessDatabase;

/// Data storage backends.
pub mod backends;

/// Error types.
pub mod errors;
