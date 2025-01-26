use crate::{errors::StorageError, network::NetworkProvider, storage::Storage};
use alloy_eips::BlockNumHash;
use alloy_primitives::B256;
use ress_primitives::witness::ExecutionWitness;
use ress_subprotocol::connection::CustomCommand;
use reth_chainspec::ChainSpec;
use reth_revm::primitives::Bytecode;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

/// Provider for retrieving blockchain data.
///
/// This type is a main entrypoint for fetching chain and supplementary state data.
#[derive(Debug)]
pub struct RessProvider {
    /// Storage provider.
    pub storage: Arc<Storage>,
    /// Network provider.
    pub network: NetworkProvider,
}

impl RessProvider {
    /// Instantiate new provider.
    pub fn new(
        network_peer_conn: UnboundedSender<CustomCommand>,
        chain_spec: Arc<ChainSpec>,
        current_canonical_head: BlockNumHash,
    ) -> Self {
        let network = NetworkProvider::new(network_peer_conn);
        let storage = Arc::new(Storage::new(chain_spec, current_canonical_head));
        Self { storage, network }
    }

    /// Fetch witness of target block hash
    pub async fn fetch_witness(&self, block_hash: B256) -> Result<ExecutionWitness, StorageError> {
        Ok(self.network.get_witness(block_hash).await?)
    }

    /// Fetch bytecode and save it to disk
    pub async fn fetch_contract_bytecode(
        &self,
        code_hash: B256,
        block_hash: B256,
    ) -> Result<Bytecode, StorageError> {
        if let Some(bytecode) = self
            .network
            .get_contract_bytecode(block_hash, code_hash)
            .await?
        {
            self.storage
                .disk
                .update_bytecode(code_hash, bytecode.clone())?;
            return Ok(bytecode);
        }
        Err(StorageError::NoCodeForCodeHash(code_hash))
    }
}
