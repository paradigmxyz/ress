use alloy_primitives::{BlockHash, BlockNumber, B256};

/// Database error type.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// No code found for the specified code hash.
    #[error("no bytecode found for: {0}")]
    NoCodeForCodeHash(B256),

    /// Invalid bytecode
    #[error("invalid bytecode: {0}")]
    InvalidBytecode(B256),

    /// Error related to disk storage operations.
    #[error("Disk storage: {0}")]
    Disk(#[from] DiskStorageError),

    /// Error related to memory storage operations.
    #[error("Memory storage: {0}")]
    Memory(#[from] MemoryStorageError),
}

/// Errors that can occur during memory storage operations.
#[derive(Debug, thiserror::Error)]
pub enum MemoryStorageError {
    /// Block not found in memory storage via block number.
    #[error("block hash not found from number: {0}")]
    BlockNotFoundFromNumber(BlockNumber),

    /// Block not found in memory storage via block hash.
    #[error("block not found from hash: {0}")]
    BlockNotFoundFromHash(BlockHash),

    /// Block does not belong to canonical chain.
    #[error("non canonical chain: {0}")]
    NonCanonicalChain(BlockHash),
}

/// Errors that can occur during disk storage operations.
#[derive(Debug, thiserror::Error)]
pub enum DiskStorageError {
    /// Database-related error.
    #[error("Database: {0}")]
    Database(#[from] rusqlite::Error),
}
