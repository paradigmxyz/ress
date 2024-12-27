use std::net::SocketAddr;

use ress_subprotocol::connection::CustomCommand;
use ress_subprotocol::protocol::event::ProtocolEvent;
use ress_subprotocol::protocol::handler::{CustomRlpxProtoHandler, ProtocolState};
use ress_subprotocol::protocol::proto::NodeType;
use reth::api::BeaconEngineMessage;
use reth::payload::test_utils::spawn_test_payload_service;
use reth::rpc::builder::auth::{AuthRpcModule, AuthServerHandle};
use reth::tasks::TokioTaskExecutor;
use reth::transaction_pool::noop::NoopTransactionPool;
use reth::transaction_pool::PeerId;
use reth::{
    beacon_consensus::{BeaconConsensusEngineEvent, BeaconConsensusEngineHandle},
    chainspec::DEV,
    providers::noop::NoopProvider,
    rpc::{
        builder::auth::AuthServerConfig,
        types::engine::{ClientCode, ClientVersionV1, JwtSecret},
    },
};
use reth_network::config::SecretKey;
use reth_network::protocol::IntoRlpxSubProtocol;
use reth_network::{EthNetworkPrimitives, NetworkConfig, NetworkHandle, NetworkManager};
use reth_node_ethereum::node::EthereumEngineValidator;
use reth_node_ethereum::EthEngineTypes;
use reth_rpc_engine_api::capabilities::EngineCapabilities;
use reth_rpc_engine_api::EngineApi;
use reth_tokio_util::EventSender;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tracing::info;

use crate::consensus_engine::ConsensusEngine;
use crate::test_utils::TestPeers;

pub struct Node {
    //  retrieve beacon engine client to interact
    pub authserve_handle: AuthServerHandle,
    /// channel to send - network
    pub network_peer_conn: UnboundedSender<CustomCommand>,
    /// channel to receive - network
    pub network_handle: NetworkHandle,

    consensus_engine_handle: tokio::task::JoinHandle<()>,
}

impl Node {
    pub async fn launch_test_node(id: &TestPeers) -> Self {
        let (authserve_handle, from_beacon_engine) =
            Self::launch_auth_server(id.get_jwt_key(), id.get_authserver_addr()).await;

        let (subnetwork_handle, from_peer) =
            Self::launch_subprotocol_network(id.get_key(), id.get_network_addr()).await;

        // connect peer to own network
        subnetwork_handle.peers_handle().add_peer(
            id.get_peer().get_peer_id(),
            id.get_peer().get_network_addr(),
        );
        info!("added peer_id: {:?}", id.get_peer().get_peer_id());

        // get a handle to the network to interact with it
        let handle = subnetwork_handle.handle().clone();
        // spawn the network
        tokio::task::spawn(subnetwork_handle);

        let network_peer_conn =
            Self::setup_subprotocol_network(from_peer, id.get_peer().get_peer_id()).await;

        // Initialize and spawn the consensus engine
        let consensus_engine = ConsensusEngine::new(from_beacon_engine);
        let consensus_engine_handle = tokio::spawn(async move {
            consensus_engine.run().await;
        });

        Self {
            authserve_handle,
            network_peer_conn,
            network_handle: handle,
            consensus_engine_handle,
        }
    }

    async fn launch_auth_server(
        jwt_key: JwtSecret,
        socket: SocketAddr,
    ) -> (
        AuthServerHandle,
        UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
    ) {
        let config = AuthServerConfig::builder(jwt_key)
            .socket_addr(socket)
            .build();
        let (tx, rx) = unbounded_channel();
        let event_handler: EventSender<BeaconConsensusEngineEvent> = Default::default();
        // Create a listener for the events
        let mut _listener = event_handler.new_listener();
        let beacon_engine_handle =
            BeaconConsensusEngineHandle::<EthEngineTypes>::new(tx, event_handler);
        let client = ClientVersionV1 {
            code: ClientCode::RH,
            name: "Ress".to_string(),
            version: "v0.1.0".to_string(),
            commit: "defa64b2".to_string(),
        };
        let engine_api = EngineApi::new(
            NoopProvider::default(),
            DEV.clone(),
            beacon_engine_handle,
            spawn_test_payload_service().into(),
            NoopTransactionPool::default(),
            Box::<TokioTaskExecutor>::default(),
            client,
            EngineCapabilities::default(),
            EthereumEngineValidator::new(DEV.clone()),
        );
        let module = AuthRpcModule::new(engine_api);
        let handle = module.start_server(config).await.unwrap();
        (handle, rx)
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

        info!(target:"rlpx-subprotocol",  "Connection established!");

        // =================================================================

        // Step 1. Type message subprotocol
        info!(target:"rlpx-subprotocol", "1️⃣ check connection valiadation");
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
        info!(target:"rlpx-subprotocol",?response, "Connection validation finished");

        peer_conn
    }

    // gracefully shutdown the node
    pub async fn shutdown(self) {
        self.consensus_engine_handle.abort();
    }
}
