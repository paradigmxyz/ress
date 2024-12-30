use alloy_primitives::{Address, B256};
use ress_subprotocol::{connection::CustomCommand, protocol::proto::StateWitness};
use reth::revm::db::BenchmarkDB;
use reth::revm::primitives::TxEnv;
use reth::revm::Evm;
use reth::rpc::types::engine::PayloadStatus;
use reth::{api::BeaconEngineMessage, revm::Database};
use reth_node_ethereum::EthEngineTypes;
use std::collections::HashMap;
use std::str::FromStr;
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
    /// head block hash
    pub head_block_hash: B256,
}

impl ConsensusEngine {
    pub fn new(
        beacon_rx: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
        network_peer_conn: UnboundedSender<CustomCommand>,
        head_block_hash: B256,
    ) -> Self {
        Self {
            beacon_rx,
            network_peer_conn,
            head_block_hash,
        }
    }

    pub async fn run(mut self) {
        while let Some(beacon_msg) = self.beacon_rx.recv().await {
            self.handle_beacon_message(beacon_msg).await;
        }
    }

    async fn handle_beacon_message(&mut self, msg: BeaconEngineMessage<EthEngineTypes>) {
        match msg {
            BeaconEngineMessage::NewPayload {
                payload: new_payload,
                sidecar: _,
                tx,
            } => {
                // step 1. determine what witness i need to get from the payload via retrievd block hash
                let block_hash = new_payload.block_hash();
                info!(
                    "received new payload, trying to get block hash: {:?}",
                    block_hash
                );

                // step2. request witness to stateful/stateless(?) peers
                // TODO: yet implemented subprotocol detail from getting state from stateful ( somehow using `TrieWitness::compute` )
                let state_witness: StateWitness = self.request_witness(block_hash).await;

                // step3. construct witness provider from retrieved witness
                // TODO: for initial start, i also need to get block hashes but if not i can just add the latest one
                let mut witness_provider = WitnessStateProvider::new(
                    state_witness,
                    HashMap::new(),
                    BytecodeProvider::new(self.network_peer_conn.clone()),
                );

                // request bytecode dynamically
                // TODO: not sure when and who call this bytecode
                let bytecode = witness_provider.code_by_hash(B256::random()).unwrap();
                info!("bytecode:{:?}", bytecode);

                // step 4. execute on EVM
                // TODO: somehow how to dump the contexts above
                let _evm = Evm::builder()
                    .with_db(BenchmarkDB::new_bytecode(bytecode))
                    .with_tx_env(TxEnv::default())
                    .build();

                // TODO: also need to do validation with stateroot from witness <> stateroot from payload
                let state_root_from_payload = new_payload.into_v1().state_root;
                println!("state_root_from_payload:{}", state_root_from_payload);
                // TODO: rn by calling `basic` I printing out stateroot from witness. Prob need to return value back somehow
                let _basic_account = witness_provider
                    .basic(Address::from_str("0xfef955f3c66c14d005d5dd719dc3c838eb5232be").unwrap())
                    .unwrap();

                // TODO: also seems somehow validate `tx_result`?
                //  let _tx_result = evm.transact().unwrap().result;
                //

                // after validation, update head hash?
                self.head_block_hash = block_hash;

                let _ = tx.send(Ok(PayloadStatus::from_status(
                    reth::rpc::types::engine::PayloadStatusEnum::Accepted,
                )));
            }
            BeaconEngineMessage::ForkchoiceUpdated {
                state,
                payload_attrs: _,
                version: _,
                tx: _,
            } => {
                // - validate head block hash to the latest block hash that ress saved
                let head_block_hash = state.head_block_hash;
                assert_eq!(head_block_hash, self.head_block_hash);
                // - `safe_block_hash` ?
                let _safe_block_hash = state.safe_block_hash;
                //  - update the `block_hashes` to the point until `finalized_block_hash`
                let _finalized_block_hash = state.finalized_block_hash;

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

        info!(target:"rlpx-subprotocol", ?response, "Witness received");
        response
    }
}
