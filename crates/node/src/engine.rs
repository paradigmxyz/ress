use std::sync::Arc;

use alloy_primitives::U256;
use alloy_rpc_types_engine::PayloadStatus;
use alloy_rpc_types_engine::PayloadStatusEnum;
use ress_storage::Storage;
use ress_vm::executor::BlockExecutor;
use reth_chainspec::ChainSpec;
use reth_consensus::Consensus;
use reth_consensus::FullConsensus;
use reth_consensus::HeaderValidator;
use reth_consensus::PostExecutionInput;
use reth_node_api::BeaconEngineMessage;
use reth_node_api::PayloadValidator;
use reth_node_ethereum::consensus::EthBeaconConsensus;
use reth_node_ethereum::node::EthereumEngineValidator;
use reth_node_ethereum::EthEngineTypes;
use reth_primitives::BlockWithSenders;
use reth_primitives::SealedBlock;
use reth_primitives::SealedHeader;
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
                payload,
                sidecar,
                tx,
            } => {
                // ===================== Validation =====================

                // q: total_difficulty
                let total_difficulty = self
                    .storage
                    .chain_spec
                    .get_final_paris_total_difficulty()
                    .unwrap();
                let parent_hash_from_payload = payload.parent_hash();
                let storage = self.storage.clone();
                let parent_header = storage
                    .get_block_header_by_hash(parent_hash_from_payload)
                    .unwrap()
                    .unwrap();
                // to retrieve `SealedBlock` object we using `ensure_well_formed_payload`
                // q. is there any other way to retrieve block object from payload without using payload validator?
                let block = self
                    .payload_validator
                    .ensure_well_formed_payload(payload, sidecar)
                    .unwrap();
                self.validate_header(&block, total_difficulty, parent_header);
                info!(target: "engine", "received valid new payload");

                // ===================== Execution =====================

                let mut block_executor = BlockExecutor::new(storage, parent_hash_from_payload);
                let senders = block.senders().unwrap();
                let block = BlockWithSenders {
                    block: block.unseal(),
                    senders,
                };
                let output = block_executor.execute(&block, total_difficulty).unwrap();

                // ===================== Post Validation =====================

                self.consensus
                    .validate_block_post_execution(
                        &block,
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

    /// validate new payload's block via consensus
    fn validate_header(
        &self,
        block: &SealedBlock,
        total_difficulty: U256,
        parent_header: SealedHeader,
    ) {
        if let Err(e) = self.consensus.validate_header(block) {
            error!(target: "engine", "Failed to validate header {}: {e}", block.header.hash());
        }
        if let Err(e) = self
            .consensus
            .validate_header_with_total_difficulty(block, total_difficulty)
        {
            error!(target: "engine", "Failed to validate header {} against totoal difficulty: {e}", block.header.hash());
        }
        if let Err(e) = self
            .consensus
            .validate_header_against_parent(block, &parent_header)
        {
            error!(target: "engine", "Failed to validate header {} against parent: {e}", block.header.hash());
        }
        if let Err(e) = self.consensus.validate_block_pre_execution(block) {
            error!(target: "engine", "Failed to pre vavalidate header {} : {e}", block.header.hash());
        }
    }
}
