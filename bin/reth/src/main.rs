//! Reth node that supports ress subprotocol.

use alloy_consensus::BlockHeader;
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
use reth_node_builder::Block;
use reth_node_builder::{NodeHandle, NodeTypesWithDB};
use reth_node_ethereum::EthereumNode;
use reth_primitives::Header;
use tokio::sync::mpsc;

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
    N: ProviderNodeTypes,
    E: BlockExecutorProvider<Primitives = N::Primitives>,
{
    fn bytecode(&self, code_hash: B256) -> ProviderResult<Option<Bytes>> {
        Ok(self
            .provider
            .latest()?
            .bytecode_by_hash(&code_hash)?
            .map(|bytecode| bytecode.original_bytes()))
    }

    fn witness(&self, block_hash: B256) -> ProviderResult<Option<B256HashMap<Bytes>>> {
        let block = self
            .provider
            .block_with_senders(block_hash.into(), TransactionVariant::default())?
            .ok_or(ProviderError::BlockHashNotFound(block_hash))?;
        let state_provider = self.provider.history_by_block_hash(block_hash)?;
        let db = StateProviderDatabase::new(&state_provider);
        let mut record = ExecutionWitnessRecord::default();
        let _ = self
            .block_executor
            .executor(db)
            .execute_with_state_closure(&block, |state: &State<_>| {
                record.record_executed_state(state);
            })
            .map_err(|err| ProviderError::TrieWitnessError(err.to_string()))?;
        Ok(Some(
            state_provider.witness(Default::default(), record.hashed_state)?,
        ))
    }

    fn header(&self, block_hash: B256) -> ProviderResult<Option<Header>> {
        let block = self
            .provider
            .block_with_senders(block_hash.into(), TransactionVariant::default())?
            .ok_or(ProviderError::BlockHashNotFound(block_hash))?;
        let header = block.block.header();
        let header = Header {
            parent_hash: header.parent_hash(),
            ommers_hash: header.ommers_hash(),
            beneficiary: header.beneficiary(),
            state_root: header.state_root(),
            transactions_root: header.transactions_root(),
            receipts_root: header.receipts_root(),
            logs_bloom: header.logs_bloom(),
            difficulty: header.difficulty(),
            number: header.number(),
            gas_limit: header.gas_limit(),
            gas_used: header.gas_used(),
            timestamp: header.timestamp(),
            extra_data: header.extra_data().clone(),
            mix_hash: header.mix_hash().expect("invalid mix hash"),
            nonce: header.nonce().expect("invalid nonce"),
            base_fee_per_gas: header.base_fee_per_gas(),
            withdrawals_root: header.withdrawals_root(),
            blob_gas_used: header.blob_gas_used(),
            excess_blob_gas: header.excess_blob_gas(),
            parent_beacon_block_root: header.parent_beacon_block_root(),
            requests_hash: header.requests_hash(),
        };
        Ok(Some(header))
    }
}
