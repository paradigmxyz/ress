use std::{collections::HashMap, sync::RwLock};

use alloy_primitives::{Address, BlockHash, BlockNumber, B256, U256};
use reth_chainspec::ChainSpec;
use reth_primitives::{Header, SealedHeader};
use reth_revm::primitives::AccountInfo;

use crate::errors::StorageError;

pub struct MemoryStorage {
    headers: RwLock<HashMap<BlockHash, Header>>,
    canonical_hashes: RwLock<HashMap<BlockNumber, BlockHash>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            headers: RwLock::new(HashMap::new()),
            canonical_hashes: RwLock::new(HashMap::new()),
        }
    }

    pub fn set_block_hash(&self, block_hash: B256, block_number: BlockNumber) {
        self.canonical_hashes
            .write()
            .unwrap()
            .insert(block_number, block_hash);
    }

    pub fn set_block_header(&self, block_hash: B256, header: Header) {
        self.headers.write().unwrap().insert(block_hash, header);
    }

    pub fn get_account_info_by_hash(
        &self,
        _block_hash: B256,
        _address: Address,
    ) -> Result<Option<AccountInfo>, StorageError> {
        todo!()
    }

    pub fn get_storage_at_hash(
        &self,
        _block_hash: B256,
        _address: Address,
        _storage_key: B256,
    ) -> Result<Option<U256>, StorageError> {
        todo!()
    }

    pub fn get_block_header(
        &self,
        _block_number: BlockNumber,
    ) -> Result<Option<Header>, StorageError> {
        todo!()
    }

    pub fn get_chain_config(&self) -> Result<ChainSpec, StorageError> {
        todo!()
    }

    pub fn get_block_header_by_hash(
        &self,
        _block_hash: B256,
    ) -> Result<Option<SealedHeader>, StorageError> {
        // todo: get header from memeory
        // self.engine.get_block_header_by_hash(block_hash)
        return Ok(Some(SealedHeader::default()));
    }
}
