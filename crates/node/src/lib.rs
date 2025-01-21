use std::sync::Arc;

use alloy_eips::BlockNumHash;
use engine::ConsensusEngine;
use ress_common::test_utils::TestPeers;
use ress_network::p2p::P2pHandler;
use ress_provider::provider::RessProvider;
use reth_chainspec::ChainSpec;
use reth_rpc_builder::auth::AuthServerHandle;

pub mod engine;
pub mod errors;

pub struct Node {
    pub p2p_handler: P2pHandler,
    pub authserver_handler: AuthServerHandle,
    consensus_engine_handle: tokio::task::JoinHandle<()>,
    pub provider: Arc<RessProvider>,
}

impl Node {
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
            p2p_handler,
            authserver_handler: rpc_handler.authserver_handle,
            consensus_engine_handle,
            provider,
        }
    }

    // gracefully shutdown the node
    pub async fn shutdown(self) {
        self.consensus_engine_handle.abort();
    }
}
