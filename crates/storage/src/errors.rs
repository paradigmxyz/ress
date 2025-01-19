use alloy_primitives::{BlockNumber, B256};
use ress_subprotocol::connection::CustomCommand;
use tokio::sync::{mpsc::error::SendError, oneshot::error::RecvError};

/// Database error type.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// No code found
    #[error("no code found for: {0}")]
    NoCodeForCodeHash(B256),

    /// block not found
    #[error("block not found: {0}")]
    BlockNotFound(BlockNumber),

    #[error("Network storage: {0}")]
    Network(#[from] NetworkStorageError),

    #[error("disk storage")]
    Disk(String),

    #[error("in memory storage")]
    Memory(String),
}

#[derive(Debug, thiserror::Error)]
pub enum NetworkStorageError {
    #[error("Failed to send request through channel: {0}")]
    ChannelSend(#[from] SendError<CustomCommand>),

    #[error("Failed to receive response from channel: {0}")]
    ChannelReceive(#[from] RecvError),
}
