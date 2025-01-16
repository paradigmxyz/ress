use alloy_primitives::B256;
use alloy_primitives::U256;
use alloy_provider::network::primitives::BlockTransactionsKind;
use alloy_provider::network::AnyNetwork;
use alloy_provider::Provider;
use alloy_provider::ProviderBuilder;
use alloy_rpc_types_engine::PayloadStatus;
use alloy_rpc_types_engine::PayloadStatusEnum;
use jsonrpsee_http_client::HttpClientBuilder;
use ress_common::constant::get_witness_path;
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
use reth_node_api::EngineValidator;
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
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::error;
use tracing::info;

use crate::errors::EngineError;

/// ress consensus engine
pub struct ConsensusEngine {
    pending_state: Option<Arc<PendingState>>,
    consensus: Arc<dyn FullConsensus<Error = ConsensusError>>,
    engine_validator: EthereumEngineValidator,
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
        let engine_validator = EthereumEngineValidator::new(chain_spec.clone().into());
        let consensus: Arc<dyn FullConsensus<Error = ConsensusError>> = Arc::new(
            EthBeaconConsensus::<ChainSpec>::new(chain_spec.clone().into()),
        );
        Self {
            pending_state: None,
            consensus,
            engine_validator,
            storage,
            from_beacon_engine,
            node_state: NodeState::default(),
        }
    }

    /// run engine to handle receiving consensus message.
    pub async fn run(mut self) {
        while let Some(beacon_msg) = self.from_beacon_engine.recv().await {
            self.handle_beacon_message(beacon_msg).await.unwrap();
        }
    }

    async fn handle_beacon_message(
        &mut self,
        msg: BeaconEngineMessage<EthEngineTypes>,
    ) -> Result<(), EngineError> {
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
                let storage = self.storage.clone();

                let block_hash = payload.block_hash();
                storage.set_block_hash(block_hash, payload.block_number());

                // ===================== Witness =====================

                // todo: we will get witness from fullnode connection later
                let client = HttpClientBuilder::default()
                    .max_response_size(50 * 1024 * 1024)
                    .request_timeout(Duration::from_secs(200))
                    .build(std::env::var("RPC_URL").expect("RPC_URL"))
                    .unwrap();
                let witness_from_rpc =
                    DebugApiClient::debug_execution_witness(&client, payload.block_number().into())
                        .await
                        .map_err(|e| EngineError::Submit(format!("{:?}", e)))?;
                let json_data = serde_json::to_string(&witness_from_rpc).unwrap();

                std::fs::write(get_witness_path(block_hash), json_data)
                    .expect("Unable to write file");
                info!(?block_hash, "🟢 we got witness");

                // ===================== Validation =====================

                let total_difficulty = U256::MAX;
                let parent_hash_from_payload = payload.parent_hash();

                assert!(storage.is_canonical_blocks_exist(payload.block_number()));

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
                    .engine_validator
                    .ensure_well_formed_payload(payload, sidecar)?;
                let payload_header = block.sealed_header();
                self.validate_header(&block, total_difficulty, parent_header);
                info!("🟢 new payload is valid");

                // ===================== Witness =====================

                let execution_witness = storage.get_witness(block_hash)?;
                let mut trie = SparseStateTrie::default().with_updates(true);
                trie.reveal_witness(state_root_of_parent, &execution_witness.state_witness)
                    .unwrap();
                let db = WitnessDatabase::new(trie, storage.clone());

                // ===================== Execution =====================
                info!("start execution");
                let start_time = std::time::Instant::now();
                let mut block_executor = BlockExecutor::new(db, storage);
                let senders = block.senders().expect("no senders");
                let block = BlockWithSenders::new(block.clone().unseal(), senders).unwrap();
                let output = block_executor.execute(&block)?;
                info!("end execution in {:?}", start_time.elapsed());

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

                let latest_valid_hash = self
                    .node_state
                    .head_block_hash
                    .unwrap_or(payload_header.parent_hash);

                info!(?latest_valid_hash, "🎉 executed new payload");

                tx.send(Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid)
                    .with_latest_valid_hash(latest_valid_hash)))
                    .map_err(|e| EngineError::Submit(format!("{:?}", e)))?;
            }
            BeaconEngineMessage::ForkchoiceUpdated {
                state,
                payload_attrs,
                version,
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
                // check head is same as pending state header from payload
                assert_eq!(pending_state.header.hash_slow(), state.head_block_hash);
                // check finalized bock hash and safe block hash all in storage
                assert!(self.storage.find_block_hash(state.finalized_block_hash));
                assert!(self.storage.find_block_hash(state.safe_block_hash));
                let header = pending_state.header.clone();

                if payload_attrs.is_some() {
                    let attrs = payload_attrs.expect("payload not none in this branch");
                    EngineValidator::<EthEngineTypes>::ensure_well_formed_attributes(
                        &self.engine_validator,
                        version,
                        &attrs,
                    )?;

                    EngineValidator::<EthEngineTypes>::validate_payload_attributes_against_header(
                        &self.engine_validator,
                        &attrs,
                        &header,
                    )?;
                }

                if state.head_block_hash.is_zero() {
                    tx.send(Ok(OnForkChoiceUpdated::invalid_state()))
                        .map_err(|e| EngineError::Submit(format!("{:?}", e)))?;
                } else {
                    self.storage.set_block(header.clone().unseal());
                    self.storage.remove_oldest_block();
                    self.node_state.head_block_hash = Some(state.head_block_hash);
                    self.node_state.safe_block_hash = Some(state.safe_block_hash);
                    self.node_state.finalized_block_hash = Some(state.finalized_block_hash);

                    tx.send(Ok(OnForkChoiceUpdated::valid(
                        PayloadStatus::from_status(PayloadStatusEnum::Valid)
                            .with_latest_valid_hash(state.head_block_hash),
                    )))
                    .map_err(|e| EngineError::Submit(format!("{:?}", e)))?;
                }
            }
            BeaconEngineMessage::TransitionConfigurationExchanged => {
                // Implement transition configuration handling
                todo!()
            }
        }

        Ok(())
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
