use std::sync::Arc;

use alloy_primitives::{
    keccak256, map::HashMap, Address, BlockHash, BlockNumber, Bytes, B256, U256,
};
use eyre::Ok;
use ress_storage::{errors::StorageError, Storage};
use reth_revm::{
    primitives::{Account, AccountInfo, Bytecode, KECCAK_EMPTY},
    Database, DatabaseCommit,
};
use reth_trie_sparse::SparseStateTrie;

use crate::errors::WitnessStateProviderError;

pub struct WitnessDatabase {
    pub sparse_trie: SparseStateTrie,
    canonical_hashes: HashMap<BlockNumber, BlockHash>,
}

impl Database for WitnessDatabase {
    #[doc = " The witness state provider error type."]
    type Error = ();

    #[doc = " Get basic account information."]
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match self.sparse_trie.get_account_value(&keccak256(address)) {
            Some(bytes) => Ok(Some(AccountInfo::from_bytecode(Bytecode::LegacyRaw(
                Bytes::from_static(&bytes),
            )))),
            None => Ok(None),
        }
    }

    #[doc = " Get storage value of address at index."]
    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Ok(U256::from_be_slice(
            self.sparse_trie
                .get_storage_slot_value(&keccak256(address), &keccak256(B256::from(index)))
                .unwrap(),
        ))
    }

    #[doc = " Get account code by its hash."]
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.bytecode_provider.get(&code_hash) // this is the only place where we might go to the network or disk
    }

    #[doc = " Get block hash by block number."]
    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        Ok(self.canonical_hashes.get(&number).unwrap().to_owned())
    }
}
