use crate::{chain_state::ChainState, database::RessDatabase};
use alloy_eips::BlockNumHash;
use alloy_primitives::{map::B256HashSet, BlockHash, BlockNumber, Bytes, B256};
use reth_chainspec::ChainSpec;
use reth_db::DatabaseError;
use reth_primitives::{Block, BlockBody, Bytecode, Header, RecoveredBlock, SealedHeader};
use reth_ress_protocol::RessProtocolProvider;
use reth_storage_errors::provider::ProviderResult;
use std::sync::Arc;

/// Provider for retrieving blockchain data.
///
/// This type is a main entrypoint for fetching chain and supplementary state data.
#[derive(Clone, Debug)]
pub struct RessProvider {
    chain_spec: Arc<ChainSpec>,
    database: RessDatabase,
    chain_state: ChainState,
}

impl RessProvider {
    /// Instantiate new storage.
    pub fn new(chain_spec: Arc<ChainSpec>, database: RessDatabase) -> Self {
        Self { chain_spec, database, chain_state: ChainState::default() }
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

    /// Returns `true` if bytecode exists in the database.
    pub fn bytecode_exists(&self, code_hash: B256) -> Result<bool, DatabaseError> {
        self.database.bytecode_exists(code_hash)
    }

    /// Get contract bytecode from given code hash from the disk
    pub fn get_bytecode(&self, code_hash: B256) -> Result<Option<Bytecode>, DatabaseError> {
        self.database.get_bytecode(code_hash)
    }

    /// Insert bytecode into the database.
    pub fn insert_bytecode(
        &self,
        code_hash: B256,
        bytecode: Bytecode,
    ) -> Result<(), DatabaseError> {
        self.database.insert_bytecode(code_hash, bytecode)
    }

    /// Filter the collection of code hashes for the ones that are missing from the database.
    pub fn missing_code_hashes(
        &self,
        code_hashes: B256HashSet,
    ) -> Result<B256HashSet, DatabaseError> {
        self.database.missing_code_hashes(code_hashes)
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

    fn bytecode(&self, code_hash: B256) -> ProviderResult<Option<Bytes>> {
        Ok(self.database.get_bytecode(code_hash)?.map(|b| b.original_bytes()))
    }

    // TODO: implement
    async fn witness(&self, _block_hash: B256) -> ProviderResult<Vec<Bytes>> {
        Ok(Vec::new())
    }
}
