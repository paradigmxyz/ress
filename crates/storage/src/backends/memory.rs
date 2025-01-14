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

    pub fn remove_oldest_block(&self) {
        let mut inner = self.inner.write();

        if let Some(&oldest_block_number) = inner.canonical_hashes.keys().min() {
            let block_hash = inner.canonical_hashes.remove(&oldest_block_number);
            if let Some(block_hash) = block_hash {
                inner.headers.remove(&block_hash);
            }
        }
    }

    pub fn is_canonical_blocks_exist(&self, target_block: BlockNumber) -> bool {
        let inner = self.inner.read();
        (target_block.saturating_sub(255)..=target_block).all(|block_number| {
            inner.canonical_hashes.contains_key(&block_number)
                && inner.headers.contains_key(
                    inner
                        .canonical_hashes
                        .get(&block_number)
                        .unwrap_or(&BlockHash::default()),
                )
        })
    }

    pub fn overwrite_block_headers(&self, block_headers: HashMap<BlockHash, Header>) {
        let mut inner = self.inner.write();
        inner.headers = block_headers;
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
