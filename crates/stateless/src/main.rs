use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use ress_subprotocol::{
    connection::CustomCommand,
    protocol::{
        event::ProtocolEvent,
        handler::{CustomRlpxProtoHandler, ProtocolState},
    },
};
use reth::builder::{Node, NodeHandle};
use reth_network::{
    config::SecretKey, protocol::IntoRlpxSubProtocol, EthNetworkPrimitives, NetworkConfig,
    NetworkManager,
};
use reth_network_api::{test_utils::PeersHandleProvider, NetworkInfo};
use reth_node_ethereum::EthereumNode;
use tokio::sync::mpsc;
use tracing::info;

// TODO: how to spin up stateless node
fn main() -> eyre::Result<()> {
    reth::cli::Cli::parse_args().run(|builder, _args| async move {
        let NodeHandle {
            node,
            node_exit_future,
        } = builder
            .node(EthereumNode::components_builder(&self))
            .launch()
            .await?;
        // creates a separate network instance and adds the custom network subprotocol
        let secret_key = SecretKey::new(&mut rand::thread_rng());
        let (tx, mut _from_peer1) = mpsc::unbounded_channel();
        let custom_rlpx_handler_2 = CustomRlpxProtoHandler {
            state: ProtocolState { events: tx },
        };
        let net_cfg = NetworkConfig::builder(secret_key)
            .listener_addr(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0)))
            .disable_discovery()
            .add_rlpx_sub_protocol(custom_rlpx_handler_2.into_rlpx_sub_protocol())
            .build_with_noop_provider(node.chain_spec());

        // spawn the second network instance
        let subnetwork = NetworkManager::<EthNetworkPrimitives>::new(net_cfg).await?;
        let subnetwork_peer_id = *subnetwork.peer_id();
        let subnetwork_peer_addr = subnetwork.local_addr();
        let subnetwork_handle = subnetwork.peers_handle();
        node.task_executor.spawn(subnetwork);

        // connect the launched node to the subnetwork
        node.network
            .peers_handle()
            .add_peer(subnetwork_peer_id, subnetwork_peer_addr);

        node_exit_future.await
    })
}
