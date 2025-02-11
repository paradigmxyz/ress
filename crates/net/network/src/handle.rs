use alloy_primitives::{Bytes, B256};
use ress_protocol::{RessPeerRequest, StateWitnessNet};
use reth_network::NetworkHandle;
use reth_primitives::{BlockBody, Header};
use tokio::sync::{mpsc, mpsc::UnboundedSender, oneshot};
use tracing::trace;

/// Ress networking handle.
#[derive(Clone, Debug)]
pub struct RessNetworkHandle {
    /// Handle for interacting with the network.
    pub network_handle: NetworkHandle,
    /// Sender for forwarding network commands.
    pub network_peer_conn: UnboundedSender<RessPeerRequest>,
}

impl RessNetworkHandle {
    /// Get header by block hash.
    pub async fn fetch_header(&self, block_hash: B256) -> Result<Header, NetworkStorageError> {
        trace!(target: "ress::net", %block_hash, "requesting header");
        let (tx, rx) = oneshot::channel();
        self.network_peer_conn.send(RessPeerRequest::GetHeader { block_hash, tx })?;
        let response = rx.await?;
        trace!(target: "ress::net", %block_hash, "header received");
        Ok(response)
    }

    /// Get block body by block hash.
    pub async fn fetch_block_body(
        &self,
        block_hash: B256,
    ) -> Result<BlockBody, NetworkStorageError> {
        trace!(target: "ress::net", %block_hash, "requesting block body");
        let (tx, rx) = oneshot::channel();
        self.network_peer_conn.send(RessPeerRequest::GetBlockBody { block_hash, tx })?;
        let response = rx.await?;
        trace!(target: "ress::net", %block_hash, "block body received");
        Ok(response)
    }

    /// Get contract bytecode by code hash.
    pub async fn fetch_bytecode(&self, code_hash: B256) -> Result<Bytes, NetworkStorageError> {
        trace!(target: "ress::net", %code_hash, "requesting bytecode");
        let (tx, rx) = oneshot::channel();
        self.network_peer_conn.send(RessPeerRequest::GetBytecode { code_hash, tx })?;
        let response = rx.await?;
        trace!(target: "ress::net", %code_hash, "bytecode received");
        Ok(response)
    }

    /// Get StateWitness from block hash
    pub async fn fetch_witness(
        &self,
        block_hash: B256,
    ) -> Result<StateWitnessNet, NetworkStorageError> {
        trace!(target: "ress::net", %block_hash, "requesting witness");
        let (tx, rx) = oneshot::channel();
        self.network_peer_conn.send(RessPeerRequest::GetWitness { block_hash, tx })?;
        let response = rx.await?;
        trace!(target: "ress::net", %block_hash, "witness received");
        Ok(response)
    }
}

// TODO: rename
/// Errors that can occur during network storage operations.
#[derive(Debug, thiserror::Error)]
pub enum NetworkStorageError {
    /// Failed to send a request through the channel.
    #[error("Failed to send request through channel: {0}")]
    ChannelSend(#[from] mpsc::error::SendError<RessPeerRequest>),

    /// Failed to receive a response from the channel.
    #[error("Failed to receive response from channel: {0}")]
    ChannelReceive(#[from] oneshot::error::RecvError),
}

/// Error type for download operations.
#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    /// Returned when the maximum number of download retry attempts has been exceeded.
    #[error("Too many retries")]
    TooManyRetries,
    /// Returned when a network-related error occurs during download.
    #[error("Network error: {0}")]
    Network(NetworkStorageError),
}
