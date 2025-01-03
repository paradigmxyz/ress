use alloy_primitives::BlockNumber;
use alloy_primitives::B256;
use ress_network::p2p::P2pHandler;
use ress_storage::Storage;
use ress_subprotocol::connection::CustomCommand;
use ress_vm::executor::BlockExecutor;
use reth::api::BeaconEngineMessage;
use reth::api::PayloadValidator;
use reth::beacon_consensus::EthBeaconConsensus;
use reth::chainspec::ChainSpec;
use reth::consensus::HeaderValidator;
use reth::primitives::Block;
use reth::primitives::BlockWithSenders;
use reth::primitives::TransactionSigned;
use reth::rpc::types::engine::PayloadStatus;
use reth_node_ethereum::node::EthereumEngineValidator;
use reth_node_ethereum::EthEngineTypes;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tracing::info;
use tracing::warn;

/// ress consensus engine
/// ### `BeaconEngineMessage::NewPayload`
/// - determine required witness
pub struct ConsensusEngine {
    finalized_block_number: Option<BlockNumber>,
    safe_block_number: Option<BlockNumber>,
    eth_beacon_consensus: EthBeaconConsensus<ChainSpec>,
    payload_validator: EthereumEngineValidator,
    network_peer_conn: UnboundedSender<CustomCommand>,
    from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
}

impl ConsensusEngine {
    pub fn new(
        chain_spec: &ChainSpec,
        network_peer_conn: UnboundedSender<CustomCommand>,
        from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
    ) -> Self {
        // we have it in auth server for now to leaverage the mothods in here, we also init new validator
        let payload_validator = EthereumEngineValidator::new(chain_spec.clone().into());
        let eth_beacon_consensus = EthBeaconConsensus::new(chain_spec.clone().into());
        Self {
            finalized_block_number: None,
            safe_block_number: None,
            eth_beacon_consensus,
            payload_validator,
            network_peer_conn,
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
                info!("gm new payload");

                //  basic standalone payload validation is handled from AuthServer's `EthereumEngineValidator` inside there `ExecutionPayloadValidator`
                // ===================== Additional Validation =====================
                // additionally we need to verify new payload against parent header from our storeage

                let block_hash_from_payload = new_payload.block_hash();
                let parent_hash_from_payload = new_payload.parent_hash();
                let block_number_from_payload = new_payload.block_number();

                // initiate state with parent hash
                let storage = Storage::new(self.network_peer_conn.clone());
                let parent_header = storage
                    .get_block_header_by_hash(parent_hash_from_payload)
                    .unwrap()
                    .unwrap();

                // todo: current payload from script error on ensure_well_formed_payload
                // to retrieve `SealedBlock` object we using `ensure_well_formed_payload`
                let block = self
                    .payload_validator
                    .ensure_well_formed_payload(new_payload, sidecar)
                    .unwrap();

                info!("hi block had well formed");

                // if let Err(e) = self
                //     .eth_beacon_consensus
                //     .validate_header_against_parent(&block, &parent_header)
                // {
                //     warn!(target: "engine::tree", ?block, "Failed to validate header {} against parent: {e}", block.header.hash());
                // }

                info!(
                    "received new payload, block hash: {:?} on block number :{:?}",
                    block_hash_from_payload, block_number_from_payload
                );

                // ===================== Execution =====================

                // testing purpose for bytecode
                let bytescode = storage.get_account_code(B256::random());
                info!("received bytecode:{:?}", bytescode);

                let mut block_executor = BlockExecutor::new(storage, parent_hash_from_payload);
                let output = block_executor.execute(&block).unwrap();
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
                // self.finalized_block_number = Some(finalized_block_hash);
            }
            BeaconEngineMessage::TransitionConfigurationExchanged => {
                // Implement transition configuration handling
                todo!()
            }
        }
    }
}
