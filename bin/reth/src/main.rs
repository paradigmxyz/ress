//! Reth node that supports ress subprotocol.

use alloy_consensus::BlockHeader as _;
use alloy_primitives::{map::B256HashMap, Bytes, B256};
use futures::StreamExt;
use parking_lot::RwLock;
use ress_protocol::{NodeType, ProtocolState, RessProtocolHandler, RessProtocolProvider};
use reth::{
    network::{protocol::IntoRlpxSubProtocol, NetworkProtocols},
    providers::{
        providers::{BlockchainProvider, ProviderNodeTypes},
        BlockReader, BlockSource, ProviderError, ProviderResult, StateProvider,
        StateProviderFactory,
    },
    revm::{database::StateProviderDatabase, witness::ExecutionWitnessRecord, State},
};
use reth_chain_state::{ExecutedBlockWithTrieUpdates, MemoryOverlayStateProvider};
use reth_evm::execute::{BlockExecutorProvider, Executor};
use reth_node_builder::{BeaconConsensusEngineEvent, Block as _, NodeHandle, NodeTypesWithDB};
use reth_node_ethereum::EthereumNode;
use reth_primitives::{Block, BlockBody, Bytecode, EthPrimitives, Header, RecoveredBlock};
use reth_tokio_util::EventStream;
use reth_trie::TrieInput;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::mpsc;

fn main() -> eyre::Result<()> {
    reth::cli::Cli::parse_args().run(|builder, _args| async move {
        // launch the stateful node
        let NodeHandle { node, node_exit_future } =
            builder.node(EthereumNode::default()).launch().await?;

        let pending_state = PendingState::default();

        // Spawn maintenance task for pending state.
        let events = node.add_ons_handle.engine_events.new_listener();
        let pending_ = pending_state.clone();
        node.task_executor.spawn(maintain_pending_state(events, pending_));

        // add the custom network subprotocol to the launched node
        let (tx, mut _rx) = mpsc::unbounded_channel();
        let provider = RethBlockchainProvider {
            provider: node.provider,
            block_executor: node.block_executor,
            pending_state,
        };
        let protocol_handler = RessProtocolHandler {
            provider,
            state: ProtocolState { events: tx },
            node_type: NodeType::Stateful,
        };
        node.network.add_rlpx_sub_protocol(protocol_handler.into_rlpx_sub_protocol());

        node_exit_future.await
    })
}

/// Reth provider implementing [`RessProtocolProvider`].
#[derive(Clone)]
struct RethBlockchainProvider<N: NodeTypesWithDB, E> {
    provider: BlockchainProvider<N>,
    block_executor: E,
    pending_state: PendingState,
}

impl<N, E> RethBlockchainProvider<N, E>
where
    N: ProviderNodeTypes<Primitives = EthPrimitives>,
    E: BlockExecutorProvider<Primitives = N::Primitives>,
{
    fn block_by_hash(
        &self,
        block_hash: B256,
    ) -> ProviderResult<Option<Arc<RecoveredBlock<Block>>>> {
        // NOTE: we keep track of the pending state locally because reth does not provider a way
        // to access non-canonical or invalid blocks via the provider.
        let maybe_block = if let Some(block) = self.pending_state.recovered_block(&block_hash) {
            Some(block)
        } else if let Some(block) =
            self.provider.find_block_by_hash(block_hash, BlockSource::Any)?
        {
            let signers = block.recover_signers()?;
            Some(Arc::new(block.into_recovered_with_signers(signers)))
        } else {
            // we attempt to look up invalid block last
            self.pending_state.invalid_recovered_block(&block_hash)
        };
        Ok(maybe_block)
    }
}

