use alloy_eips::BlockNumHash;
use alloy_primitives::{BlockHash, BlockNumber, B256};
use backends::{disk::DiskStorage, memory::MemoryStorage};
use reth_chainspec::ChainSpec;
use reth_primitives::Header;
use reth_revm::primitives::Bytecode;
use std::{collections::HashMap, sync::Arc};

use crate::errors::StorageError;

pub mod backends;
pub mod trie;

/// Orchestrate 3 different type of backends (in-memory, disk, network)
#[derive(Debug)]
pub struct Storage {
    chain_spec: Arc<ChainSpec>,
    pub memory: MemoryStorage,
    pub disk: DiskStorage,
}

impl Storage {
    pub fn new(chain_spec: Arc<ChainSpec>, current_canonical_head: BlockNumHash) -> Self {
        let memory = MemoryStorage::new(current_canonical_head);
        let disk = DiskStorage::new("test.db");
        Self {
            chain_spec,
            memory,
            disk,
        }
    }

    pub fn remove_canonical_until(&self, upper_bound: BlockNumber, last_persisted_hash: B256) {
        self.memory
            .remove_canonical_until(upper_bound, last_persisted_hash);
    }

    pub fn executed_block_by_hash(&self, hash: B256) -> Option<Header> {
        self.memory.executed_block_by_hash(hash)
    }

    /// Insert executed block into the state.
    pub fn insert_executed(&self, executed: Header) {
        self.memory.insert_executed(executed);
    }

    /// Returns whether or not the hash is part of the canonical chain.
    pub fn is_canonical(&self, hash: B256) -> bool {
        self.memory.is_canonical(hash)
    }

    pub fn set_canonical_head(&self, new_head: BlockNumHash) {
        self.memory.set_canonical_head(new_head);
    }

    /// Remove oldest block hash and header
    pub fn remove_oldest_canonical_hash(&self) {
        self.memory.remove_oldest_canonical_hash();
    }

    /// Find if target block hash is exist in the memory
    pub fn find_block_hash(&self, block_hash: BlockHash) -> bool {
        self.memory.find_block_hash(block_hash)
    }

    /// Get contract bytecode from given codehash from the disk
    pub fn get_contract_bytecode(&self, code_hash: B256) -> Result<Bytecode, StorageError> {
        if let Some(bytecode) = self.disk.get_bytecode(code_hash)? {
            return Ok(bytecode);
        }
        Err(StorageError::NoCodeForCodeHash(code_hash))
    }

    /// Filter code hashes only not stored in disk already
    pub fn filter_code_hashes(&self, code_hashes: Vec<B256>) -> Vec<B256> {
        self.disk.filter_code_hashes(code_hashes)
    }

    /// Set canonical block hash that historical 256 blocks from canonical head
    pub fn set_canonical_hash(&self, block_hash: B256, block_number: BlockNumber) {
        self.memory.set_canonical_hash(block_hash, block_number);
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
}
