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
        block_number: BlockNumber,
    ) -> Result<Option<Header>, StorageError> {
        let canonical_hashes = self.canonical_hashes.lock().unwrap();
        if let Some(block_hash) = canonical_hashes.get(&block_number) {
            let headers = self.headers.lock().unwrap();
            Ok(headers.get(block_hash).cloned())
        } else {
            Ok(None)
        }
    }

    pub fn get_chain_config(&self) -> Result<ChainSpec, StorageError> {
        todo!()
    }

    pub fn get_block_header_by_hash(
        &self,
        block_hash: B256,
    ) -> Result<Option<SealedHeader>, StorageError> {
        let headers = self.headers.lock().unwrap();
        if let Some(header) = headers.get(&block_hash) {
            Ok(Some(SealedHeader::new(header.clone(), block_hash)))
        } else {
            Ok(None)
        }
    }
}
