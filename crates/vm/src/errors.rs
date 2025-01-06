use ress_storage::errors::StorageError;

/// Database error type.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WitnessStateProviderError {
    /// Block hash not found.
    #[error("block hash not found")]
    BlockHashNotFound,

    /// Error when decoding RLP or trie nodes
    #[error("failed to decode data")]
    DecodingError,

    /// Error from StorageError
    #[error(transparent)]
    BytecodeProviderError(#[from] StorageError),
}

#[derive(Debug, thiserror::Error)]
pub enum EvmError {
    #[error("Invalid Transaction: {0}")]
    Transaction(String),
    #[error("Invalid Header: {0}")]
    Header(String),
    #[error("DB error: {0}")]
    DB(#[from] StorageError),
    #[error("{0}")]
    Custom(String),
    #[error("{0}")]
    Precompile(String),
}
