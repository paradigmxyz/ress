use std::{collections::HashMap, sync::RwLock};

use alloy_primitives::{Address, BlockHash, BlockNumber, B256, U256};
use reth::{
    chainspec::ChainSpec,
    primitives::{Header, SealedHeader},
    revm::primitives::AccountInfo,
};
use reth_trie_sparse::SparseStateTrie;

use crate::errors::StorageError;

pub struct MemoryStorage {
    headers: RwLock<HashMap<BlockHash, Header>>,
    canonical_hashes: RwLock<HashMap<BlockNumber, BlockHash>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            headers: RwLock::new(HashMap::new()),
            canonical_hashes: RwLock::new(HashMap::new()),
        }
    }

    pub fn canonical_hashes(&self, block_hash: B256, block_number: BlockNumber) {
        self.canonical_hashes
            .write()
            .unwrap()
            .insert(block_number, block_hash);
    }

    pub fn store_header(&self, block_hash: B256, header: Header) {
        self.headers.write().unwrap().insert(block_hash, header);
    }
}

impl MemoryStorage {
    pub fn get_account_info_by_hash(
        &self,
        block_hash: B256,
        address: Address,
    ) -> Result<Option<AccountInfo>, StorageError> {
        todo!()
    }

    // get storge value from a
    pub fn get_storage_at_hash(
        &self,
        block_hash: B256,
        address: Address,
        storage_key: B256,
    ) -> Result<Option<U256>, StorageError> {
        // let Some(storage_trie) = self.storage_trie(block_hash, address)? else {
        //     return Ok(None);
        // };
        // let hashed_key = Keccak256::new(&storage_key).finalize().to_vec();
        // storage_trie
        //     .get(&hashed_key)?
        //     .map(|rlp| U256::decode(&rlp).map_err(StorageError::RLPDecode))
        //     .transpose();
        todo!()
    }

    pub fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Header>, StorageError> {
        todo!()
    }

    pub fn get_chain_config(&self) -> Result<ChainSpec, StorageError> {
        todo!()
    }

    pub fn storage_trie(
        &self,
        block_hash: B256,
        address: Address,
    ) -> Result<Option<SparseStateTrie>, StorageError> {
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

    pub fn state_trie(&self, block_hash: B256) -> Result<Option<SparseStateTrie>, StorageError> {
        // let Some(header) = self.get_block_header_by_hash(block_hash)? else {
        //     return Ok(None);
        // };
        // Ok(Some(self.engine.open_state_trie(header.state_root)))
        todo!()
    }

    pub fn get_block_header_by_hash(
        &self,
        block_hash: B256,
    ) -> Result<Option<SealedHeader>, StorageError> {
        // todo: get header from memeory
        // self.engine.get_block_header_by_hash(block_hash)
        return Ok(Some(SealedHeader::default()));
    }
}
