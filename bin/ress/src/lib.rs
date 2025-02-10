//! Ress node launcher.

use alloy_eips::BlockNumHash;
use ress_common::test_utils::TestPeers;
use ress_engine::engine::ConsensusEngine;
use ress_network::{RessNetworkHandle, RessNetworkLauncher};
use ress_provider::{provider::RessProvider, storage::Storage};
use ress_rpc::RpcHandle;
use ress_testing::rpc_adapter::RpcAdapterProvider;
use reth_chainspec::ChainSpec;
use reth_network_peers::TrustedPeer;
use reth_node_ethereum::{consensus::EthBeaconConsensus, node::EthereumEngineValidator};
use reth_rpc_builder::auth::AuthServerHandle;
use std::sync::Arc;

/// Ress node components.
#[derive(Debug)]
pub struct Node {
    /// Ress data provider.
    pub provider: RessProvider,
    /// P2P handle.
    pub network_handle: RessNetworkHandle,
    /// Auth RPC server handle.
    pub authserver_handle: AuthServerHandle,
    /// Consensus engine handle.
    pub consensus_engine_handle: tokio::task::JoinHandle<()>,
}

/// Launch the test node.
pub async fn launch_test_node(
    id: TestPeers,
    chain_spec: Arc<ChainSpec>,
    remote_peer: Option<TrustedPeer>,
    current_head: BlockNumHash,
    rpc_adapter: Option<RpcAdapterProvider>,
) -> Node {
    let storage = Storage::new(chain_spec.clone(), current_head);

    let network_handle = if let Some(rpc_adapter) = rpc_adapter {
        RessNetworkLauncher::new(chain_spec.clone(), rpc_adapter).launch(id, remote_peer).await
    } else {
        RessNetworkLauncher::new(chain_spec.clone(), storage.clone()).launch(id, remote_peer).await
    };
    let rpc_handle = RpcHandle::start_server(id, chain_spec.clone()).await;

    // ================ initial update ==================

    let provider = RessProvider::new(storage, network_handle.clone());
    let beacon_consensus = EthBeaconConsensus::new(chain_spec.clone());
    let engine_validator = EthereumEngineValidator::new(chain_spec.clone());
    let consensus_engine = ConsensusEngine::new(
        provider.clone(),
        beacon_consensus,
        engine_validator,
        network_handle.clone(),
        rpc_handle.from_beacon_engine,
    );
    let consensus_engine_handle = tokio::spawn(consensus_engine);

    Node {
        network_handle,
        authserver_handle: rpc_handle.authserver_handle,
        consensus_engine_handle,
        provider,
    }
}
