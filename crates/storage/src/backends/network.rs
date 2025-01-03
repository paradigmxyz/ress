use alloy_primitives::B256;
use ress_network::p2p::P2pHandler;
use ress_subprotocol::{connection::CustomCommand, protocol::proto::StateWitness};
use reth::revm::primitives::Bytecode;
use std::sync::Arc;
use tracing::info;

use crate::errors::{NetworkStorageError, StorageError};

pub struct NetworkStorage {
    p2p_handler: Arc<P2pHandler>,
}

impl NetworkStorage {
    pub fn new(p2p_handler: &Arc<P2pHandler>) -> Self {
        Self {
            p2p_handler: p2p_handler.clone(),
        }
    }

    /// fallbacked from disk
    pub fn get_account_code(&self, code_hash: B256) -> Result<Option<Bytecode>, StorageError> {
        info!(target:"rlpx-subprotocol", "request bytecode");
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.p2p_handler
            .network_peer_conn
            .send(CustomCommand::Bytecode {
                code_hash,
                response: tx,
            })
            .map_err(|e| StorageError::Network(NetworkStorageError::ChannelSend(e.to_string())))?;

        let response = tokio::task::block_in_place(|| rx.blocking_recv()).map_err(|e| {
            StorageError::Network(NetworkStorageError::ChannelReceive(e.to_string()))
        })?;

        info!(target:"rlpx-subprotocol", ?response, "Bytecode received");
        Ok(Some(response))
    }

    /// request to get StateWitness from block hash
    pub async fn get_witness(&self, block_hash: B256) -> Result<StateWitness, StorageError> {
        info!(target:"rlpx-subprotocol", "request witness");
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.p2p_handler
            .network_peer_conn
            .send(CustomCommand::Witness {
                block_hash,
                response: tx,
            })
            .unwrap();
        let response = rx.await.unwrap();

        info!(target:"rlpx-subprotocol", ?response, "Witness received");
        Ok(response)
    }
}
