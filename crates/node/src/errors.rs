use ress_storage::errors::StorageError;
use ress_vm::errors::EvmError;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("storage error")]
    Storage(#[from] StorageError),

    #[error("evm error")]
    Evm(#[from] EvmError),

    #[error("submit error:{0}")]
    Submit(String),
}
