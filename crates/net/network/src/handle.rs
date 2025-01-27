use alloy_primitives::{Bytes, B256};
use ress_protocol::{RessPeerRequest, StateWitnessNet};
use reth_network::NetworkHandle;
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
    /// Get contract bytecode by code hash.
    pub async fn fetch_bytecode(&self, code_hash: B256) -> Result<Option<Bytes>, RessNetworkError> {
        trace!(target: "ress::net", %code_hash, "requesting bytecode");
        let (tx, rx) = oneshot::channel();
        self.network_peer_conn
            .send(RessPeerRequest::GetBytecode { code_hash, tx })?;
        let response = rx.await?;
        trace!(target: "ress::net", "bytecode received");
        Ok(Some(response))
    }

    /// Get StateWitness from block hash
    pub async fn fetch_witness(
        &self,
        block_hash: B256,
    ) -> Result<StateWitnessNet, RessNetworkError> {
        trace!(target: "ress::net", %block_hash, "requesting witness");
        let (tx, rx) = oneshot::channel();
        self.network_peer_conn
            .send(RessPeerRequest::GetWitness { block_hash, tx })?;
        let response = rx.await?;
        trace!(target: "ress::net", "witness received");
        Ok(response)
    }
}

/// Errors that can occur during ress network
#[derive(Debug, thiserror::Error)]
pub enum RessNetworkError {
    /// Failed to send a request through the channel.
    #[error("Failed to send request through channel: {0}")]
    ChannelSend(#[from] mpsc::error::SendError<RessPeerRequest>),

    /// Failed to receive a response from the channel.
    #[error("Failed to receive response from channel: {0}")]
    ChannelReceive(#[from] oneshot::error::RecvError),
}
