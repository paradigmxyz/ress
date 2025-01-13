use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use alloy_primitives::{BlockHash, BlockNumber, B256};
use backends::{disk::DiskStorage, memory::MemoryStorage, network::NetworkStorage};
use errors::StorageError;
use ress_primitives::witness::ExecutionWitness;
use ress_subprotocol::connection::CustomCommand;
use reth_chainspec::ChainSpec;
use reth_primitives::{Header, SealedHeader};
use reth_revm::primitives::Bytecode;
use tokio::sync::mpsc::UnboundedSender;

pub mod backends;
pub mod errors;

/// orchestrate 3 different type of backends (in-memory, disk, network)
#[derive(Debug, Clone)]
pub struct Storage {
    pub chain_spec: Arc<ChainSpec>,
    pub memory: Arc<MemoryStorage>,
    pub disk: Arc<Mutex<DiskStorage>>,
    pub network: Arc<NetworkStorage>,
}

impl Storage {
    pub fn new(
        network_peer_conn: UnboundedSender<CustomCommand>,
        chain_spec: Arc<ChainSpec>,
    ) -> Self {
        let memory = Arc::new(MemoryStorage::new());
        let disk = Arc::new(Mutex::new(DiskStorage::new("test.db")));
        let network = Arc::new(NetworkStorage::new(network_peer_conn));
        Self {
            chain_spec,
            memory,
            disk,
            network,
        }
    }

    pub fn get_witness(&self, block_hash: B256) -> Result<ExecutionWitness, StorageError> {
        self.network.get_witness(block_hash)
    }

    /// set block hash and set block header
    pub fn set_block(&self, block_hash: B256, header: Header) {
        self.memory.set_block_hash(block_hash, header.number);
        self.memory.set_block_header(block_hash, header);
    }

    pub fn set_block_hash(&self, block_hash: B256, block_number: BlockNumber) {
        self.memory.set_block_hash(block_hash, block_number);
    }

    /// overwrite block hashes mapping
    pub fn overwrite_block_hashes(&self, block_hashes: HashMap<BlockNumber, B256>) {
        self.memory.overwrite_block_hashes(block_hashes);
    }

    pub fn get_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StorageError> {
        self.memory.get_block_hash(block_number)
    }

    /// get bytecode from disk -> fallback network
    pub fn code_by_hash(&self, code_hash: B256) -> Result<Option<Bytecode>, StorageError> {
        let disk = self.disk.lock().unwrap();
        if let Some(bytecode) = disk.get_account_code(code_hash)? {
            return Ok(Some(bytecode));
        }

        if let Some(bytecode) = self.network.get_account_code(code_hash)? {
            disk.update_account_code(code_hash, bytecode.clone())?;
            return Ok(Some(bytecode));
        }
        Ok(None)
    }

    pub fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Header>, StorageError> {
        self.memory.get_block_header(block_number)
    }

    pub fn get_chain_config(&self) -> Arc<ChainSpec> {
        self.chain_spec.clone()
    }

    pub fn get_block_header_by_hash(
        &self,
        block_hash: B256,
    ) -> Result<Option<SealedHeader>, StorageError> {
        self.memory.get_block_header_by_hash(block_hash)
    }
}
