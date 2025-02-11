use crate::{RessNetworkHandle, RessNetworkManager};
use ress_protocol::{
    NodeType, ProtocolEvent, ProtocolState, RessProtocolHandler, RessProtocolProvider,
};
use reth_chainspec::ChainSpec;
use reth_network::{
    config::SecretKey, protocol::IntoRlpxSubProtocol, EthNetworkPrimitives, NetworkConfig,
    NetworkManager,
};
use reth_network_peers::TrustedPeer;
use std::{net::SocketAddr, sync::Arc};
use tokio::sync::mpsc::{self, UnboundedReceiver};
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
        secret_key: SecretKey,
        address: SocketAddr,
        remote_peer: Option<TrustedPeer>,
    ) -> RessNetworkHandle {
        let (subnetwork_handle, protocol_events) =
            self.launch_subprotocol_network(secret_key, address).await;

        if let Some(remote_peer) = remote_peer {
            let remote_id = remote_peer.id;
            let remote_addr = remote_peer.resolve_blocking().expect("peer").tcp_addr();
            subnetwork_handle.peers_handle().add_peer(remote_id, remote_addr);
        }

        // get a handle to the network to interact with it
        let network_handle = subnetwork_handle.handle().clone();
        // spawn the network
        tokio::spawn(subnetwork_handle);

        let (peer_requests_tx, peer_requests_rx) = mpsc::unbounded_channel();
        // spawn ress network manager
        tokio::spawn(RessNetworkManager::new(
            UnboundedReceiverStream::from(protocol_events),
            UnboundedReceiverStream::from(peer_requests_rx),
        ));
        RessNetworkHandle::new(network_handle, peer_requests_tx)
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
}
