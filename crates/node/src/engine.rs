use alloy_primitives::b256;
use alloy_primitives::U256;
use alloy_provider::network::primitives::BlockTransactionsKind;
use alloy_provider::network::AnyNetwork;
use alloy_provider::Provider;
use alloy_provider::ProviderBuilder;
use alloy_rlp::Decodable;
use alloy_rpc_types_engine::ForkchoiceState;
use alloy_rpc_types_engine::PayloadStatus;
use alloy_rpc_types_engine::PayloadStatusEnum;
use alloy_trie::nodes::TrieNode;
use alloy_trie::TrieAccount;
use futures::StreamExt;
use jsonrpsee_http_client::HttpClientBuilder;
use ress_common::utils::get_witness_path;
use ress_primitives::witness::ExecutionWitness;
use ress_primitives::witness_rpc::ExecutionWitnessFromRpc;
use ress_provider::provider::RessProvider;
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
use tracing::warn;

use crate::errors::EngineError;

/// ress consensus engine
pub struct ConsensusEngine {
    pending_state: Option<Arc<PendingState>>,
    consensus: Arc<dyn FullConsensus<Error = ConsensusError>>,
    engine_validator: EthereumEngineValidator,
    provider: Arc<RessProvider>,
    from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
    forkchoice_state: Option<ForkchoiceState>,
}

pub struct PendingState {
    header: SealedHeader,
}

impl ConsensusEngine {
    pub fn new(
        chain_spec: &ChainSpec,
        provider: Arc<RessProvider>,
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
            provider,
            from_beacon_engine,
            forkchoice_state: None,
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
                let storage = self.provider.storage.clone();
                let block_hash = payload.block_hash();
                storage.set_block_hash(block_hash, payload.block_number());

                // ===================== Witness =====================

                // todo: we will get witness from fullnode connection later
                let client = HttpClientBuilder::default()
                    .max_response_size(50 * 1024 * 1024)
                    .request_timeout(Duration::from_secs(500))
                    .build(std::env::var("RPC_URL").expect("RPC_URL"))
                    .map_err(|e| EngineError::DebugApiClient(e.to_string()))?;
                let execution_witness =
                    DebugApiClient::debug_execution_witness(&client, payload.block_number().into())
                        .await
                        .map_err(|e| EngineError::DebugApiClient(e.to_string()))?;
                let json_data = serde_json::to_string(&ExecutionWitnessFromRpc::new(
                    execution_witness.state,
                    execution_witness.codes,
                ))?;
                std::fs::write(get_witness_path(block_hash), json_data)
                    .expect("Unable to write file");
                info!(?block_hash, "🟢 we got witness");

                // ===================== Validation =====================

                // todo: invalid_ancestors check

                let total_difficulty = U256::MAX;
                let parent_hash_from_payload = payload.parent_hash();
                // assert!(storage.is_canonical_hashes_exist(payload.block_number()));

                // ====
                // todo: i think we needed to have the headers ready before running
                let parent_header = match storage
                    .get_block_header_by_hash(parent_hash_from_payload)?
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

                // ===================== Witness =====================
                let execution_witness = self.provider.fetch_witness(block_hash).await?;
                self.prefetch_all_bytecodes(&execution_witness).await;
                let mut trie = SparseStateTrie::default().with_updates(true);
                trie.reveal_witness(state_root_of_parent, &execution_witness.state_witness)?;
                let db = WitnessDatabase::new(trie, storage.clone());

                // ===================== Execution =====================

                info!("start execution");
                let start_time = std::time::Instant::now();
                let mut block_executor = BlockExecutor::new(db, storage.clone());
                let senders = block.senders().expect("no senders");
                let block = BlockWithSenders::new(block.clone().unseal(), senders)
                    .expect("cannot construct block");
                let output = block_executor.execute(&block)?;
                info!(elapsed = ?start_time.elapsed(), "🎉 executed new payload");

                // ===================== Post Validation =====================

                self.consensus.validate_block_post_execution(
                    &block,
                    PostExecutionInput::new(&output.receipts, &output.requests),
                )?;

                // ===================== Update state =====================

                self.pending_state = Some(Arc::new(PendingState {
                    header: payload_header.clone(),
                }));

                let latest_valid_hash = match self.forkchoice_state {
                    Some(fcu_state) => fcu_state.head_block_hash,
                    None => parent_hash_from_payload,
                };

                info!(?latest_valid_hash, "🟢 new payload is valid");

                if let Err(e) = tx.send(Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid)
                    .with_latest_valid_hash(latest_valid_hash)))
                {
                    error!("Failed to send payload status: {:?}", e);
                }
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

                // todo: invalid_ancestors check

                // check head is same as pending state header from payload
                let pending_state = self
                    .pending_state
                    .clone()
                    .expect("must have pending state on fcu");
                assert_eq!(pending_state.header.hash_slow(), state.head_block_hash);

                let storage = &self.provider.storage;

                // check finalized bock hash and safe block hash all in storage
                assert!(storage.find_block_hash(state.finalized_block_hash));
                assert!(storage.find_block_hash(state.safe_block_hash));

                // payload attributes, version validation
                if let Some(attrs) = payload_attrs {
                    EngineValidator::<EthEngineTypes>::ensure_well_formed_attributes(
                        &self.engine_validator,
                        version,
                        &attrs,
                    )?;

                    EngineValidator::<EthEngineTypes>::validate_payload_attributes_against_header(
                        &self.engine_validator,
                        &attrs,
                        &pending_state.header,
                    )?;
                }

                if state.head_block_hash.is_zero() {
                    if let Err(e) = tx.send(Ok(OnForkChoiceUpdated::invalid_state())) {
                        error!("Failed to send invalid state: {:?}", e);
                    }
                } else {
                    storage.set_block(pending_state.header.clone().unseal());
                    storage.remove_oldest_block();
                    self.forkchoice_state = Some(state);

                    let witness_path = get_witness_path(state.head_block_hash);
                    // remove witness of the fcu
                    if let Err(e) = std::fs::remove_file(witness_path) {
                        warn!("Unable to remove witness file: {:?}", e);
                    }
                    if let Err(e) = tx.send(Ok(OnForkChoiceUpdated::valid(
                        PayloadStatus::from_status(PayloadStatusEnum::Valid)
                            .with_latest_valid_hash(state.head_block_hash),
                    ))) {
                        error!("Failed to send valid state: {:?}", e);
                    }
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

    /// prefetch all bytecodes in parallel
    pub async fn prefetch_all_bytecodes(&self, execution_witness: &ExecutionWitness) {
        let code_hashes: Vec<_> = execution_witness
            .state_witness
            .iter()
            .filter_map(|(_, encoded)| {
                let node = TrieNode::decode(&mut &encoded[..]).ok()?;
                let TrieNode::Leaf(leaf) = node else {
                    return None;
                };
                let account = TrieAccount::decode(&mut &leaf.value[..]).ok()?;
                // Skip "empty" code hashes
                if account.code_hash
                    == b256!("c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470")
                {
                    return None;
                }
                Some(account.code_hash)
            })
            .collect();

        let concurrency = 25;

        futures::stream::iter(code_hashes)
            .map(|code_hash| {
                let provider = self.provider.clone();
                async move {
                    if let Err(e) = provider.fetch_contract_bytecode(code_hash).await {
                        warn!("Failed to fetch codehash={:?}: {e}", code_hash);
                    }
                }
            })
            .buffer_unordered(concurrency)
            .for_each(|_| async {})
            .await;
    }
}
