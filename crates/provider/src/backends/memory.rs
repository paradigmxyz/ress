use alloy_eips::BlockNumHash;
use alloy_primitives::{BlockHash, BlockNumber, B256};
use parking_lot::RwLock;
use reth_primitives::Header;
use std::{
    collections::{
        btree_map,
        hash_map::{self, Entry},
        BTreeMap, HashMap, HashSet,
    },
    sync::Arc,
};
use tracing::debug;

use crate::errors::MemoryStorageError;

/// Keeps track of the state of the tree.
///
/// ## Invariants
///
/// - This only stores headers that are connected to the canonical chain.
/// - All executed headers are valid and have been executed.
#[derive(Clone, Debug)]
pub struct MemoryStorage {
    inner: Arc<RwLock<MemoryStorageInner>>,
}

impl MemoryStorage {
    /// Create new in-memory storage.
    pub fn new(current_canonical_head: BlockNumHash) -> Self {
        Self { inner: Arc::new(RwLock::new(MemoryStorageInner::new(current_canonical_head))) }
    }

    /// Removes canonical blocks below the upper bound, only if the last persisted hash is
    /// part of the canonical chain.
    pub(crate) fn remove_canonical_until(
        &self,
        upper_bound: BlockNumber,
        last_persisted_hash: B256,
    ) {
        let mut inner = self.inner.write();
        inner.remove_canonical_until(upper_bound, last_persisted_hash);
    }

    pub(crate) fn header_by_hash(&self, hash: B256) -> Option<Header> {
        let inner = self.inner.read();
        inner.headers_by_hash.get(&hash).cloned()
    }

    /// Insert header into the state.
    pub(crate) fn insert_header(&self, header: Header) {
        let mut inner = self.inner.write();
        inner.insert_header(header);
    }

    /// Inserts canonical hash for block number.
    pub(crate) fn insert_canonical_hash(&self, number: BlockNumber, hash: BlockHash) {
        self.inner.write().canonical_hashes.insert(number, hash);
    }

    /// Remove canonical hash for block number if it matches.
    pub(crate) fn remove_canonical_hash(&self, number: BlockNumber, hash: BlockHash) {
        let mut inner = self.inner.write();
        if let Entry::Occupied(entry) = inner.canonical_hashes.entry(number) {
            if entry.get() == &hash {
                entry.remove();
            }
        }
    }

    /// Return whether or not the hash is part of the canonical chain.
    ///
    /// This method is simply look up the canonical hashmap
    pub(crate) fn is_canonical_lookup(&self, hash: B256) -> bool {
        let inner = self.inner.read();
        inner.is_canonical_lookup(hash)
    }

    pub(crate) fn overwrite_block_hashes(&self, block_hashes: HashMap<BlockNumber, B256>) {
        let mut inner = self.inner.write();
        inner.canonical_hashes = block_hashes;
    }

    pub(crate) fn get_canonical_head(&self) -> BlockNumHash {
        let inner = self.inner.read();
        inner.current_canonical_head
    }

    pub(crate) fn set_canonical_hash(
        &self,
        block_hash: B256,
        block_number: BlockNumber,
    ) -> Result<(), MemoryStorageError> {
        let mut inner = self.inner.write();
        if inner.is_canonical_by_walk_through(block_hash) {
            inner.canonical_hashes.insert(block_number, block_hash);
            Ok(())
        } else {
            Err(MemoryStorageError::NonCanonicalChain(block_hash))
        }
    }

    pub(crate) fn get_block_hash(
        &self,
        block_number: BlockNumber,
    ) -> Result<BlockHash, MemoryStorageError> {
        let inner = self.inner.read();
        if let Some(block_hash) = inner.canonical_hashes.get(&block_number) {
            Ok(*block_hash)
        } else {
            Err(MemoryStorageError::BlockNotFoundFromNumber(block_number))
        }
    }

    pub(crate) fn get_block_number(
        &self,
        block_hash: BlockHash,
    ) -> Result<BlockNumber, MemoryStorageError> {
        let inner = self.inner.read();
        if let Some(header) = inner.headers_by_hash.get(&block_hash).cloned() {
            Ok(header.number)
        } else {
            Err(MemoryStorageError::BlockNotFoundFromHash(block_hash))
        }
    }

    pub(crate) fn set_canonical_head(&self, new_head: BlockNumHash) {
        let mut inner = self.inner.write();
        inner.set_canonical_head(new_head);
    }
}

#[derive(Debug, Default)]
struct MemoryStorageInner {
    /// __All__ unique executed headers by block hash that are connected to the canonical chain.
    ///
    /// This includes headers of all forks.
    headers_by_hash: HashMap<B256, Header>,
    /// Executed headers grouped by their respective block number.
    ///
    /// This maps unique block number to all known headers for that height.
    ///
    /// Note: there can be multiple headers at the same height due to forks.
    headers_by_number: BTreeMap<BlockNumber, Vec<Header>>,
    /// Map of any parent block hash to its children.
    parent_to_child: HashMap<B256, HashSet<B256>>,

    /// Currently tracked canonical head of the chain.
    current_canonical_head: BlockNumHash,

    /// Keep canonical 256 blocks hash from current_canonical_head
    ///
    /// This is for faster lookup `BLOCK_HASH` opcode
    canonical_hashes: HashMap<BlockNumber, BlockHash>,
}

