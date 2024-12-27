use std::collections::HashMap;

use alloy_primitives::B256;
use ress_subprotocol::{connection::CustomCommand, protocol::proto::StateWitness};
use reth::rpc::types::engine::PayloadStatus;
use reth::{api::BeaconEngineMessage, revm::Database};
use reth_node_ethereum::EthEngineTypes;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::info;

use crate::{bytecode_provider::BytecodeProvider, witness_provider::WitnessStateProvider};

/// ress consensus engine
/// ### `BeaconEngineMessage::NewPayload`
/// - determine required witness
pub struct ConsensusEngine {
    beacon_rx: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
    /// channel to send - network
    pub network_peer_conn: UnboundedSender<CustomCommand>,
}

impl ConsensusEngine {
    pub fn new(
        beacon_rx: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
        network_peer_conn: UnboundedSender<CustomCommand>,
    ) -> Self {
        Self {
            beacon_rx,
            network_peer_conn,
        }
    }

    pub async fn run(mut self) {
        while let Some(beacon_msg) = self.beacon_rx.recv().await {
            self.handle_beacon_message(beacon_msg).await;
        }
    }

    async fn handle_beacon_message(&self, msg: BeaconEngineMessage<EthEngineTypes>) {
        match msg {
            BeaconEngineMessage::NewPayload {
                payload: new_payload,
                sidecar: _,
                tx,
            } => {
                // step1. determine what witness i need to get from the payload/ i think it's blocknumber?
                info!("received new payload: {:?}", new_payload);
                // get this from payload i guess
                let block_hash = B256::random();

                // step2. request witness to stateful/stateless(?) peers
                let state_witness: StateWitness = self.request_witness(block_hash).await;

                // step3. construct witness provider from retirved witness
                let mut witness_provider = WitnessStateProvider::new(
                    state_witness,
                    HashMap::new(),
                    BytecodeProvider::new(self.network_peer_conn.clone()),
                );

                // request bytecode dynamically
                let bytecode = witness_provider.code_by_hash(B256::random()).unwrap();
                info!("bytecode:{:?}", bytecode);

                let _ = tx.send(Ok(PayloadStatus::from_status(
                    reth::rpc::types::engine::PayloadStatusEnum::Accepted,
                )));
            }
            BeaconEngineMessage::ForkchoiceUpdated {
                state: _,
                payload_attrs: _,
                version: _,
                tx: _,
            } => {
                // Implement forkchoice updated handling
                todo!()
            }
            BeaconEngineMessage::TransitionConfigurationExchanged => {
                // Implement transition configuration handling
                todo!()
            }
        }
    }

    async fn request_witness(&self, block_hash: B256) -> StateWitness {
        info!(target:"rlpx-subprotocol", "2️⃣ request witness");
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.network_peer_conn
            .send(CustomCommand::Witness {
                block_hash,
                response: tx,
            })
            .unwrap();
        let response = rx.await.unwrap();
        // [mock]
        let mut state_witness = StateWitness::default();
        state_witness.insert(B256::ZERO, [0x00].into());
        assert_eq!(response, state_witness);
        info!(target:"rlpx-subprotocol", ?response, "Witness received");
        state_witness
    }
}
