use std::str::FromStr;

use alloy_primitives::{Bytes, B256};
use ress_subprotocol::connection::CustomCommand;
use reth::revm::primitives::Bytecode;
use tokio::sync::{mpsc::UnboundedSender, oneshot};
use tracing::info;

pub trait BytecodeProviderTrait {
    type Error;

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error>;
}

/// BytecodeProvider error type.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum BytecodeProviderError {
    #[error("Failed to send request through channel: {0}")]
    ChannelSend(String),

    #[error("Failed to receive response from channel: {0}")]
    ChannelReceive(String),
}

pub struct BytecodeProvider {
    /// channel to send - network
    pub network_peer_conn: UnboundedSender<CustomCommand>,
}

impl BytecodeProvider {
    pub fn new(network_peer_conn: UnboundedSender<CustomCommand>) -> Self {
        Self { network_peer_conn }
    }
}

impl BytecodeProviderTrait for BytecodeProvider {
    type Error = BytecodeProviderError;

    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        // TODO: in the future this should first look up on disk and then fall back to network if missing

        // Step 3. Request bytecode
        info!(target:"rlpx-subprotocol", "3️⃣ request bytecode");
        let (tx, rx) = oneshot::channel();
        self.network_peer_conn
            .send(CustomCommand::Bytecode {
                code_hash,
                response: tx,
            })
            .map_err(|e| BytecodeProviderError::ChannelSend(e.to_string()))?;

        let response = tokio::task::block_in_place(|| rx.blocking_recv())
            .map_err(|e| BytecodeProviderError::ChannelReceive(e.to_string()))?;

        // [mock]
        let bytecode: Bytecode = Bytecode::LegacyRaw(Bytes::from_str("0xabcd").unwrap());
        // TODO: somehow this diff type
        assert_eq!(response, bytecode);
        info!(target:"rlpx-subprotocol", ?response, "Bytecode received");
        Ok(bytecode)
    }
}
