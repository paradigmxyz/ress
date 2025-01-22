use alloy_primitives::keccak256;
use alloy_primitives::B256;
use alloy_primitives::U256;
use alloy_rlp::Decodable;
use alloy_rpc_types_engine::ForkchoiceState;
use alloy_rpc_types_engine::PayloadStatus;
use alloy_rpc_types_engine::PayloadStatusEnum;
use alloy_trie::nodes::TrieNode;
use alloy_trie::Nibbles;
use alloy_trie::TrieAccount;
use alloy_trie::KECCAK_EMPTY;
use jsonrpsee_http_client::HttpClientBuilder;
use rayon::iter::ParallelBridge;
use rayon::iter::ParallelIterator;
use ress_common::utils::get_witness_path;
use ress_primitives::witness::ExecutionWitness;
use ress_primitives::witness_rpc::ExecutionWitnessFromRpc;
use ress_provider::errors::StorageError;
use ress_provider::provider::RessProvider;
use ress_vm::db::WitnessDatabase;
use ress_vm::executor::BlockExecutor;
use reth_chainspec::ChainSpec;
use reth_consensus::Consensus;
use reth_consensus::ConsensusError;
use reth_consensus::FullConsensus;
use reth_consensus::HeaderValidator;
use reth_consensus::PostExecutionInput;
use reth_errors::RethError;
use reth_errors::RethResult;
use reth_node_api::BeaconEngineMessage;
use reth_node_api::EngineValidator;
use reth_node_api::OnForkChoiceUpdated;
use reth_node_api::PayloadTypes;
use reth_node_api::PayloadValidator;
use reth_node_ethereum::consensus::EthBeaconConsensus;
use reth_node_ethereum::node::EthereumEngineValidator;
use reth_node_ethereum::EthEngineTypes;
use reth_primitives::BlockWithSenders;
use reth_primitives::SealedBlock;
use reth_primitives_traits::SealedHeader;
use reth_rpc_api::DebugApiClient;
use reth_trie_sparse::errors::SparseStateTrieResult;
use reth_trie_sparse::errors::SparseTrieErrorKind;
use reth_trie_sparse::SparseStateTrie;
use std::result::Result::Ok;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::debug;
use tracing::error;
use tracing::info;
use tracing::warn;

use crate::errors::EngineError;

/// ress consensus engine
pub struct ConsensusEngine {
    consensus: Arc<dyn FullConsensus<Error = ConsensusError>>,
    engine_validator: EthereumEngineValidator,
    provider: Arc<RessProvider>,
    from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
    forkchoice_state: Option<ForkchoiceState>,
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
                let block_number = payload.block_number();
                let block_hash = payload.block_hash();
                info!(?block_hash, ?block_number, "👋 new payload");
                let storage = self.provider.storage.clone();

                // ===================== Witness =====================

                // todo: we will get witness from fullnode connection later
                let start_time = std::time::Instant::now();
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
                info!(elapsed = ?start_time.elapsed(), "🟢 fetched witness");

                // ===================== Validation =====================

                // todo: invalid_ancestors check
                let total_difficulty = U256::MAX;
                let parent_hash_from_payload = payload.parent_hash();
                if !storage.is_canonical(parent_hash_from_payload) {
                    warn!(?parent_hash_from_payload, "not in canonical");
                }

                // ====

                let parent_header: SealedHeader = SealedHeader::new(
                    storage
                        .get_executed_header_by_hash(parent_hash_from_payload)
                        .unwrap_or_else(|| {
                            panic!("should have parent header: {}", parent_hash_from_payload)
                        }),
                    parent_hash_from_payload,
                );
                let state_root_of_parent = parent_header.state_root;
                let block = self
                    .engine_validator
                    .ensure_well_formed_payload(payload, sidecar)?;
                self.validate_header(&block, total_difficulty, parent_header);

                // ===================== Witness =====================
                let execution_witness = self.provider.fetch_witness(block_hash).await?;
                let start_time = std::time::Instant::now();
                self.prefetch_all_bytecodes(&execution_witness, block_hash)
                    .await;
                info!(elapsed = ?start_time.elapsed(), "✨ prefetched all bytes");
                let mut trie = SparseStateTrie::default().with_updates(true);
                trie.reveal_witness(state_root_of_parent, &execution_witness.state_witness)?;
                let db = WitnessDatabase::new(trie, storage.clone());

