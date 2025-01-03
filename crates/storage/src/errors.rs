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
    Network(String),

    #[error("disk storage")]
    Disk(String),

    #[error("in memory storage")]
    Memory(String),
}
