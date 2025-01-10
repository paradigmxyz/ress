use alloy_primitives::B256;
use ress_primitives::witness::ExecutionWitness;
use ress_subprotocol::connection::CustomCommand;
use reth_revm::primitives::Bytecode;
use tokio::sync::mpsc::UnboundedSender;
use tracing::info;

use crate::errors::{NetworkStorageError, StorageError};

#[derive(Debug, Clone)]
pub struct NetworkStorage {
    network_peer_conn: UnboundedSender<CustomCommand>,
}

impl NetworkStorage {
    pub fn new(network_peer_conn: UnboundedSender<CustomCommand>) -> Self {
        Self { network_peer_conn }
    }

    /// fallbacked from disk
    pub fn get_account_code(&self, code_hash: B256) -> Result<Option<Bytecode>, StorageError> {
        info!(target:"network storage", "Request bytecode");
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.network_peer_conn
            .send(CustomCommand::Bytecode {
                code_hash,
                response: tx,
            })
            .map_err(|e| StorageError::Network(NetworkStorageError::ChannelSend(e.to_string())))?;

        let response = tokio::task::block_in_place(|| rx.blocking_recv()).map_err(|e| {
            StorageError::Network(NetworkStorageError::ChannelReceive(e.to_string()))
        })?;

        info!(target:"network storage", ?response, "Bytecode received");
        Ok(Some(response))
    }

    /// request to get StateWitness from block hash
    pub fn get_witness(&self, block_hash: B256) -> Result<ExecutionWitness, StorageError> {
        info!(target:"rlpx-subprotocol", "Request witness");
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.network_peer_conn
            .send(CustomCommand::Witness {
                block_hash,
                response: tx,
            })
            .unwrap();
        let response = tokio::task::block_in_place(|| rx.blocking_recv()).map_err(|e| {
            StorageError::Network(NetworkStorageError::ChannelReceive(e.to_string()))
        })?;

        info!(target:"network storage", "Witness received");
        Ok(response)
    }
}
