use std::sync::Arc;

use alloy_rpc_types_engine::PayloadStatus;
use alloy_rpc_types_engine::PayloadStatusEnum;
use ress_storage::Storage;
use ress_vm::executor::BlockExecutor;
use reth_beacon_consensus::EthBeaconConsensus;
use reth_chainspec::ChainSpec;
use reth_consensus::Consensus;
use reth_consensus::FullConsensus;
use reth_consensus::HeaderValidator;
use reth_consensus::PostExecutionInput;
use reth_node_api::BeaconEngineMessage;
use reth_node_api::PayloadValidator;
use reth_node_ethereum::node::EthereumEngineValidator;
use reth_node_ethereum::EthEngineTypes;
use reth_primitives::Block;
use reth_primitives::BlockWithSenders;
use reth_primitives::TransactionSigned;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::error;
use tracing::info;

/// ress consensus engine
pub struct ConsensusEngine {
    consensus: Arc<dyn FullConsensus>,
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
        let consensus: Arc<dyn FullConsensus> =
            Arc::new(EthBeaconConsensus::new(chain_spec.clone().into()));
        Self {
            consensus,
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
                // ===================== Additional Validation =====================
                // basic standalone payload validation is handled from AuthServer's `EthereumEngineValidator` inside there `ExecutionPayloadValidator`
                // additionally we need to verify new payload against parent header from our storeage

                let parent_hash_from_payload = new_payload.parent_hash();
                let block_number_from_payload = new_payload.block_number();
                let storage = self.storage.clone();

                let parent_header = storage
                    .get_block_header_by_hash(parent_hash_from_payload)
                    .unwrap()
                    .unwrap();

                // to retrieve `SealedBlock` object we using `ensure_well_formed_payload`
                let block = self
                    .payload_validator
                    .ensure_well_formed_payload(new_payload, sidecar)
                    .unwrap();

                if let Err(e) = self
                    .consensus
                    .validate_header_with_total_difficulty(&block, alloy_primitives::U256::MAX)
                {
                    error!(target: "engine", "Failed to validate header {} against totoal difficulty: {e}", block.header.hash());
                }

                if let Err(e) = self
                    .consensus
                    .validate_header_against_parent(&block, &parent_header)
                {
                    error!(target: "engine", "Failed to validate header {} against parent: {e}", block.header.hash());
                }

                if let Err(e) = self.consensus.validate_block_pre_execution(&block) {
                    error!(target: "engine", "Failed to pre vavalidate header {} : {e}", block.header.hash());
                }

                info!(target: "engine",
                    "received valid new payload on block number: {:?}",
                    block_number_from_payload
                );

                // ===================== Execution =====================

                let mut block_executor = BlockExecutor::new(storage, parent_hash_from_payload);
                let output = block_executor.execute(&block).unwrap();
                let senders = block.senders().unwrap();
                let block: Block<TransactionSigned> = block.unseal();
                let unsealed_block: BlockWithSenders<Block> = BlockWithSenders { block, senders };

                // ===================== Post Validation, Execution =====================

                // todo: rn error
                self.consensus
                    .validate_block_post_execution(
                        &unsealed_block,
                        PostExecutionInput::new(&output.receipts, &output.requests),
                    )
                    .unwrap();

                tx.send(Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid)))
                    .unwrap();
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
