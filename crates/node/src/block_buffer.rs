use crate::tree::metrics::BlockBufferMetrics;
use alloy_consensus::BlockHeader;
use alloy_primitives::{BlockHash, BlockNumber};
use reth_primitives_traits::{Block, RecoveredBlock};
use schnellru::{ByLength, LruMap};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Contains the tree of pending blocks that cannot be executed due to missing parent.
/// It allows to store unconnected blocks for potential future inclusion.
///
/// The buffer has three main functionalities:
/// * [`BlockBuffer::insert_block`] for inserting blocks inside the buffer.
/// * [`BlockBuffer::remove_block_with_children`] for connecting blocks if the parent gets received
///   and inserted.
/// * [`BlockBuffer::remove_old_blocks`] to remove old blocks that precede the finalized number.
///
/// Note: Buffer is limited by number of blocks that it can contain and eviction of the block
/// is done by last recently used block.
#[derive(Debug)]
pub struct BlockBuffer<B: Block> {
    /// All blocks in the buffer stored by their block hash.
    pub(crate) blocks: HashMap<BlockHash, RecoveredBlock<B>>,
    /// Map of any parent block hash (even the ones not currently in the buffer)
    /// to the buffered children.
    /// Allows connecting buffered blocks by parent.
    pub(crate) parent_to_child: HashMap<BlockHash, HashSet<BlockHash>>,
    /// `BTreeMap` tracking the earliest blocks by block number.
    /// Used for removal of old blocks that precede finalization.
    pub(crate) earliest_blocks: BTreeMap<BlockNumber, HashSet<BlockHash>>,
    /// LRU used for tracing oldest inserted blocks that are going to be
    /// first in line for evicting if `max_blocks` limit is hit.
    ///
    /// Used as counter of amount of blocks inside buffer.
    pub(crate) lru: LruMap<BlockHash, ()>,
    /// Various metrics for the block buffer.
    pub(crate) metrics: BlockBufferMetrics,
}

impl<B: Block> BlockBuffer<B> {
    /// Create new buffer with max limit of blocks
    pub fn new(limit: u32) -> Self {
        Self {
            blocks: Default::default(),
            parent_to_child: Default::default(),
            earliest_blocks: Default::default(),
            lru: LruMap::new(ByLength::new(limit)),
            metrics: Default::default(),
        }
    }

    /// Return reference to the requested block.
    pub fn block(&self, hash: &BlockHash) -> Option<&RecoveredBlock<B>> {
        self.blocks.get(hash)
    }

    /// Return a reference to the lowest ancestor of the given block in the buffer.
    pub fn lowest_ancestor(&self, hash: &BlockHash) -> Option<&RecoveredBlock<B>> {
        let mut current_block = self.blocks.get(hash)?;
        while let Some(parent) = self.blocks.get(&current_block.parent_hash()) {
            current_block = parent;
        }
        Some(current_block)
    }

    /// Insert a correct block inside the buffer.
    pub fn insert_block(&mut self, block: RecoveredBlock<B>) {
        let hash = block.hash();

        self.parent_to_child
            .entry(block.parent_hash())
            .or_default()
            .insert(hash);
        self.earliest_blocks
            .entry(block.number())
            .or_default()
            .insert(hash);
        self.blocks.insert(hash, block);

        if let Some(evicted_hash) = self.insert_hash_and_get_evicted(hash) {
            // evict the block if limit is hit
            if let Some(evicted_block) = self.remove_block(&evicted_hash) {
                // evict the block if limit is hit
                self.remove_from_parent(evicted_block.parent_hash(), &evicted_hash);
            }
        }
        self.metrics.blocks.set(self.blocks.len() as f64);
    }

    /// Inserts the hash and returns the oldest evicted hash if any.
    fn insert_hash_and_get_evicted(&mut self, entry: BlockHash) -> Option<BlockHash> {
        let new = self.lru.peek(&entry).is_none();
        let evicted = if new && self.lru.limiter().max_length() as usize <= self.lru.len() {
            self.lru.pop_oldest().map(|(k, ())| k)
        } else {
            None
        };
        self.lru.get_or_insert(entry, || ());
        evicted
    }

    /// Removes the given block from the buffer and also all the children of the block.
    ///
    /// This is used to get all the blocks that are dependent on the block that is included.
    ///
    /// Note: that order of returned blocks is important and the blocks with lower block number
    /// in the chain will come first so that they can be executed in the correct order.
    pub fn remove_block_with_children(
        &mut self,
        parent_hash: &BlockHash,
    ) -> Vec<RecoveredBlock<B>> {
        let removed = self
            .remove_block(parent_hash)
            .into_iter()
            .chain(self.remove_children(vec![*parent_hash]))
            .collect();
        self.metrics.blocks.set(self.blocks.len() as f64);
        removed
    }

    /// Discard all blocks that precede block number from the buffer.
    pub fn remove_old_blocks(&mut self, block_number: BlockNumber) {
        let mut block_hashes_to_remove = Vec::new();

        // discard all blocks that are before the finalized number.
        while let Some(entry) = self.earliest_blocks.first_entry() {
            if *entry.key() > block_number {
                break;
            }
            let block_hashes = entry.remove();
            block_hashes_to_remove.extend(block_hashes);
        }

        // remove from other collections.
        for block_hash in &block_hashes_to_remove {
            // It's fine to call
            self.remove_block(block_hash);
        }

        self.remove_children(block_hashes_to_remove);
        self.metrics.blocks.set(self.blocks.len() as f64);
    }

    /// Remove block entry
    fn remove_from_earliest_blocks(&mut self, number: BlockNumber, hash: &BlockHash) {
        if let Some(entry) = self.earliest_blocks.get_mut(&number) {
            entry.remove(hash);
            if entry.is_empty() {
                self.earliest_blocks.remove(&number);
            }
        }
    }

    /// Remove from parent child connection. This method does not remove children.
    fn remove_from_parent(&mut self, parent_hash: BlockHash, hash: &BlockHash) {
        // remove from parent to child connection, but only for this block parent.
        if let Some(entry) = self.parent_to_child.get_mut(&parent_hash) {
            entry.remove(hash);
            // if set is empty remove block entry.
            if entry.is_empty() {
                self.parent_to_child.remove(&parent_hash);
            }
        }
    }

    /// Removes block from inner collections.
    /// This method will only remove the block if it's present inside `self.blocks`.
    /// The block might be missing from other collections, the method will only ensure that it has
    /// been removed.
    fn remove_block(&mut self, hash: &BlockHash) -> Option<RecoveredBlock<B>> {
        let block = self.blocks.remove(hash)?;
        self.remove_from_earliest_blocks(block.number(), hash);
        self.remove_from_parent(block.parent_hash(), hash);
        self.lru.remove(hash);
        Some(block)
    }

    /// Remove all children and their descendants for the given blocks and return them.
    fn remove_children(&mut self, parent_hashes: Vec<BlockHash>) -> Vec<RecoveredBlock<B>> {
        // remove all parent child connection and all the child children blocks that are connected
        // to the discarded parent blocks.
        let mut remove_parent_children = parent_hashes;
        let mut removed_blocks = Vec::new();
        while let Some(parent_hash) = remove_parent_children.pop() {
            // get this child blocks children and add them to the remove list.
            if let Some(parent_children) = self.parent_to_child.remove(&parent_hash) {
                // remove child from buffer
                for child_hash in &parent_children {
                    if let Some(block) = self.remove_block(child_hash) {
                        removed_blocks.push(block);
                    }
                }
                remove_parent_children.extend(parent_children);
            }
        }
        removed_blocks
    }
}