impl<N, E> RessProtocolProvider for RethBlockchainProvider<N, E>
where
    N: ProviderNodeTypes<Primitives = EthPrimitives>,
    E: BlockExecutorProvider<Primitives = N::Primitives>,
{
    fn header(&self, block_hash: B256) -> ProviderResult<Option<Header>> {
        Ok(self.block_by_hash(block_hash)?.map(|b| b.header().clone()))
    }

    fn block_body(&self, block_hash: B256) -> ProviderResult<Option<BlockBody>> {
        Ok(self.block_by_hash(block_hash)?.map(|b| b.body().clone()))
    }

    fn bytecode(&self, code_hash: B256) -> ProviderResult<Option<Bytes>> {
        let maybe_bytecode = 'bytecode: {
            if let Some(bytecode) = self.pending_state.find_bytecode(code_hash) {
                break 'bytecode Some(bytecode);
            }

            self.provider.latest()?.bytecode_by_hash(&code_hash)?
        };

        Ok(maybe_bytecode.map(|bytecode| bytecode.original_bytes()))
    }

    fn witness(&self, block_hash: B256) -> ProviderResult<Option<B256HashMap<Bytes>>> {
        let block =
            self.block_by_hash(block_hash)?.ok_or(ProviderError::BlockHashNotFound(block_hash))?;

        let mut executed_ancestors = Vec::new();
        let mut ancestor_hash = block.parent_hash();
        let historical = 'sp: loop {
            match self.provider.state_by_block_hash(ancestor_hash) {
                Ok(state_provider) => break 'sp state_provider,
                Err(_) => {
                    let executed = self
                        .pending_state
                        .executed_block(&ancestor_hash)
                        .ok_or(ProviderError::StateForHashNotFound(ancestor_hash))?;
                    ancestor_hash = executed.sealed_block().parent_hash();
                    executed_ancestors.push(executed);
                }
            };
        };
        let db = StateProviderDatabase::new(MemoryOverlayStateProvider::new(
            historical,
            executed_ancestors.clone(),
        ));
        let mut record = ExecutionWitnessRecord::default();
        let _ = self
            .block_executor
            .executor(db)
            .execute_with_state_closure(&block, |state: &State<_>| {
                record.record_executed_state(state);
            })
            .map_err(|err| ProviderError::TrieWitnessError(err.to_string()))?;

        // NOTE: there might be a race condition where target ancestor hash gets evicted from the
        // database.
        let witness_state_provider = self.provider.state_by_block_hash(ancestor_hash)?;
        let mut trie_input = TrieInput::default();
        // TODO: use prepend_cached
        for block in executed_ancestors.into_iter().rev() {
            trie_input.append_ref(&block.hashed_state);
        }
        let witness = witness_state_provider.witness(trie_input, record.hashed_state)?;
        Ok(Some(witness))
    }
}

#[derive(Default, Debug)]
struct PendingStateInner {
    blocks_by_hash: HashMap<B256, ExecutedBlockWithTrieUpdates>,
    invalid_blocks_by_hash: HashMap<B256, Arc<RecoveredBlock<Block>>>,
    // TODO: block_hashes_by_number: BTreeMap<BlockNumber, HashSet<B256>>,
}

#[derive(Clone, Default, Debug)]
struct PendingState(Arc<RwLock<PendingStateInner>>);

impl PendingState {
    fn insert_block(&self, block: ExecutedBlockWithTrieUpdates) {
        let block_hash = block.recovered_block.hash();
        self.0.write().blocks_by_hash.insert(block_hash, block);
    }

    fn insert_invalid_block(&self, block: Arc<RecoveredBlock<Block>>) {
        let block_hash = block.hash();
        self.0.write().invalid_blocks_by_hash.insert(block_hash, block);
    }

    /// Returns only valid executed blocks by hash.
    fn executed_block(&self, hash: &B256) -> Option<ExecutedBlockWithTrieUpdates> {
        self.0.read().blocks_by_hash.get(hash).cloned()
    }

    /// Returns valid recovered block.
    fn recovered_block(&self, hash: &B256) -> Option<Arc<RecoveredBlock<Block>>> {
        self.executed_block(hash).map(|b| b.recovered_block.clone())
    }

    /// Returns invalid recovered block.
    fn invalid_recovered_block(&self, hash: &B256) -> Option<Arc<RecoveredBlock<Block>>> {
        self.0.read().invalid_blocks_by_hash.get(hash).cloned()
    }

    fn find_bytecode(&self, code_hash: B256) -> Option<Bytecode> {
        for block in self.0.read().blocks_by_hash.values() {
            if let Some(contract) = block.execution_output.bytecode(&code_hash) {
                return Some(contract);
            }
        }
        None
    }
}

async fn maintain_pending_state(
    mut events: EventStream<BeaconConsensusEngineEvent>,
    state: PendingState,
) {
    while let Some(event) = events.next().await {
        match event {
            BeaconConsensusEngineEvent::CanonicalBlockAdded(block, _) |
            BeaconConsensusEngineEvent::ForkBlockAdded(block, _) => {
                state.insert_block(block);
            }
            BeaconConsensusEngineEvent::InvalidBlock(block) => {
                if let Ok(block) = block.try_recover() {
                    state.insert_invalid_block(Arc::new(block));
                }
            }
            BeaconConsensusEngineEvent::ForkchoiceUpdated(_state, status) => {
                if status.is_valid() {
                    // TODO: clean up all blocks before finalized
                }
            }
            // ignore
            BeaconConsensusEngineEvent::CanonicalChainCommitted(_, _) |
            BeaconConsensusEngineEvent::LiveSyncProgress(_) => (),
        }
    }
}
