use std::{collections::HashMap, sync::Arc};

use alloy_primitives::{BlockHash, BlockNumber, B256};
use parking_lot::RwLock;
use reth_primitives::{Header, SealedHeader};

use crate::errors::StorageError;

#[derive(Debug, Clone)]
pub struct MemoryStorage {
    inner: Arc<RwLock<MemoryStorageInner>>,
}

#[derive(Debug)]
pub struct MemoryStorageInner {
    /// keep historical headers for validations
    headers: HashMap<BlockHash, Header>,
    /// keep historical 256 block's hash
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

    pub fn overwrite_block_hashes(&self, block_hashes: HashMap<BlockNumber, B256>) {
        let mut inner = self.inner.write();
        inner.canonical_hashes = block_hashes;
    }

    pub fn set_block_hash(&self, block_hash: B256, block_number: BlockNumber) {
        let mut inner = self.inner.write();
        inner.canonical_hashes.insert(block_number, block_hash);
    }

    pub fn set_block_header(&self, block_hash: B256, header: Header) {
        let mut inner = self.inner.write();
        inner.headers.insert(block_hash, header);
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

    pub fn get_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<BlockHash>, StorageError> {
        let inner = self.inner.read();
        if let Some(block_hash) = inner.canonical_hashes.get(&block_number) {
            Ok(Some(*block_hash))
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
