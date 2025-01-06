use alloy_primitives::B256;
use ress_storage::Storage;
use reth_chainspec::ChainSpec;
use reth_primitives::{Receipt, SealedBlock};
use reth_provider::BlockExecutionOutput;
use reth_revm::{
    db::State,
    primitives::{BlockEnv, SpecId, TxEnv},
    Evm, StateBuilder,
};

use crate::{db::WitnessState, errors::EvmError};

pub struct BlockExecutor {
    state: State<WitnessState>,
}

impl BlockExecutor {
    pub fn new(storage: Storage, block_hash: B256) -> Self {
        Self {
            state: StateBuilder::new_with_database(WitnessState {
                storage,
                block_hash,
            })
            .with_bundle_update()
            .without_state_clear()
            .build(),
        }
    }

    pub fn database(&self) -> Option<&Storage> {
        Some(&self.state.database.storage)
    }

    pub fn chain_config(&self) -> Result<ChainSpec, EvmError> {
        self.state
            .database
            .storage
            .get_chain_config()
            .map_err(EvmError::from)
    }

    pub fn execute(
        &mut self,
        block: &SealedBlock,
    ) -> Result<BlockExecutionOutput<Receipt>, EvmError> {
        let mut receipts = Vec::new();
        let mut cumulative_gas_used = 0;
        // let mut bundle_state = get_state_transitions(state);

        for transaction in block.body.transactions.iter() {
            let _block_header = &block.header;
            // todo: turn block header into block env
            let block_env = BlockEnv::default();
            // todo: turn tx into tx env
            let tx_env = TxEnv::default();
            // todo: get actual spec id
            let spec_id = SpecId::ARROW_GLACIER;
            let evm_builder = Evm::builder()
                .with_block_env(block_env)
                .with_tx_env(tx_env)
                .modify_cfg_env(|cfg| cfg.chain_id = self.chain_config().unwrap().chain.id())
                .with_spec_id(spec_id);
            let db = &mut self.state.database;
            let mut evm = evm_builder.with_db(db).build();
            let result = evm.transact_commit().unwrap();

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
            //todo: process withdrawl
        }

        todo!()
    }
}
