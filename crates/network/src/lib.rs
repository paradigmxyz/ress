use std::sync::Arc;

use p2p::P2pHandler;
use ress_common::test_utils::TestPeers;
use reth_chainspec::ChainSpec;
use reth_network_peers::TrustedPeer;
use rpc::RpcHandler;

pub mod p2p;
pub mod rpc;

/// spawn p2p network and rpc server
pub async fn start_network(
    id: TestPeers,
    chain_spec: Arc<ChainSpec>,
    remote_peer: Option<TrustedPeer>,
) -> (P2pHandler, RpcHandler) {
    (
        P2pHandler::start_server(id, chain_spec.clone(), remote_peer).await,
        RpcHandler::start_server(id, chain_spec).await,
    )
}
