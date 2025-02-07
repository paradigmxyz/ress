use alloy_primitives::map::B256HashSet;
use alloy_primitives::U256;
use alloy_rpc_types_engine::ForkchoiceState;
use alloy_rpc_types_engine::PayloadStatus;
use alloy_rpc_types_engine::PayloadStatusEnum;
use rayon::iter::IntoParallelRefIterator;
use ress_provider::errors::StorageError;
use ress_provider::provider::RessProvider;
use ress_vm::db::WitnessDatabase;
use ress_vm::errors::EvmError;
use ress_vm::executor::BlockExecutor;
use reth_chainspec::ChainSpec;
use reth_consensus::Consensus;
use reth_consensus::ConsensusError;
use reth_consensus::FullConsensus;
use reth_consensus::HeaderValidator;
use reth_consensus::PostExecutionInput;
use reth_engine_tree::tree::error::InsertBlockError;
use reth_engine_tree::tree::error::InsertBlockErrorKind;
use reth_engine_tree::tree::error::InsertBlockFatalError;
use reth_engine_tree::tree::BlockStatus;
use reth_engine_tree::tree::InsertPayloadOk;
use reth_errors::ProviderError;
use reth_errors::RethError;
use reth_errors::RethResult;
use reth_node_api::BeaconEngineMessage;
use reth_node_api::BeaconOnNewPayloadError;
use reth_node_api::EngineValidator;
use reth_node_api::ExecutionData;
use reth_node_api::OnForkChoiceUpdated;
use reth_node_api::PayloadTypes;
use reth_node_api::PayloadValidator;
use reth_node_ethereum::consensus::EthBeaconConsensus;
use reth_node_ethereum::node::EthereumEngineValidator;
use reth_node_ethereum::EthEngineTypes;
use reth_primitives::Block;
use reth_primitives::EthPrimitives;
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

