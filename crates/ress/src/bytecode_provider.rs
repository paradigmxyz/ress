use alloy_primitives::B256;
use ress_subprotocol::connection::CustomCommand;
use reth::revm::primitives::Bytecode;
use tokio::sync::{mpsc::UnboundedSender, oneshot};
use tracing::info;

pub trait BytecodeProviderTrait {
    type Error;

    async fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error>;
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

    // TODO: actually If I have `BytecodeProvider::code_by_hash` method to request subprotocol lazly,
    // TODO: this need to be async method. However the above `WitnessStateProvider` that implemented `Database` trait
    // TODO: which every method is defined in non-async. If this case :
    // TODO: A) turn `BytecodeProvider`'s method sync, when initialize bytecode provider it fetch everything save in disk and `code_by_hash` is just Disk get method(non-async)
    // TODO: B) make `WitnessStateProvider` not bound to `Database` trait.
    async fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        // Step 3. Request bytecode
        info!(target:"rlpx-subprotocol", "3️⃣ request bytecode");
        let (tx, rx) = oneshot::channel();
        self.network_peer_conn
            .send(CustomCommand::Bytecode {
                code_hash,
                response: tx,
            })
            .map_err(|e| BytecodeProviderError::ChannelSend(e.to_string()))?;

        let response = rx
            .await
            .map_err(|e| BytecodeProviderError::ChannelReceive(e.to_string()))?;

        // [mock]
        let bytecode: Bytecode = Bytecode::default();
        assert_eq!(response, bytecode);
        info!(target:"rlpx-subprotocol", ?response, "Bytecode received");
        Ok(bytecode)
    }
}
