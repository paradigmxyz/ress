//! Types for new canonical chain

use reth_primitives::Header;

/// Non-empty chain of blocks.
#[derive(Debug, Clone)]
pub enum NewCanonicalChain {
    /// A simple append to the current canonical head
    Commit {
        /// all blocks that lead back to the canonical head
        new: Vec<Header>,
    },
    /// A reorged chain consists of two chains that trace back to a shared ancestor block at which
    /// point they diverge.
    Reorg {
        /// All blocks of the _new_ chain
        new: Vec<Header>,
        /// All blocks of the _old_ chain
        ///
        /// These are not [`ExecutedBlockWithTrieUpdates`] because we don't always have the trie
        /// updates for the old canonical chain. For example, in case of node being restarted right
        /// before the reorg [`TrieUpdates`] can't be fetched from database.
        old: Vec<Header>,
    },
}

impl NewCanonicalChain {
    /// Returns the length of the new chain.
    pub fn new_block_count(&self) -> usize {
        match self {
            Self::Commit { new } | Self::Reorg { new, .. } => new.len(),
        }
    }

    /// Returns the length of the reorged chain.
    pub fn reorged_block_count(&self) -> usize {
        match self {
            Self::Commit { .. } => 0,
            Self::Reorg { old, .. } => old.len(),
        }
    }

    /// Returns the new tip of the chain.
    ///
    /// Returns the new tip for [`Self::Reorg`] and [`Self::Commit`] variants which commit at least
    /// 1 new block.
    pub fn tip(&self) -> &Header {
        match self {
            Self::Commit { new } | Self::Reorg { new, .. } => new.last().expect("non empty blocks"),
        }
    }
}
