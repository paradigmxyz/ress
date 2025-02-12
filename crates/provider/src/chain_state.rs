use alloy_primitives::{map::B256HashSet, BlockHash, BlockNumber, B256};
use itertools::Itertools;
use parking_lot::RwLock;
use reth_primitives::{Block, BlockBody, Header, RecoveredBlock, SealedHeader};
use std::{
    collections::{btree_map, BTreeMap, HashMap},
    sync::Arc,
};

/// In-memory blockchain tree state.
/// Stores all validated blocks as well as keeps track of the ones
/// that form the canonical chain.
#[derive(Clone, Default, Debug)]
pub(crate) struct ChainState(Arc<RwLock<ChainStateInner>>);

#[derive(Default, Debug)]
struct ChainStateInner {
    /// Canonical block hashes stored by respective block number.
    canonical_hashes_by_number: BTreeMap<BlockNumber, B256>,
    /// __All__ validated blocks by block hash that are connected to the canonical chain.
    ///
    /// This includes blocks for all forks.
    blocks_by_hash: HashMap<B256, RecoveredBlock<Block>>,
    /// __All__ block hashes stored by their number.
    block_hashes_by_number: BTreeMap<BlockNumber, B256HashSet>,
}

impl ChainState {
    /// Returns `true` if block hash is canonical.
    pub(crate) fn is_hash_canonical(&self, hash: &BlockHash) -> bool {
        self.0.read().canonical_hashes_by_number.values().contains(hash)
    }

    /// Returns block hash for a given block number.
    /// If no canonical hash is found, returns the first available hash for that block number.
    // TODO: from B256HashSet is first element is best hash?
    pub(crate) fn block_hash(&self, number: &BlockNumber) -> Option<BlockHash> {
        let inner = self.0.read();
        inner.canonical_hashes_by_number.get(number).cloned().or_else(|| {
            inner
                .block_hashes_by_number
                .get(number)
                .and_then(|hashes| hashes.iter().next().cloned())
        })
    }

    /// Inserts canonical hash for block number.
    pub(crate) fn insert_canonical_hash(&self, number: BlockNumber, hash: BlockHash) {
        self.0.write().canonical_hashes_by_number.insert(number, hash);
    }

    /// Remove canonical hash for block number if it matches.
    pub(crate) fn remove_canonical_hash(&self, number: BlockNumber, hash: BlockHash) {
        let mut this = self.0.write();
        if let btree_map::Entry::Occupied(entry) = this.canonical_hashes_by_number.entry(number) {
            if entry.get() == &hash {
                entry.remove();
            }
        }
    }

    /// Returns header by hash.
    pub(crate) fn header(&self, hash: &BlockHash) -> Option<Header> {
        self.map_recovered_block(hash, RecoveredBlock::clone_header)
    }

    /// Returns sealed header by hash.
    pub(crate) fn sealed_header(&self, hash: &BlockHash) -> Option<SealedHeader> {
        self.map_recovered_block(hash, RecoveredBlock::clone_sealed_header)
    }

    /// Returns block body by hash.
    pub(crate) fn block_body(&self, hash: &BlockHash) -> Option<BlockBody> {
        self.map_recovered_block(hash, |b| b.body().clone())
    }

    /// Insert recovered block.
    pub(crate) fn insert_block(&self, block: RecoveredBlock<Block>) {
        let mut this = self.0.write();
        this.block_hashes_by_number.entry(block.number).or_default().insert(block.hash());
        this.blocks_by_hash.insert(block.hash(), block);
    }

    /// Remove all blocks before finalized as well as
    /// all canonical block hashes before `finalized.number - 256`.
    pub(crate) fn remove_blocks_on_finalized(&self, finalized_hash: &B256) {
        let mut this = self.0.write();
        if let Some(finalized) = this.blocks_by_hash.get(finalized_hash) {
            let finalized_number = finalized.number;

            // Remove blocks before finalized.
            while this
                .block_hashes_by_number
                .first_key_value()
                .is_some_and(|(number, _)| number <= &finalized_number)
            {
                let (_, block_hashes) = this.block_hashes_by_number.pop_first().unwrap();
                for block_hash in block_hashes {
                    this.blocks_by_hash.remove(&block_hash);
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
