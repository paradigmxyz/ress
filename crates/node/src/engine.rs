use alloy_primitives::map::B256HashSet;
use alloy_primitives::U256;
use alloy_rpc_types_engine::ForkchoiceState;
use alloy_rpc_types_engine::PayloadStatus;
use alloy_rpc_types_engine::PayloadStatusEnum;
use rayon::iter::IntoParallelRefIterator;
use ress_common::utils::get_witness_path;
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
use reth_primitives::GotExpected;
use reth_primitives::SealedBlock;
use reth_primitives_traits::SealedHeader;
use reth_trie::HashedPostState;
use reth_trie::KeccakKeyHasher;
use reth_trie_sparse::SparseStateTrie;
use std::result::Result::Ok;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::*;

use crate::errors::EngineError;
use crate::root::calculate_state_root;

/// Ress consensus engine.
#[allow(missing_debug_implementations)]
pub struct ConsensusEngine {
    provider: RessProvider,
    consensus: Arc<dyn FullConsensus<Error = ConsensusError>>,
    engine_validator: EthereumEngineValidator,
    from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
    forkchoice_state: Option<ForkchoiceState>,
}

impl ConsensusEngine {
    /// Initialize consensus engine.
    pub fn new(
        provider: RessProvider,
        from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
    ) -> Self {
        // we have it in auth server for now to leverage the methods in here, we also init new validator
        let chain_spec = provider.storage.chain_spec();
        let engine_validator = EthereumEngineValidator::new(chain_spec.clone());
        let consensus: Arc<dyn FullConsensus<Error = ConsensusError>> =
            Arc::new(EthBeaconConsensus::<ChainSpec>::new(chain_spec));
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
                info!(%block_hash, block_number, "👋 new payload");

                // ===================== Validation =====================
                // todo: invalid_ancestors check
                let total_difficulty = U256::MAX;
                let parent_hash_from_payload = payload.parent_hash();
                if !self.provider.storage.is_canonical(parent_hash_from_payload) {
                    warn!(?parent_hash_from_payload, "not in canonical");
                }

                let parent_header: SealedHeader = SealedHeader::new(
                    self.provider
                        .storage
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
                let bytecode_hashes = execution_witness.get_bytecode_hashes();
                let bytecode_hashes_len = bytecode_hashes.len();
                self.prefetch_bytecodes(bytecode_hashes).await;
                info!(elapsed = ?start_time.elapsed(), len = bytecode_hashes_len, "✨ ensured all bytecodes are present");
                let mut trie = SparseStateTrie::default();
                trie.reveal_witness(state_root_of_parent, &execution_witness.state_witness)?;
                let database = WitnessDatabase::new(self.provider.storage.clone(), &trie);

                // ===================== Execution =====================

                let start_time = std::time::Instant::now();
                let mut block_executor =
                    BlockExecutor::new(self.provider.storage.chain_spec(), database);
                let senders = block.senders().expect("no senders");
                let block = BlockWithSenders::new(block.clone().unseal(), senders)
                    .expect("cannot construct block");
                let output = block_executor.execute(&block)?;
                info!(elapsed = ?start_time.elapsed(), "🎉 executed new payload");

                // ===================== Post Execution Validation =====================
                self.consensus.validate_block_post_execution(
                    &block,
                    PostExecutionInput::new(&output.receipts, &output.requests),
                )?;

                // ===================== State Root =====================
                let hashed_state = HashedPostState::from_bundle_state::<KeccakKeyHasher>(
                    output.state.state.par_iter(),
                );
                let state_root = calculate_state_root(&mut trie, hashed_state)?;
                if state_root != block.state_root {
                    return Err(ConsensusError::BodyStateRootDiff(
                        GotExpected {
                            got: state_root,
                            expected: block.state_root,
                        }
                        .into(),
                    )
                    .into());
                }

                // ===================== Update Node State =====================
                let header_from_payload = block.header.clone();
                self.provider.storage.insert_executed(header_from_payload);
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
        info!(
            head = %state.head_block_hash,
            safe = %state.safe_block_hash,
            finalized = %state.finalized_block_hash,
            "👋 new fork choice"
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
                .on_fcu_reorg_update(head, state.finalized_block_hash)
                .map_err(|e: StorageError| RethError::Other(Box::new(e)))?;
        } else {
            // fcu is on canonical chain
            self.provider
                .storage
                .on_fcu_update(head, state.finalized_block_hash)
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

    /// Prefetch all bytecodes found in the witness.
    pub async fn prefetch_bytecodes(&self, bytecode_hashes: B256HashSet) {
        for code_hash in bytecode_hashes {
            if let Err(error) = self.provider.ensure_bytecode_exists(code_hash).await {
                // TODO: handle this error
                error!("Failed to prefetch: {error}");
            }
        }
    }
}
