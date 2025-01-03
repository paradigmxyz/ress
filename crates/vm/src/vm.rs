use alloy_primitives::B256;
use ress_storage::Storage;
use reth::{
    chainspec::ChainSpec,
    primitives::{Receipt, SealedBlock},
    providers::BlockExecutionOutput,
    revm::{
        db::{State, StateBuilder},
        primitives::{BlockEnv, ExecutionResult, SpecId, TxEnv},
        Evm,
    },
};

use crate::{db::WitnessState, errors::EvmError};

pub enum EvmState {
    State(State<WitnessState>),
}

impl EvmState {
    pub fn new(storage: Storage, block_hash: B256) -> Self {
        EvmState::State(
            StateBuilder::new_with_database(WitnessState {
                storage,
                block_hash,
            })
            .with_bundle_update()
            .without_state_clear()
            .build(),
        )
    }

    #[allow(irrefutable_let_patterns)]
    pub fn database(&self) -> Option<&Storage> {
        if let EvmState::State(db) = self {
            Some(&db.database.storage)
        } else {
            None
        }
    }

    /// Gets the stored chain config
    pub fn chain_config(&self) -> Result<ChainSpec, EvmError> {
        match self {
            EvmState::State(db) => db
                .database
                .storage
                .get_chain_config()
                .map_err(EvmError::from),
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
            EvmState::State(db) => {
                let mut evm = evm_builder.with_db(db).build();
                evm.transact_commit().unwrap()
            }
        }
    };
    Ok(tx_result)
}

/// Executes all transactions in a block and returns their receipts.
pub fn execute_block(
    block: &SealedBlock,
    state: &mut EvmState,
) -> Result<BlockExecutionOutput<Receipt>, EvmError> {
    let mut receipts = Vec::new();
    let mut cumulative_gas_used = 0;
    // let mut bundle_state = get_state_transitions(state);

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

    if let Some(_withdrawals) = &block.body.withdrawals {
        // process_withdrawals(state, withdrawals)?;
        //todo: process withdrawl
    }

    todo!()
}
