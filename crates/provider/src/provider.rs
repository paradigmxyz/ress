use std::sync::Arc;

use alloy_primitives::B256;
use ress_primitives::witness::ExecutionWitness;
use ress_subprotocol::connection::CustomCommand;
use reth_chainspec::ChainSpec;
use reth_revm::primitives::Bytecode;
use tokio::sync::mpsc::UnboundedSender;

use crate::{errors::StorageError, network::NetworkProvider, storage::Storage};

#[derive(Debug)]
pub struct RessProvider {
    pub storage: Arc<Storage>,
    network: NetworkProvider,
}

impl RessProvider {
    pub fn new(
        network_peer_conn: UnboundedSender<CustomCommand>,
        chain_spec: Arc<ChainSpec>,
    ) -> Self {
        let network = NetworkProvider::new(network_peer_conn);
        let storage = Arc::new(Storage::new(chain_spec));
        Self { storage, network }
    }

    /// Fetch witness of target block hash
    pub async fn fetch_witness(&self, block_hash: B256) -> Result<ExecutionWitness, StorageError> {
        Ok(self.network.get_witness(block_hash).await?)
    }

    /// Fetch bytecode and save it to disk
    pub async fn fetch_contract_bytecode(&self, code_hash: B256) -> Result<Bytecode, StorageError> {
        let latest_block_hash = self.storage.memory.get_latest_block_hash();
        if let Some(bytecode) = self
            .network
            .get_contract_bytecode(
                latest_block_hash.expect("need latest block hash"),
                code_hash,
            )
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
