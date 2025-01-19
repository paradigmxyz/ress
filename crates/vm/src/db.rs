use alloy_primitives::{keccak256, Address, B256, U256};
use alloy_rlp::Decodable;
use alloy_trie::TrieAccount;
use ress_storage::Storage;
use reth_provider::ProviderError;
use reth_revm::{
    primitives::{AccountInfo, Bytecode},
    Database,
};
use reth_trie_sparse::SparseStateTrie;
use std::sync::Arc;
use tracing::debug;

pub struct WitnessDatabase {
    trie: SparseStateTrie,
    storage: Arc<Storage>,
}

impl WitnessDatabase {
    pub fn new(trie: SparseStateTrie, storage: Arc<Storage>) -> Self {
        Self { trie, storage }
    }
}

impl Database for WitnessDatabase {
    #[doc = " The witness state provider error type."]
    type Error = ProviderError;

    #[doc = " Get basic account information."]
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        debug!("request for basic for address: {}", address);
        match self.trie.get_account_value(&keccak256(address)) {
            Some(bytes) => {
                let account = TrieAccount::decode(&mut bytes.as_slice()).unwrap();
                let account_info = AccountInfo {
                    balance: account.balance,
                    nonce: account.nonce,
                    code_hash: account.code_hash,
                    code: None,
                };
                debug!("account info: {:?}", account_info);
                Ok(Some(account_info))
            }
            None => Ok(None),
        }
    }

    #[doc = " Get storage value of address at index."]
    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        debug!("request for storage: {}, index: {}", address, index);
        let storage_value = match self
            .trie
            .get_storage_slot_value(&keccak256(address), &keccak256(B256::from(index)))
        {
            Some(value) => U256::decode(&mut value.as_slice()).unwrap(),
            None => U256::ZERO,
        };
        debug!("storage value {:?}", storage_value);
        Ok(storage_value)
    }

    #[doc = " Get account code by its hash."]
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        debug!("request for code_hash: {}", code_hash);
        self.storage
            .get_contract_bytecode(code_hash)
            .map_err(|e| ProviderError::TrieWitnessError(e.to_string()))
    }

    #[doc = " Get block hash by block number."]
    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        debug!("request for blockhash: {}", number);
        self.storage
            .get_block_hash(number)
            .map_err(|_| ProviderError::StateForNumberNotFound(number))
    }
}
