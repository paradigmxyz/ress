use std::sync::Arc;

use alloy_primitives::B256;
use ress_network::p2p::P2pHandler;
use ress_subprotocol::{connection::CustomCommand, protocol::proto::StateWitness};
use reth::revm::primitives::Bytecode;
use tracing::info;

use crate::errors::StorageError;

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
    fn get_account_code(&self, _code_hash: B256) -> Result<Option<Bytecode>, StorageError> {
        todo!("p2p call for code")
    }

    /// request to get StateWitness from block hash
    async fn get_witness(&self, block_hash: B256) -> Result<StateWitness, StorageError> {
        info!(target:"rlpx-subprotocol", "2️⃣ request witness");
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
