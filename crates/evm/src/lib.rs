//! Ress evm implementation.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod db;

mod executor;
pub use executor::BlockExecutor;
