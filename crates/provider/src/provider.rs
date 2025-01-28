use crate::{errors::StorageError, storage::Storage};
use alloy_primitives::B256;
use ress_network::RessNetworkHandle;
use ress_primitives::witness::{ExecutionWitness, StateWitness};
use reth_primitives::Header;
use reth_revm::primitives::Bytecode;

/// Provider for retrieving blockchain data.
///
/// This type is a main entrypoint for fetching chain and supplementary state data.
#[derive(Clone, Debug)]
pub struct RessProvider {
    /// Storage provider.
    pub storage: Storage,
    /// Network handle.
    pub network: RessNetworkHandle,
}

impl RessProvider {
    /// Instantiate new provider.
    pub fn new(storage: Storage, network: RessNetworkHandle) -> Self {
        Self { storage, network }
    }

    /// Fetch header of target block hash
    pub async fn fetch_header(&self, block_hash: B256) -> Result<Header, StorageError> {
        let header = self.network.fetch_header(block_hash).await?;
        Ok(header)
    }

    /// Fetch witness of target block hash
    pub async fn fetch_witness(&self, block_hash: B256) -> Result<ExecutionWitness, StorageError> {
        let state_witness = self.network.fetch_witness(block_hash).await?;
        let witness = ExecutionWitness::new(StateWitness::from_iter(
            state_witness.0.into_iter().map(|e| (e.hash, e.bytes)),
        ));
        Ok(witness)
    }

    /// Ensure that bytecode exists on disk, download and write to disk if missing.
    pub async fn ensure_bytecode_exists(&self, code_hash: B256) -> Result<(), StorageError> {
        if self.storage.disk.code_hash_exists_in_db(&code_hash) {
            return Ok(());
        }

        let bytecode = self
            .network
            .fetch_bytecode(code_hash)
            .await?
            .ok_or(StorageError::NoCodeForCodeHash(code_hash))?;
        self.storage
            .disk
            .update_bytecode(code_hash, Bytecode::new_raw(bytecode))?;
        Ok(())
    }
}
