use alloy_rpc_types_engine::PayloadError;
use ress_storage::errors::StorageError;
use ress_vm::errors::EvmError;
use reth_node_api::{EngineObjectValidationError, InvalidPayloadAttributesError};

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("storage error")]
    Storage(#[from] StorageError),

    #[error("evm error")]
    Evm(#[from] EvmError),

    #[error("submit error:{0}")]
    Submit(String),

    #[error("payload error:{0}")]
    Payload(#[from] PayloadError),

    #[error("engine object validation error:{0}")]
    EngineObjectValidation(#[from] EngineObjectValidationError),

    #[error("invalid payload attributes error:{0}")]
    InvalidPayloadAttributes(#[from] InvalidPayloadAttributesError),
}
