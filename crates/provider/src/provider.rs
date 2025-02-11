use crate::{backends::memory::MemoryStorage, database::RessDatabase, errors::StorageError};
use alloy_eips::BlockNumHash;
use alloy_primitives::{
    map::{B256HashMap, B256HashSet},
    BlockHash, BlockNumber, Bytes, B256,
};
use ress_protocol::RessProtocolProvider;
use reth_chainspec::ChainSpec;
use reth_db::DatabaseError;
use reth_primitives::{Bytecode, Header, SealedHeader};
use reth_storage_errors::provider::ProviderResult;
use std::{collections::HashMap, sync::Arc};

/// Provider for retrieving blockchain data.
///
/// This type is a main entrypoint for fetching chain and supplementary state data.
#[derive(Clone, Debug)]
pub struct RessProvider {
    chain_spec: Arc<ChainSpec>,
    database: RessDatabase,
    memory: MemoryStorage,
}

impl RessProvider {
    /// Instantiate new storage.
    pub fn new(
        chain_spec: Arc<ChainSpec>,
        database: RessDatabase,
        current_head: BlockNumHash,
    ) -> Self {
        let memory = MemoryStorage::new(current_head);
        Self { chain_spec, memory, database }
    }

    /// Get chain spec.
    pub fn chain_spec(&self) -> Arc<ChainSpec> {
        self.chain_spec.clone()
    }

    /// Update canonical state.
    pub fn on_chain_update(&self, new: Vec<SealedHeader>, old: Vec<SealedHeader>) {
        for header in old {
            self.memory.remove_canonical_hash(header.number, header.hash());
        }
        for header in new {
            self.memory.insert_canonical_hash(header.number, header.hash());
        }
    }

    /// Remove old canonical blocks on new finalized hash.
    pub fn on_new_finalized_hash(&self, finalized_hash: B256) -> Result<(), StorageError> {
        if !finalized_hash.is_zero() {
            let upper_bound = self.memory.get_block_number(finalized_hash)?;
            self.memory.remove_canonical_until(upper_bound, finalized_hash);
        }
        Ok(())
    }

    /// Return block header by hash.
    pub fn header(&self, hash: B256) -> Option<Header> {
        self.memory.header_by_hash(hash)
    }

    /// Return sealed block header by hash.
    pub fn sealed_header(&self, hash: B256) -> Option<SealedHeader> {
        Some(SealedHeader::new(self.memory.header_by_hash(hash)?, hash))
    }

    /// Insert header into the state.
    pub fn insert_header(&self, header: Header) {
        self.memory.insert_header(header);
    }

    /// Return whether or not the hash is part of the canonical chain.
    pub fn is_canonical(&self, hash: B256) -> bool {
        self.memory.is_canonical_lookup(hash)
    }

    /// Update current canonical head.
    pub fn set_canonical_head(&self, new_head: BlockNumHash) {
        self.memory.set_canonical_head(new_head);
    }

    /// Return current canonical head.
    pub fn get_canonical_head(&self) -> BlockNumHash {
        self.memory.get_canonical_head()
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

    /// Set safe canonical block hash that historical 256 blocks from canonical head
    ///
    /// It have canonical hash validation check
    pub fn set_canonical_hash(
        &self,
        block_hash: B256,
        block_number: BlockNumber,
    ) -> Result<(), StorageError> {
        self.memory.set_canonical_hash(block_hash, block_number)?;
        Ok(())
    }

    /// Overwrite block hashes mapping
    pub fn overwrite_block_hashes(&self, block_hashes: HashMap<BlockNumber, B256>) {
        self.memory.overwrite_block_hashes(block_hashes);
    }

    /// Overwrite block hashes by passing block headers
    pub fn overwrite_block_hashes_by_headers(&self, block_headers: Vec<Header>) {
        let mut block_hashes = HashMap::new();

        for header in block_headers {
            let block_number = header.number;
            let block_hash = header.hash_slow();
            block_hashes.insert(block_number, block_hash);
        }

        self.memory.overwrite_block_hashes(block_hashes);
    }

    /// Get block hash from memory of target block number
    pub fn get_block_hash(&self, block_number: BlockNumber) -> Result<BlockHash, StorageError> {
        self.memory.get_block_hash(block_number).map_err(StorageError::Memory)
    }
}

// TODO: implement
impl RessProtocolProvider for RessProvider {
    fn header(&self, block_hash: B256) -> ProviderResult<Option<Header>> {
        Ok(self.header(block_hash))
    }

    fn block_body(&self, _block_hash: B256) -> ProviderResult<Option<reth_primitives::BlockBody>> {
        Ok(None)
    }

    fn bytecode(&self, code_hash: B256) -> ProviderResult<Option<Bytes>> {
        Ok(self.get_bytecode(code_hash)?.map(|bytecode| bytecode.original_bytes()))
    }

    fn witness(&self, _block_hash: B256) -> ProviderResult<Option<B256HashMap<Bytes>>> {
        Ok(None)
    }
}
