use crate::chain_state::ChainState;
use alloy_eips::BlockNumHash;
use alloy_primitives::{BlockHash, BlockNumber, Bytes, B256};
use reth_chainspec::ChainSpec;
use reth_primitives::{Block, BlockBody, Header, RecoveredBlock, SealedHeader};
use reth_ress_protocol::{RLPExecutionWitness, RessProtocolProvider};
use reth_storage_errors::provider::ProviderResult;
use std::{future::Future, sync::Arc};

/// Provider for retrieving blockchain data.
///
/// This type is a main entrypoint for fetching chain and supplementary state data.
#[derive(Clone, Debug)]
pub struct RessProvider {
    chain_spec: Arc<ChainSpec>,
    chain_state: ChainState,
}

impl RessProvider {
    /// Instantiate new storage.
    pub fn new(chain_spec: Arc<ChainSpec>) -> Self {
        Self { chain_spec, chain_state: ChainState::default() }
    }

    /// Get chain spec.
    pub fn chain_spec(&self) -> Arc<ChainSpec> {
        self.chain_spec.clone()
    }

    /// Returns `true` if block hash is canonical.
    pub fn is_hash_canonical(&self, hash: &BlockHash) -> bool {
        self.chain_state.is_hash_canonical(hash)
    }

    /// Finds block hash in memory for the target block number.
    /// Includes both canonical and pending blocks.
    pub fn block_hash(&self, parent: BlockNumHash, number: BlockNumber) -> Option<BlockHash> {
        self.chain_state.block_hash(parent, number)
    }

    /// Return block number by hash.
    pub fn block_number(&self, hash: &B256) -> Option<BlockNumber> {
        self.chain_state.block_number(hash)
    }

    /// Return sealed block header by hash.
    pub fn sealed_header(&self, hash: &B256) -> Option<SealedHeader> {
        self.chain_state.sealed_header(hash)
    }

    /// Insert recovered block.
    pub fn insert_block(&self, block: RecoveredBlock<Block>) {
        self.chain_state.insert_block(block);
    }

    /// Inserts canonical hash for block number.
    pub fn insert_canonical_hash(&self, number: BlockNumber, hash: BlockHash) {
        self.chain_state.insert_canonical_hash(number, hash);
    }

    /// Update canonical hashes in chain state.
    pub fn on_chain_update(&self, new: Vec<SealedHeader>, old: Vec<SealedHeader>) {
        for header in old {
            self.chain_state.remove_canonical_hash(header.number, header.hash());
        }
        for header in new {
            self.chain_state.insert_canonical_hash(header.number, header.hash());
        }
    }

    /// Remove blocks from chain state on finalized.
    pub fn on_finalized(&self, finalized_hash: &B256) {
        if !finalized_hash.is_zero() {
            self.chain_state.remove_blocks_on_finalized(finalized_hash);
        }
    }
}

impl RessProtocolProvider for RessProvider {
    fn header(&self, block_hash: B256) -> ProviderResult<Option<Header>> {
        Ok(self.chain_state.header(&block_hash))
    }

    fn block_body(&self, block_hash: B256) -> ProviderResult<Option<BlockBody>> {
        Ok(self.chain_state.block_body(&block_hash))
    }

    async fn proof(&self, block_hash: B256) -> ProviderResult<Bytes> {
        todo!()
    }

    async fn witness(&self, block_hash: B256) -> ProviderResult<RLPExecutionWitness> {
        todo!()
    }
}
