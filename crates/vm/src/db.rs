use std::sync::Arc;

use alloy_primitives::{keccak256, map::HashMap, Address, B256, U256};
use ress_storage::{errors::StorageError, Storage};
use reth_revm::{
    primitives::{Account, AccountInfo, Bytecode, KECCAK_EMPTY},
    Database, DatabaseCommit,
};
use reth_trie_sparse::SparseStateTrie;

use crate::errors::WitnessStateProviderError;

pub struct WitnessDatabase {
    pub sparse_trie: SparseStateTrie,
    pub block_hash: B256,
}

impl Database for WitnessDatabase {
    #[doc = " The witness state provider error type."]
    type Error = WitnessStateProviderError;

    #[doc = " Get basic account information."]
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        self.sparse_trie.get_account_value(&keccak256(address))
    }

    #[doc = " Get storage value of address at index."]
    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        self.sparse_trie
            .get_storage_slot_value(&keccak256(address), &keccak256(B256::from(index)))
    }

    #[doc = " Get account code by its hash."]
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.bytecode_provider.get(&code_hash) // this is the only place where we might go to the network or disk
    }

    #[doc = " Get block hash by block number."]
    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.block_hashes.get(&number)
    }
}
