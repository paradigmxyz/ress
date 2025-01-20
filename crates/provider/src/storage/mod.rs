use alloy_primitives::{BlockHash, BlockNumber, B256};
use backends::{disk::DiskStorage, memory::MemoryStorage};
use reth_chainspec::ChainSpec;
use reth_primitives::{Header, SealedHeader};
use reth_revm::primitives::Bytecode;
use std::{collections::HashMap, sync::Arc};

use crate::errors::StorageError;

pub mod backends;

/// Orchestrate 3 different type of backends (in-memory, disk, network)
#[derive(Debug)]
pub struct Storage {
    chain_spec: Arc<ChainSpec>,
    pub memory: MemoryStorage,
    pub disk: DiskStorage,
}

impl Storage {
    pub fn new(chain_spec: Arc<ChainSpec>) -> Self {
        let memory = MemoryStorage::new();
        let disk = DiskStorage::new("test.db");
        Self {
            chain_spec,
            memory,
            disk,
        }
    }

    /// Remove oldest block hash and header
    pub fn remove_oldest_block(&self) {
        self.memory.remove_oldest_block();
    }

    /// Find if target block hash is exist in the memory
    pub fn find_block_hash(&self, block_hash: BlockHash) -> bool {
        self.memory.find_block_hash(block_hash)
    }

    /// Get contract bytecode from given codehash.
    ///
    /// First try fetch from disk and fallback to netowork if it doesn't exist.
    pub fn get_contract_bytecode(&self, code_hash: B256) -> Result<Bytecode, StorageError> {
        if let Some(bytecode) = self.disk.get_bytecode(code_hash)? {
            return Ok(bytecode);
        }

        Err(StorageError::NoCodeForCodeHash(code_hash))
    }

    /// Set block hash and set block header
    pub fn set_block(&self, header: Header) {
        self.memory
            .set_block_hash(header.hash_slow(), header.number);
        self.memory.set_block_header(header.hash_slow(), header);
    }

    /// Set block header
    pub fn set_block_header(&self, header: Header) {
        self.memory.set_block_header(header.hash_slow(), header);
    }

    /// Set block hash
    pub fn set_block_hash(&self, block_hash: B256, block_number: BlockNumber) {
        self.memory.set_block_hash(block_hash, block_number);
    }

    /// Overwrite block hashes mapping
    pub fn overwrite_block_hashes(&self, block_hashes: HashMap<BlockNumber, B256>) {
        self.memory.overwrite_block_hashes(block_hashes);
    }

    /// Overwrite block hashes by passing block headers
    pub fn overwrite_block_hashes_by_headers(&self, block_headers: Vec<Header>) {
        let mut block_hashes = HashMap::new();

        for header in block_headers {
            let block_number = header.number;
            let block_hash = header.hash_slow();
            block_hashes.insert(block_number, block_hash);
        }

        self.memory.overwrite_block_hashes(block_hashes);
    }

    /// Check if 256 canonical hashes are exist from target block
    pub fn is_canonical_hashes_exist(&self, target_block: BlockNumber) -> bool {
        self.memory.is_canonical_hashes_exist(target_block)
    }

    /// Get block hash from memory of target block number
    pub fn get_block_hash(&self, block_number: BlockNumber) -> Result<BlockHash, StorageError> {
        self.memory
            .get_block_hash(block_number)
            .map_err(StorageError::Memory)
    }

    /// Get chain config
    pub fn get_chain_config(&self) -> Arc<ChainSpec> {
        self.chain_spec.clone()
    }

    /// Get block header by block hash
    pub fn get_block_header_by_hash(
        &self,
        block_hash: B256,
    ) -> Result<Option<SealedHeader>, StorageError> {
        self.memory
            .get_block_header_by_hash(block_hash)
            .map_err(StorageError::Memory)
    }
}
