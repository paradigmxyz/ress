//! EVM block executor implementation.

use alloy_eips::BlockNumHash;
use ress_provider::RessProvider;
use reth_evm::execute::{BlockExecutionError, BlockExecutionStrategy, ExecuteOutput};
use reth_evm_ethereum::{execute::EthExecutionStrategy, EthEvmConfig};
use reth_primitives::{Block, Receipt, RecoveredBlock};
use reth_provider::BlockExecutionOutput;
use reth_revm::StateBuilder;
use reth_trie_sparse::SparseStateTrie;

use crate::db::WitnessDatabase;

/// An evm block executor that uses a [`EthExecutionStrategy`] to
/// execute blocks by using state from [`SparseStateTrie`].
#[allow(missing_debug_implementations)]
pub struct BlockExecutor<'a> {
    strategy: EthExecutionStrategy<WitnessDatabase<'a>, EthEvmConfig>,
}

impl<'a> BlockExecutor<'a> {
    /// Instantiate new block executor with chain spec and witness database.
    pub fn new(provider: RessProvider, parent: BlockNumHash, trie: &'a SparseStateTrie) -> Self {
        let chain_spec = provider.chain_spec();
        let db = WitnessDatabase::new(provider, parent, trie);
        let eth_evm_config = EthEvmConfig::new(chain_spec.clone());
        let state =
            StateBuilder::new_with_database(db).with_bundle_update().without_state_clear().build();
        let strategy = EthExecutionStrategy::new(state, chain_spec, eth_evm_config);
        Self { strategy }
    }

    /// Execute a block.
    pub fn execute(
        mut self,
        block: &RecoveredBlock<Block>,
    ) -> Result<BlockExecutionOutput<Receipt>, BlockExecutionError> {
        self.strategy.apply_pre_execution_changes(block)?;
        let ExecuteOutput { receipts, gas_used } = self.strategy.execute_transactions(block)?;
        let requests = self.strategy.apply_post_execution_changes(block, &receipts)?;
        let mut state = self.strategy.into_state();
        Ok(BlockExecutionOutput { state: state.take_bundle(), receipts, requests, gas_used })
    }
}
