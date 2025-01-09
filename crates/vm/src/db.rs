use std::sync::Arc;

use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use ress_storage::Storage;
use reth_revm::{
    primitives::{AccountInfo, Bytecode},
    Database,
};
use reth_trie_sparse::SparseStateTrie;

use crate::errors::WitnessStateProviderError;

pub struct WitnessDatabase {
    trie: Arc<SparseStateTrie>,
    storage: Arc<Storage>,
}

impl WitnessDatabase {
    pub fn new(trie: Arc<SparseStateTrie>, storage: Arc<Storage>) -> Self {
        Self { trie, storage }
    }
}

impl Database for WitnessDatabase {
    #[doc = " The witness state provider error type."]
    type Error = WitnessStateProviderError;

    #[doc = " Get basic account information."]
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        match self.trie.get_account_value(&keccak256(address)) {
            Some(bytes) => {
                let owned_bytes = Bytes::copy_from_slice(bytes);
                let code = Bytecode::LegacyRaw(owned_bytes);
                Ok(Some(AccountInfo::from_bytecode(code)))
            }
            None => Ok(None),
        }
    }

    #[doc = " Get storage value of address at index."]
    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Ok(U256::from_be_slice(
            self.trie
                .get_storage_slot_value(&keccak256(address), &keccak256(B256::from(index)))
                .unwrap(),
        ))
    }

    #[doc = " Get account code by its hash."]
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        Ok(self.storage.code_by_hash(code_hash).unwrap().unwrap())
    }

    #[doc = " Get block hash by block number."]
    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        Ok(self.storage.get_block_hash(number).unwrap().unwrap())
    }
}
