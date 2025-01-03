use std::sync::{Arc, Mutex};

use alloy_primitives::{Address, BlockNumber, B256, U256};
use backends::{disk::DiskStorage, memory::MemoryStorage, network::NetworkStorage};
use errors::StorageError;
use ress_network::p2p::P2pHandler;
use reth::{
    chainspec::ChainSpec,
    primitives::{Header, SealedHeader},
    revm::primitives::{AccountInfo, Bytecode},
};
use reth_trie_sparse::SparseStateTrie;

pub mod backends;
pub mod errors;

/// orchestract 3 different type of backends (in memory, disk, network)
pub struct Storage {
    memory: Arc<MemoryStorage>,
    disk: Arc<DiskStorage>,
    network: Arc<NetworkStorage>,
}

impl Storage {
    pub fn new(p2p_handler: &Arc<P2pHandler>) -> Self {
        let memory = Arc::new(MemoryStorage::new());
        let disk = Arc::new(DiskStorage::new("my_db_path"));
        let network = Arc::new(NetworkStorage::new(p2p_handler));
        Self {
            memory,
            disk,
            network,
        }
    }

    pub fn get_account_info_by_hash(
        &self,
        block_hash: B256,
        address: Address,
    ) -> Result<Option<AccountInfo>, StorageError> {
        todo!()
    }

    /// get bytecode from libmbdx -> fall back network
    pub fn get_account_code(&self, code_hash: B256) -> Result<Option<Bytecode>, StorageError> {
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
