/// Database error type.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    /// No code found
    #[error("no code found")]
    NoCodeForCodeHash,

    /// block not found
    #[error("block not found")]
    BlockNotFound,
}
