//! Reth node that supports ress subprotocol.

use alloy_consensus::BlockHeader as _;
use alloy_primitives::{
    keccak256,
    map::{B256HashMap, B256HashSet},
    Address, BlockNumber, Bytes, B256, U256,
};
use futures::StreamExt;
use parking_lot::{Mutex, RwLock};
use ress_protocol::{
    NodeType, ProtocolState, RessProtocolHandler, RessProtocolProvider, StateWitnessNet,
};
use reth::{
    network::{protocol::IntoRlpxSubProtocol, NetworkProtocols},
    providers::{
        providers::{BlockchainProvider, ProviderNodeTypes},
        BlockNumReader, BlockReader, BlockSource, ProviderError, ProviderResult, StateProvider,
        StateProviderFactory,
    },
    revm::{database::StateProviderDatabase, witness::ExecutionWitnessRecord, Database, State},
};
use reth_chain_state::{ExecutedBlock, ExecutedBlockWithTrieUpdates, MemoryOverlayStateProvider};
use reth_evm::execute::{BlockExecutorProvider, Executor};
use reth_node_builder::{BeaconConsensusEngineEvent, Block as _, NodeHandle, NodeTypesWithDB};
use reth_node_ethereum::EthereumNode;
use reth_primitives::{Block, BlockBody, Bytecode, EthPrimitives, Header, RecoveredBlock};
use reth_tasks::pool::BlockingTaskPool;
use reth_tokio_util::EventStream;
use reth_trie::{HashedPostState, HashedStorage, MultiProofTargets, Nibbles, TrieInput};
use schnellru::{ByLength, LruMap};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::Instant,
};
use tokio::sync::mpsc;
use tracing::*;

fn main() -> eyre::Result<()> {
    reth::cli::Cli::parse_args().run(|builder, _args| async move {
        // launch the stateful node
        let NodeHandle { node, node_exit_future } =
            builder.node(EthereumNode::default()).launch().await?;

        let pending_state = PendingState::default();

        // Spawn maintenance task for pending state.
        let events = node.add_ons_handle.engine_events.new_listener();
        let provider = node.provider.clone();
        let pending_ = pending_state.clone();
        node.task_executor.spawn(maintain_pending_state(events, provider, pending_));

        // add the custom network subprotocol to the launched node
        let (tx, mut _rx) = mpsc::unbounded_channel();
        let provider = RethBlockchainProvider {
            provider: node.provider,
            block_executor: node.block_executor,
            pending_state,
            witness_task_pool: BlockingTaskPool::new(
                BlockingTaskPool::builder().num_threads(5).build()?,
            ),
            witness_cache: Arc::new(Mutex::new(LruMap::new(ByLength::new(10)))),
        };
        let protocol_handler = RessProtocolHandler {
            provider,
            state: ProtocolState { events_sender: tx },
            node_type: NodeType::Stateful,
        };
        node.network.add_rlpx_sub_protocol(protocol_handler.into_rlpx_sub_protocol());

        node_exit_future.await
    })
}

/// Reth provider implementing [`RessProtocolProvider`].
struct RethBlockchainProvider<N: NodeTypesWithDB, E> {
    provider: BlockchainProvider<N>,
    block_executor: E,
    pending_state: PendingState,
    witness_task_pool: BlockingTaskPool,
    witness_cache: Arc<Mutex<LruMap<B256, Arc<StateWitnessNet>>>>,
}

impl<N: NodeTypesWithDB, E: Clone> Clone for RethBlockchainProvider<N, E> {
    fn clone(&self) -> Self {
        Self {
            provider: self.provider.clone(),
            block_executor: self.block_executor.clone(),
            pending_state: self.pending_state.clone(),
            witness_task_pool: self.witness_task_pool.clone(),
            witness_cache: self.witness_cache.clone(),
        }
    }
}