                // ===================== Execution =====================

                let start_time = std::time::Instant::now();
                let mut block_executor = BlockExecutor::new(db, storage.clone());
                let senders = block.senders().expect("no senders");
                let block = BlockWithSenders::new(block.clone().unseal(), senders)
                    .expect("cannot construct block");
                let output = block_executor.execute(&block)?;
                info!(elapsed = ?start_time.elapsed(), "🎉 executed new payload");

                let mut trie = SparseStateTrie::default();
                trie.reveal_witness(state_root_of_parent, &execution_witness.state_witness)?;

                // ===================== Update trie =====================
                let state = output.state;

                // Update storage slots with new values and calculate storage roots.
                let (storage_tx, storage_rx) = mpsc::channel();
                state
                    .state()
                    .into_iter()
                    .map(|(address, bundle_account)| {
                        (
                            address,
                            bundle_account,
                            trie.take_storage_trie(&keccak256(address)),
                        )
                    })
                    .par_bridge()
                    .map(|(address, bundle_account, storage_trie)| {
                        let mut storage_trie = storage_trie.ok_or(SparseTrieErrorKind::Blind)?;

                        if bundle_account.was_destroyed() {
                            storage_trie.wipe()?;
                        }
                        for (slot, value) in &bundle_account.storage {
                            let slot_nibbles = Nibbles::unpack(&slot.to_be_bytes_vec());
                            if value.present_value.is_zero() {
                                storage_trie.remove_leaf(&slot_nibbles)?;
                            } else {
                                storage_trie.update_leaf(
                                    slot_nibbles,
                                    alloy_rlp::encode_fixed_size(&value.present_value).to_vec(),
                                )?;
                            }
                        }

                        storage_trie.root();

                        SparseStateTrieResult::Ok((address, storage_trie))
                    })
                    .for_each_init(
                        || storage_tx.clone(),
                        |storage_tx, result| storage_tx.send(result).unwrap(),
                    );
                drop(storage_tx);

                for result in storage_rx {
                    let (address, storage_trie) = result?;
                    trie.insert_storage_trie(keccak256(address), storage_trie);
                }

                for (address, bundled_account) in &state.state {
                    if bundled_account.is_info_changed() {
                        trie.update_account(
                            keccak256(address),
                            bundled_account.account_info().unwrap_or_default().into(),
                        )?;
                    }
                }

                // ===================== Post Validation =====================

                self.consensus.validate_block_post_execution(
                    &block,
                    PostExecutionInput::new(&output.receipts, &output.requests),
                )?;
                assert_eq!(trie.root().unwrap(), block.state_root);

                // ===================== Update state =====================

                let header_from_payload = block.header.clone();
                self.provider.storage.insert_executed(header_from_payload);
                let latest_valid_hash = match self.forkchoice_state {
                    Some(fcu_state) => fcu_state.head_block_hash,
                    None => parent_hash_from_payload,
                };

                debug!(?latest_valid_hash, "🟢 new payload is valid");

