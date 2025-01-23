use std::{net::SocketAddr, sync::Arc};

use ress_common::test_utils::TestPeers;
use ress_subprotocol::{
    connection::CustomCommand,
    protocol::{
        event::ProtocolEvent,
        handler::{CustomRlpxProtoHandler, ProtocolState},
        proto::NodeType,
    },
    utils::setup_subprotocol_network,
};

use reth_chainspec::ChainSpec;
use reth_network::{
    config::SecretKey, protocol::IntoRlpxSubProtocol, EthNetworkPrimitives, NetworkConfig,
    NetworkHandle, NetworkManager,
};
use reth_network_peers::TrustedPeer;
use reth_primitives::EthPrimitives;
use reth_provider::noop::NoopProvider;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::info;

pub struct P2pHandler {
    /// channel to receive - network
    pub network_handle: NetworkHandle,

    // channel to send - network
    pub network_peer_conn: UnboundedSender<CustomCommand>,
}

impl P2pHandler {
    pub(crate) async fn start_server(
        id: TestPeers,
        chain_spec: Arc<ChainSpec>,
        remote_peer: Option<TrustedPeer>,
    ) -> Self {
        let (subnetwork_handle, from_peer) =
            Self::launch_subprotocol_network(id.get_key(), id.get_network_addr(), chain_spec).await;

        let (remote_id, remote_addr) = if let Some(remote_peer) = remote_peer {
            (
                remote_peer.id,
                remote_peer.resolve_blocking().expect("peer").tcp_addr(),
            )
        } else {
            (
                id.get_peer().get_peer_id(),
                id.get_peer().get_network_addr(),
            )
        };

        // connect peer to own network
        subnetwork_handle
            .peers_handle()
            .add_peer(remote_id, remote_addr);

        // get a handle to the network to interact with it
        let network_handle = subnetwork_handle.handle().clone();
        // spawn the network
        tokio::task::spawn(subnetwork_handle);

        let network_peer_conn =
            setup_subprotocol_network(from_peer, remote_id, NodeType::Stateless).await;

        Self {
            network_handle,
            network_peer_conn,
        }
    }

    async fn launch_subprotocol_network(
        secret_key: SecretKey,
        socket: SocketAddr,
        chain_spec: Arc<ChainSpec>,
    ) -> (NetworkManager, UnboundedReceiver<ProtocolEvent>) {
        // This block provider implementation is used for testing purposes.
        let client = NoopProvider::<ChainSpec, EthPrimitives>::new(chain_spec);

        let (tx, from_peer) = tokio::sync::mpsc::unbounded_channel();
        let custom_rlpx_handler = CustomRlpxProtoHandler {
            state: ProtocolState { events: tx },
            node_type: NodeType::Stateless,
            state_provider: None,
        };

        // Configure the network
        let config = NetworkConfig::builder(secret_key)
            .listener_addr(socket)
            .disable_discovery()
            .add_rlpx_sub_protocol(custom_rlpx_handler.into_rlpx_sub_protocol())
            .build(client);

        // create the network instance
        let subnetwork = NetworkManager::<EthNetworkPrimitives>::new(config)
            .await
            .unwrap();

        let subnetwork_peer_id = *subnetwork.peer_id();
        let subnetwork_peer_addr = subnetwork.local_addr();

        info!(
            "subnetwork | peer_id: {}, peer_addr: {} ",
            subnetwork_peer_id, subnetwork_peer_addr
        );

        (subnetwork, from_peer)
    }
}
