use std::net::SocketAddr;

use ress_common::test_utils::TestPeers;
use ress_subprotocol::{
    connection::CustomCommand,
    protocol::{
        event::ProtocolEvent,
        handler::{CustomRlpxProtoHandler, ProtocolState},
        proto::NodeType,
    },
};

use reth_network::{
    config::SecretKey, protocol::IntoRlpxSubProtocol, EthNetworkPrimitives, NetworkConfig,
    NetworkHandle, NetworkManager,
};
use reth_provider::noop::NoopProvider;
use reth_transaction_pool::PeerId;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::info;

pub struct P2pHandler {
    /// channel to receive - network
    pub network_handle: NetworkHandle,

    // channel to send - network
    pub network_peer_conn: UnboundedSender<CustomCommand>,
}

impl P2pHandler {
    pub async fn start_server(id: TestPeers) -> Self {
        let (subnetwork_handle, from_peer) =
            Self::launch_subprotocol_network(id.get_key(), id.get_network_addr()).await;
        // connect peer to own network
        subnetwork_handle.peers_handle().add_peer(
            id.get_peer().get_peer_id(),
            id.get_peer().get_network_addr(),
        );
        info!("added peer_id: {:?}", id.get_peer().get_peer_id());

        // get a handle to the network to interact with it
        let network_handle = subnetwork_handle.handle().clone();
        // spawn the network
        tokio::task::spawn(subnetwork_handle);

        let network_peer_conn =
            Self::setup_subprotocol_network(from_peer, id.get_peer().get_peer_id()).await;

        Self {
            network_handle,
            network_peer_conn,
        }
    }

    async fn launch_subprotocol_network(
        secret_key: SecretKey,
        socket: SocketAddr,
    ) -> (NetworkManager, UnboundedReceiver<ProtocolEvent>) {
        // This block provider implementation is used for testing purposes.
        let client = NoopProvider::default();

        let (tx, from_peer) = tokio::sync::mpsc::unbounded_channel();
        let custom_rlpx_handler = CustomRlpxProtoHandler {
            state: ProtocolState { events: tx },
            node_type: NodeType::Stateless,
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
        let subnet_secret = subnetwork.secret_key();
        let subnetwork_peer_addr = subnetwork.local_addr();

        info!("subnetwork_peer_id {}", subnetwork_peer_id);
        info!("subnetwork_peer_addr {}", subnetwork_peer_addr);
        info!("subnet_secret {:?}", subnet_secret);

        (subnetwork, from_peer)
    }

    /// Establish connection and send type checking
    async fn setup_subprotocol_network(
        mut from_peer: UnboundedReceiver<ProtocolEvent>,
        peer_id: PeerId,
    ) -> UnboundedSender<CustomCommand> {
        // Establish connection between peer0 and peer1
        let peer_to_peer = from_peer.recv().await.expect("peer connecting");
        let peer_conn = match peer_to_peer {
            ProtocolEvent::Established {
                direction: _,
                peer_id: received_peer_id,
                to_connection,
            } => {
                assert_eq!(received_peer_id, peer_id);
                to_connection
            }
        };
        info!(target:"rlpx-subprotocol",  "connection established!");

        // =================================================================

        //  Type message subprotocol
        // TODO: for now we initiate original node type on protocol state above, but after conenction we send msg to trigger connection validation. Is there a way to explicitly mention node type one time?
        let (tx, rx) = tokio::sync::oneshot::channel();
        peer_conn
            .send(CustomCommand::NodeType {
                node_type: NodeType::Stateless,
                response: tx,
            })
            .unwrap();
        let response = rx.await.unwrap();
        assert!(response);
        info!(target:"rlpx-subprotocol",?response, "connection validation finished");

        peer_conn
    }
}