impl<N, E> RethBlockchainProvider<N, E>
where
    N: ProviderNodeTypes<Primitives = EthPrimitives>,
    E: BlockExecutorProvider<Primitives = N::Primitives> + Clone,
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

    fn generate_witness(&self, block_hash: B256) -> ProviderResult<Option<StateWitnessNet>> {
        if let Some(witness) = self.witness_cache.lock().get(&block_hash).cloned() {
            return Ok(Some(witness.as_ref().clone()))
        }

        let block =
            self.block_by_hash(block_hash)?.ok_or(ProviderError::BlockHashNotFound(block_hash))?;

        let mut executed_ancestors = Vec::new();
        let mut ancestor_hash = block.parent_hash();
        let historical = 'sp: loop {
            match self.provider.state_by_block_hash(ancestor_hash) {
                Ok(state_provider) => break 'sp state_provider,
                Err(_) => {
                    let mut executed = self.pending_state.executed_block(&ancestor_hash);
                    if executed.is_none() {
                        if let Some(invalid) =
                            self.pending_state.invalid_recovered_block(&ancestor_hash)
                        {
                            trace!(target: "reth::ress_provider", %block_hash, %ancestor_hash, "Using invalid ancestor block for witness construction");
                            executed = Some(ExecutedBlockWithTrieUpdates {
                                block: ExecutedBlock {
                                    recovered_block: invalid,
                                    ..Default::default()
                                },
                                ..Default::default()
                            });
                        }
                    }

                    let Some(executed) = executed else {
                        return Err(ProviderError::StateForHashNotFound(ancestor_hash))
                    };
                    ancestor_hash = executed.sealed_block().parent_hash();
                    executed_ancestors.push(executed);
                }
            };
        };

        let mut db = StateWitnessRecorderDatabase::new(StateProviderDatabase::new(
            MemoryOverlayStateProvider::new(historical, executed_ancestors.clone()),
        ));
        let mut record = ExecutionWitnessRecord::default();
        if let Err(error) = self.block_executor.executor(&mut db).execute_with_state_closure(
            &block,
            |state: &State<_>| {
                record.record_executed_state(state);
            },
        ) {
            debug!(target: "reth::ress_provider", %block_hash, %error, "Error executing the block");
        }

        // NOTE: there might be a race condition where target ancestor hash gets evicted from the
        // database.
        let witness_state_provider = self.provider.state_by_block_hash(ancestor_hash)?;
        let mut trie_input = TrieInput::default();
        for block in executed_ancestors.into_iter().rev() {
            trie_input.append_cached_ref(&block.trie, &block.hashed_state);
        }
        let mut hashed_state = db.state;
        hashed_state.extend(record.hashed_state);
        let witness = if hashed_state.is_empty() {
            let multiproof = witness_state_provider.multiproof(
                trie_input,
                MultiProofTargets::from_iter([(B256::ZERO, Default::default())]),
            )?;
            let mut witness = B256HashMap::default();
            if let Some(root_node) =
                multiproof.account_subtree.into_inner().remove(&Nibbles::default())
            {
                witness.insert(keccak256(&root_node), root_node.clone());
            }
            witness
        } else {
            witness_state_provider.witness(trie_input, hashed_state)?
        };

        let state_witness = StateWitnessNet::from_iter(witness);
        self.witness_cache.lock().insert(block_hash, Arc::new(state_witness.clone()));

        Ok(Some(state_witness))
    }
}

impl<N, E> RessProtocolProvider for RethBlockchainProvider<N, E>
where
    N: ProviderNodeTypes<Primitives = EthPrimitives>,
    E: BlockExecutorProvider<Primitives = N::Primitives> + Clone,
{
    fn header(&self, block_hash: B256) -> ProviderResult<Option<Header>> {
        trace!(target: "reth::ress_provider", %block_hash, "Serving header");
        Ok(self.block_by_hash(block_hash)?.map(|b| b.header().clone()))
    }

    fn block_body(&self, block_hash: B256) -> ProviderResult<Option<BlockBody>> {
        trace!(target: "reth::ress_provider", %block_hash, "Serving block body");
        Ok(self.block_by_hash(block_hash)?.map(|b| b.body().clone()))
    }

    fn bytecode(&self, code_hash: B256) -> ProviderResult<Option<Bytes>> {
        trace!(target: "reth::ress_provider", %code_hash, "Serving bytecode");
        let maybe_bytecode = 'bytecode: {
            if let Some(bytecode) = self.pending_state.find_bytecode(code_hash) {
                break 'bytecode Some(bytecode);
            }

            self.provider.latest()?.bytecode_by_hash(&code_hash)?
        };

        Ok(maybe_bytecode.map(|bytecode| bytecode.original_bytes()))
    }

    async fn witness(&self, block_hash: B256) -> ProviderResult<Option<StateWitnessNet>> {
        trace!(target: "reth::ress_provider", %block_hash, "Serving witness");
        let started_at = Instant::now();
        let this = self.clone();
        match self.witness_task_pool.spawn(move || this.generate_witness(block_hash)).await {
            Ok(Ok(witness)) => {
                trace!(target: "reth::ress_provider", %block_hash, elapsed = ?started_at.elapsed(), "Computed witness");
                Ok(witness)
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Err(ProviderError::TrieWitnessError("dropped".to_owned())),
        }
    }
}

#[derive(Default, Debug)]
struct PendingStateInner {
    blocks_by_hash: HashMap<B256, ExecutedBlockWithTrieUpdates>,
    invalid_blocks_by_hash: HashMap<B256, Arc<RecoveredBlock<Block>>>,
    block_hashes_by_number: BTreeMap<BlockNumber, B256HashSet>,
}

