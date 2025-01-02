use std::collections::HashMap;

use alloy_primitives::{Address, Bytes, B256, U256};
use ress_storage::Store;
use reth::{
    chainspec::ChainSpec,
    primitives::{Block, Receipt},
    revm::{
        db::{states::bundle_state::BundleRetention, AccountStatus, State, StateBuilder},
        primitives::{AccountInfo, BlockEnv, ExecutionResult, SpecId, TxEnv},
        Evm,
    },
};

use crate::{db::WitnessState, errors::EvmError};

pub enum EvmState {
    Store(State<WitnessState>),
}

impl EvmState {
    pub fn new(store: Store, block_hash: B256) -> Self {
        EvmState::Store(
            StateBuilder::new_with_database(WitnessState { store, block_hash })
                .with_bundle_update()
                .without_state_clear()
                .build(),
        )
    }

    #[allow(irrefutable_let_patterns)]
    pub fn database(&self) -> Option<&Store> {
        if let EvmState::Store(db) = self {
            Some(&db.database.store)
        } else {
            None
        }
    }

    /// Gets the stored chain config
    pub fn chain_config(&self) -> Result<ChainSpec, EvmError> {
        match self {
            EvmState::Store(db) => db.database.store.get_chain_config().map_err(EvmError::from),
        }
    }
}

/// Runs EVM, doesn't perform state transitions, but stores them
fn run_evm(
    tx_env: TxEnv,
    block_env: BlockEnv,
    state: &mut EvmState,
    spec_id: SpecId,
) -> Result<ExecutionResult, EvmError> {
    let tx_result = {
        let chain_spec = state.chain_config()?;
        #[allow(unused_mut)]
        let mut evm_builder = Evm::builder()
            .with_block_env(block_env)
            .with_tx_env(tx_env)
            .modify_cfg_env(|cfg| cfg.chain_id = chain_spec.chain.id())
            .with_spec_id(spec_id);

        match state {
            EvmState::Store(db) => {
                let mut evm = evm_builder.with_db(db).build();
                evm.transact_commit().unwrap()
            }
        }
    };
    Ok(tx_result)
}

/// Executes all transactions in a block and returns their receipts.
pub fn execute_block(
    block: &Block,
    state: &mut EvmState,
) -> Result<(Vec<Receipt>, Vec<AccountUpdate>), EvmError> {
    // let witness_state = Arc::new(WitnessState {
    //     store: state.database().unwrap(),
    //     block_hash: block.header.parent_hash,
    // });

    let mut receipts = Vec::new();
    let mut cumulative_gas_used = 0;
    let mut account_updates = get_state_transitions(state);

    for transaction in block.body.transactions.iter() {
        let block_header = &block.header;
        // todo: turn block header into block env
        let block_env = BlockEnv::default();
        // todo: turn tx into tx env
        let tx_env = TxEnv::default();
        // todo: get actual spec id
        let spec_id = SpecId::ARROW_GLACIER;
        let result = run_evm(tx_env, block_env, state, spec_id)?;

        cumulative_gas_used += result.gas_used();
        let receipt = Receipt {
            tx_type: transaction.tx_type(),
            success: result.is_success(),
            cumulative_gas_used: cumulative_gas_used,
            logs: result.logs().to_vec(),
        };
        receipts.push(receipt);
    }

    if let Some(withdrawals) = &block.body.withdrawals {
        // process_withdrawals(state, withdrawals)?;
        //todo: process withdrawl
    }

    Ok((receipts, account_updates))
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct AccountUpdate {
    pub address: Address,
    pub removed: bool,
    pub info: Option<AccountInfo>,
    pub code: Option<Bytes>,
    pub added_storage: HashMap<B256, U256>,
    // Matches TODO in code
    // removed_storage_keys: Vec<H256>,
}

impl AccountUpdate {
    /// Creates new empty update for the given account
    pub fn new(address: Address) -> AccountUpdate {
        AccountUpdate {
            address,
            ..Default::default()
        }
    }

    /// Creates new update representing an account removal
    pub fn removed(address: Address) -> AccountUpdate {
        AccountUpdate {
            address,
            removed: true,
            ..Default::default()
        }
    }
}

/// Merges transitions stored when executing transactions and returns the resulting account updates
/// Doesn't update the DB
pub fn get_state_transitions(state: &mut EvmState) -> Vec<AccountUpdate> {
    match state {
        EvmState::Store(db) => {
            db.merge_transitions(BundleRetention::PlainState);
            let bundle = db.take_bundle();

            // Update accounts
            let mut account_updates = Vec::new();
            for (address, account) in bundle.state() {
                if account.status.is_not_modified() {
                    continue;
                }
                let address = Address::from_slice(address.0.as_slice());
                // Remove account from DB if destroyed (Process DestroyedChanged as changed account)
                if matches!(
                    account.status,
                    AccountStatus::Destroyed | AccountStatus::DestroyedAgain
                ) {
                    account_updates.push(AccountUpdate::removed(address));
                    continue;
                }

                // If account is empty, do not add to the database
                if account
                    .account_info()
                    .is_some_and(|acc_info| acc_info.is_empty())
                {
                    continue;
                }

                // Apply account changes to DB
                let mut account_update = AccountUpdate::new(address);
                // If the account was changed then both original and current info will be present in the bundle account
                if account.is_info_changed() {
                    // Update account info in DB
                    if let Some(new_acc_info) = account.account_info() {
                        let code_hash = B256::from_slice(new_acc_info.code_hash.as_slice());
                        let account_info = AccountInfo {
                            code: None,
                            code_hash,
                            balance: U256::from_le_slice(new_acc_info.balance.as_le_slice()),
                            nonce: new_acc_info.nonce,
                        };
                        account_update.info = Some(account_info);
                        if account.is_contract_changed() {
                            // Update code in db
                            if let Some(code) = new_acc_info.code {
                                account_update.code = Some(code.original_bytes().clone());
                            }
                        }
                    }
                }
                // Update account storage in DB
                for (key, slot) in account.storage.iter() {
                    if slot.is_changed() {
                        // TODO check if we need to remove the value from our db when value is zero
                        // if slot.present_value().is_zero() {
                        //     account_update.removed_keys.push(H256::from_uint(&U256::from_little_endian(key.as_le_slice())))
                        // }
                        account_update.added_storage.insert(
                            U256::from_le_slice(key.as_le_slice()).into(),
                            U256::from_le_slice(slot.present_value().as_le_slice()),
                        );
                    }
                }
                account_updates.push(account_update)
            }
            account_updates
        }
    }
}
