use std::sync::Arc;

use alloy_primitives::U256;
use alloy_rpc_types_engine::PayloadStatus;
use alloy_rpc_types_engine::PayloadStatusEnum;
use ress_primitives::witness::ExecutionWitness;
use ress_storage::Storage;
use ress_vm::db::WitnessDatabase;
use ress_vm::executor::BlockExecutor;
use reth_chainspec::ChainSpec;
use reth_consensus::Consensus;
use reth_consensus::ConsensusError;
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
use reth_primitives_traits::SealedHeader;

use reth_trie_sparse::SparseStateTrie;
use reth_trie_sparse::SparseTrie;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::error;
use tracing::info;

/// ress consensus engine
pub struct ConsensusEngine {
    witness: ExecutionWitness,
    consensus: Arc<dyn FullConsensus<Error = ConsensusError>>,
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
        let consensus: Arc<dyn FullConsensus<Error = ConsensusError>> = Arc::new(
            EthBeaconConsensus::<ChainSpec>::new(chain_spec.clone().into()),
        );
        Self {
            witness: ExecutionWitness::default(),
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
                let block_hash = payload.block_hash();
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

                // ===================== Witness =====================

                // fetch new witness
                let execution_witness = storage.get_witness(block_hash).unwrap();
                let execution_witness_block_hashes = execution_witness.clone().block_hashes;
                let state_root = block.state_root;
                let mut trie = SparseStateTrie::from_state(SparseTrie::blind());
                trie.reveal_witness(state_root, execution_witness.state_witness.clone())
                    .unwrap();
                storage.overwrite_block_hashes(execution_witness_block_hashes);
                let db = WitnessDatabase::new(trie, storage.clone());

                // ===================== Execution =====================

                let mut block_executor = BlockExecutor::new(db, storage);
                let senders = block.senders().unwrap();
                let block = BlockWithSenders {
                    block: block.unseal(),
                    senders,
                };
                let output = block_executor.execute(&block).unwrap();

                // ===================== Post Validation =====================

                self.consensus
                    .validate_block_post_execution(
                        &block,
                        PostExecutionInput::new(&output.receipts, &output.requests),
                    )
                    .unwrap();

                // ===================== Update state =====================
                self.witness = execution_witness;

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
        let header = block.sealed_header();
        if let Err(e) = self.consensus.validate_header(header) {
            error!(target: "engine", "Failed to validate header: {e}");
        }
        if let Err(e) = self
            .consensus
            .validate_header_with_total_difficulty(header, total_difficulty)
        {
            error!(target: "engine", "Failed to validate header against totoal difficulty: {e}");
        }
        if let Err(e) = self
            .consensus
            .validate_header_against_parent(header, &parent_header)
        {
            error!(target: "engine", "Failed to validate header against parent: {e}");
        }
        if let Err(e) = self.consensus.validate_block_pre_execution(block) {
            error!(target: "engine", "Failed to pre vavalidate header : {e}");
        }
    }
}
