use std::sync::Arc;

use alloy_rpc_types_engine::PayloadStatus;
use alloy_rpc_types_engine::PayloadStatusEnum;
use ress_storage::Storage;
use ress_vm::executor::BlockExecutor;
use reth_beacon_consensus::EthBeaconConsensus;
use reth_chainspec::ChainSpec;
use reth_consensus::HeaderValidator;
use reth_node_api::BeaconEngineMessage;
use reth_node_api::PayloadValidator;
use reth_node_ethereum::node::EthereumEngineValidator;
use reth_node_ethereum::EthEngineTypes;
use reth_primitives::Block;
use reth_primitives::BlockWithSenders;
use reth_primitives::TransactionSigned;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::info;
use tracing::warn;

/// ress consensus engine
/// ### `BeaconEngineMessage::NewPayload`
/// - determine required witness
pub struct ConsensusEngine {
    eth_beacon_consensus: EthBeaconConsensus<ChainSpec>,
    payload_validator: EthereumEngineValidator,
    storage: Arc<Storage>,
    from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
}

impl ConsensusEngine {
    pub fn new(
        chain_spec: &ChainSpec,
        storage: Arc<Storage>,
        from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
    ) -> Self {
        // we have it in auth server for now to leaverage the mothods in here, we also init new validator
        let payload_validator = EthereumEngineValidator::new(chain_spec.clone().into());
        let eth_beacon_consensus = EthBeaconConsensus::new(chain_spec.clone().into());
        Self {
            eth_beacon_consensus,
            payload_validator,
            storage,
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
                let storage = self.storage.clone();

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

                if let Err(e) = self
                    .eth_beacon_consensus
                    .validate_header_against_parent(&block, &parent_header)
                {
                    warn!(target: "engine::tree", "Failed to validate header {} against parent: {e}", block.header.hash());
                }

                info!(
                    "received new payload, block hash: {:?} on block number :{:?}",
                    block_hash_from_payload, block_number_from_payload
                );

                // ===================== Execution =====================

                // testing purpose for bytecode
                // let bytescode = storage.get_account_code(B256::random());
                // info!("received bytecode:{:?}", bytescode);

                let mut block_executor = BlockExecutor::new(storage, parent_hash_from_payload);
                let _output = block_executor.execute(&block).unwrap();
                let senders = block.senders().unwrap();
                let block: Block<TransactionSigned> = block.unseal();
                let _unsealed_block: BlockWithSenders<Block> = BlockWithSenders { block, senders };

                // ===================== Post Validation, Execution =====================

                // todo: rn error
                // let _ = self
                //     .eth_beacon_consensus
                //     .validate_block_post_execution(
                //         &unsealed_block,
                //         PostExecutionInput::new(&output.receipts, &output.requests),
                //     )
                //     .unwrap();

                let _ = tx.send(Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid)));
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
                // let new_head_hash = state.head_block_hash;
                // let finalized_block_hash = state.finalized_block_hash;

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
