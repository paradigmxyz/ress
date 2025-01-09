use alloy_primitives::{B256, U256};
use ress_storage::Storage;
use reth_evm::execute::{BlockExecutionStrategy, ExecuteOutput};
use reth_evm_ethereum::{execute::EthExecutionStrategy, EthEvmConfig};
use reth_primitives::{BlockWithSenders, Receipt};
use reth_provider::BlockExecutionOutput;
use reth_revm::StateBuilder;
use std::sync::Arc;

use crate::{db::WitnessState, errors::EvmError};

pub struct BlockExecutor {
    strategy: EthExecutionStrategy<WitnessState, EthEvmConfig>,
}

impl BlockExecutor {
    /// specific block's executor by initiate with parent block post execution state and hash
    pub fn new(storage: Arc<Storage>, block_hash: B256) -> Self {
        let chain_spec = storage.chain_spec.clone();
        let eth_evm_config = EthEvmConfig::new(chain_spec.clone());
        let state = StateBuilder::new_with_database(WitnessState {
            storage,
            block_hash,
        })
        .with_bundle_update()
        .without_state_clear()
        .build();
        let strategy = EthExecutionStrategy::new(state, chain_spec, eth_evm_config);
        Self { strategy }
    }

    /// from `BasicBlockExecutor`'s execute
    pub fn execute(
        &mut self,
        block: &BlockWithSenders,
        total_difficulty: U256,
    ) -> Result<BlockExecutionOutput<Receipt>, EvmError> {
        self.strategy
            .apply_pre_execution_changes(block, total_difficulty)
            .unwrap();
        let ExecuteOutput { receipts, gas_used } = self
            .strategy
            .execute_transactions(block, total_difficulty)
            .unwrap();
        let requests = self
            .strategy
            .apply_post_execution_changes(block, total_difficulty, &receipts)
            .unwrap();
        let state = self.strategy.finish();

        Ok(BlockExecutionOutput {
            state,
            receipts,
            requests,
            gas_used,
        })
    }
}
