use ress_provider::errors::StorageError;
use reth_evm::execute::BlockExecutionError;

#[derive(Debug, thiserror::Error)]
pub enum EvmError {
    #[error("DB error: {0}")]
    DB(#[from] StorageError),

    #[error("Block execution error: {0}")]
    BlockExecution(#[from] BlockExecutionError),
}
