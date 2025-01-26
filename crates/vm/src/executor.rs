//! EVM block executor.

use ress_provider::storage::Storage;
use reth_evm::execute::{BlockExecutionStrategy, ExecuteOutput};
use reth_evm_ethereum::{execute::EthExecutionStrategy, EthEvmConfig};
use reth_primitives::{BlockWithSenders, Receipt};
use reth_provider::BlockExecutionOutput;
use reth_revm::StateBuilder;
use std::sync::Arc;

use crate::{db::WitnessDatabase, errors::EvmError};

/// Block executor that wraps reth's [`EthExecutionStrategy`].
#[allow(missing_debug_implementations)]
pub struct BlockExecutor<'a> {
    strategy: EthExecutionStrategy<WitnessDatabase<'a>, EthEvmConfig>,
}

impl<'a> BlockExecutor<'a> {
    /// Instantiate new block executor with witness and storage.
    pub fn new(db: WitnessDatabase<'a>, storage: Arc<Storage>) -> Self {
        let chain_spec = storage.get_chain_config();
        let eth_evm_config = EthEvmConfig::new(chain_spec.clone());
        let state = StateBuilder::new_with_database(db)
            .with_bundle_update()
            .without_state_clear()
            .build();
        let strategy = EthExecutionStrategy::new(state, chain_spec, eth_evm_config);
        Self { strategy }
    }

    /// Execute a block.
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
