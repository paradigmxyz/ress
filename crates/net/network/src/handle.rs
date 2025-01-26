use alloy_primitives::{Bytes, B256};
use ress_protocol::{RessProtocolCommand, StateWitnessNet};
use reth_network::NetworkHandle;
use tokio::sync::{mpsc, mpsc::UnboundedSender, oneshot};
use tracing::trace;

/// Ress networking handle.
#[derive(Clone, Debug)]
pub struct RessNetworkHandle {
    /// Handle for interacting with the network.
    pub network_handle: NetworkHandle,
    /// Sender for forwarding network commands.
    pub network_peer_conn: UnboundedSender<RessProtocolCommand>,
}

impl RessNetworkHandle {
    /// Get contract bytecode by code hash.
    pub async fn fetch_bytecode(
        &self,
        code_hash: B256,
    ) -> Result<Option<Bytes>, NetworkStorageError> {
        trace!(target: "ress::network", %code_hash, "requesting bytecode");
        let (tx, rx) = oneshot::channel();
        self.network_peer_conn.send(RessProtocolCommand::Bytecode {
            code_hash,
            response: tx,
        })?;
        let response = rx.await?;
        trace!(target: "ress::network", "bytecode received");
        Ok(Some(response))
    }

    /// Get StateWitness from block hash
    pub async fn fetch_witness(
        &self,
        block_hash: B256,
    ) -> Result<StateWitnessNet, NetworkStorageError> {
        trace!(target: "ress::network", ?block_hash, "requesting witness");
        let (tx, rx) = oneshot::channel();
        self.network_peer_conn.send(RessProtocolCommand::Witness {
            block_hash,
            response: tx,
        })?;
        let response = rx.await?;
        trace!(target: "ress::network", "witness received");
        Ok(response)
    }
}

// TODO: rename
/// Errors that can occur during network storage operations.
#[derive(Debug, thiserror::Error)]
pub enum NetworkStorageError {
    /// Failed to send a request through the channel.
    #[error("Failed to send request through channel: {0}")]
    ChannelSend(#[from] mpsc::error::SendError<RessProtocolCommand>),

    /// Failed to receive a response from the channel.
    #[error("Failed to receive response from channel: {0}")]
    ChannelReceive(#[from] oneshot::error::RecvError),
}
