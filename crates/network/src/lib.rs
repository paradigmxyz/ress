//! P2P networking implementation for the ress node.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

use ress_common::test_utils::TestPeers;
use reth_chainspec::ChainSpec;
use std::sync::Arc;

mod p2p;
pub use p2p::*;

mod rpc;
pub use rpc::RpcHandler;

/// spawn p2p network and rpc server
pub async fn start_network(id: TestPeers, chain_spec: Arc<ChainSpec>) -> (P2pHandle, RpcHandler) {
    (
        P2pHandle::start_server(id).await,
        RpcHandler::start_server(id, chain_spec).await,
    )
}
