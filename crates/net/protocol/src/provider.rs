use alloy_primitives::{map::B256HashMap, Bytes, B256};
use reth_storage_errors::provider::ProviderResult;

/// A provider trait for ress protocol.
pub trait RessProtocolProvider: Send + Sync {
    /// Return bytecode by code hash.
    fn bytecode(&self, code_hash: B256) -> ProviderResult<Option<Bytes>>;

    /// Return witness by block hash.
    fn witness(&self, block_hash: B256) -> ProviderResult<Option<B256HashMap<Bytes>>>;
}
