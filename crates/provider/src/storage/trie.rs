use std::collections::{btree_map, hash_map, BTreeMap, HashMap, HashSet};

use alloy_eips::BlockNumHash;
use alloy_primitives::{BlockNumber, B256};
use reth_primitives::Header;
use tracing::{debug, info};

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
        Self {
            number: value.best_number,
            hash: value.best_hash,
        }
    }
}

/// Keeps track of the state of the tree.
///
/// ## Invariants
///
/// - This only stores blocks that are connected to the canonical chain.
/// - All executed blocks are valid and have been executed.
#[derive(Debug, Default)]
pub struct TreeState {
    /// __All__ unique executed blocks by block hash that are connected to the canonical chain.
    ///
    /// This includes blocks of all forks.
    blocks_by_hash: HashMap<B256, Header>,
    /// Executed blocks grouped by their respective block number.
    ///
    /// This maps unique block number to all known blocks for that height.
    ///
    /// Note: there can be multiple blocks at the same height due to forks.
    blocks_by_number: BTreeMap<BlockNumber, Vec<Header>>,
    /// Map of any parent block hash to its children.
    parent_to_child: HashMap<B256, HashSet<B256>>,

    /// Currently tracked canonical head of the chain.
    current_canonical_head: BlockNumHash,
}

impl TreeState {
    /// Returns a new, empty tree state that points to the given canonical head.
    pub fn new(current_canonical_head: BlockNumHash) -> Self {
        Self {
            blocks_by_hash: HashMap::default(),
            blocks_by_number: BTreeMap::new(),
            current_canonical_head,
            parent_to_child: HashMap::default(),
        }
    }

    /// Returns the [`Header`] by hash.
    pub(crate) fn executed_block_by_hash(&self, hash: B256) -> Option<&Header> {
        self.blocks_by_hash.get(&hash)
    }

    /// Insert executed block into the state.
    pub(crate) fn insert_executed(&mut self, executed: Header) {
        let hash = executed.hash_slow();
        let parent_hash = executed.parent_hash;
        let block_number = executed.number;

        if self.blocks_by_hash.contains_key(&hash) {
            return;
        }

        self.blocks_by_hash.insert(hash, executed.clone());

        self.blocks_by_number
            .entry(block_number)
            .or_default()
            .push(executed);

        self.parent_to_child
            .entry(parent_hash)
            .or_default()
            .insert(hash);

        if let Some(existing_blocks) = self.blocks_by_number.get(&block_number) {
            if existing_blocks.len() > 1 {
                self.parent_to_child
                    .entry(parent_hash)
                    .or_default()
                    .insert(hash);
            }
        }

        for children in self.parent_to_child.values_mut() {
            children.retain(|child| self.blocks_by_hash.contains_key(child));
        }
    }

    /// Returns whether or not the hash is part of the canonical chain.
    pub(crate) fn is_canonical(&self, hash: B256) -> bool {
        info!("current head:{:?}", self.current_canonical_head);
        let mut current_block = self.current_canonical_head.hash;
        if current_block == hash {
            return true;
        }

        while let Some(executed) = self.blocks_by_hash.get(&current_block) {
            current_block = executed.parent_hash;
            if current_block == hash {
                return true;
            }
        }

        false
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
        if !self.is_canonical(last_persisted_hash) {
            return;
        }

        // First, let's walk back the canonical chain and remove canonical blocks lower than the
        // upper bound
        let mut current_block = self.current_canonical_head.hash;
        while let Some(executed) = self.blocks_by_hash.get(&current_block) {
            current_block = executed.parent_hash;
            if executed.number <= upper_bound {
                debug!(target: "engine::tree", number=?executed.number, "Attempting to remove block walking back from the head");
                if let Some((removed, _)) = self.remove_by_hash(executed.hash_slow()) {
                    debug!(target: "engine::tree", number=?removed.number, "Removed block walking back from the head");
                }
            }
        }
        debug!(target: "engine::tree", ?upper_bound, ?last_persisted_hash, "Removed canonical blocks from the tree");
    }

    /// Remove single executed block by its hash.
    ///
    /// ## Returns
    ///
    /// The removed block and the block hashes of its children.
    fn remove_by_hash(&mut self, hash: B256) -> Option<(Header, HashSet<B256>)> {
        let executed = self.blocks_by_hash.remove(&hash)?;

        // Remove this block from collection of children of its parent block.
        let parent_entry = self.parent_to_child.entry(executed.parent_hash);
        if let hash_map::Entry::Occupied(mut entry) = parent_entry {
            entry.get_mut().remove(&hash);

            if entry.get().is_empty() {
                entry.remove();
            }
        }

        // Remove point to children of this block.
        let children = self.parent_to_child.remove(&hash).unwrap_or_default();

        // Remove this block from `blocks_by_number`.
        let block_number_entry = self.blocks_by_number.entry(executed.number);
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

        Some((executed, children))
    }

    /// Updates the canonical head to the given block.
    pub(crate) fn set_canonical_head(&mut self, new_head: BlockNumHash) {
        self.current_canonical_head = new_head;
    }
}
