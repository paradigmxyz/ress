use crate::{bytecode_provider::BytecodeProvider, witness_provider::WitnessStateProvider};
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
use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::Mutex;
use tracing::info;

/// ress consensus engine
/// ### `BeaconEngineMessage::NewPayload`
/// - determine required witness
pub struct ConsensusEngine {
    beacon_rx: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
    network_peer_conn: UnboundedSender<CustomCommand>,
    /// head block hash
    pub witness_provider: Arc<Mutex<WitnessStateProvider<BytecodeProvider>>>,
}

impl ConsensusEngine {
    pub fn new(
        beacon_rx: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
        network_peer_conn: UnboundedSender<CustomCommand>,
    ) -> Self {
        // TODO: first initial start dump block hashes head ~ 256 blocks: from rpc?
        let mut block_hashes = HashMap::new();
        block_hashes.insert(
            43334,
            B256::from_str("0xa5ddd3f286f429458a39cafc13ffe89295a7efa8eb363cf89a1a4887dbcf272b")
                .unwrap(),
        );
        Self {
            beacon_rx,
            witness_provider: Arc::new(Mutex::new(WitnessStateProvider::new(
                StateWitness::default(),
                block_hashes,
                BytecodeProvider::new(network_peer_conn.clone()),
            ))),
            network_peer_conn,
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
                // ===================== Validation =====================
                // Step 1: Check the "head block" from the witness_provider
                {
                    let witness_provider = self.witness_provider.lock().await;
                    let (head_block_number, head_block_hash) = witness_provider.get_head_block();

                    // Basic payload validation from previous FCU update of block N, etc.
                    let block_hash_from_payload = new_payload.block_hash();
                    let block_number_from_payload = new_payload.block_number();
                    assert_eq!(head_block_hash, block_hash_from_payload);
                    assert_eq!(head_block_number, block_number_from_payload);

                    info!(
                        "received new payload, block hash: {:?} on block number :{:?}",
                        block_hash_from_payload, block_number_from_payload
                    );
                }

                // ===================== Preparing witness (stateless only) =====================

                // step2. request witness to stateful/stateless(?) peers
                // TODO: yet implemented subprotocol detail from getting state from stateful ( somehow using `TrieWitness::compute` )
                let state_witness: StateWitness =
                    self.request_witness(new_payload.block_hash()).await;
                {
                    // Step 3: Update the witness_provider with the new state witness
                    let mut witness_provider = self.witness_provider.lock().await;
                    witness_provider.update_state_witness(state_witness);
                }

                // ===================== Execution =====================

                // request bytecode dynamically
                // TODO: not sure when and who call this bytecode
                {
                    let random_code_hash = B256::random();
                    let bytecode = {
                        let mut witness_provider = self.witness_provider.lock().await;
                        witness_provider.code_by_hash(random_code_hash).unwrap()
                    };

                    // Now create EVM purely in this block
                    let evm = Evm::builder()
                        .with_db(BenchmarkDB::new_bytecode(bytecode))
                        .with_tx_env(TxEnv::default())
                        .build();

                    // do immediate EVM usage or calls (synchronously in the sense of no .await)
                    let _ = evm; // whatever logic
                }

                // TODO: also need to do validation with stateroot from witness <> stateroot from payload
                let state_root_from_payload = new_payload.into_v1().state_root;
                println!("state_root_from_payload:{}", state_root_from_payload);
                // TODO: rn by calling `basic` I printing out stateroot from witness. Prob need to return value back somehow
                let _basic_account = {
                    let mut witness_provider = self.witness_provider.lock().await;
                    witness_provider
                        .basic(
                            Address::from_str("0xfef955f3c66c14d005d5dd719dc3c838eb5232be")
                                .unwrap(),
                        )
                        .unwrap()
                };

                // TODO: also seems somehow validate `tx_result`?
                //  let _tx_result = evm.transact().unwrap().result;
                //

                // ===================== Post Validation, Execution =====================

                let _ = tx.send(Ok(PayloadStatus::from_status(
                    reth::rpc::types::engine::PayloadStatusEnum::Valid,
                )));
            }
            BeaconEngineMessage::ForkchoiceUpdated {
                state,
                payload_attrs: _,
                version: _,
                tx: _,
            } => {
                // `safe_block_hash` ?
                let _safe_block_hash = state.safe_block_hash;

                // N + 1 hash
                let new_head_hash = state.head_block_hash;
                let finalized_block_hash = state.finalized_block_hash;

                // FCU msg update head block + also clean up block hashes stored in memeory up to finalized block
                let mut witness_provider = self.witness_provider.lock().await;
                witness_provider.update_block_hashes(new_head_hash, finalized_block_hash);
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
