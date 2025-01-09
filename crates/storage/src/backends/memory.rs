use std::{collections::HashMap, sync::Arc};

use alloy_primitives::{Address, BlockHash, BlockNumber, B256, U256};
use parking_lot::RwLock;
use reth_primitives::{Header, SealedHeader};
use reth_revm::{db::DbAccount, primitives::AccountInfo};

use crate::errors::StorageError;

#[derive(Debug, Clone)]
pub struct MemoryStorage {
    inner: Arc<RwLock<MemoryStorageInner>>,
}

#[derive(Debug, Clone)]
pub struct MemoryStorageInner {
    // map with account and storage
    accounts: HashMap<Address, DbAccount>,
    headers: HashMap<BlockHash, Header>,
    canonical_hashes: HashMap<BlockNumber, BlockHash>,
}

impl Default for MemoryStorageInner {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStorageInner {
    pub fn new() -> Self {
        Self {
            accounts: HashMap::new(),
            headers: HashMap::new(),
            canonical_hashes: HashMap::new(),
        }
    }
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(MemoryStorageInner::new())),
        }
    }

    pub fn set_block_hash(&self, block_hash: B256, block_number: BlockNumber) {
        let mut inner = self.inner.write();
        inner.canonical_hashes.insert(block_number, block_hash);
    }

    pub fn set_block_header(&self, block_hash: B256, header: Header) {
        let mut inner = self.inner.write();
        inner.headers.insert(block_hash, header);
    }

    // todo: why blockhash is needed? aren't we storing latest block's state only memory?
    pub fn get_account_info_by_hash(
        &self,
        _block_hash: B256,
        address: Address,
    ) -> Result<Option<AccountInfo>, StorageError> {
        let inner = self.inner.read();
        if let Some(db_account) = inner.accounts.get(&address) {
            Ok(Some(db_account.info.clone()))
        } else {
            Ok(None)
        }
    }

    // todo: why blockhash is needed? aren't we storing latest block's state only memory?
    pub fn get_storage_at_hash(
        &self,
        _block_hash: B256,
        address: Address,
        storage_key: U256,
    ) -> Result<Option<U256>, StorageError> {
        let inner = self.inner.read();
        if let Some(db_account) = inner.accounts.get(&address) {
            Ok(db_account.storage.get(&storage_key).copied())
        } else {
            Ok(None)
        }
    }

    pub fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Header>, StorageError> {
        let inner = self.inner.read();
        if let Some(block_hash) = inner.canonical_hashes.get(&block_number) {
            Ok(inner.headers.get(block_hash).cloned())
        } else {
            Ok(None)
        }
    }

    pub fn get_block_header_by_hash(
        &self,
        block_hash: B256,
    ) -> Result<Option<SealedHeader>, StorageError> {
        let inner = self.inner.read();
        if let Some(header) = inner.headers.get(&block_hash) {
            Ok(Some(SealedHeader::new(header.clone(), block_hash)))
        } else {
            Ok(None)
        }
    }
}