impl MemoryStorageInner {
    /// Return a new, empty tree state that points to the given canonical head.
    fn new(current_canonical_head: BlockNumHash) -> Self {
        Self {
            canonical_hashes: HashMap::new(),
            headers_by_hash: HashMap::default(),
            headers_by_number: BTreeMap::new(),
            current_canonical_head,
            parent_to_child: HashMap::default(),
        }
    }

    /// Return whether or not the hash is part of the canonical chain.
    ///
    /// This method takes O(n) of complexity by walk through all the executed headers to check
    /// canonical chain.
    pub(crate) fn is_canonical_by_walk_through(&self, hash: B256) -> bool {
        let mut current_block = self.current_canonical_head.hash;
        if current_block == hash {
            return true;
        }

        while let Some(executed) = self.headers_by_hash.get(&current_block) {
            current_block = executed.parent_hash;
            if current_block == hash {
                return true;
            }
        }

        false
    }

    /// Return whether or not the hash is part of the canonical chain.
    ///
    /// This method is simply look up the canonical hashmap
    pub(crate) fn is_canonical_lookup(&self, hash: B256) -> bool {
        self.canonical_hashes.values().any(|&canonical_hash| canonical_hash == hash)
    }

    /// Removes canonical blocks below the upper bound, only if the last persisted hash is
    /// part of the canonical chain.
    pub(crate) fn remove_canonical_until(
        &mut self,
        upper_bound: BlockNumber,
        last_persisted_hash: B256,
    ) {
        debug!(target: "engine::tree", ?upper_bound, ?last_persisted_hash, "Removing canonical blocks from the tree");

        // If the last persisted hash is not canonical, then we don't want to remove any canonical
        // blocks yet.
        if !self.is_canonical_lookup(last_persisted_hash) {
            return;
        }

        // First, let's walk back the canonical chain and remove canonical blocks lower than the
        // upper bound
        let mut current_hash = self.current_canonical_head.hash;
        while let Some(header) = self.headers_by_hash.get(&current_hash) {
            let next_hash = header.parent_hash;
            // we don't want to remove upper bound
            if header.number < upper_bound {
                debug!(target: "engine::tree", number=header.number, "Attempting to remove block walking back from the head");
                if let Some((removed, _)) = self.remove_by_hash(current_hash) {
                    debug!(target: "engine::tree", number=removed.number, "Removed block walking back from the head");
                }
            }
            current_hash = next_hash;
        }
        debug!(target: "engine::tree", ?upper_bound, ?last_persisted_hash, "Removed canonical blocks from the tree");
    }

    /// Remove single executed block by its hash.
    ///
    /// ## Returns
    ///
    /// The removed block and the block hashes of its children.
    fn remove_by_hash(&mut self, hash: B256) -> Option<(Header, HashSet<B256>)> {
        let header = self.headers_by_hash.remove(&hash)?;

        // Remove this block from collection of children of its parent block.
        let parent_entry = self.parent_to_child.entry(header.parent_hash);
        if let hash_map::Entry::Occupied(mut entry) = parent_entry {
            entry.get_mut().remove(&hash);

            if entry.get().is_empty() {
                entry.remove();
            }
        }

        // Remove point to children of this block.
        let children = self.parent_to_child.remove(&hash).unwrap_or_default();

        // Remove this block from `headers_by_number`.
        let block_number_entry = self.headers_by_number.entry(header.number);
        if let btree_map::Entry::Occupied(mut entry) = block_number_entry {
            // We have to find the index of the block since it exists in a vec
            if let Some(index) = entry.get().iter().position(|b| b.hash_slow() == hash) {
                entry.get_mut().swap_remove(index);

                // If there are no blocks left then remove the entry for this block
                if entry.get().is_empty() {
                    entry.remove();
                }
            }
        }

        Some((header, children))
    }

    /// Insert header into the state.
    ///
    /// This does not update any canonical chain regarding information.
    pub(crate) fn insert_header(&mut self, executed: Header) {
        let hash = executed.hash_slow();
        let parent_hash = executed.parent_hash;
        let block_number = executed.number;

        if self.headers_by_hash.contains_key(&hash) {
            return;
        }

        self.headers_by_hash.insert(hash, executed.clone());
        self.headers_by_number.entry(block_number).or_default().push(executed);

        self.parent_to_child.entry(parent_hash).or_default().insert(hash);

        if let Some(existing_blocks) = self.headers_by_number.get(&block_number) {
            if existing_blocks.len() > 1 {
                self.parent_to_child.entry(parent_hash).or_default().insert(hash);
            }
        }

        for children in self.parent_to_child.values_mut() {
            children.retain(|child| self.headers_by_hash.contains_key(child));
        }
    }

    /// Updates the canonical head to the given block.
    pub(crate) fn set_canonical_head(&mut self, new_head: BlockNumHash) {
        self.current_canonical_head = new_head;
    }
}

/// Current status of the blockchain's head.
#[derive(Default, Copy, Clone, Debug, Eq, PartialEq)]
pub struct ChainInfo {
    /// The block hash of the highest fully synced block.
    pub best_hash: B256,
    /// The block number of the highest fully synced block.
    pub best_number: BlockNumber,
}

impl From<ChainInfo> for BlockNumHash {
    fn from(value: ChainInfo) -> Self {
        Self { number: value.best_number, hash: value.best_hash }
    }
}
