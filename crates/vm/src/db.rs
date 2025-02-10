//! EVM database implementation.

use alloy_primitives::{keccak256, Address, B256, U256};
use alloy_rlp::Decodable;
use alloy_trie::TrieAccount;
use ress_provider::storage::Storage;
use reth_provider::ProviderError;
use reth_revm::{
    primitives::{AccountInfo, Bytecode},
    Database,
};
use reth_trie_sparse::SparseStateTrie;
use tracing::debug;

/// EVM database implementation that uses state witness for account and storage data retrieval.
/// Block hashes and bytecodes are retrieved from ress node storage.
#[derive(Debug)]
pub struct WitnessDatabase<'a> {
    storage: Storage,
    trie: &'a SparseStateTrie,
}

impl<'a> WitnessDatabase<'a> {
    /// Create new witness database.
    pub fn new(storage: Storage, trie: &'a SparseStateTrie) -> Self {
        Self { trie, storage }
    }
}

impl Database for WitnessDatabase<'_> {
    /// The witness state provider error type.
    type Error = ProviderError;

    /// Get basic account information.
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

    /// Get storage value of address at index.
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

    /// Get account code by its hash.
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        debug!("request for code_hash: {}", code_hash);
        self.storage
            .get_bytecode(code_hash)
            .map_err(|e| ProviderError::TrieWitnessError(e.to_string()))
    }

    /// Get block hash by block number.
    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        debug!("request for blockhash: {}", number);
        self.storage
            .get_block_hash(number)
            .map_err(|_| ProviderError::StateForNumberNotFound(number))
    }
}
