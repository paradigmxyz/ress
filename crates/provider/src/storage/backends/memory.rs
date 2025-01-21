use alloy_eips::BlockNumHash;
use alloy_primitives::{BlockHash, BlockNumber, B256};
use parking_lot::RwLock;
use reth_primitives::{Header, SealedHeader};
use std::{collections::HashMap, sync::Arc};

use crate::{errors::MemoryStorageError, storage::trie::TreeState};

#[derive(Debug)]
pub struct MemoryStorage {
    inner: Arc<RwLock<MemoryStorageInner>>,
}

#[derive(Debug)]
pub struct MemoryStorageInner {
    /// tracking unfinalized tree state
    tree_state: TreeState,
    /// keep historical headers for validations
    headers: HashMap<BlockHash, Header>,
    /// keep historical 256 block's hash
    canonical_hashes: HashMap<BlockNumber, BlockHash>,
}

impl MemoryStorageInner {
    pub fn new(current_canonical_head: BlockNumHash) -> Self {
        Self {
            headers: HashMap::new(),
            canonical_hashes: HashMap::new(),
            tree_state: TreeState::new(current_canonical_head),
        }
    }
}

impl MemoryStorage {
    pub fn new(current_canonical_head: BlockNumHash) -> Self {
        Self {
            inner: Arc::new(RwLock::new(MemoryStorageInner::new(current_canonical_head))),
        }
    }

    pub(crate) fn executed_block_by_hash(&self, hash: B256) -> Option<Header> {
        let inner = self.inner.read();
        inner.tree_state.executed_block_by_hash(hash).cloned()
    }

    /// Insert executed block into the state.
    pub(crate) fn insert_executed(&self, executed: Header) {
        let mut inner = self.inner.write();
        inner.tree_state.insert_executed(executed);
    }

    /// Returns whether or not the hash is part of the canonical chain.
    pub(crate) fn is_canonical(&self, hash: B256) -> bool {
        let inner = self.inner.read();
        inner.tree_state.is_canonical(hash)
    }

    pub(crate) fn find_block_hash(&self, block_hash: BlockHash) -> bool {
        let inner = self.inner.read();
        inner
            .canonical_hashes
            .values()
            .any(|&hash| hash == block_hash)
    }

    pub(crate) fn remove_oldest_block(&self) {
        let mut inner = self.inner.write();
        if let Some(&oldest_block_number) = inner.canonical_hashes.keys().min() {
            let block_hash = inner.canonical_hashes.remove(&oldest_block_number);
            if let Some(block_hash) = block_hash {
                inner.headers.remove(&block_hash);
            }
        }
    }

    pub(crate) fn is_canonical_hashes_exist(&self, target_block: BlockNumber) -> bool {
        let inner = self.inner.read();
        (target_block.saturating_sub(255)..target_block)
            .all(|block_number| inner.canonical_hashes.contains_key(&block_number))
    }

    pub(crate) fn get_latest_block_hash(&self) -> Option<BlockHash> {
        let inner = self.inner.read();
        if let Some(&latest_block_number) = inner.canonical_hashes.keys().max() {
            inner.canonical_hashes.get(&latest_block_number).copied()
        } else {
            None
        }
    }

    pub(crate) fn overwrite_block_hashes(&self, block_hashes: HashMap<BlockNumber, B256>) {
        let mut inner = self.inner.write();
        inner.canonical_hashes = block_hashes;
    }

    pub(crate) fn set_block_hash(&self, block_hash: B256, block_number: BlockNumber) {
        let mut inner = self.inner.write();
        inner.canonical_hashes.insert(block_number, block_hash);
    }

    pub(crate) fn set_block_header(&self, block_hash: B256, header: Header) {
        let mut inner = self.inner.write();
        inner.headers.insert(block_hash, header);
    }

    pub(crate) fn get_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<BlockHash, MemoryStorageError> {
        let inner = self.inner.read();
        if let Some(block_hash) = inner.canonical_hashes.get(&block_number) {
            Ok(*block_hash)
        } else {
            Err(MemoryStorageError::BlockNotFound(block_number))
        }
    }

    pub(crate) fn get_block_header_by_hash(
        &self,
        block_hash: B256,
    ) -> Result<Option<SealedHeader>, MemoryStorageError> {
        let inner = self.inner.read();
        if let Some(header) = inner.headers.get(&block_hash) {
            Ok(Some(SealedHeader::new(header.clone(), block_hash)))
        } else {
            Ok(None)
        }
    }

    pub(crate) fn set_canonical_head(&self, new_head: BlockNumHash) {
        let mut inner = self.inner.write();
        inner.tree_state.set_canonical_head(new_head);
    }
}