                if let Err(e) = tx.send(Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid)
                    .with_latest_valid_hash(latest_valid_hash)))
                {
                    error!("Failed to send payload status: {:?}", e);
                }
            }
            BeaconEngineMessage::ForkchoiceUpdated {
                state,
                payload_attrs,
                tx,
                version: _,
            } => {
                let outcome = self.on_forkchoice_update(state, payload_attrs);
                let head = self.provider.storage.get_canonical_head();
                info!("head: {:#?}", head);
                if let Err(e) = tx.send(outcome) {
                    error!("Failed to send forkchoice outcome: {e:?}");
                }
            }
            BeaconEngineMessage::TransitionConfigurationExchanged => {
                // Implement transition configuration handling
                todo!()
            }
        }

        Ok(())
    }

    fn on_forkchoice_update(
        &mut self,
        state: ForkchoiceState,
        payload_attrs: Option<<EthEngineTypes as PayloadTypes>::PayloadAttributes>,
    ) -> RethResult<OnForkChoiceUpdated> {
        info!("👋 new fork choice");
        debug!(
            "head={:#x}, safe={:#x}, finalized={:#x}",
            state.head_block_hash, state.safe_block_hash, state.finalized_block_hash
        );

        // ===================== Validation =====================

        if state.head_block_hash.is_zero() {
            return Ok(OnForkChoiceUpdated::invalid_state());
        }
        // todo: invalid_ancestors check

        // check finalized bock hash and safe block hash all in storage
        if let Err(outcome) = self.ensure_consistent_forkchoice_state(state) {
            return Ok(outcome);
        }

        // retrieve head by hash
        let Some(head) = self
            .provider
            .storage
            .get_executed_header_by_hash(state.head_block_hash)
        else {
            return Ok(OnForkChoiceUpdated::valid(PayloadStatus::from_status(
                PayloadStatusEnum::Syncing,
            )));
        };

        // payload attributes, version validation
        if let Some(attrs) = payload_attrs {
            if let Err(err) =
                EngineValidator::<EthEngineTypes>::validate_payload_attributes_against_header(
                    &self.engine_validator,
                    &attrs,
                    &head,
                )
            {
                warn!(%err, ?head, "Invalid payload attributes");
                return Ok(OnForkChoiceUpdated::invalid_payload_attributes());
            }
        }

        // ===================== Handle Reorg =====================

        if self.provider.storage.get_canonical_head().number + 1 != head.number {
            // fcu is pointing fork chain
            warn!(?head.number,"reorg detacted");
            self.provider
                .storage
                .post_fcu_reorg_update(head, state.finalized_block_hash)
                .map_err(|e: StorageError| RethError::Other(Box::new(e)))?;
        } else {
            // fcu is on canonical chain
            self.provider
                .storage
                .post_fcu_update(head, state.finalized_block_hash)
                .map_err(|e: StorageError| RethError::Other(Box::new(e)))?;
        }

        self.forkchoice_state = Some(state);

        // ========================================================

        // todo: also need to prune witness, but will handle by getting witness from stateful node
        let witness_path = get_witness_path(state.safe_block_hash);
        // remove witness of the fcu
        if let Err(e) = std::fs::remove_file(witness_path) {
            debug!("Unable to remove finalized witness file: {:?}", e);
        }

        Ok(OnForkChoiceUpdated::valid(
            PayloadStatus::from_status(PayloadStatusEnum::Valid)
                .with_latest_valid_hash(state.head_block_hash),
        ))
    }

    /// Ensures that the given forkchoice state is consistent, assuming the head block has been
    /// made canonical.
    ///
    /// If the forkchoice state is consistent, this will return Ok(()). Otherwise, this will
    /// return an instance of [`OnForkChoiceUpdated`] that is INVALID.
    fn ensure_consistent_forkchoice_state(
        &self,
        state: ForkchoiceState,
    ) -> Result<(), OnForkChoiceUpdated> {
        if !state.finalized_block_hash.is_zero()
            && !self
                .provider
                .storage
                .is_canonical(state.finalized_block_hash)
        {
            return Err(OnForkChoiceUpdated::invalid_state());
        }
        if !state.safe_block_hash.is_zero()
            && !self.provider.storage.is_canonical(state.safe_block_hash)
        {
            return Err(OnForkChoiceUpdated::invalid_state());
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
    pub async fn prefetch_all_bytecodes(
        &self,
        execution_witness: &ExecutionWitness,
        block_hash: B256,
    ) {
        let code_hashes: Vec<_> = execution_witness
            .state_witness
            .iter()
            .filter_map(|(_, encoded)| {
                let node = TrieNode::decode(&mut &encoded[..]).ok()?;
                let TrieNode::Leaf(leaf) = node else {
                    return None;
                };
                let account = TrieAccount::decode(&mut &leaf.value[..]).ok()?;
                // Skip EOA
                if account.code_hash == KECCAK_EMPTY {
                    return None;
                }

                Some(account.code_hash)
            })
            .collect();
        for code_hash in self.provider.storage.filter_code_hashes(code_hashes) {
            if let Err(e) = self
                .provider
                .fetch_contract_bytecode(code_hash, block_hash)
                .await
            {
                // code hashes decoded from witness might not used during execution so it's ok to not fetch from code from `debug_executionWitness`
                debug!("Failed to prefetch: {e}");
            }
        }
    }
}
