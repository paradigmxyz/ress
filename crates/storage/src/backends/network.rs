use alloy_primitives::{Address, B256};
use ress_common::provider::ProviderWrapper;
use ress_subprotocol::{connection::CustomCommand, protocol::proto::StateWitness};
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

        // todo: for testing so we call provider. rpc, address, block number how i can link with code hash?
        let provider = ProviderWrapper::new("https://ethereum-rpc.publicnode.com".to_string());
        let bytes = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async { provider.get_code(Address::random(), None).await })
        });

        let bytecode = Bytecode::LegacyRaw(bytes.unwrap());
        info!(target:"network storage", ?response, "Bytecode received");
        Ok(Some(bytecode))
    }

    /// request to get StateWitness from block hash
    pub fn get_witness(&self, block_hash: B256) -> Result<StateWitness, StorageError> {
        info!(target:"network storage", "Request witness");
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.network_peer_conn
            .send(CustomCommand::Witness {
                block_hash,
                response: tx,
            })
            .unwrap();
        // todo: rn witness is dummy
        let response = tokio::task::block_in_place(|| rx.blocking_recv()).map_err(|e| {
            StorageError::Network(NetworkStorageError::ChannelReceive(e.to_string()))
        })?;

        info!(target:"network storage", ?response, "Witness received");
        Ok(response)
    }
}
