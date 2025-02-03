use alloy_primitives::map::B256HashSet;
use alloy_primitives::B256;
use alloy_primitives::U256;
use alloy_rpc_types_engine::ExecutionPayload;
use alloy_rpc_types_engine::ExecutionPayloadSidecar;
use alloy_rpc_types_engine::ForkchoiceState;
use alloy_rpc_types_engine::PayloadStatus;
use alloy_rpc_types_engine::PayloadStatusEnum;
use rayon::iter::IntoParallelRefIterator;
use ress_protocol::RessProtocolProvider;
use ress_provider::errors::MemoryStorageError;
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
use reth_primitives::EthPrimitives;
use reth_primitives::GotExpected;
use reth_primitives::RecoveredBlock;
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
    consensus: Arc<dyn FullConsensus<EthPrimitives, Error = ConsensusError>>,
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
        let consensus: Arc<dyn FullConsensus<EthPrimitives, Error = ConsensusError>> =
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
            self.on_engine_message(beacon_msg).await;
        }
    }

    async fn on_engine_message(&mut self, message: BeaconEngineMessage<EthEngineTypes>) {
        match message {
            BeaconEngineMessage::NewPayload {
                payload,
                sidecar,
                tx,
            } => {
                let outcome = match self.on_new_payload(payload, sidecar).await {
                    Ok(status) => status,
                    Err(error) => {
                        error!(target: "ress::engine", ?error, "Error on new payload");
                        PayloadStatus::from_status(PayloadStatusEnum::Invalid {
                            validation_error: error.to_string(),
                        })
                    }
                };
                if let Err(error) = tx.send(Ok(outcome)) {
                    error!(target: "ress::engine", ?error, "Failed to send payload status");
                }
            }
            BeaconEngineMessage::ForkchoiceUpdated {
                state,
                payload_attrs,
                tx,
                version: _,
            } => {
                let outcome = self.on_forkchoice_update(state, payload_attrs);
                if let Err(error) = tx.send(outcome) {
                    error!(target: "ress::engine", ?error, "Failed to send forkchoice outcome");
                }
            }
            BeaconEngineMessage::TransitionConfigurationExchanged => {
                // Implement transition configuration handling
                todo!()
            }
        }
    }

    fn on_forkchoice_update(
        &mut self,
        state: ForkchoiceState,
        payload_attrs: Option<<EthEngineTypes as PayloadTypes>::PayloadAttributes>,
    ) -> RethResult<OnForkChoiceUpdated> {
        info!(
            target: "ress::engine",
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
        let Some(head) = self.provider.storage.header_by_hash(state.head_block_hash) else {
            return Ok(OnForkChoiceUpdated::valid(PayloadStatus::from_status(
                PayloadStatusEnum::Syncing,
            )));
        };

        // payload attributes, version validation
        if let Some(attrs) = payload_attrs {
            if let Err(error) =
                EngineValidator::<EthEngineTypes>::validate_payload_attributes_against_header(
                    &self.engine_validator,
                    &attrs,
                    &head,
                )
            {
                warn!(target: "ress::engine", %error, ?head, "Invalid payload attributes");
                return Ok(OnForkChoiceUpdated::invalid_payload_attributes());
            }
        }

        // ===================== Handle Reorg =====================
        let canonical_head = self.provider.storage.get_canonical_head().number;
        if canonical_head + 1 != head.number {
            // fcu is pointing fork chain
            warn!(target: "ress::engine", block_number = head.number, ?canonical_head, "Reorg or hash inconsistency detected");
            self.provider
                .storage
                .on_fcu_reorg_update(head, state.finalized_block_hash)
                .map_err(|e: StorageError| RethError::Other(Box::new(e)))?;
        } else {
            let block_hash = if state.finalized_block_hash != B256::ZERO {
                let block = self
                    .provider
                    .storage
                    .header(state.finalized_block_hash)?
                    .unwrap();
                self.provider
                    .storage
                    .get_block_hash(block.number - 1)
                    .map_err(|e: StorageError| RethError::Other(Box::new(e)))?
            } else {
                B256::ZERO
            };

            // fcu is on canonical chain
            self.provider
                .storage
                .on_fcu_update(head, block_hash)
                .map_err(|e: StorageError| RethError::Other(Box::new(e)))?;
        }

        self.forkchoice_state = Some(state);

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

    async fn on_new_payload(
        &self,
        payload: ExecutionPayload,
        sidecar: ExecutionPayloadSidecar,
    ) -> Result<PayloadStatus, EngineError> {
        let block_number = payload.block_number();
        let block_hash = payload.block_hash();
        info!(target: "ress::engine", %block_hash, block_number, "👋 new payload");

        // ===================== Validation =====================
        // todo: invalid_ancestors check
        let parent_hash = payload.parent_hash();
        if !self.provider.storage.is_canonical(parent_hash) {
            warn!(target: "ress::engine", %parent_hash, "Parent is not canonical, fetching from network");
            let header = self.provider.fetch_header(parent_hash).await?;
            self.provider.storage.insert_header(header);
        }
        let parent =
            self.provider
                .storage
                .header_by_hash(parent_hash)
                .ok_or(StorageError::Memory(
                    MemoryStorageError::BlockNotFoundFromHash(parent_hash),
                ))?;
        let parent_header: SealedHeader = SealedHeader::new(parent, parent_hash);
        let state_root_of_parent = parent_header.state_root;
        let block = self
            .engine_validator
            .ensure_well_formed_payload(payload, sidecar)?;
        self.validate_block(&block, parent_header)?;

        // ===================== Witness =====================
        let execution_witness = self.provider.fetch_witness(block_hash).await?;
        let start_time = std::time::Instant::now();
        let bytecode_hashes = execution_witness.get_bytecode_hashes();
        let bytecode_hashes_len = bytecode_hashes.len();
        self.prefetch_bytecodes(bytecode_hashes).await;
        info!(target: "ress::engine", elapsed = ?start_time.elapsed(), len = bytecode_hashes_len, "✨ ensured all bytecodes are present");
        let mut trie = SparseStateTrie::default();
        trie.reveal_witness(state_root_of_parent, &execution_witness.state_witness)?;
        let database = WitnessDatabase::new(self.provider.storage.clone(), &trie);

        // ===================== Execution =====================

        let start_time = std::time::Instant::now();
        let mut block_executor = BlockExecutor::new(self.provider.storage.chain_spec(), database);
        let senders = block.senders().expect("no senders");
        let block = RecoveredBlock::new_unhashed(block.clone().unseal(), senders);
        let output = block_executor.execute(&block)?;
        info!(target: "ress::engine", elapsed = ?start_time.elapsed(), "🎉 executed new payload");

        // ===================== Post Execution Validation =====================
        self.consensus.validate_block_post_execution(
            &block,
            PostExecutionInput::new(&output.receipts, &output.requests),
        )?;

        // ===================== State Root =====================
        let hashed_state =
            HashedPostState::from_bundle_state::<KeccakKeyHasher>(output.state.state.par_iter());
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
        let header_from_payload = block.sealed_block().header().clone();
        self.provider.storage.insert_header(header_from_payload);
        let latest_valid_hash = block_hash;

        info!(target: "ress::engine", ?latest_valid_hash, "🟢 new payload is valid");
        Ok(PayloadStatus::from_status(PayloadStatusEnum::Valid)
            .with_latest_valid_hash(latest_valid_hash))
    }

    /// Validate if block is correct and satisfies all the consensus rules that concern the header
    /// and block body itself.
    fn validate_block(
        &self,
        block: &SealedBlock,
        parent_header: SealedHeader,
    ) -> Result<(), ConsensusError> {
        let header = block.sealed_header();

        self.consensus
            .validate_header(header)
            .inspect_err(|error| {
                error!(target: "ress::engine", %error, "Failed to validate header");
            })?;

        self.consensus.validate_header_with_total_difficulty(header, U256::MAX).inspect_err(|error| {
            error!(target: "ress::engine", %error, "Failed to validate header against total difficulty");
        })?;

        self.consensus
            .validate_header_against_parent(header, &parent_header)
            .inspect_err(|error| {
                error!(target: "ress::engine", %error, "Failed to validate header against parent");
            })?;

        self.consensus
            .validate_block_pre_execution(block)
            .inspect_err(|error| {
                error!(target: "ress::engine", %error, "Failed to validate block");
            })?;

        Ok(())
    }

    /// Prefetch all bytecodes found in the witness.
    pub async fn prefetch_bytecodes(&self, bytecode_hashes: B256HashSet) {
        for code_hash in bytecode_hashes {
            if let Err(error) = self.provider.ensure_bytecode_exists(code_hash).await {
                // TODO: handle this error
                error!(target: "ress::engine", %error, "Failed to prefetch");
            }
        }
    }
}
