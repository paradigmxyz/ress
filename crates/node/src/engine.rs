use alloy_primitives::map::B256HashSet;
use alloy_primitives::B256;
use alloy_primitives::U256;
use alloy_rpc_types_engine::ForkchoiceState;
use alloy_rpc_types_engine::PayloadStatus;
use alloy_rpc_types_engine::PayloadStatusEnum;
use alloy_rpc_types_engine::PayloadValidationError;
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
use reth_engine_tree::tree::BlockBuffer;
use reth_engine_tree::tree::BlockStatus;
use reth_engine_tree::tree::InsertPayloadOk;
use reth_engine_tree::tree::InvalidHeaderCache;
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
    block_buffer: BlockBuffer<Block>,
    invalid_headers: InvalidHeaderCache,
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
            block_buffer: BlockBuffer::new(256),
            invalid_headers: InvalidHeaderCache::new(256),
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
        &mut self,
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
                let latest_valid_hash =
                    if error.is_block_hash_mismatch() || error.is_invalid_versioned_hashes() {
                        // Engine-API rules:
                        // > `latestValidHash: null` if the blockHash validation has failed (<https://github.com/ethereum/execution-apis/blob/fe8e13c288c592ec154ce25c534e26cb7ce0530d/src/engine/shanghai.md?plain=1#L113>)
                        // > `latestValidHash: null` if the expected and the actual arrays don't match (<https://github.com/ethereum/execution-apis/blob/fe8e13c288c592ec154ce25c534e26cb7ce0530d/src/engine/cancun.md?plain=1#L103>)
                        None
                    } else {
                        self.latest_valid_hash_for_invalid_payload(parent_hash)
                    };
                let status = PayloadStatus::new(PayloadStatusEnum::from(error), latest_valid_hash);
                return Ok(status);
            }
        };

        let block_hash = block.hash();
        let mut lowest_buffered_ancestor = self.lowest_buffered_ancestor_or(block_hash);
        if lowest_buffered_ancestor == block_hash {
            lowest_buffered_ancestor = block.parent_hash;
        }

        // now check the block itself
        if let Some(status) =
            self.check_invalid_ancestor_with_head(lowest_buffered_ancestor, block_hash)
        {
            return Ok(status);
        }

        let mut latest_valid_hash = None;
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
        &mut self,
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
            // we don't have the state required to execute this block, buffering it and find the
            // missing parent block
            let missing_ancestor = self
                .block_buffer
                .lowest_ancestor(&block.parent_hash)
                .map(|block| block.parent_num_hash())
                .unwrap_or_else(|| block.parent_num_hash());
            self.block_buffer.insert_block(block);
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

    /// Checks if the given `check` hash points to an invalid header, inserting the given `head`
    /// block into the invalid header cache if the `check` hash has a known invalid ancestor.
    ///
    /// Returns a payload status response according to the engine API spec if the block is known to
    /// be invalid.
    fn check_invalid_ancestor_with_head(
        &mut self,
        check: B256,
        head: B256,
    ) -> Option<PayloadStatus> {
        // check if the check hash was previously marked as invalid
        let header = self.invalid_headers.get(&check)?;

        // populate the latest valid hash field
        let status = self.prepare_invalid_response(header.parent);

        // insert the head block into the invalid header cache
        self.invalid_headers
            .insert_with_invalid_ancestor(head, header);

        Some(status)
    }

    /// Prepares the invalid payload response for the given hash, checking the
    /// database for the parent hash and populating the payload status with the latest valid hash
    /// according to the engine api spec.
    fn prepare_invalid_response(&mut self, mut parent_hash: B256) -> PayloadStatus {
        // Edge case: the `latestValid` field is the zero hash if the parent block is the terminal
        // PoW block, which we need to identify by looking at the parent's block difficulty
        if let Some(parent) = self.provider.storage.header_by_hash(parent_hash) {
            if !parent.difficulty.is_zero() {
                parent_hash = B256::ZERO;
            }
        }

        let valid_parent_hash = self.latest_valid_hash_for_invalid_payload(parent_hash);
        PayloadStatus::from_status(PayloadStatusEnum::Invalid {
            validation_error: PayloadValidationError::LinksToRejectedPayload.to_string(),
        })
        .with_latest_valid_hash(valid_parent_hash.unwrap_or_default())
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

    /// Return the parent hash of the lowest buffered ancestor for the requested block, if there
    /// are any buffered ancestors. If there are no buffered ancestors, and the block itself does
    /// not exist in the buffer, this returns the hash that is passed in.
    ///
    /// Returns the parent hash of the block itself if the block is buffered and has no other
    /// buffered ancestors.
    fn lowest_buffered_ancestor_or(&self, hash: B256) -> B256 {
        self.block_buffer
            .lowest_ancestor(&hash)
            .map(|block| block.parent_hash)
            .unwrap_or_else(|| hash)
    }

    /// If validation fails, the response MUST contain the latest valid hash:
    ///
    ///   - The block hash of the ancestor of the invalid payload satisfying the following two
    ///     conditions:
    ///     - It is fully validated and deemed VALID
    ///     - Any other ancestor of the invalid payload with a higher blockNumber is INVALID
    ///   - 0x0000000000000000000000000000000000000000000000000000000000000000 if the above
    ///     conditions are satisfied by a `PoW` block.
    ///   - null if client software cannot determine the ancestor of the invalid payload satisfying
    ///     the above conditions.
    fn latest_valid_hash_for_invalid_payload(&mut self, parent_hash: B256) -> Option<B256> {
        // Check if parent exists in side chain or in canonical chain.
        if self.provider.storage.header_by_hash(parent_hash).is_some() {
            return Some(parent_hash);
        }

        // iterate over ancestors in the invalid cache
        // until we encounter the first valid ancestor
        let mut current_hash = parent_hash;
        let mut current_block = self.invalid_headers.get(&current_hash);
        while let Some(block_with_parent) = current_block {
            current_hash = block_with_parent.parent;
            current_block = self.invalid_headers.get(&current_hash);

            // If current_header is None, then the current_hash does not have an invalid
            // ancestor in the cache, check its presence in blockchain tree
            if current_block.is_none()
                && self.provider.storage.header_by_hash(current_hash).is_some()
            {
                return Some(current_hash);
            }
        }

        None
    }

    /// Handles an error that occurred while inserting a block.
    ///
    /// If this is a validation error this will mark the block as invalid.
    ///
    /// Returns the proper payload status response if the block is invalid.
    fn on_insert_block_error(
        &mut self,
        error: InsertBlockError<Block>,
    ) -> Result<PayloadStatus, InsertBlockFatalError> {
        let (block, error) = error.split();

        // if invalid block, we check the validation error. Otherwise return the fatal
        // error.
        let validation_err = error.ensure_validation_error()?;

        // If the error was due to an invalid payload, the payload is added to the invalid headers cache
        // and `Ok` with [PayloadStatusEnum::Invalid] is returned.
        warn!(target: "ress::engine", invalid_hash = %block.hash(), invalid_number = block.number, %validation_err, "Invalid block error on new payload");
        let latest_valid_hash = self.latest_valid_hash_for_invalid_payload(block.parent_hash);

        // keep track of the invalid header
        self.invalid_headers.insert(block.block_with_parent());
        Ok(PayloadStatus::new(
            PayloadStatusEnum::Invalid {
                validation_error: validation_err.to_string(),
            },
            latest_valid_hash,
        ))
    }
}
