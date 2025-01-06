use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use alloy_primitives::{Address, BlockHash, BlockNumber, B256, U256};
use reth_chainspec::ChainSpec;
use reth_primitives::{Header, SealedHeader};
use reth_revm::primitives::AccountInfo;

use crate::errors::StorageError;

#[derive(Debug, Clone)]
pub struct MemoryStorage {
    pub headers: Arc<Mutex<HashMap<BlockHash, Header>>>,
    pub canonical_hashes: Arc<Mutex<HashMap<BlockNumber, BlockHash>>>,
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            headers: Arc::new(Mutex::new(HashMap::new())),
            canonical_hashes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn set_block_hash(&self, block_hash: B256, block_number: BlockNumber) {
        let mut canonical_hashes = self.canonical_hashes.lock().unwrap();
        canonical_hashes.insert(block_number, block_hash);
    }

    pub fn set_block_header(&self, block_hash: B256, header: Header) {
        let mut headers = self.headers.lock().unwrap();
        headers.insert(block_hash, header);
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
        Ok(Some(SealedHeader::default()))
    }
}