#[derive(Clone, Default, Debug)]
struct PendingState(Arc<RwLock<PendingStateInner>>);

impl PendingState {
    fn insert_block(&self, block: ExecutedBlockWithTrieUpdates) {
        let mut this = self.0.write();
        let block_hash = block.recovered_block.hash();
        this.block_hashes_by_number
            .entry(block.recovered_block.number)
            .or_default()
            .insert(block_hash);
        this.blocks_by_hash.insert(block_hash, block);
    }

    fn insert_invalid_block(&self, block: Arc<RecoveredBlock<Block>>) {
        let mut this = self.0.write();
        let block_hash = block.hash();
        this.block_hashes_by_number.entry(block.number).or_default().insert(block_hash);
        this.invalid_blocks_by_hash.insert(block_hash, block);
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

    /// Find bytecode in executed blocks state.
    fn find_bytecode(&self, code_hash: B256) -> Option<Bytecode> {
        let this = self.0.read();
        for block in this.blocks_by_hash.values() {
            if let Some(contract) = block.execution_output.bytecode(&code_hash) {
                return Some(contract);
            }
        }
        None
    }

    fn remove_before(&self, block_number: BlockNumber) -> u64 {
        let mut removed = 0;
        let mut this = self.0.write();
        while this
            .block_hashes_by_number
            .first_key_value()
            .is_some_and(|(number, _)| number <= &block_number)
        {
            let (_, block_hashes) = this.block_hashes_by_number.pop_first().unwrap();
            for block_hash in block_hashes {
                removed += 1;
                this.blocks_by_hash.remove(&block_hash);
                this.invalid_blocks_by_hash.remove(&block_hash);
            }
        }
        removed
    }
}

async fn maintain_pending_state<N>(
    mut events: EventStream<BeaconConsensusEngineEvent>,
    provider: BlockchainProvider<N>,
    pending_state: PendingState,
) where
    N: ProviderNodeTypes<Primitives = EthPrimitives>,
{
    while let Some(event) = events.next().await {
        match event {
            BeaconConsensusEngineEvent::CanonicalBlockAdded(block, _) |
            BeaconConsensusEngineEvent::ForkBlockAdded(block, _) => {
                trace!(target: "reth::ress_provider", block = ? block.recovered_block().num_hash(), "Insert block into pending state");
                pending_state.insert_block(block);
            }
            BeaconConsensusEngineEvent::InvalidBlock(block) => {
                if let Ok(block) = block.try_recover() {
                    trace!(target: "reth::ress_provider", block = ?block.num_hash(), "Insert invalid block into pending state");
                    pending_state.insert_invalid_block(Arc::new(block));
                }
            }
            BeaconConsensusEngineEvent::ForkchoiceUpdated(state, status) => {
                if status.is_valid() {
                    let target = state.finalized_block_hash;
                    if let Ok(Some(block_number)) = provider.block_number(target) {
                        let count = pending_state.remove_before(block_number);
                        trace!(target: "reth::ress_provider", block_number, count, "Removing blocks before finalized");
                    }
                }
            }
            // ignore
            BeaconConsensusEngineEvent::CanonicalChainCommitted(_, _) |
            BeaconConsensusEngineEvent::LiveSyncProgress(_) => (),
        }
    }
}

struct StateWitnessRecorderDatabase<D> {
    database: D,
    state: HashedPostState,
}

impl<D> StateWitnessRecorderDatabase<D> {
    fn new(database: D) -> Self {
        Self { database, state: Default::default() }
    }
}

impl<D: Database> Database for StateWitnessRecorderDatabase<D> {
    type Error = D::Error;

    fn basic(
        &mut self,
        address: Address,
    ) -> Result<Option<reth::revm::primitives::AccountInfo>, Self::Error> {
        let maybe_account = self.database.basic(address)?;
        let hashed_address = keccak256(address);
        self.state.accounts.insert(hashed_address, maybe_account.as_ref().map(|acc| acc.into()));
        Ok(maybe_account)
    }

    fn storage(&mut self, address: Address, index: U256) -> Result<U256, Self::Error> {
        let value = self.database.storage(address, index)?;
        let hashed_address = keccak256(address);
        let hashed_slot = keccak256(B256::from(index));
        self.state
            .storages
            .entry(hashed_address)
            .or_insert_with(|| HashedStorage::new(false))
            .storage
            .insert(hashed_slot, value);
        Ok(value)
    }

    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        self.database.block_hash(number)
    }

    fn code_by_hash(
        &mut self,
        code_hash: B256,
    ) -> Result<reth::revm::primitives::Bytecode, Self::Error> {
        self.database.code_by_hash(code_hash)
    }
}
