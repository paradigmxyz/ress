use alloy_primitives::{Bytes, B256};
use ress_protocol::{RessPeerRequest, StateWitnessNet};
use reth_network::NetworkHandle;
use reth_primitives::{BlockBody, Header};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tracing::trace;

/// Ress networking handle.
#[derive(Clone, Debug)]
pub struct RessNetworkHandle {
    /// Handle for interacting with the network.
    network_handle: NetworkHandle,
    /// Sender for forwarding network commands.
    connection: mpsc::UnboundedSender<RessPeerRequest>,
}

impl RessNetworkHandle {
    /// Create new network handle from reth's handle and peer connection.
    pub fn new(
        network_handle: NetworkHandle,
        connection: mpsc::UnboundedSender<RessPeerRequest>,
    ) -> Self {
        Self { network_handle, connection }
    }

    /// Return reference to reth's network handle.
    pub fn inner_handle(&self) -> &NetworkHandle {
        &self.network_handle
    }

    fn send_request(&self, request: RessPeerRequest) -> Result<(), NetworkError> {
        self.connection.send(request).map_err(|_| NetworkError::ConnectionClosed)
    }
}

impl RessNetworkHandle {
    /// Get header by block hash.
    pub async fn fetch_header(&self, block_hash: B256) -> Result<Header, NetworkError> {
        trace!(target: "ress::net", %block_hash, "requesting header");
        let (tx, rx) = oneshot::channel();
        self.send_request(RessPeerRequest::GetHeader { block_hash, tx })?;
        let response = rx.await.map_err(|_| NetworkError::RequestDropped)?;
        trace!(target: "ress::net", %block_hash, "header received");
        Ok(response)
    }

    /// Get block body by block hash.
    pub async fn fetch_block_body(&self, block_hash: B256) -> Result<BlockBody, NetworkError> {
        trace!(target: "ress::net", %block_hash, "requesting block body");
        let (tx, rx) = oneshot::channel();
        self.send_request(RessPeerRequest::GetBlockBody { block_hash, tx })?;
        let response = rx.await.map_err(|_| NetworkError::RequestDropped)?;
        trace!(target: "ress::net", %block_hash, "block body received");
        Ok(response)
    }

    /// Get contract bytecode by code hash.
    pub async fn fetch_bytecode(&self, code_hash: B256) -> Result<Bytes, NetworkError> {
        trace!(target: "ress::net", %code_hash, "requesting bytecode");
        let (tx, rx) = oneshot::channel();
        self.send_request(RessPeerRequest::GetBytecode { code_hash, tx })?;
        let response = rx.await.map_err(|_| NetworkError::RequestDropped)?;
        trace!(target: "ress::net", %code_hash, "bytecode received");
        Ok(response)
    }

    /// Get StateWitness from block hash
    pub async fn fetch_witness(&self, block_hash: B256) -> Result<StateWitnessNet, NetworkError> {
        trace!(target: "ress::net", %block_hash, "requesting witness");
        let (tx, rx) = oneshot::channel();
        self.send_request(RessPeerRequest::GetWitness { block_hash, tx })?;
        let response = rx.await.map_err(|_| NetworkError::RequestDropped)?;
        trace!(target: "ress::net", %block_hash, "witness received");
        Ok(response)
    }
}

/// Peer request errors.
#[derive(Debug, Error)]
pub enum NetworkError {
    /// Request dropped.
    #[error("Peer request dropped")]
    RequestDropped,

    /// Connection closed.
    #[error("Peer connection was closed")]
    ConnectionClosed,
}