#[allow(unused_imports)]
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
            BeaconEngineMessage::NewPayload { payload, tx } => {
                let outcome = self
                    .on_new_payload(payload)
                    .await
                    .map_err(BeaconOnNewPayloadError::internal);
                if let Err(error) = tx.send(outcome) {
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

        // Client software MUST return -38002: Invalid forkchoice state error if the payload referenced by forkchoiceState.headBlockHash is VALID and a payload referenced by either forkchoiceState.finalizedBlockHash or forkchoiceState.safeBlockHash does not belong to the chain defined by forkchoiceState.headBlockHash.
        //
        // if forkchoiceState.headBlockHash references an unknown payload or a payload that can't be validated because requisite data for the validation is missing
        match self.provider.storage.header_by_hash(state.head_block_hash) {
            Some(head) => {
                // check that the finalized and safe block hashes are canonical
                if let Err(outcome) = self.ensure_consistent_forkchoice_state(state) {
                    return Ok(outcome);
                }

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
                    // fcu is on canonical chain
                    self.provider
                        .storage
                        .on_fcu_update(head, state.finalized_block_hash)
                        .map_err(|e: StorageError| RethError::Other(Box::new(e)))?;
                }

                self.forkchoice_state = Some(state);

                Ok(OnForkChoiceUpdated::valid(
                    PayloadStatus::from_status(PayloadStatusEnum::Valid)
                        .with_latest_valid_hash(state.head_block_hash),
                ))
            }
            None => Ok(OnForkChoiceUpdated::syncing()),
        }
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
        payload: ExecutionData,
    ) -> Result<PayloadStatus, InsertBlockFatalError> {
        let block_number = payload.payload.block_number();
        let block_hash = payload.payload.block_hash();
        let parent_hash = payload.payload.parent_hash();
        info!(target: "ress::engine", %block_hash, block_number, %parent_hash, "👋 new payload");

        // Ensures that the given payload does not violate any consensus rules.
        let block = match self.engine_validator.ensure_well_formed_payload(payload) {
            Ok(block) => block,
            Err(error) => {
                error!(target: "ress::engine", %error, "Invalid payload");
                // TODO: look up latest valid hash.
                // ref: <https://github.com/paradigmxyz/reth/blob/d9d73460201112d7e2e02cbf3bec46c2349644a0/crates/engine/tree/src/tree/mod.rs#L891-L899>
                let status = PayloadStatus::new(PayloadStatusEnum::from(error), None);
                return Ok(status);
            }
        };

        // TODO: invalid_ancestors check
        // ref: <https://github.com/paradigmxyz/reth/blob/d9d73460201112d7e2e02cbf3bec46c2349644a0/crates/engine/tree/src/tree/mod.rs#L906-L917>

        // TODO: insert block
        let mut latest_valid_hash = None;
        // let num_hash = block.num_hash();
        match self.insert_block(block.clone()).await {
            Ok(status) => {
                let status = match status {
                    InsertPayloadOk::Inserted(BlockStatus::Valid) => {
                        latest_valid_hash = Some(block_hash);
                        // TODO: self.try_connect_buffered_blocks(num_hash)?;
                        PayloadStatusEnum::Valid
                    }
                    InsertPayloadOk::AlreadySeen(BlockStatus::Valid) => {
                        latest_valid_hash = Some(block_hash);
                        PayloadStatusEnum::Valid
                    }
                    InsertPayloadOk::Inserted(BlockStatus::Disconnected { .. })
                    | InsertPayloadOk::AlreadySeen(BlockStatus::Disconnected { .. }) => {
                        // not known to be invalid, but we don't know anything else
                        PayloadStatusEnum::Syncing
                    }
                };

                Ok(PayloadStatus::new(status, latest_valid_hash))
            }
            Err(kind) => self.on_insert_block_error(InsertBlockError::new(block, kind)),
        }
    }

    async fn insert_block(
        &self,
        block: SealedBlock,
    ) -> Result<InsertPayloadOk, InsertBlockErrorKind> {
        let block_num_hash = block.num_hash();
        debug!(target: "ress::engine", block=?block_num_hash, parent_hash = %block.parent_hash, state_root = %block.state_root, "Inserting new block into tree");

        let block = block
            .try_recover()
            .map_err(|_| InsertBlockErrorKind::SenderRecovery)?;

        if self.provider.storage.header_by_hash(block.hash()).is_some() {
            return Ok(InsertPayloadOk::AlreadySeen(BlockStatus::Valid));
        }

        trace!(target: "ress::engine", block=?block_num_hash, "Validating block consensus");
        self.validate_block(&block)?;

        let Some(parent) = self.provider.storage.header_by_hash(block.parent_hash) else {
            // TODO: insert into the buffer
            // ref: <https://github.com/paradigmxyz/reth/blob/d9d73460201112d7e2e02cbf3bec46c2349644a0/crates/engine/tree/src/tree/mod.rs#L2381-L2390>
            let missing_ancestor = block.parent_num_hash();

            return Ok(InsertPayloadOk::Inserted(BlockStatus::Disconnected {
                head: self.provider.storage.get_canonical_head(),
                missing_ancestor,
            }));
        };

        let parent = SealedHeader::new(parent, block.parent_hash);
        if let Err(error) = self
            .consensus
            .validate_header_against_parent(block.sealed_header(), &parent)
        {
            error!(target: "ress::engine", %error, "Failed to validate header against parent");
            return Err(error.into());
        }

        // ===================== Witness =====================
        let execution_witness =
            self.provider
                .fetch_witness(block.hash())
                .await
                .map_err(|error| {
                    InsertBlockErrorKind::Provider(ProviderError::TrieWitnessError(
                        error.to_string(),
                    ))
                })?;
        let start_time = std::time::Instant::now();
        let bytecode_hashes = execution_witness.get_bytecode_hashes();
        let bytecode_hashes_len = bytecode_hashes.len();
        self.prefetch_bytecodes(bytecode_hashes).await;
        info!(target: "ress::engine", elapsed = ?start_time.elapsed(), len = bytecode_hashes_len, "✨ ensured all bytecodes are present");
        let mut trie = SparseStateTrie::default();
        trie.reveal_witness(parent.state_root, &execution_witness.state_witness)
            .map_err(|error| {
                InsertBlockErrorKind::Provider(ProviderError::TrieWitnessError(error.to_string()))
            })?;
        let database = WitnessDatabase::new(self.provider.storage.clone(), &trie);

        // ===================== Execution =====================
        let start_time = std::time::Instant::now();
        let mut block_executor = BlockExecutor::new(self.provider.storage.chain_spec(), database);
        let output = block_executor
            .execute(&block)
            .map_err(|error| match error {
                EvmError::BlockExecution(error) => InsertBlockErrorKind::Execution(error),
                EvmError::DB(error) => InsertBlockErrorKind::Other(Box::new(error)),
            })?;
        info!(target: "ress::engine", elapsed = ?start_time.elapsed(), "🎉 executed new payload");

        // ===================== Post Execution Validation =====================
        self.consensus.validate_block_post_execution(
            &block,
            PostExecutionInput::new(&output.receipts, &output.requests),
        )?;

        // ===================== State Root =====================
        let hashed_state =
            HashedPostState::from_bundle_state::<KeccakKeyHasher>(output.state.state.par_iter());
        let state_root = calculate_state_root(&mut trie, hashed_state).map_err(|error| {
            InsertBlockErrorKind::Provider(ProviderError::TrieWitnessError(error.to_string()))
        })?;
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
        self.provider.storage.insert_header(block.header().clone());

        debug!(target: "ress::engine", block=?block_num_hash, "Finished inserting block");
        Ok(InsertPayloadOk::Inserted(BlockStatus::Valid))
    }

    /// Validate if block is correct and satisfies all the consensus rules that concern the header
    /// and block body itself.
    fn validate_block(&self, block: &SealedBlock) -> Result<(), ConsensusError> {
        self.consensus.validate_header_with_total_difficulty(block.header(), U256::MAX).inspect_err(|error| {
            error!(target: "ress::engine", %error, "Failed to validate header against total difficulty");
        })?;

        self.consensus
            .validate_header(block.sealed_header())
            .inspect_err(|error| {
                error!(target: "ress::engine", %error, "Failed to validate header");
            })?;

        self.consensus
            .validate_block_pre_execution(block)
            .inspect_err(|error| {
                error!(target: "ress::engine", %error, "Failed to validate block");
            })?;

        Ok(())
    }

    /// Prefetch all bytecodes found in the witness.
    async fn prefetch_bytecodes(&self, bytecode_hashes: B256HashSet) {
        for code_hash in bytecode_hashes {
            if let Err(error) = self.provider.ensure_bytecode_exists(code_hash).await {
                // TODO: handle this error
                error!(target: "ress::engine", %error, "Failed to prefetch");
            }
        }
    }

    /// Handles an error that occurred while inserting a block.
    ///
    /// If this is a validation error this will mark the block as invalid.
    ///
    /// Returns the proper payload status response if the block is invalid.
    fn on_insert_block_error(
        &self,
        error: InsertBlockError<Block>,
    ) -> Result<PayloadStatus, InsertBlockFatalError> {
        let (block, error) = error.split();

        // if invalid block, we check the validation error. Otherwise return the fatal
        // error.
        let validation_err = error.ensure_validation_error()?;

        // If the error was due to an invalid payload, the payload is added to the invalid headers cache
        // and `Ok` with [PayloadStatusEnum::Invalid] is returned.
        warn!(target: "ress::engine", invalid_hash = %block.hash(), invalid_number = block.number, %validation_err, "Invalid block error on new payload");
        // TODO: let latest_valid_hash = self.latest_valid_hash_for_invalid_payload(block.parent_hash())?;
        let latest_valid_hash = None;

        // keep track of the invalid header
        // TODO: self.state.invalid_headers.insert(block.block_with_parent());
        Ok(PayloadStatus::new(
            PayloadStatusEnum::Invalid {
                validation_error: validation_err.to_string(),
            },
            latest_valid_hash,
        ))
    }
}
