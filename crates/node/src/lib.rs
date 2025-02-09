//! Ress node implementation.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_eips::BlockNumHash;
use ress_common::test_utils::TestPeers;
use ress_network::{RessNetworkHandle, RessNetworkLauncher};
use ress_provider::{provider::RessProvider, storage::Storage};
use ress_rpc::RpcHandle;
use ress_testing::rpc_adapter::RpcAdapterProvider;
use reth_chainspec::ChainSpec;
use reth_network_peers::TrustedPeer;
use reth_rpc_builder::auth::AuthServerHandle;
use std::sync::Arc;

/// Consensus engine implementation.
pub mod engine;
use engine::ConsensusEngine;

/// State root computation.
pub mod root;

/// Engine error types.
pub mod errors;

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

impl Node {
    /// Launch the test node.
    pub async fn launch_test_node(
        id: TestPeers,
        chain_spec: Arc<ChainSpec>,
        remote_peer: Option<TrustedPeer>,
        current_head: BlockNumHash,
        rpc_adapter: Option<RpcAdapterProvider>,
    ) -> Self {
        let storage = Storage::new(chain_spec.clone(), current_head);

        let network_handle = if let Some(rpc_adapter) = rpc_adapter {
            RessNetworkLauncher::new(chain_spec.clone(), rpc_adapter).launch(id, remote_peer).await
        } else {
            RessNetworkLauncher::new(chain_spec.clone(), storage.clone())
                .launch(id, remote_peer)
                .await
        };
        let rpc_handle = RpcHandle::start_server(id, chain_spec.clone()).await;

        // ================ initial update ==================

        let provider = RessProvider::new(storage, network_handle.clone());
        let consensus_engine =
            ConsensusEngine::new(provider.clone(), rpc_handle.from_beacon_engine);
        let consensus_engine_handle = tokio::spawn(async move { consensus_engine.run().await });

        Self {
            network_handle,
            authserver_handle: rpc_handle.authserver_handle,
            consensus_engine_handle,
            provider,
        }
    }

    /// Gracefully shutdown the node.
    pub async fn shutdown(self) {
        self.consensus_engine_handle.abort();
    }
}
