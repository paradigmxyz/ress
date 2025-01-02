use std::sync::Arc;

use engine::ConsensusEngine;
use ress_common::test_utils::TestPeers;
use ress_network::p2p::P2pHandler;
use reth::{chainspec::ChainSpec, rpc::builder::auth::AuthServerHandle};

pub mod engine;
pub mod validate;

pub struct Node {
    pub p2p_handler: Arc<P2pHandler>,
    pub authserver_handler: Arc<AuthServerHandle>,

    consensus_engine_handle: tokio::task::JoinHandle<()>,
}

impl Node {
    pub async fn launch_test_node(id: TestPeers, chain_spec: Arc<ChainSpec>) -> Self {
        let (p2p_handler, rpc_handler) =
            ress_network::start_network(id, Arc::clone(&chain_spec)).await;
        let p2p_handler = Arc::new(p2p_handler);
        let consensus_engine = ConsensusEngine::new(
            chain_spec.as_ref(),
            p2p_handler.clone(),
            rpc_handler.from_beacon_engine,
        );
        let consensus_engine_handle = tokio::spawn(async move {
            consensus_engine.run().await;
        });

        Self {
            p2p_handler,
            authserver_handler: rpc_handler.authserver_handle,
            consensus_engine_handle,
        }
    }

    // gracefully shutdown the node
    pub async fn shutdown(self) {
        self.consensus_engine_handle.abort();
    }
}

#[cfg(test)]
mod tests {
    use reth::chainspec::DEV;

    use super::*;

    #[tokio::test]
    pub async fn test_node() {
        Node::launch_test_node(TestPeers::Peer1, DEV.clone()).await;
    }
}
