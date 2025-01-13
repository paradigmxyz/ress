use alloy_primitives::B256;
use alloy_primitives::U256;
use alloy_rpc_types_engine::PayloadStatus;
use alloy_rpc_types_engine::PayloadStatusEnum;
use jsonrpsee_http_client::HttpClientBuilder;
use ress_common::constant::WITNESS_PATH;
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
use reth_consensus_debug_client::BlockProvider;
use reth_consensus_debug_client::RpcBlockProvider;
use reth_node_api::BeaconEngineMessage;
use reth_node_api::PayloadValidator;
use reth_node_ethereum::consensus::EthBeaconConsensus;
use reth_node_ethereum::node::EthereumEngineValidator;
use reth_node_ethereum::EthEngineTypes;
use reth_primitives::BlockWithSenders;
use reth_primitives::SealedBlock;
use reth_primitives_traits::SealedHeader;
use reth_rpc_api::DebugApiClient;
use std::sync::Arc;
use tracing::debug;

use reth_trie_sparse::SparseStateTrie;
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
    node_state: NodeState,
}

#[derive(Default)]
pub struct NodeState {
    /// Hash of the head block last set by fork choice update
    head_block_hash: Option<B256>,
    /// Hash of the safe block last set by fork choice update
    safe_block_hash: Option<B256>,
    /// Hash of finalized block last set by fork choice update
    finalized_block_hash: Option<B256>,
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
            node_state: NodeState::default(),
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
                info!("New payload: {:?}", payload.block_number());
                let block_hash = payload.block_hash();

                // ===================== Witness =====================

                //todo: temporary
                let client = HttpClientBuilder::default()
                    .max_response_size(50 * 1024 * 1024)
                    .build(std::env::var("RPC_URL").expect("RPC_URL"))
                    .unwrap();
                let witness_from_rpc =
                    DebugApiClient::debug_execution_witness(&client, payload.block_number().into())
                        .await
                        .unwrap();
                let json_data = serde_json::to_string(&witness_from_rpc).unwrap();
                std::fs::write(WITNESS_PATH, json_data).expect("Unable to write file");
                info!("fetched node witness");

                // ===================== Validation =====================

                let total_difficulty = U256::MAX;
                let parent_hash_from_payload = payload.parent_hash();

                let storage = self.storage.clone();
                debug!(
                    "request parent_hash_from_payload:{}",
                    parent_hash_from_payload
                );

                // ====
                // todo: i think we needed to have the headers ready before running
                let parent_header = match storage
                    .get_block_header_by_hash(parent_hash_from_payload)
                    .unwrap()
                {
                    Some(header) => header,
                    None => {
                        info!("parent header not found, fetching..");
                        // temp with `block_provider`
                        let ws_rpc_url =
                            std::env::var("WS_RPC_URL").expect("need `WS_RPC_URL` env");
                        let block_provider = RpcBlockProvider::new(ws_rpc_url);
                        let header = &block_provider
                            .get_block(payload.block_number() - 1)
                            .await
                            .unwrap()
                            .header;
                        storage.set_block(header.hash, header.clone().into_consensus());
                        SealedHeader::new(header.clone().into_consensus(), parent_hash_from_payload)
                    }
                };

                // let storage_hash = storage.memory.get_blockhashes();
                // info!("hash stored:{:?}", storage_hash);
                // ====

                let state_root_of_parent = parent_header.state_root;
                let block = self
                    .payload_validator
                    .ensure_well_formed_payload(payload, sidecar)
                    .unwrap();
                self.validate_header(&block, total_difficulty, parent_header);
                info!(target: "engine", "received valid new payload");

                // ===================== Witness =====================

                let execution_witness = storage.get_witness(block_hash).unwrap();
                let mut trie = SparseStateTrie::default().with_updates(true);
                trie.reveal_witness(state_root_of_parent, &execution_witness.state_witness)
                    .unwrap();
                let db = WitnessDatabase::new(trie, storage.clone());

                // ===================== Execution =====================

                let mut block_executor = BlockExecutor::new(db, storage);
                let senders = block.senders().unwrap();
                let block = BlockWithSenders::new(block.unseal(), senders).unwrap();
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

                info!("🎉 lgtm");

                tx.send(Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid)))
                    .unwrap();
            }
            BeaconEngineMessage::ForkchoiceUpdated {
                state,
                payload_attrs: _,
                version: _,
                tx: _,
            } => {
                info!(
                    "New fork choice head: {:#x}, safe: {:#x}, finalized: {:#x}.",
                    state.head_block_hash, state.safe_block_hash, state.finalized_block_hash
                );

                // TODO: smth like EngineApiTreeHandler.on_forkchoice_updated validation is needed. Should i just leverage EngineApiTreeHandler struct

                let safe_block_hash = state.safe_block_hash;
                let head_block_hash = state.head_block_hash;
                let finalized_block_hash = state.finalized_block_hash;

                self.node_state.head_block_hash = Some(head_block_hash);
                self.node_state.safe_block_hash = Some(safe_block_hash);
                self.node_state.finalized_block_hash = Some(finalized_block_hash);
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
