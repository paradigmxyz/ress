use crate::RessNetworkHandle;
use futures::StreamExt;
use ress_common::test_utils::TestPeers;
use ress_protocol::{
    NodeType, ProtocolEvent, ProtocolState, RessPeerRequest, RessProtocolHandler,
    RessProtocolProvider,
};
use reth_chainspec::ChainSpec;
use reth_network::{
    config::SecretKey, protocol::IntoRlpxSubProtocol, EthNetworkPrimitives, NetworkConfig,
    NetworkManager,
};
use reth_network_api::PeerId;
use reth_network_peers::TrustedPeer;
use std::{
    collections::VecDeque,
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::*;

/// Ress network launcher.
#[allow(missing_debug_implementations)]
pub struct RessNetworkLauncher<P> {
    chain_spec: Arc<ChainSpec>,
    provider: P,
}

impl<P> RessNetworkLauncher<P>
where
    P: RessProtocolProvider + Clone + Unpin + 'static,
{
    /// Instantiate the launcher.
    pub fn new(chain_spec: Arc<ChainSpec>, provider: P) -> Self {
        Self { chain_spec, provider }
    }

    /// Start network manager.
    pub async fn launch(
        &self,
        id: TestPeers,
        remote_peer: Option<TrustedPeer>,
    ) -> RessNetworkHandle {
        let (subnetwork_handle, from_peer) =
            self.launch_subprotocol_network(id.get_key(), id.get_network_addr()).await;

        let (remote_id, remote_addr) = if let Some(remote_peer) = remote_peer {
            (remote_peer.id, remote_peer.resolve_blocking().expect("peer").tcp_addr())
        } else {
            (id.get_peer().get_peer_id(), id.get_peer().get_network_addr())
        };

        // connect peer to own network
        subnetwork_handle.peers_handle().add_peer(remote_id, remote_addr);

        // get a handle to the network to interact with it
        let network_handle = subnetwork_handle.handle().clone();
        // spawn the network
        tokio::task::spawn(subnetwork_handle);

        let peer_connection = Self::setup_subprotocol_network(from_peer, remote_id).await;

        RessNetworkHandle::new(network_handle, peer_connection)
    }

    async fn launch_subprotocol_network(
        &self,
        secret_key: SecretKey,
        socket: SocketAddr,
    ) -> (NetworkManager, UnboundedReceiver<ProtocolEvent>) {
        let (tx, from_peer) = tokio::sync::mpsc::unbounded_channel();
        let protocol_handler = RessProtocolHandler {
            provider: self.provider.clone(),
            node_type: NodeType::Stateless,
            state: ProtocolState { events: tx },
        };

        // Configure the network
        let config = NetworkConfig::builder(secret_key)
            .listener_addr(socket)
            .disable_discovery()
            .add_rlpx_sub_protocol(protocol_handler.into_rlpx_sub_protocol())
            .build_with_noop_provider(self.chain_spec.clone());

        // create the network instance
        let subnetwork = NetworkManager::<EthNetworkPrimitives>::new(config).await.unwrap();

        let subnetwork_peer_id = *subnetwork.peer_id();
        let subnetwork_peer_addr = subnetwork.local_addr();

        info!("subnetwork | peer_id: {}, peer_addr: {} ", subnetwork_peer_id, subnetwork_peer_addr);

        (subnetwork, from_peer)
    }

    /// Establish connection and send type checking
    async fn setup_subprotocol_network(
        mut from_peer: UnboundedReceiver<ProtocolEvent>,
        peer_id: PeerId,
    ) -> UnboundedSender<RessPeerRequest> {
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
        info!(%peer_id, "🟢 connection established");
        peer_conn
    }
}

struct ConnectionHandle {
    peer_id: PeerId,
    to_connection: mpsc::UnboundedSender<RessPeerRequest>,
}

struct RessNetworkManager {
    protocol_events: UnboundedReceiverStream<ProtocolEvent>,
    connections: VecDeque<ConnectionHandle>,
    peer_requests: UnboundedReceiverStream<RessPeerRequest>,
}

impl RessNetworkManager {
    fn on_peer_request(&mut self, mut request: RessPeerRequest) {
        // Rotate connections for peer requests
        while let Some(connection) = self.connections.pop_front() {
            trace!(target: "ress::net", peer_id = %connection.peer_id, ?request, "Sending request to peer");
            match connection.to_connection.send(request) {
                Ok(()) => {
                    self.connections.push_back(connection);
                    return
                }
                Err(mpsc::error::SendError(request_)) => {
                    request = request_;
                    trace!(target: "ress::net", peer_id = %connection.peer_id, ?request, "Failed to send request, connection closed");
                }
            }
        }
        // TODO: consider parking the requests
        trace!(target: "ress::net", ?request, "No connections are available");
    }
}

impl Future for RessNetworkManager {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        loop {
            if let Poll::Ready(Some(event)) = this.protocol_events.poll_next_unpin(cx) {
                let ProtocolEvent::Established { direction, peer_id, to_connection } = event;
                info!(target: "ress::net", %peer_id, %direction, "Peer connection established");
                this.connections.push_back(ConnectionHandle { peer_id, to_connection });
                continue
            }

            if let Poll::Ready(Some(request)) = this.peer_requests.poll_next_unpin(cx) {
                this.on_peer_request(request);
                continue
            }

            return Poll::Pending
        }
    }
}
