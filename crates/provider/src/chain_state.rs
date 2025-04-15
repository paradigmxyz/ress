use alloy_eips::BlockNumHash;
use alloy_primitives::{
    map::{B256HashMap, B256HashSet},
    BlockHash, BlockNumber, Bytes, B256,
};
use itertools::Itertools;
use parking_lot::RwLock;
use reth_primitives::{Block, BlockBody, Header, RecoveredBlock, SealedBlock, SealedHeader};
use std::{
    collections::{btree_map, BTreeMap},
    sync::Arc,
};

/// In-memory blockchain tree state.
/// Stores all validated blocks as well as keeps track of the ones
/// that form the canonical chain.
#[derive(Clone, Default, Debug)]
pub struct ChainState(Arc<RwLock<ChainStateInner>>);

#[derive(Default, Debug)]
struct ChainStateInner {
    /// Canonical block hashes stored by respective block number.
    canonical_hashes_by_number: BTreeMap<BlockNumber, B256>,
    /// __All__ validated blocks by block hash that are connected to the canonical chain.
    ///
    /// This includes blocks for all forks.
    blocks_by_hash: B256HashMap<RecoveredBlock<Block>>,
    /// __All__ block hashes stored by their number.
    block_hashes_by_number: BTreeMap<BlockNumber, B256HashSet>,
    /// Valid block witnesses by block hash.
    witnesses: B256HashMap<Vec<Bytes>>,
}

impl ChainState {
    /// Returns `true` if block hash is canonical.
    pub fn is_hash_canonical(&self, hash: &BlockHash) -> bool {
        self.0.read().canonical_hashes_by_number.values().contains(hash)
    }

    /// Returns block hash for a given block number.
    /// If no canonical hash is found, traverses parent hashes from the given block hash
    /// to find an ancestor at the specified block number.
    pub fn block_hash(&self, parent: BlockNumHash, number: BlockNumber) -> Option<BlockHash> {
        let inner = self.0.read();

        // First traverse the ancestors and attempt to find the block number in executed blocks.
        let mut ancestor = parent;
        while let Some(block) = inner.blocks_by_hash.get(&ancestor.hash) {
            if block.number == number {
                return Some(block.hash())
            }
            ancestor = block.parent_num_hash();
        }

        // We exhausted all executed blocks, the target block must be canonical.
        if number <= ancestor.number &&
            inner.canonical_hashes_by_number.get(&ancestor.number) == Some(&ancestor.hash)
        {
            return inner.canonical_hashes_by_number.get(&number).cloned()
        }

        None
    }

    /// Inserts canonical hash for block number.
    pub fn insert_canonical_hash(&self, number: BlockNumber, hash: BlockHash) {
        self.0.write().canonical_hashes_by_number.insert(number, hash);
    }

    /// Remove canonical hash for block number if it matches.
    pub fn remove_canonical_hash(&self, number: BlockNumber, hash: BlockHash) {
        let mut this = self.0.write();
        if let btree_map::Entry::Occupied(entry) = this.canonical_hashes_by_number.entry(number) {
            if entry.get() == &hash {
                entry.remove();
            }
        }
    }

    /// Return block number by hash.
    pub fn block_number(&self, hash: &B256) -> Option<BlockNumber> {
        self.map_recovered_block(hash, |b| b.number)
    }

    /// Returns header by hash.
    pub fn header(&self, hash: &BlockHash) -> Option<Header> {
        self.map_recovered_block(hash, RecoveredBlock::clone_header)
    }

    /// Returns sealed header by hash.
    pub fn sealed_header(&self, hash: &BlockHash) -> Option<SealedHeader> {
        self.map_recovered_block(hash, RecoveredBlock::clone_sealed_header)
    }

    /// Returns block body by hash.
    pub fn block_body(&self, hash: &BlockHash) -> Option<BlockBody> {
        self.map_recovered_block(hash, |b| b.body().clone())
    }

    /// Returns sealed block by hash.
    pub fn sealed_block(&self, hash: &BlockHash) -> Option<SealedBlock> {
        self.map_recovered_block(hash, RecoveredBlock::clone_sealed_block)
    }

    /// Returns recovered block by hash.
    pub fn recovered_block(&self, hash: &BlockHash) -> Option<RecoveredBlock<Block>> {
        self.map_recovered_block(hash, Clone::clone)
    }

    /// Returns witness by block hash.
    pub fn witness(&self, hash: &BlockHash) -> Option<Vec<Bytes>> {
        self.0.read().witnesses.get(hash).cloned()
    }

    /// Insert recovered block.
    pub fn insert_block(&self, block: RecoveredBlock<Block>, maybe_witness: Option<Vec<Bytes>>) {
        let mut this = self.0.write();
        let block_hash = block.hash();
        this.block_hashes_by_number.entry(block.number).or_default().insert(block_hash);
        this.blocks_by_hash.insert(block_hash, block);
        if let Some(witness) = maybe_witness {
            this.witnesses.insert(block_hash, witness);
        }
    }

    /// Remove all blocks before finalized as well as
    /// all canonical block hashes before `finalized.number - 256`.
    pub fn remove_blocks_on_finalized(&self, finalized_hash: &B256) {
        let mut this = self.0.write();
        if let Some(finalized) = this.blocks_by_hash.get(finalized_hash) {
            let finalized_number = finalized.number;

            // Remove blocks before finalized.
            while this
                .block_hashes_by_number
                .first_key_value()
                .is_some_and(|(number, _)| number < &finalized_number)
            {
                let (_, block_hashes) = this.block_hashes_by_number.pop_first().unwrap();
                for block_hash in block_hashes {
                    this.blocks_by_hash.remove(&block_hash);
                    this.witnesses.remove(&block_hash);
                }
            }

            // Remove canonical hashes before `finalized.number - 256`.
            let last_block_hash_number = finalized_number.saturating_sub(256);
            while this
                .canonical_hashes_by_number
                .first_key_value()
                .is_some_and(|(number, _)| number < &last_block_hash_number)
            {
                this.canonical_hashes_by_number.pop_first();
            }
        }
    }

    /// Returns recovered block by hash mapped to desired type.
    fn map_recovered_block<F, R>(&self, hash: &BlockHash, to_type: F) -> Option<R>
    where
        F: FnOnce(&RecoveredBlock<Block>) -> R,
    {
        self.0.read().blocks_by_hash.get(hash).map(to_type)
    }
}
