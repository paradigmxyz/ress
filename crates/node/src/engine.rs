use alloy_primitives::B256;
use ress_network::p2p::P2pHandler;
use ress_network::rpc::RpcHandler;
use ress_storage::Store;
use ress_subprotocol::connection::CustomCommand;
use ress_subprotocol::protocol::proto::StateWitness;
use ress_vm::vm::execute_block;
use ress_vm::vm::EvmState;
use reth::api::BeaconEngineMessage;
use reth::api::PayloadValidator;
use reth::beacon_consensus::EthBeaconConsensus;
use reth::chainspec::ChainSpec;
use reth::consensus::Consensus;
use reth::consensus::FullConsensus;
use reth::consensus::HeaderValidator;
use reth::consensus::PostExecutionInput;
use reth::primitives::Block;
use reth::primitives::BlockWithSenders;
use reth::primitives::TransactionSigned;
use reth::revm::db::BenchmarkDB;
use reth::revm::primitives::TxEnv;
use reth::revm::Evm;
use reth::rpc::types::engine::PayloadStatus;
use reth_node_ethereum::node::EthereumEngineValidator;
use reth_node_ethereum::EthEngineTypes;
use tracing::error;
use tracing::warn;

use std::collections::HashMap;

use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::Mutex;
use tracing::info;

/// ress consensus engine
/// ### `BeaconEngineMessage::NewPayload`
/// - determine required witness
pub struct ConsensusEngine {
    eth_beacon_consensus: EthBeaconConsensus<ChainSpec>,
    payload_validator: EthereumEngineValidator,
    p2p_handler: Arc<P2pHandler>,
    from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
}

impl ConsensusEngine {
    pub fn new(
        chain_spec: &ChainSpec,
        p2p_handler: Arc<P2pHandler>,
        from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
    ) -> Self {
        // TODO: first initial start dump block hashes head ~ 256 blocks: from rpc?
        // let mut block_hashes = HashMap::new();
        // block_hashes.insert(
        //     43334,
        //     B256::from_str("0xa5ddd3f286f429458a39cafc13ffe89295a7efa8eb363cf89a1a4887dbcf272b")
        //         .unwrap(),
        // );
        // we have it in auth server for now to leaverage the mothods in here, we also init new validator
        let payload_validator = EthereumEngineValidator::new(chain_spec.clone().into());
        let eth_beacon_consensus = EthBeaconConsensus::new(chain_spec.clone().into());
        Self {
            eth_beacon_consensus,
            payload_validator,
            p2p_handler,
            from_beacon_engine,
        }
    }

    /// run engine to handle receiving consensus message.
    pub async fn run(mut self) {
        while let Some(beacon_msg) = self.from_beacon_engine.recv().await {
            self.handle_beacon_message(beacon_msg).await;
        }
    }

    async fn handle_beacon_message(&mut self, msg: BeaconEngineMessage<EthEngineTypes>) {
        match msg {
            BeaconEngineMessage::NewPayload {
                payload: new_payload,
                sidecar,
                tx,
            } => {
                // ===================== Validation =====================

                // note: basic payload validation is cared from AuthServer's `EthereumEngineValidator` inside there `ExecutionPayloadValidator`
                let block_hash_from_payload = new_payload.block_hash();
                let parent_hash_from_payload = new_payload.parent_hash();
                let block_number_from_payload = new_payload.block_number();
                // initiate state with parent hash
                let store = Store::new(&self.p2p_handler);
                let parent_header = store
                    .get_block_header_by_hash(parent_hash_from_payload)
                    .unwrap()
                    .unwrap();

                // to retrieve `SealedBlock` object we using `ensure_well_formed_payload`
                let block = self
                    .payload_validator
                    .ensure_well_formed_payload(new_payload, sidecar)
                    .unwrap();

                if let Err(e) = self
                    .eth_beacon_consensus
                    .validate_header_against_parent(&block, &parent_header)
                {
                    warn!(target: "engine::tree", ?block, "Failed to validate header {} against parent: {e}", block.header.hash());
                }

                info!(
                    "received new payload, block hash: {:?} on block number :{:?}",
                    block_hash_from_payload, block_number_from_payload
                );

                // ===================== Execution =====================

                let mut evm_state = EvmState::new(store, parent_hash_from_payload);
                let output = execute_block(&block, &mut evm_state).unwrap();
                let senders = block.senders().unwrap();
                let block: reth::primitives::Block<TransactionSigned> = block.unseal();
                let unsealed_block: BlockWithSenders<Block> = BlockWithSenders { block, senders };

                // ===================== Post Validation, Execution =====================

                // todo: rn error
                // let _ = self
                //     .eth_beacon_consensus
                //     .validate_block_post_execution(
                //         &unsealed_block,
                //         PostExecutionInput::new(&output.receipts, &output.requests),
                //     )
                //     .unwrap();

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
                // let mut witness_provider = self.witness_provider.lock().await;
                // witness_provider.update_block_hashes(new_head_hash, finalized_block_hash);
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
        self.p2p_handler
            .network_peer_conn
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
