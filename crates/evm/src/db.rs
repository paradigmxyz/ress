//! EVM database implementation.

use alloy_primitives::{keccak256, Address, B256, U256};
use alloy_rlp::Decodable;
use alloy_trie::TrieAccount;
use ress_provider::RessProvider;
use reth_provider::ProviderError;
use reth_revm::{
    primitives::{AccountInfo, Bytecode},
    Database,
};
use reth_trie_sparse::SparseStateTrie;
use tracing::trace;

/// EVM database implementation that uses state witness for account and storage data retrieval.
/// Block hashes and bytecodes are retrieved from ress node storage.
#[derive(Debug)]
pub struct WitnessDatabase<'a> {
    provider: RessProvider,
    trie: &'a SparseStateTrie,
}

impl<'a> WitnessDatabase<'a> {
    /// Create new witness database.
    pub fn new(provider: RessProvider, trie: &'a SparseStateTrie) -> Self {
        Self { trie, provider }
    }
}

impl Database for WitnessDatabase<'_> {
    /// The witness state provider error type.
    type Error = ProviderError;

    /// Get basic account information.
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let hashed_address = keccak256(address);
        trace!(target: "ress::vm", %address, %hashed_address, "retrieving account");
        let Some(bytes) = self.trie.get_account_value(&hashed_address) else {
            trace!(target: "ress::vm", %address, %hashed_address, "no account found");
            return Ok(None)
        };
        let account = TrieAccount::decode(&mut bytes.as_slice())?;
        let account_info = AccountInfo {
            balance: account.balance,
            nonce: account.nonce,
            code_hash: account.code_hash,
            code: None,
        };
        trace!(target: "ress::vm", %address, %hashed_address, ?account_info, "account retrieved");
        Ok(Some(account_info))
    }

    /// Get storage value of address at slot.
    fn storage(&mut self, address: Address, slot: U256) -> Result<U256, Self::Error> {
        let slot = B256::from(slot);
        let hashed_address = keccak256(address);
        let hashed_slot = keccak256(slot);
        trace!(target: "ress::vm", %address, %hashed_address, %slot, %hashed_slot, "retrieving storage slot");
        let value = match self.trie.get_storage_slot_value(&hashed_address, &hashed_slot) {
            Some(value) => U256::decode(&mut value.as_slice())?,
            None => U256::ZERO,
        };
        trace!(target: "ress::vm", %address, %hashed_address, %slot, %hashed_slot, %value, "storage slot retrieved");
        Ok(value)
    }

    /// Get account code by its hash.
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        trace!(target: "ress::vm", %code_hash, "retrieving bytecode");
        let bytecode = self.provider.get_bytecode(code_hash)?.ok_or_else(|| {
            ProviderError::TrieWitnessError(format!("bytecode for {code_hash} not found"))
        })?;
        Ok(bytecode.0)
    }

    /// Get block hash by block number.
    fn block_hash(&mut self, block_number: u64) -> Result<B256, Self::Error> {
        trace!(target: "ress::vm", block_number, "retrieving block hash");
        self.provider
            .get_block_hash(block_number)
            .map_err(|_| ProviderError::StateForNumberNotFound(block_number))
    }
}
