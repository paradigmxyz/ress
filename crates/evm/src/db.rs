//! EVM database implementation.

use std::collections::BTreeMap;

use alloy_primitives::{keccak256, map::B256Map, Address, Bytes, B256, U256};
use alloy_rlp::Decodable;
use alloy_trie::TrieAccount;
use reth_provider::ProviderError;
use reth_revm::{bytecode::Bytecode, state::AccountInfo, Database};
use reth_trie_sparse::SparseStateTrie;
use tracing::trace;

/// EVM database implementation that uses a [`SparseStateTrie`] for account and storage data
/// retrieval. Block hashes and bytecodes are retrieved from the [`RessProvider`].
#[derive(Debug)]
pub struct WitnessDatabase<'a> {
    trie: &'a SparseStateTrie,
    bytecode_mapping: B256Map<Bytecode>,
    blockhashes: BTreeMap<u64, B256>,
}

impl<'a> WitnessDatabase<'a> {
    /// Create new witness database.
    pub fn new(
        trie: &'a SparseStateTrie,
        codes: Vec<Bytes>,
        blockhashes: BTreeMap<u64, B256>,
    ) -> Self {
        let mut bytecode_mapping = B256Map::default();
        for code in codes {
            let code_hash = keccak256(&code);
            bytecode_mapping.insert(code_hash, Bytecode::new_raw(code));
        }
        Self { trie, bytecode_mapping, blockhashes }
    }
}

impl Database for WitnessDatabase<'_> {
    /// The database error type.
    type Error = ProviderError;

    /// Get basic account information.
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let hashed_address = keccak256(address);
        trace!(target: "ress::evm", %address, %hashed_address, "retrieving account");
        let Some(bytes) = self.trie.get_account_value(&hashed_address) else {
            trace!(target: "ress::evm", %address, %hashed_address, "no account found");
            return Ok(None);
        };
        let account = TrieAccount::decode(&mut bytes.as_slice())?;
        let account_info = AccountInfo {
            balance: account.balance,
            nonce: account.nonce,
            code_hash: account.code_hash,
            code: None,
        };
        trace!(target: "ress::evm", %address, %hashed_address, ?account_info, "account retrieved");
        Ok(Some(account_info))
    }

    /// Get storage value of address at slot.
    fn storage(&mut self, address: Address, slot: U256) -> Result<U256, Self::Error> {
        let slot = B256::from(slot);
        let hashed_address = keccak256(address);
        let hashed_slot = keccak256(slot);
        trace!(target: "ress::evm", %address, %hashed_address, %slot, %hashed_slot, "retrieving storage slot");
        let value = match self.trie.get_storage_slot_value(&hashed_address, &hashed_slot) {
            Some(value) => U256::decode(&mut value.as_slice())?,
            None => U256::ZERO,
        };
        trace!(target: "ress::evm", %address, %hashed_address, %slot, %hashed_slot, %value, "storage slot retrieved");
        Ok(value)
    }

    /// Get account code by its hash.
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        trace!(target: "ress::evm", %code_hash, "retrieving bytecode");
        match self.bytecode_mapping.get(&code_hash) {
            Some(bytecode) => Ok(bytecode.clone()),
            None => {
                Err(ProviderError::TrieWitnessError(format!("bytecode for {code_hash} not found")))
            }
        }
    }

    /// Get block hash by block number.
    fn block_hash(&mut self, block_number: u64) -> Result<B256, Self::Error> {
        trace!(target: "ress::evm", block_number, "retrieving block hash");
        match self.blockhashes.get(&block_number) {
            Some(hash) => Ok(hash.clone()),
            None => Err(ProviderError::StateForNumberNotFound(block_number)),
        }
    }
}
