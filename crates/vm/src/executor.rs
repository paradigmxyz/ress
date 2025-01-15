use ress_storage::Storage;
use reth_evm::execute::{BlockExecutionStrategy, ExecuteOutput};
use reth_evm_ethereum::{execute::EthExecutionStrategy, EthEvmConfig};
use reth_primitives::{BlockWithSenders, Receipt};
use reth_provider::BlockExecutionOutput;
use reth_revm::StateBuilder;
use std::sync::Arc;

use crate::{db::WitnessDatabase, errors::EvmError};

pub struct BlockExecutor {
    strategy: EthExecutionStrategy<WitnessDatabase, EthEvmConfig>,
}

impl BlockExecutor {
    /// specific block's executor by initiate with parent block post execution state and hash
    pub fn new(db: WitnessDatabase, storage: Arc<Storage>) -> Self {
        let chain_spec = storage.chain_spec.clone();
        let eth_evm_config = EthEvmConfig::new(chain_spec.clone());
        let state = StateBuilder::new_with_database(db)
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
    ) -> Result<BlockExecutionOutput<Receipt>, EvmError> {
        self.strategy.apply_pre_execution_changes(block)?;
        let ExecuteOutput { receipts, gas_used } = self.strategy.execute_transactions(block)?;
        let requests = self
            .strategy
            .apply_post_execution_changes(block, &receipts)?;
        let state = self.strategy.finish();
        Ok(BlockExecutionOutput {
            state,
            receipts,
            requests,
            gas_used,
        })
    }
}
