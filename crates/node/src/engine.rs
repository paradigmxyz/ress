use alloy_primitives::B256;
use alloy_primitives::U256;
use alloy_provider::network::primitives::BlockTransactionsKind;
use alloy_provider::network::AnyNetwork;
use alloy_provider::Provider;
use alloy_provider::ProviderBuilder;
use alloy_rpc_types_engine::PayloadStatus;
use alloy_rpc_types_engine::PayloadStatusEnum;
use jsonrpsee_http_client::HttpClientBuilder;
use ress_common::constant::WITNESS_PATH;
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
use reth_node_api::OnForkChoiceUpdated;
use reth_node_api::PayloadValidator;
use reth_node_ethereum::consensus::EthBeaconConsensus;
use reth_node_ethereum::node::EthereumEngineValidator;
use reth_node_ethereum::EthEngineTypes;
use reth_primitives::BlockWithSenders;
use reth_primitives::SealedBlock;
use reth_primitives_traits::SealedHeader;
use reth_rpc_api::DebugApiClient;
use reth_trie_sparse::SparseStateTrie;
use std::result::Result::Ok;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::error;
use tracing::info;

/// ress consensus engine
pub struct ConsensusEngine {
    pending_state: Option<Arc<PendingState>>,
    consensus: Arc<dyn FullConsensus<Error = ConsensusError>>,
    payload_validator: EthereumEngineValidator,
    storage: Arc<Storage>,
    from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
    node_state: NodeState,
}

pub struct PendingState {
    header: SealedHeader,
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
            pending_state: None,
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
                info!(
                    "👋 new payload: {:?}, fetching witness...",
                    payload.block_number()
                );
                let block_hash = payload.block_hash();

                // ===================== Witness =====================

                // todo: we will get witness from fullnode connection later
                let client = HttpClientBuilder::default()
                    .max_response_size(50 * 1024 * 1024)
                    .build(std::env::var("RPC_URL").expect("RPC_URL"))
                    .unwrap();
                let witness_from_rpc =
                    DebugApiClient::debug_execution_witness(&client, payload.block_number().into())
                        .await
                        .unwrap();
                // todo: we just overwrite witness with latest data, which as we running 2 nodes on demo this cus issue
                let json_data = serde_json::to_string(&witness_from_rpc).unwrap();
                std::fs::write(WITNESS_PATH, json_data).expect("Unable to write file");
                info!("🟢 we got witness");

                // ===================== Validation =====================

                let total_difficulty = U256::MAX;
                let parent_hash_from_payload = payload.parent_hash();
                let storage = self.storage.clone();

                // ====
                // todo: i think we needed to have the headers ready before running
                let parent_header = match storage
                    .get_block_header_by_hash(parent_hash_from_payload)
                    .unwrap()
                {
                    Some(header) => header,
                    None => {
                        // this should not happen actually
                        info!("‼ parent header not found, fetching..");
                        let rpc_block_provider = ProviderBuilder::new()
                            .network::<AnyNetwork>()
                            .on_http(std::env::var("RPC_URL").expect("need rpc").parse().unwrap());
                        let block = &rpc_block_provider
                            .get_block_by_number(
                                (payload.block_number() - 1).into(),
                                BlockTransactionsKind::Hashes,
                            )
                            .await
                            .unwrap()
                            .unwrap();
                        let block_header = block
                            .header
                            .clone()
                            .into_consensus()
                            .into_header_with_defaults();
                        storage.set_block_hash(block_header.hash_slow(), block_header.number);
                        SealedHeader::new(block_header, parent_hash_from_payload)
                    }
                };

                let state_root_of_parent = parent_header.state_root;
                let block = self
                    .payload_validator
                    .ensure_well_formed_payload(payload, sidecar)
                    .unwrap();
                let payload_header = block.sealed_header();
                self.validate_header(&block, total_difficulty, parent_header);
                info!("🟢 new payload is valid");

                // ===================== Witness =====================

                let execution_witness = storage.get_witness(block_hash).unwrap();
                let mut trie = SparseStateTrie::default().with_updates(true);
                trie.reveal_witness(state_root_of_parent, &execution_witness.state_witness)
                    .unwrap();
                let db = WitnessDatabase::new(trie, storage.clone());

                // ===================== Execution =====================

                let mut block_executor = BlockExecutor::new(db, storage);
                let senders = block.senders().unwrap();
                let block = BlockWithSenders::new(block.clone().unseal(), senders).unwrap();
                let output = block_executor.execute(&block).unwrap();

                // ===================== Post Validation =====================

                self.consensus
                    .validate_block_post_execution(
                        &block,
                        PostExecutionInput::new(&output.receipts, &output.requests),
                    )
                    .unwrap();

                // ===================== Update state =====================
                self.pending_state = Some(Arc::new(PendingState {
                    header: payload_header.clone(),
                }));

                info!("🎉 executed new payload successfully");

                tx.send(Ok(PayloadStatus::new(
                    PayloadStatusEnum::Valid,
                    self.node_state.head_block_hash,
                )))
                .unwrap();
            }
            BeaconEngineMessage::ForkchoiceUpdated {
                state,
                payload_attrs: _,
                version: _,
                tx,
            } => {
                info!(
                    "👋 new fork choice | head: {:#x}, safe: {:#x}, finalized: {:#x}.",
                    state.head_block_hash, state.safe_block_hash, state.finalized_block_hash
                );

                let pending_state = self
                    .pending_state
                    .clone()
                    .expect("must have pending state on fcu");

                self.storage
                    .set_block(pending_state.header.clone().unseal());
                self.storage.remove_oldest_block();

                // TODO: smth like EngineApiTreeHandler.on_forkchoice_updated validation is needed. Should i just leverage EngineApiTreeHandler struct

                let safe_block_hash = state.safe_block_hash;
                let head_block_hash = state.head_block_hash;
                let finalized_block_hash = state.finalized_block_hash;

                self.node_state.head_block_hash = Some(head_block_hash);
                self.node_state.safe_block_hash = Some(safe_block_hash);
                self.node_state.finalized_block_hash = Some(finalized_block_hash);

                tx.send(Ok(OnForkChoiceUpdated::valid(PayloadStatus::from_status(
                    PayloadStatusEnum::Valid,
                ))))
                .unwrap();
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
            error!("Failed to validate header: {e}");
        }
        if let Err(e) = self
            .consensus
            .validate_header_with_total_difficulty(header, total_difficulty)
        {
            error!("Failed to validate header against totoal difficulty: {e}");
        }
        if let Err(e) = self
            .consensus
            .validate_header_against_parent(header, &parent_header)
        {
            error!("Failed to validate header against parent: {e}");
        }
        if let Err(e) = self.consensus.validate_block_pre_execution(block) {
            error!("Failed to pre vavalidate header : {e}");
        }
    }
}
