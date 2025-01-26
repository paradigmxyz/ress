//! EVM errors.

use ress_provider::errors::StorageError;
use reth_evm::execute::BlockExecutionError;

/// Error variants for EVM execution.
#[derive(Debug, thiserror::Error)]
pub enum EvmError {
    /// Database error.
    #[error("DB error: {0}")]
    DB(#[from] StorageError),

    /// Block execution error.
    #[error("Block execution error: {0}")]
    BlockExecution(#[from] BlockExecutionError),
}
