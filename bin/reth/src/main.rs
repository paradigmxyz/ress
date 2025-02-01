//! Reth node that supports ress subprotocol.

use alloy_primitives::{map::B256HashMap, Bytes, B256};
use ress_protocol::{NodeType, ProtocolState, RessProtocolHandler, RessProtocolProvider};
use reth::{
    network::{protocol::IntoRlpxSubProtocol, NetworkProtocols},
    providers::{
        providers::{BlockchainProvider, ProviderNodeTypes},
        BlockReader, ProviderError, ProviderResult, StateProviderFactory, TransactionVariant,
    },
    revm::{database::StateProviderDatabase, witness::ExecutionWitnessRecord, State},
};
use reth_evm::execute::{BlockExecutorProvider, Executor};
use reth_node_builder::{NodeHandle, NodeTypesWithDB};
use reth_node_ethereum::EthereumNode;
use reth_primitives::{EthPrimitives, Header};
use tokio::sync::mpsc;
use tracing::info;

fn main() -> eyre::Result<()> {
    reth::cli::Cli::parse_args().run(|builder, _args| async move {
        // launch the stateful node
        let NodeHandle {
            node,
            node_exit_future,
        } = builder.node(EthereumNode::default()).launch().await?;

        // add the custom network subprotocol to the launched node
        let (tx, mut _from_peer0) = mpsc::unbounded_channel();
        let provider = RethBlockchainProvider {
            provider: node.provider,
            block_executor: node.block_executor,
        };
        let protocol_handler = RessProtocolHandler {
            provider,
            state: ProtocolState { events: tx },
            node_type: NodeType::Stateful,
        };
        node.network
            .add_rlpx_sub_protocol(protocol_handler.into_rlpx_sub_protocol());

        node_exit_future.await
    })
}

/// Reth provider implementing [`RessProtocolProvider`].
#[derive(Clone)]
struct RethBlockchainProvider<N: NodeTypesWithDB, E> {
    provider: BlockchainProvider<N>,
    block_executor: E,
}

impl<N, E> RessProtocolProvider for RethBlockchainProvider<N, E>
where
    N: ProviderNodeTypes<Primitives = EthPrimitives>,
    E: BlockExecutorProvider<Primitives = N::Primitives>,
{
    fn header(&self, block_hash: B256) -> ProviderResult<Option<Header>> {
        let block = self
            .provider
            .block_with_senders(block_hash.into(), TransactionVariant::default())?
            .ok_or(ProviderError::BlockHashNotFound(block_hash))?;
        Ok(Some(block.header().clone()))
    }

    fn bytecode(&self, code_hash: B256) -> ProviderResult<Option<Bytes>> {
        Ok(self
            .provider
            .latest()?
            .bytecode_by_hash(&code_hash)?
            .map(|bytecode| bytecode.original_bytes()))
    }

    fn witness(&self, block_hash: B256) -> ProviderResult<Option<B256HashMap<Bytes>>> {
        // TODO: this is a workaround because reth's `find_block_by_hash` does not work as expected
        let block = if let Some(pending) = self
            .provider
            .pending_block_with_senders()?
            .filter(|b| b.hash() == block_hash)
        {
            pending
        } else {
            self.provider
                .block_with_senders(block_hash.into(), TransactionVariant::default())?
                .ok_or(ProviderError::BlockHashNotFound(block_hash))?
        };
        let state_provider = self.provider.state_by_block_hash(block.parent_hash)?;
        let db = StateProviderDatabase::new(&state_provider);
        let mut record = ExecutionWitnessRecord::default();
        let _ = self
            .block_executor
            .executor(db)
            .execute_with_state_closure(&block, |state: &State<_>| {
                record.record_executed_state(state);
            })
            .map_err(|err| ProviderError::TrieWitnessError(err.to_string()))?;
        info!("record: {:?}", record);
        let witness = state_provider.witness(Default::default(), record.hashed_state)?;
        info!("witness: {:?}", witness);
        Ok(Some(witness))
    }
}
