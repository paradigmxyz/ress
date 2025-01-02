use std::sync::Arc;

use alloy_primitives::{Address, BlockNumber, B256, U256};
use engine::StoreEngine;
use errors::StoreError;
use ress_network::p2p::P2pHandler;
use reth::{
    chainspec::ChainSpec,
    primitives::{Header, SealedHeader},
    revm::primitives::{AccountInfo, Bytecode},
};
use reth_trie_sparse::SparseStateTrie;

pub mod engine;
pub mod errors;

pub struct Store {
    engine: Arc<StoreEngine>,
}

impl Store {
    pub fn new(p2p_handler: &Arc<P2pHandler>) -> Self {
        Self {
            engine: StoreEngine::new(p2p_handler).into(),
        }
    }

    pub fn get_account_info_by_hash(
        &self,
        block_hash: B256,
        address: Address,
    ) -> Result<Option<AccountInfo>, StoreError> {
        todo!()
    }

    /// get bytecode from libmbdx -> fall back network
    pub fn get_account_code(&self, code_hash: B256) -> Result<Option<Bytecode>, StoreError> {
        self.engine.get_account_code(code_hash)
    }

    // get storge value from a
    pub fn get_storage_at_hash(
        &self,
        block_hash: B256,
        address: Address,
        storage_key: B256,
    ) -> Result<Option<U256>, StoreError> {
        // let Some(storage_trie) = self.storage_trie(block_hash, address)? else {
        //     return Ok(None);
        // };
        // let hashed_key = Keccak256::new(&storage_key).finalize().to_vec();
        // storage_trie
        //     .get(&hashed_key)?
        //     .map(|rlp| U256::decode(&rlp).map_err(StoreError::RLPDecode))
        //     .transpose();
        todo!()
    }

    pub fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Header>, StoreError> {
        todo!()
    }

    pub fn get_chain_config(&self) -> Result<ChainSpec, StoreError> {
        todo!()
    }

    pub fn storage_trie(
        &self,
        block_hash: B256,
        address: Address,
    ) -> Result<Option<SparseStateTrie>, StoreError> {
        // Fetch Account from state_trie
        // let Some(state_trie) = self.state_trie(block_hash)? else {
        //     return Ok(None);
        // };
        // let hashed_address = hash_address(&address);
        // let Some(encoded_account) = state_trie.get(&hashed_address)? else {
        //     return Ok(None);
        // };
        // let account = AccountState::decode(&encoded_account)?;
        // // Open storage_trie
        // let storage_root = account.storage_root;
        // Ok(Some(self.engine.open_storage_trie(
        //     H256::from_slice(&hashed_address),
        //     storage_root,
        // )))
        todo!()
    }

    pub fn state_trie(&self, block_hash: B256) -> Result<Option<SparseStateTrie>, StoreError> {
        // let Some(header) = self.get_block_header_by_hash(block_hash)? else {
        //     return Ok(None);
        // };
        // Ok(Some(self.engine.open_state_trie(header.state_root)))
        todo!()
    }

    pub fn get_block_header_by_hash(
        &self,
        block_hash: B256,
    ) -> Result<Option<SealedHeader>, StoreError> {
        // todo: get header from memeory
        // self.engine.get_block_header_by_hash(block_hash)
        return Ok(Some(SealedHeader::default()));
    }
}
