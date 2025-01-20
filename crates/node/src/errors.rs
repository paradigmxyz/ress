use alloy_rpc_types_engine::PayloadError;
use ress_storage::errors::StorageError;
use ress_vm::errors::EvmError;
use reth_consensus::ConsensusError;
use reth_node_api::{EngineObjectValidationError, InvalidPayloadAttributesError};
use reth_trie_sparse::errors::SparseStateTrieError;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// Error related to storage operations.
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    /// Error related to EVM operations.
    #[error("Evm error: {0}")]
    Evm(#[from] EvmError),

    /// Error from the debug API client.
    #[error("Debug api client: {0}")]
    DebugApiClient(String),

    /// Serialization error.
    #[error("Serialize error: {0}")]
    Serialize(#[from] serde_json::Error),

    /// Error related to payload processing.
    #[error("Payload error: {0}")]
    Payload(#[from] PayloadError),

    /// Error during engine object validation.
    #[error("Engine object validation error: {0}")]
    EngineObjectValidation(#[from] EngineObjectValidationError),

    /// Error due to invalid payload attributes.
    #[error("Invalid payload attributes error: {0}")]
    InvalidPayloadAttributes(#[from] InvalidPayloadAttributesError),

    /// Error related to sparse state trie operations.
    #[error("Sparse state trie error: {0}")]
    SparseStateTrie(#[from] SparseStateTrieError),

    /// Consensus-related error.
    #[error("Consensus error: {0}")]
    Consensus(#[from] ConsensusError),
}
