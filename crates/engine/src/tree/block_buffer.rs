use alloy_consensus::BlockHeader;
use alloy_primitives::{map::B256HashSet, BlockHash, BlockNumber, B256};
use metrics::Gauge;
use ress_primitives::witness::ExecutionWitness;
use reth_metrics::Metrics;
use reth_primitives_traits::{Block, RecoveredBlock};
use schnellru::{ByLength, LruMap};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Metrics for the blockchain tree block buffer
#[derive(Metrics)]
#[metrics(scope = "engine.tree.block_buffer")]
pub(crate) struct BlockBufferMetrics {
    /// Total blocks in the block buffer.
    pub blocks: Gauge,
    /// Total witnesses in the block buffer.
    pub witnesses: Gauge,
}

/// Contains the tree of pending blocks that cannot be executed due to missing parent.
/// It allows to store unconnected blocks for potential future inclusion.
///
/// The buffer has three main functionalities:
/// * [`BlockBuffer::insert_block`] for inserting blocks inside the buffer.
/// * [`BlockBuffer::remove_block_with_children`] for connecting blocks if the parent gets received
///   and inserted.
/// * [`BlockBuffer::evict_old_blocks`] to evict old blocks that precede the finalized number.
///
/// Note: Buffer is limited by number of blocks that it can contain and eviction of the block
/// is done by last recently used block.
#[derive(Debug)]
pub struct BlockBuffer<B: Block> {
    /// All blocks in the buffer stored by their block hash.
    pub(crate) blocks: HashMap<BlockHash, RecoveredBlock<B>>,
    /// All witnesses stored by their block hash.
    pub(crate) witnesses: HashMap<BlockHash, ExecutionWitness>,
    /// Missing bytecodes by block hash.
    pub(crate) missing_bytecodes: HashMap<BlockHash, B256HashSet>,
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
            witnesses: Default::default(),
            missing_bytecodes: Default::default(),
            parent_to_child: Default::default(),
            earliest_blocks: Default::default(),
            lru: LruMap::new(ByLength::new(limit)),
            metrics: Default::default(),
        }
    }

    /// Return reference to the requested witness.
    pub fn witness(&self, hash: &BlockHash) -> Option<&ExecutionWitness> {
        self.witnesses.get(hash)
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

        self.parent_to_child.entry(block.parent_hash()).or_default().insert(hash);
        self.earliest_blocks.entry(block.number()).or_default().insert(hash);
        self.blocks.insert(hash, block);

        if let Some(evicted_hash) = self.insert_hash_and_get_evicted(hash) {
            // evict the block if limit is hit
            if let Some(evicted_block) = self.evict_block(&evicted_hash) {
                // evict the block if limit is hit
                self.remove_from_parent(evicted_block.parent_hash(), &evicted_hash);
            }
        }
        self.metrics.blocks.set(self.blocks.len() as f64);
    }

    /// Insert a witness in the buffer.
    pub fn insert_witness(
        &mut self,
        block_hash: BlockHash,
        witness: ExecutionWitness,
        missing_bytecodes: B256HashSet,
    ) {
        self.witnesses.insert(block_hash, witness);
        self.metrics.witnesses.set(self.witnesses.len() as f64);
        if !missing_bytecodes.is_empty() {
            self.missing_bytecodes.insert(block_hash, missing_bytecodes);
        }
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
        parent_hash: BlockHash,
    ) -> Vec<(RecoveredBlock<B>, ExecutionWitness)> {
        let removed = self
            .remove_block(&parent_hash)
            .into_iter()
            .chain(self.remove_children(Vec::from([parent_hash])))
            .collect();
        self.metrics.blocks.set(self.blocks.len() as f64);
        self.metrics.witnesses.set(self.witnesses.len() as f64);
        removed
    }

    /// Discard all blocks that precede block number from the buffer.
    pub fn evict_old_blocks(&mut self, block_number: BlockNumber) {
        let mut block_hashes_to_remove = Vec::new();

        // discard all blocks that are before the finalized number.
        while let Some(entry) = self.earliest_blocks.first_entry() {
            if *entry.key() > block_number {
                break
            }
            let block_hashes = entry.remove();
            block_hashes_to_remove.extend(block_hashes);
        }

        // remove from other collections.
        for block_hash in &block_hashes_to_remove {
            // It's fine to call
            self.evict_block(block_hash);
        }

        self.evict_children(block_hashes_to_remove);
        self.metrics.blocks.set(self.blocks.len() as f64);
        self.metrics.witnesses.set(self.witnesses.len() as f64);
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
    pub fn remove_block(
        &mut self,
        hash: &BlockHash,
    ) -> Option<(RecoveredBlock<B>, ExecutionWitness)> {
        if !self.blocks.contains_key(hash) {
            return None
        }
        let witness = self.remove_witness(hash)?;
        let block = self.blocks.remove(hash).unwrap();
        self.remove_from_earliest_blocks(block.number(), hash);
        self.remove_from_parent(block.parent_hash(), hash);
        self.lru.remove(hash);
        Some((block, witness))
    }

    /// Evicts the block from inner collections.
    /// This method will only remove the block if it's present inside `self.blocks`.
    fn evict_block(&mut self, hash: &BlockHash) -> Option<RecoveredBlock<B>> {
        let block = self.blocks.remove(hash)?;
        self.witnesses.remove(hash);
        self.missing_bytecodes.remove(hash);
        self.remove_from_earliest_blocks(block.number(), hash);
        self.remove_from_parent(block.parent_hash(), hash);
        self.lru.remove(hash);
        Some(block)
    }

    /// Remove all children and their descendants for the given blocks and return them.
    fn remove_children(
        &mut self,
        parent_hashes: Vec<BlockHash>,
    ) -> Vec<(RecoveredBlock<B>, ExecutionWitness)> {
        // remove all parent child connection and all the child children blocks that are connected
        // to the discarded parent blocks.
        let mut remove_parent_children = parent_hashes;
        let mut removed_blocks = Vec::new();
        while let Some(parent_hash) = remove_parent_children.pop() {
            // get this child blocks children and add them to the remove list.
            if let Some(parent_children) = self.parent_to_child.remove(&parent_hash) {
                // remove child from buffer
                for child_hash in &parent_children {
                    if let Some((block, witness)) = self.remove_block(child_hash) {
                        removed_blocks.push((block, witness));
                    }
                }
                remove_parent_children.extend(parent_children);
            }
        }
        removed_blocks
    }

    /// Remove all children and their descendants for the given blocks and return them.
    fn evict_children(&mut self, parent_hashes: Vec<BlockHash>) {
        // remove all parent child connection and all the child children blocks that are connected
        // to the discarded parent blocks.
        let mut remove_parent_children = parent_hashes;
        while let Some(parent_hash) = remove_parent_children.pop() {
            // get this child blocks children and add them to the remove list.
            if let Some(parent_children) = self.parent_to_child.remove(&parent_hash) {
                // remove child from buffer
                for child_hash in &parent_children {
                    self.evict_block(child_hash);
                }
                remove_parent_children.extend(parent_children);
            }
        }
    }

    /// Remove witness from the buffer.
    pub fn remove_witness(&mut self, block_hash: &BlockHash) -> Option<ExecutionWitness> {
        // Remove the witness only if there are no missing bytecodes for it.
        if self.missing_bytecodes.get(block_hash).is_none_or(|b| b.is_empty()) {
            self.missing_bytecodes.remove(block_hash);
            return self.witnesses.remove(block_hash)
        }
        None
    }

    /// Update missing bytecodes on bytecode received.
    /// Returns block hashes that are ready for insertion.
    pub fn on_bytecode_received(&mut self, code_hash: B256) -> B256HashSet {
        let mut block_hashes = B256HashSet::default();
        self.missing_bytecodes.retain(|block_hash, missing| {
            missing.remove(&code_hash);
            if missing.is_empty() {
                block_hashes.insert(*block_hash);
                false
            } else {
                true
            }
        });
        block_hashes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_eips::BlockNumHash;
    use alloy_primitives::BlockHash;
    use reth_primitives_traits::RecoveredBlock;
    use reth_testing_utils::generators::{self, random_block, BlockParams, Rng};
    use std::collections::HashMap;

    /// Create random block with specified number and parent hash.
    fn create_block<R: Rng>(
        rng: &mut R,
        number: u64,
        parent: BlockHash,
    ) -> RecoveredBlock<reth_primitives::Block> {
        let block =
            random_block(rng, number, BlockParams { parent: Some(parent), ..Default::default() });
        block.try_recover().unwrap()
    }

    /// Insert block with default witness.
    fn insert_block_with_witness<B: Block>(buffer: &mut BlockBuffer<B>, block: RecoveredBlock<B>) {
        buffer.insert_witness(block.hash(), Default::default(), Default::default());
        buffer.insert_block(block);
    }

    /// Assert that all buffer collections have the same data length.
    fn assert_buffer_lengths<B: Block>(buffer: &BlockBuffer<B>, expected: usize) {
        assert_eq!(buffer.blocks.len(), expected);
        assert_eq!(buffer.lru.len(), expected);
        assert_eq!(
            buffer.parent_to_child.iter().fold(0, |acc, (_, hashes)| acc + hashes.len()),
            expected
        );
        assert_eq!(
            buffer.earliest_blocks.iter().fold(0, |acc, (_, hashes)| acc + hashes.len()),
            expected
        );
    }

    /// Assert that the block was removed from all buffer collections.
    fn assert_block_removal<B: Block>(
        buffer: &BlockBuffer<B>,
        block: &RecoveredBlock<reth_primitives::Block>,
    ) {
        assert!(!buffer.blocks.contains_key(&block.hash()));
        assert!(buffer
            .parent_to_child
            .get(&block.parent_hash)
            .and_then(|p| p.get(&block.hash()))
            .is_none());
        assert!(buffer
            .earliest_blocks
            .get(&block.number)
            .and_then(|hashes| hashes.get(&block.hash()))
            .is_none());
    }

    #[test]
    fn simple_insertion() {
        let mut rng = generators::rng();
        let parent = rng.random();
        let block1 = create_block(&mut rng, 10, parent);
        let mut buffer = BlockBuffer::new(3);

        buffer.insert_block(block1.clone());
        assert_buffer_lengths(&buffer, 1);
        assert_eq!(buffer.blocks.get(&block1.hash()), Some(&block1));
    }

    #[test]
    fn take_entire_chain_of_children() {
        let mut rng = generators::rng();

        let main_parent_hash = rng.random();
        let block1 = create_block(&mut rng, 10, main_parent_hash);
        let block2 = create_block(&mut rng, 11, block1.hash());
        let block3 = create_block(&mut rng, 12, block2.hash());
        let parent4 = rng.random();
        let block4 = create_block(&mut rng, 14, parent4);

        let mut buffer = BlockBuffer::new(5);

        insert_block_with_witness(&mut buffer, block1.clone());
        insert_block_with_witness(&mut buffer, block2.clone());
        insert_block_with_witness(&mut buffer, block3.clone());
        insert_block_with_witness(&mut buffer, block4.clone());

        assert_buffer_lengths(&buffer, 4);
        assert_eq!(buffer.blocks.get(&block4.hash()), Some(&block4));
        assert_eq!(buffer.blocks.get(&block2.hash()), Some(&block2));
        assert_eq!(buffer.blocks.get(&main_parent_hash), None);

        assert_eq!(buffer.lowest_ancestor(&block4.hash()), Some(&block4));
        assert_eq!(buffer.lowest_ancestor(&block3.hash()), Some(&block1));
        assert_eq!(buffer.lowest_ancestor(&block1.hash()), Some(&block1));
        assert_eq!(
            buffer.remove_block_with_children(main_parent_hash),
            Vec::from_iter([block1, block2, block3].map(|b| (b, Default::default())))
        );
        assert_buffer_lengths(&buffer, 1);
    }

    #[test]
    fn take_all_multi_level_children() {
        let mut rng = generators::rng();

        let main_parent_hash = rng.random();
        let block1 = create_block(&mut rng, 10, main_parent_hash);
        let block2 = create_block(&mut rng, 11, block1.hash());
        let block3 = create_block(&mut rng, 11, block1.hash());
        let block4 = create_block(&mut rng, 12, block2.hash());

        let mut buffer = BlockBuffer::new(5);

        insert_block_with_witness(&mut buffer, block1.clone());
        insert_block_with_witness(&mut buffer, block2.clone());
        insert_block_with_witness(&mut buffer, block3.clone());
        insert_block_with_witness(&mut buffer, block4.clone());

        assert_buffer_lengths(&buffer, 4);
        assert_eq!(
            buffer
                .remove_block_with_children(main_parent_hash)
                .into_iter()
                .map(|(b, _)| (b.hash(), b))
                .collect::<HashMap<_, _>>(),
            HashMap::from([
                (block1.hash(), block1),
                (block2.hash(), block2),
                (block3.hash(), block3),
                (block4.hash(), block4)
            ])
        );
        assert_buffer_lengths(&buffer, 0);
    }

    #[test]
    fn take_block_with_children() {
        let mut rng = generators::rng();

        let main_parent = BlockNumHash::new(9, rng.random());
        let block1 = create_block(&mut rng, 10, main_parent.hash);
        let block2 = create_block(&mut rng, 11, block1.hash());
        let block3 = create_block(&mut rng, 11, block1.hash());
        let block4 = create_block(&mut rng, 12, block2.hash());

        let mut buffer = BlockBuffer::new(5);

        insert_block_with_witness(&mut buffer, block1.clone());
        insert_block_with_witness(&mut buffer, block2.clone());
        insert_block_with_witness(&mut buffer, block3.clone());
        insert_block_with_witness(&mut buffer, block4.clone());

        assert_buffer_lengths(&buffer, 4);
        assert_eq!(
            buffer
                .remove_block_with_children(block1.hash())
                .into_iter()
                .map(|(b, _)| (b.hash(), b))
                .collect::<HashMap<_, _>>(),
            HashMap::from([
                (block1.hash(), block1),
                (block2.hash(), block2),
                (block3.hash(), block3),
                (block4.hash(), block4)
            ])
        );
        assert_buffer_lengths(&buffer, 0);
    }

    #[test]
    fn remove_chain_of_children() {
        let mut rng = generators::rng();

        let main_parent = BlockNumHash::new(9, rng.random());
        let block1 = create_block(&mut rng, 10, main_parent.hash);
        let block2 = create_block(&mut rng, 11, block1.hash());
        let block3 = create_block(&mut rng, 12, block2.hash());
        let parent4 = rng.random();
        let block4 = create_block(&mut rng, 14, parent4);

        let mut buffer = BlockBuffer::new(5);

        buffer.insert_block(block1.clone());
        buffer.insert_block(block2);
        buffer.insert_block(block3);
        buffer.insert_block(block4);

        assert_buffer_lengths(&buffer, 4);
        buffer.evict_old_blocks(block1.number);
        assert_buffer_lengths(&buffer, 1);
    }

    #[test]
    fn remove_all_multi_level_children() {
        let mut rng = generators::rng();

        let main_parent = BlockNumHash::new(9, rng.random());
        let block1 = create_block(&mut rng, 10, main_parent.hash);
        let block2 = create_block(&mut rng, 11, block1.hash());
        let block3 = create_block(&mut rng, 11, block1.hash());
        let block4 = create_block(&mut rng, 12, block2.hash());

        let mut buffer = BlockBuffer::new(5);

        buffer.insert_block(block1.clone());
        buffer.insert_block(block2);
        buffer.insert_block(block3);
        buffer.insert_block(block4);

        assert_buffer_lengths(&buffer, 4);
        buffer.evict_old_blocks(block1.number);
        assert_buffer_lengths(&buffer, 0);
    }

    #[test]
    fn remove_multi_chains() {
        let mut rng = generators::rng();

        let main_parent = BlockNumHash::new(9, rng.random());
        let block1 = create_block(&mut rng, 10, main_parent.hash);
        let block1a = create_block(&mut rng, 10, main_parent.hash);
        let block2 = create_block(&mut rng, 11, block1.hash());
        let block2a = create_block(&mut rng, 11, block1.hash());
        let random_parent1 = rng.random();
        let random_block1 = create_block(&mut rng, 10, random_parent1);
        let random_parent2 = rng.random();
        let random_block2 = create_block(&mut rng, 11, random_parent2);
        let random_parent3 = rng.random();
        let random_block3 = create_block(&mut rng, 12, random_parent3);

        let mut buffer = BlockBuffer::new(10);

        buffer.insert_block(block1.clone());
        buffer.insert_block(block1a.clone());
        buffer.insert_block(block2.clone());
        buffer.insert_block(block2a.clone());
        buffer.insert_block(random_block1.clone());
        buffer.insert_block(random_block2.clone());
        buffer.insert_block(random_block3.clone());

        // check that random blocks are their own ancestor, and that chains have proper ancestors
        assert_eq!(buffer.lowest_ancestor(&random_block1.hash()), Some(&random_block1));
        assert_eq!(buffer.lowest_ancestor(&random_block2.hash()), Some(&random_block2));
        assert_eq!(buffer.lowest_ancestor(&random_block3.hash()), Some(&random_block3));

        // descendants have ancestors
        assert_eq!(buffer.lowest_ancestor(&block2a.hash()), Some(&block1));
        assert_eq!(buffer.lowest_ancestor(&block2.hash()), Some(&block1));

        // roots are themselves
        assert_eq!(buffer.lowest_ancestor(&block1a.hash()), Some(&block1a));
        assert_eq!(buffer.lowest_ancestor(&block1.hash()), Some(&block1));

        assert_buffer_lengths(&buffer, 7);
        buffer.evict_old_blocks(10);
        assert_buffer_lengths(&buffer, 2);
    }

    #[test]
    fn evict_with_gap() {
        let mut rng = generators::rng();

        let main_parent = BlockNumHash::new(9, rng.random());
        let block1 = create_block(&mut rng, 10, main_parent.hash);
        let block2 = create_block(&mut rng, 11, block1.hash());
        let block3 = create_block(&mut rng, 12, block2.hash());
        let parent4 = rng.random();
        let block4 = create_block(&mut rng, 13, parent4);

        let mut buffer = BlockBuffer::new(3);

        buffer.insert_block(block1.clone());
        buffer.insert_block(block2.clone());
        buffer.insert_block(block3.clone());

        // pre-eviction block1 is the root
        assert_eq!(buffer.lowest_ancestor(&block3.hash()), Some(&block1));
        assert_eq!(buffer.lowest_ancestor(&block2.hash()), Some(&block1));
        assert_eq!(buffer.lowest_ancestor(&block1.hash()), Some(&block1));

        buffer.insert_block(block4.clone());

        assert_eq!(buffer.lowest_ancestor(&block4.hash()), Some(&block4));

        // block1 gets evicted
        assert_block_removal(&buffer, &block1);

        // check lowest ancestor results post eviction
        assert_eq!(buffer.lowest_ancestor(&block3.hash()), Some(&block2));
        assert_eq!(buffer.lowest_ancestor(&block2.hash()), Some(&block2));
        assert_eq!(buffer.lowest_ancestor(&block1.hash()), None);

        assert_buffer_lengths(&buffer, 3);
    }

    #[test]
    fn simple_eviction() {
        let mut rng = generators::rng();

        let main_parent = BlockNumHash::new(9, rng.random());
        let block1 = create_block(&mut rng, 10, main_parent.hash);
        let block2 = create_block(&mut rng, 11, block1.hash());
        let block3 = create_block(&mut rng, 12, block2.hash());
        let parent4 = rng.random();
        let block4 = create_block(&mut rng, 13, parent4);

        let mut buffer = BlockBuffer::new(3);

        buffer.insert_block(block1.clone());
        buffer.insert_block(block2);
        buffer.insert_block(block3);
        buffer.insert_block(block4);

        // block3 gets evicted
        assert_block_removal(&buffer, &block1);

        assert_buffer_lengths(&buffer, 3);
    }
}
