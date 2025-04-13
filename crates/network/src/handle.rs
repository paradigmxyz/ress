use alloy_primitives::B256;
use reth_network::NetworkHandle;
use reth_primitives::{BlockBody, Header};
use reth_ress_protocol::{GetHeaders, RLPExecutionWitness, RessPeerRequest};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tracing::trace;

/// Ress networking handle.
#[derive(Clone, Debug)]
pub struct RessNetworkHandle {
    /// Handle for interacting with the network.
    network_handle: NetworkHandle,
    /// Sender for forwarding peer requests.
    peer_requests_sender: mpsc::UnboundedSender<RessPeerRequest>,
}

impl RessNetworkHandle {
    /// Create new network handle from reth's handle and peer connection.
    pub fn new(
        network_handle: NetworkHandle,
        peer_requests_sender: mpsc::UnboundedSender<RessPeerRequest>,
    ) -> Self {
        Self { network_handle, peer_requests_sender }
    }

    /// Return reference to reth's network handle.
    pub fn inner(&self) -> &NetworkHandle {
        &self.network_handle
    }

    fn send_request(&self, request: RessPeerRequest) -> Result<(), PeerRequestError> {
        self.peer_requests_sender.send(request).map_err(|_| PeerRequestError::ConnectionClosed)
    }
}

impl RessNetworkHandle {
    /// Get block headers.
    pub async fn fetch_headers(
        &self,
        request: GetHeaders,
    ) -> Result<Vec<Header>, PeerRequestError> {
        trace!(target: "ress::net", ?request, "requesting header");
        let (tx, rx) = oneshot::channel();
        self.send_request(RessPeerRequest::GetHeaders { request, tx })?;
        let response = rx.await.map_err(|_| PeerRequestError::RequestDropped)?;
        trace!(target: "ress::net", ?request, "headers received");
        Ok(response)
    }

    /// Get block bodies.
    pub async fn fetch_block_bodies(
        &self,
        request: Vec<B256>,
    ) -> Result<Vec<BlockBody>, PeerRequestError> {
        trace!(target: "ress::net", ?request, "requesting block bodies");
        let (tx, rx) = oneshot::channel();
        self.send_request(RessPeerRequest::GetBlockBodies { request: request.clone(), tx })?;
        let response = rx.await.map_err(|_| PeerRequestError::RequestDropped)?;
        trace!(target: "ress::net", ?request, "block bodies received");
        Ok(response)
    }

    /// Get ExecutionWitness for block hash
    pub async fn fetch_witness(
        &self,
        block_hash: B256,
    ) -> Result<RLPExecutionWitness, PeerRequestError> {
        trace!(target: "ress::net", %block_hash, "requesting witness");
        let (tx, rx) = oneshot::channel();
        self.send_request(RessPeerRequest::GetWitness { block_hash, tx })?;
        let response = rx.await.map_err(|_| PeerRequestError::RequestDropped)?;
        trace!(target: "ress::net", %block_hash, "witness received");
        Ok(response)
    }
}

/// Peer request errors.
#[derive(Debug, Error)]
pub enum PeerRequestError {
    /// Request dropped.
    #[error("Peer request dropped")]
    RequestDropped,

    /// Connection closed.
    #[error("Peer connection was closed")]
    ConnectionClosed,
}
