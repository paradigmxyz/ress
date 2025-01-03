use alloy_primitives::{map::HashMap, Address, B256, U256};
use ress_storage::{errors::StorageError, Storage};
use reth::revm::{
    primitives::{Account, AccountInfo, Bytecode},
    Database, DatabaseCommit,
};

use crate::errors::WitnessStateProviderError;

pub struct WitnessState {
    pub storage: Storage,
    pub block_hash: B256,
}

impl Database for WitnessState {
    #[doc = " The witness state provider error type."]
    type Error = WitnessStateProviderError;

    #[doc = " Get basic account information."]
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let acc_info = self
            .storage
            .get_account_info_by_hash(self.block_hash, address)
            .unwrap()
            .unwrap_or_default();

        let code = self.storage.get_account_code(acc_info.code_hash).unwrap();

        Ok(Some(AccountInfo {
            balance: acc_info.balance,
            nonce: acc_info.nonce,
            code_hash: acc_info.code_hash,
            code,
        }))
    }

    #[doc = " Get account code by its hash."]
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        Ok(self
            .storage
            .get_account_code(code_hash)?
            .ok_or_else(|| StorageError::NoCodeForCodeHash)?)
    }

    #[doc = " Get storage value of address at index."]
    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Ok(self
            .storage
            .get_storage_at_hash(self.block_hash, address, index.into())?
            .unwrap_or_else(|| U256::ZERO))
    }

    #[doc = " Get block hash by block number."]
    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        Ok(self
            .storage
            .get_block_header(number)?
            .map(|header| header.hash_slow())
            .ok_or_else(|| StorageError::BlockNotFound)?)
    }
}

impl DatabaseCommit for WitnessState {
    #[doc = " Commit changes to the database."]
    fn commit(&mut self, _changes: HashMap<Address, Account>) {
        todo!()
    }
}
