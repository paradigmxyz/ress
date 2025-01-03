/// Database error type.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StorageError {
    /// No code found
    #[error("no code found")]
    NoCodeForCodeHash,

    /// block not found
    #[error("block not found")]
    BlockNotFound,

    #[error("network storage")]
    Network(#[from] NetworkStorageError),

    #[error("disk storage")]
    Disk(String),

    #[error("in memory storage")]
    Memory(String),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NetworkStorageError {
    #[error("Failed to send request through channel: {0}")]
    ChannelSend(String),

    #[error("Failed to receive response from channel: {0}")]
    ChannelReceive(String),
}
