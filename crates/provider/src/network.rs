use crate::errors::NetworkStorageError;
use alloy_primitives::{BlockHash, B256};
use ress_primitives::witness::ExecutionWitness;
use ress_subprotocol::connection::CustomCommand;
use reth_revm::primitives::Bytecode;
use tokio::sync::mpsc::UnboundedSender;
use tracing::debug;

/// Provider for interacting with the network.
#[derive(Debug)]
pub struct NetworkProvider {
    network_peer_conn: UnboundedSender<CustomCommand>,
}

impl NetworkProvider {
    /// Create new network provider.
    pub fn new(network_peer_conn: UnboundedSender<CustomCommand>) -> Self {
        Self { network_peer_conn }
    }

    /// Get contract bytecode from given block hash and codehash
    pub(crate) async fn get_contract_bytecode(
        &self,
        block_hash: BlockHash,
        code_hash: B256,
    ) -> Result<Option<Bytecode>, NetworkStorageError> {
        debug!(target:"network storage", ?block_hash,?code_hash, "Request bytecode");
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.network_peer_conn.send(CustomCommand::Bytecode {
            block_hash,
            code_hash,
            response: tx,
        })?;
        let response = rx.await?;
        debug!(target:"network storage", "Bytecode received");
        Ok(response)
    }

    /// Get StateWitness from block hash
    pub(crate) async fn get_witness(
        &self,
        block_hash: B256,
    ) -> Result<ExecutionWitness, NetworkStorageError> {
        debug!(target:"network storage", ?block_hash, "Request witness");
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.network_peer_conn.send(CustomCommand::Witness {
            block_hash,
            response: tx,
        })?;
        let response = rx.await?;
        debug!(target:"network storage", "Witness received");
        Ok(response)
    }
}
