//! Ress node implementation.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use alloy_eips::BlockNumHash;
use ress_common::test_utils::TestPeers;
use ress_network::P2pHandle;
use ress_provider::provider::RessProvider;
use reth_chainspec::ChainSpec;
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
    /// P2P handle.
    pub p2p_handle: P2pHandle,
    /// Auth RPC server handle.
    pub authserver_handle: AuthServerHandle,
    /// Consensus engine handle.
    pub consensus_engine_handle: tokio::task::JoinHandle<()>,
    /// Ress data provider.
    pub provider: Arc<RessProvider>,
}

impl Node {
    /// Launch the test node.
    pub async fn launch_test_node(
        id: TestPeers,
        chain_spec: Arc<ChainSpec>,
        current_canonical_head: BlockNumHash,
    ) -> Self {
        let (p2p_handler, rpc_handler) =
            ress_network::start_network(id, Arc::clone(&chain_spec)).await;

        // ================ initial update ==================

        // initiate state with parent hash
        let provider = Arc::new(RessProvider::new(
            p2p_handler.network_peer_conn.clone(),
            Arc::clone(&chain_spec),
            current_canonical_head,
        ));

        let consensus_engine = ConsensusEngine::new(
            chain_spec.as_ref(),
            provider.clone(),
            rpc_handler.from_beacon_engine,
        );
        let consensus_engine_handle = tokio::spawn(async move { consensus_engine.run().await });

        Self {
            p2p_handle: p2p_handler,
            authserver_handle: rpc_handler.authserver_handle,
            consensus_engine_handle,
            provider,
        }
    }

    /// Gracefully shutdown the node.
    pub async fn shutdown(self) {
        self.consensus_engine_handle.abort();
    }
}
