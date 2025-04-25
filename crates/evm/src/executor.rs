//! EVM block executor implementation.

use std::{collections::BTreeMap, sync::Arc};

use alloy_primitives::{Bytes, B256};
use reth_chainspec::ChainSpec;
use reth_evm::{
    execute::{BlockExecutionError, BlockExecutor as _},
    ConfigureEvm,
};
use reth_evm_ethereum::EthEvmConfig;
use reth_primitives::{Block, Receipt, RecoveredBlock};
use reth_provider::BlockExecutionOutput;
use reth_revm::db::{states::bundle_state::BundleRetention, State};
use reth_trie_sparse::SparseStateTrie;

use crate::db::WitnessDatabase;

/// An evm block executor that uses a reth's block executor to execute blocks by
/// using state from [`SparseStateTrie`].
#[allow(missing_debug_implementations)]
pub struct BlockExecutor<'a> {
    evm_config: EthEvmConfig,
    state: State<WitnessDatabase<'a>>,
}

impl<'a> BlockExecutor<'a> {
    /// Instantiate new block executor with chain spec and witness database.
    pub fn new(
        chain_spec: Arc<ChainSpec>,
        trie: &'a SparseStateTrie,
        codes: Vec<Bytes>,
        block_hashes: BTreeMap<u64, B256>,
    ) -> Self {
        let evm_config = EthEvmConfig::new(chain_spec);
        let db = WitnessDatabase::new(trie, codes, block_hashes);
        let state =
            State::builder().with_database(db).with_bundle_update().without_state_clear().build();
        Self { evm_config, state }
    }

    /// Execute a block.
    pub fn execute(
        mut self,
        block: &RecoveredBlock<Block>,
    ) -> Result<BlockExecutionOutput<Receipt>, BlockExecutionError> {
        let mut strategy = self.evm_config.executor_for_block(&mut self.state, block);
        strategy.apply_pre_execution_changes()?;
        for tx in block.transactions_recovered() {
            strategy.execute_transaction(tx)?;
        }
        let result = strategy.apply_post_execution_changes()?;
        self.state.merge_transitions(BundleRetention::PlainState);
        Ok(BlockExecutionOutput { state: self.state.take_bundle(), result })
    }
}
