use crate::tree::EngineTree;
use alloy_primitives::B256;
use ress_network::RessNetworkHandle;
use ress_primitives::witness::{ExecutionWitness, StateWitness};
use ress_provider::provider::RessProvider;
use reth_chainspec::ChainSpec;
use reth_ethereum_engine_primitives::EthereumEngineValidator;
use reth_node_api::{BeaconEngineMessage, BeaconOnNewPayloadError, ExecutionPayload};
use reth_node_ethereum::{consensus::EthBeaconConsensus, EthEngineTypes};
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::*;

/// Ress consensus engine.
#[allow(missing_debug_implementations)]
pub struct ConsensusEngine {
    tree: EngineTree,
    network: RessNetworkHandle,
    from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
}

impl ConsensusEngine {
    /// Initialize consensus engine.
    pub fn new(
        provider: RessProvider,
        consensus: EthBeaconConsensus<ChainSpec>,
        engine_validator: EthereumEngineValidator,
        network: RessNetworkHandle,
        from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
    ) -> Self {
        let tree = EngineTree::new(provider, consensus, engine_validator);
        Self { from_beacon_engine, network, tree }
    }

    /// Fetch witness of target block hash
    async fn fetch_witness(&self, block_hash: B256) -> ExecutionWitness {
        let state_witness = self.network.fetch_witness(block_hash).await.unwrap(); // TODO:
        ExecutionWitness::new(StateWitness::from_iter(
            state_witness.0.into_iter().map(|e| (e.hash, e.bytes)),
        ))
    }

    async fn prefetch_bytecodes(&self, witness: &ExecutionWitness) {
        let start_time = std::time::Instant::now();
        let bytecode_hashes = witness.get_bytecode_hashes();
        let bytecode_hashes_len = bytecode_hashes.len();
        for code_hash in bytecode_hashes {
            if let Err(error) = self.tree.provider.ensure_bytecode_exists(code_hash).await {
                // TODO: handle this error
                error!(target: "ress::engine", %error, "Failed to prefetch");
            }
        }
        info!(target: "ress::engine", elapsed = ?start_time.elapsed(), len = bytecode_hashes_len, "✨ ensured all bytecodes are present");
    }

    async fn on_engine_message(&mut self, message: BeaconEngineMessage<EthEngineTypes>) {
        match message {
            BeaconEngineMessage::NewPayload { payload, tx } => {
                // TODO: fetch witness and bytecodes only after partially validating the payload
                let witness = self.fetch_witness(payload.block_hash()).await;
                self.prefetch_bytecodes(&witness).await;
                let outcome = self.tree.on_new_payload(payload, witness);
                if let Err(error) = tx.send(outcome.map_err(BeaconOnNewPayloadError::internal)) {
                    error!(target: "ress::engine", ?error, "Failed to send payload status");
                }
            }
            BeaconEngineMessage::ForkchoiceUpdated { state, payload_attrs, tx, version } => {
                let outcome = self.tree.on_forkchoice_updated(state, payload_attrs, version);
                if let Err(error) = tx.send(outcome) {
                    error!(target: "ress::engine", ?error, "Failed to send forkchoice outcome");
                }
            }
            BeaconEngineMessage::TransitionConfigurationExchanged => {
                warn!(target: "ress::engine", "Received unsupported `TransitionConfigurationExchanged` message");
            }
        }
    }

    /// run engine to handle receiving consensus message.
    pub async fn run(mut self) {
        while let Some(beacon_msg) = self.from_beacon_engine.recv().await {
            self.on_engine_message(beacon_msg).await;
        }
    }
}
