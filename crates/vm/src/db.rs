use std::sync::Arc;

use alloy_primitives::{map::HashMap, Address, B256, U256};
use ress_storage::{errors::StorageError, Storage};
use reth_revm::{
    db::AccountState,
    primitives::{Account, AccountInfo, Bytecode, KECCAK_EMPTY},
    Database, DatabaseCommit,
};

use crate::errors::WitnessStateProviderError;

pub struct WitnessState {
    pub storage: Arc<Storage>,
    pub block_hash: B256,
}

impl WitnessState {
    /// Inserts the account's code into the cache.
    ///
    /// Accounts objects and code are stored separately in the cache, this will take the code from the account and instead map it to the code hash.
    ///
    /// Note: This will not insert into the underlying external database.
    pub fn insert_contract(&mut self, account: &mut AccountInfo) {
        if let Some(code) = &account.code {
            if !code.is_empty() {
                if account.code_hash == KECCAK_EMPTY {
                    account.code_hash = code.hash_slow();
                }
                let disk = self.storage.disk.lock().unwrap();
                disk.update_account_code(account.code_hash, code.clone())
                    .unwrap();
            }
        }
        if account.code_hash.is_zero() {
            account.code_hash = KECCAK_EMPTY;
        }
    }
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
            .ok_or(StorageError::NoCodeForCodeHash)?)
    }

    #[doc = " Get storage value of address at index."]
    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        Ok(self
            .storage
            .get_storage_at_hash(self.block_hash, address, index.into())?
            .unwrap_or(U256::ZERO))
    }

    #[doc = " Get block hash by block number."]
    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        Ok(self
            .storage
            .get_block_header(number)?
            .map(|header| header.hash_slow())
            .ok_or(StorageError::BlockNotFound)?)
    }
}

// from https://github.com/bluealloy/revm/blob/04688b769bd7ffe66eccf5a255e13bb8502fa451/crates/database/src/in_memory_db.rs#L164
impl DatabaseCommit for WitnessState {
    #[doc = " Commit changes to the database."]
    fn commit(&mut self, changes: HashMap<Address, Account>) {
        for (address, mut account) in changes {
            if !account.is_touched() {
                continue;
            }
            if account.is_selfdestructed() {
                let mut accounts = self.storage.memory.accounts.lock().unwrap();
                let db_account = accounts.entry(address).or_default();
                db_account.storage.clear();
                db_account.account_state = AccountState::NotExisting;
                db_account.info = AccountInfo::default();
                continue;
            }
            let is_newly_created = account.is_created();
            self.insert_contract(&mut account.info);

            let mut accounts = self.storage.memory.accounts.lock().unwrap();
            let db_account = accounts.entry(address).or_default();
            db_account.info = account.info;

            db_account.account_state = if is_newly_created {
                db_account.storage.clear();
                AccountState::StorageCleared
            } else if db_account.account_state.is_storage_cleared() {
                // Preserve old account state if it already exists
                AccountState::StorageCleared
            } else {
                AccountState::Touched
            };
            db_account.storage.extend(
                account
                    .storage
                    .into_iter()
                    .map(|(key, value)| (key, value.present_value())),
            );
        }
    }
}
