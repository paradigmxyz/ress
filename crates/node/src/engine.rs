use alloy_primitives::{map::B256HashSet, B256, U256};
use alloy_rpc_types_engine::{
    ForkchoiceState, PayloadAttributes, PayloadStatus, PayloadStatusEnum, PayloadValidationError,
};
use rayon::iter::IntoParallelRefIterator;
use ress_provider::provider::RessProvider;
use ress_vm::{db::WitnessDatabase, errors::EvmError, executor::BlockExecutor};
use reth_chainspec::ChainSpec;
use reth_consensus::{
    Consensus, ConsensusError, FullConsensus, HeaderValidator, PostExecutionInput,
};
use reth_engine_tree::tree::{
    error::{InsertBlockError, InsertBlockErrorKind, InsertBlockFatalError},
    BlockBuffer, BlockStatus, InsertPayloadOk, InvalidHeaderCache,
};
use reth_errors::{ProviderError, RethResult};
use reth_ethereum_engine_primitives::EthPayloadBuilderAttributes;
use reth_node_api::{
    BeaconEngineMessage, BeaconOnNewPayloadError, EngineApiMessageVersion, EngineValidator,
    ExecutionData, OnForkChoiceUpdated, PayloadBuilderAttributes, PayloadValidator,
};
use reth_node_ethereum::{
    consensus::EthBeaconConsensus, node::EthereumEngineValidator, EthEngineTypes,
};
use reth_primitives::{Block, EthPrimitives, GotExpected, Header, SealedBlock};
use reth_primitives_traits::SealedHeader;
use reth_trie::{HashedPostState, KeccakKeyHasher};
use reth_trie_sparse::SparseStateTrie;
use std::{result::Result::Ok, sync::Arc};
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
        // we have it in auth server for now to leverage the methods in here, we also init new
        // validator
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
                let outcome =
                    self.on_new_payload(payload).await.map_err(BeaconOnNewPayloadError::internal);
                if let Err(error) = tx.send(outcome) {
                    error!(target: "ress::engine", ?error, "Failed to send payload status");
                }
            }
            BeaconEngineMessage::ForkchoiceUpdated { state, payload_attrs, tx, version } => {
                let outcome = self.on_forkchoice_update(state, payload_attrs, version);
                if let Ok(updated) = &outcome {
                    if updated.forkchoice_status().is_valid() {
                        self.forkchoice_state = Some(state);
                    }
                }
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
        payload_attrs: Option<PayloadAttributes>,
        version: EngineApiMessageVersion,
    ) -> RethResult<OnForkChoiceUpdated> {
        info!(
            target: "ress::engine",
            head = %state.head_block_hash,
            safe = %state.safe_block_hash,
            finalized = %state.finalized_block_hash,
            "👋 new fork choice"
        );

        // ===================== Validation =====================
        if let Some(on_updated) = self.pre_validate_forkchoice_update(state) {
            return Ok(on_updated);
        }

        // Process the forkchoice update by trying to make the head block canonical
        //
        // We can only process this forkchoice update if:
        // - we have the `head` block
        // - the head block is part of a chain that is connected to the canonical chain. This
        //   includes reorgs.
        //
        // Performing a FCU involves:
        // - marking the FCU's head block as canonical
        // - updating in memory state to reflect the new canonical chain
        // - updating canonical state trackers
        // - emitting a canonicalization event for the new chain (including reorg)
        // - if we have payload attributes, delegate them to the payload service

        // 1. ensure we have a new head block
        if self.provider.storage.get_canonical_head().hash == state.head_block_hash {
            trace!(target: "ress::engine", "fcu head hash is already canonical");

            // update the safe and finalized blocks and ensure their values are valid
            if let Err(outcome) = self.ensure_consistent_forkchoice_state(state) {
                // safe or finalized hashes are invalid
                return Ok(outcome);
            }

            // we still need to process payload attributes if the head is already canonical
            if let Some(attr) = payload_attrs {
                let head = self.provider.storage.get_canonical_head();
                let tip = self.provider.storage.header(head.hash).ok_or_else(|| {
                    // If we can't find the canonical block, then something is wrong and we need
                    // to return an error
                    ProviderError::HeaderNotFound(state.head_block_hash.into())
                })?;
                if let Some(status) = self.validate_payload_attributes(attr, &tip, state, version) {
                    return Ok(status); // payload attributes are invalid
                }
            }

            // the head block is already canonical
            return Ok(OnForkChoiceUpdated::valid(PayloadStatus::new(
                PayloadStatusEnum::Valid,
                Some(state.head_block_hash),
            )));
        }

        // 2. ensure we can apply a new chain update for the head block
        if let Some(chain_update) = self.on_new_head(state.head_block_hash) {
            let tip = chain_update.tip().clone();
            self.on_canonical_chain_update(chain_update);

            // update the safe and finalized blocks and ensure their values are valid
            if let Err(outcome) = self.ensure_consistent_forkchoice_state(state) {
                // safe or finalized hashes are invalid
                return Ok(outcome);
            }

            // TODO: figure out why this causes 2 additional hive failures
            // self.provider
            //     .storage
            //     .on_new_finalized_hash(state.finalized_block_hash)
            //     .map_err(RethError::other)?;

            if let Some(attr) = payload_attrs {
                if let Some(status) = self.validate_payload_attributes(attr, &tip, state, version) {
                    return Ok(status); // payload attributes are invalid
                }
            }

            return Ok(OnForkChoiceUpdated::valid(PayloadStatus::new(
                PayloadStatusEnum::Valid,
                Some(state.head_block_hash),
            )));
        }

        // 3. check if the head is already part of the canonical chain
        if let Some(header) = self.provider.storage.header(state.head_block_hash) {
            debug!(target: "ress::engine", head = header.number, "fcu head block is already canonical");

            // 2. Client software MAY skip an update of the forkchoice state and MUST NOT begin a
            //    payload build process if `forkchoiceState.headBlockHash` references a `VALID`
            //    ancestor of the head of canonical chain, i.e. the ancestor passed payload
            //    validation process and deemed `VALID`. In the case of such an event, client
            //    software MUST return `{payloadStatus: {status: VALID, latestValidHash:
            //    forkchoiceState.headBlockHash, validationError: null}, payloadId: null}`

            // the head block is already canonical, so we're not triggering a payload job and can
            // return right away
            return Ok(OnForkChoiceUpdated::valid(PayloadStatus::new(
                PayloadStatusEnum::Valid,
                Some(state.head_block_hash),
            )));
        }

        // TODO: download missing block(s)
        // ref: <https://github.com/paradigmxyz/reth/blob/d147a2093ec6d0a463610292e30a91fced6c44d7/crates/engine/tree/src/tree/mod.rs#L1227-L1245>

        Ok(OnForkChoiceUpdated::valid(PayloadStatus::from_status(PayloadStatusEnum::Syncing)))
    }

    /// Pre-validate forkchoice update and check whether it can be processed.
    ///
    /// This method returns the update outcome if validation fails or
    /// the node is syncing and the update cannot be processed at the moment.
    fn pre_validate_forkchoice_update(
        &mut self,
        state: ForkchoiceState,
    ) -> Option<OnForkChoiceUpdated> {
        if state.head_block_hash.is_zero() {
            return Some(OnForkChoiceUpdated::invalid_state());
        }

        // check if the new head hash is connected to any ancestor that we previously marked as
        // invalid
        let lowest_buffered_ancestor_fcu = self.lowest_buffered_ancestor_or(state.head_block_hash);
        if let Some(status) = self.check_invalid_ancestor(lowest_buffered_ancestor_fcu) {
            return Some(OnForkChoiceUpdated::with_invalid(status));
        }

        None
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
        if !state.finalized_block_hash.is_zero() &&
            !self.provider.storage.is_canonical(state.finalized_block_hash)
        {
            return Err(OnForkChoiceUpdated::invalid_state());
        }
        if !state.safe_block_hash.is_zero() &&
            !self.provider.storage.is_canonical(state.safe_block_hash)
        {
            return Err(OnForkChoiceUpdated::invalid_state());
        }

        Ok(())
    }

    /// Returns the new chain for the given head.
    ///
    /// This also handles reorgs.
    ///
    /// Note: This does not update the tracked state and instead returns the new chain based on the
    /// given head.
    fn on_new_head(&self, new_head: B256) -> Option<NewCanonicalChain> {
        let new_head_block = self.provider.storage.sealed_header(new_head)?;

        let mut current_canonical_number = self.provider.storage.get_canonical_head().number;
        let (mut current_number, mut current_hash) =
            new_head_block.parent_num_hash().into_components();
        let mut new_chain = vec![new_head_block];

        // Walk back the new chain until we reach a block we know about
        //
        // This is only done for in-memory blocks, because we should not have persisted any blocks
        // that are _above_ the current canonical head.
        while current_number > current_canonical_number {
            if let Some(header) = self.provider.storage.sealed_header(current_hash) {
                current_hash = header.parent_hash;
                current_number -= 1;
                new_chain.push(header);
            } else {
                // This should never happen as we're walking back a chain that should connect to the
                // canonical chain.
                warn!(target: "ress::engine", %current_hash, "Sidechain block not found in TreeState");
                return None;
            }
        }

        // If we have reached the current canonical head by walking back from the target, then we
        // know this represents an extension of the canonical chain.
        if current_hash == self.provider.storage.get_canonical_head().hash {
            new_chain.reverse();
            return Some(NewCanonicalChain::Commit { new: new_chain });
        }

        // We have a reorg. Walk back both chains to find the fork point.
        let mut old_chain = Vec::new();
        let mut old_hash = self.provider.storage.get_canonical_head().hash;

        // If the canonical chain is ahead of the new chain,
        // gather all blocks until new head number.
        while current_canonical_number > current_number {
            if let Some(header) = self.provider.storage.sealed_header(old_hash) {
                old_hash = header.parent_hash;
                current_canonical_number -= 1;
                old_chain.push(header);
            } else {
                // This shouldn't happen as we're walking back the canonical chain
                warn!(target: "engine::tree", current_hash=?old_hash, "Canonical block not found in TreeState");
                return None;
            }
        }

        // Both new and old chain pointers are now at the same height.
        debug_assert_eq!(current_number, current_canonical_number);

        // Walk both chains from specified hashes at same height until
        // a common ancestor (fork block) is reached.
        while old_hash != current_hash {
            if let Some(header) = self.provider.storage.sealed_header(old_hash) {
                old_hash = header.parent_hash;
                old_chain.push(header);
            } else {
                // This shouldn't happen as we're walking back the canonical chain
                warn!(target: "engine::tree", current_hash=?old_hash, "Canonical block not found in TreeState");
                return None;
            }

            if let Some(header) = self.provider.storage.sealed_header(current_hash) {
                current_hash = header.parent_hash;
                new_chain.push(header);
            } else {
                // This shouldn't happen as we've already walked this path
                warn!(target: "engine::tree", invalid_hash=?current_hash, "New chain block not found in TreeState");
                return None;
            }
        }
        new_chain.reverse();
        old_chain.reverse();

        Some(NewCanonicalChain::Reorg { new: new_chain, old: old_chain })
    }

    /// Validates the payload attributes with respect to the header and fork choice state.
    ///
    /// Note: At this point, the fork choice update is considered to be VALID, however, we can still
    /// return an error if the payload attributes are invalid.
    fn validate_payload_attributes(
        &self,
        attrs: PayloadAttributes,
        head: &Header,
        state: ForkchoiceState,
        version: EngineApiMessageVersion,
    ) -> Option<OnForkChoiceUpdated> {
        if let Err(err) =
            EngineValidator::<EthEngineTypes>::validate_payload_attributes_against_header(
                &self.engine_validator,
                &attrs,
                head,
            )
        {
            warn!(target: "engine::tree", %err, ?head, "Invalid payload attributes");
            return Some(OnForkChoiceUpdated::invalid_payload_attributes());
        }

        // 8. Client software MUST begin a payload build process building on top of
        //    forkchoiceState.headBlockHash and identified via buildProcessId value if
        //    payloadAttributes is not null and the forkchoice state has been updated successfully.
        //    The build process is specified in the Payload building section.
        if EthPayloadBuilderAttributes::try_new(state.head_block_hash, attrs, version as u8)
            .is_err()
        {
            return Some(OnForkChoiceUpdated::invalid_payload_attributes());
        }

        None
    }

    /// Invoked when we the canonical chain has been updated.
    ///
    /// This is invoked on a valid forkchoice update, or if we can make the target block canonical.
    fn on_canonical_chain_update(&mut self, update: NewCanonicalChain) {
        trace!(target: "ress::engine", new_blocks = %update.new_block_count(), reorged_blocks = %update.reorged_block_count(), "applying new chain update");
        self.provider.storage.set_canonical_head(update.tip().num_hash());
        let (new, old) = match update {
            NewCanonicalChain::Commit { new } => (new, Vec::new()),
            NewCanonicalChain::Reorg { new, old } => (new, old),
        };
        self.provider.storage.on_chain_update(new, old);
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
                    InsertPayloadOk::Inserted(BlockStatus::Disconnected { .. }) |
                    InsertPayloadOk::AlreadySeen(BlockStatus::Disconnected { .. }) => {
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

        let block = block.try_recover().map_err(|_| InsertBlockErrorKind::SenderRecovery)?;

        if self.provider.storage.header(block.hash()).is_some() {
            return Ok(InsertPayloadOk::AlreadySeen(BlockStatus::Valid));
        }

        trace!(target: "ress::engine", block=?block_num_hash, "Validating block consensus");
        self.validate_block(&block)?;

        let Some(parent) = self.provider.storage.header(block.parent_hash) else {
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
        if let Err(error) =
            self.consensus.validate_header_against_parent(block.sealed_header(), &parent)
        {
            error!(target: "ress::engine", %error, "Failed to validate header against parent");
            return Err(error.into());
        }

        // ===================== Witness =====================
        let execution_witness =
            self.provider.fetch_witness(block.hash()).await.map_err(|error| {
                InsertBlockErrorKind::Provider(ProviderError::TrieWitnessError(error.to_string()))
            })?;
        let start_time = std::time::Instant::now();
        let bytecode_hashes = execution_witness.get_bytecode_hashes();
        let bytecode_hashes_len = bytecode_hashes.len();
        self.prefetch_bytecodes(bytecode_hashes).await;
        info!(target: "ress::engine", elapsed = ?start_time.elapsed(), len = bytecode_hashes_len, "✨ ensured all bytecodes are present");
        let mut trie = SparseStateTrie::default();
        trie.reveal_witness(parent.state_root, &execution_witness.state_witness).map_err(
            |error| {
                InsertBlockErrorKind::Provider(ProviderError::TrieWitnessError(error.to_string()))
            },
        )?;
        let database = WitnessDatabase::new(self.provider.storage.clone(), &trie);

        // ===================== Execution =====================
        let start_time = std::time::Instant::now();
        let mut block_executor = BlockExecutor::new(self.provider.storage.chain_spec(), database);
        let output = block_executor.execute(&block).map_err(|error| match error {
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
                GotExpected { got: state_root, expected: block.state_root }.into(),
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
        self.invalid_headers.insert_with_invalid_ancestor(head, header);

        Some(status)
    }

    /// Checks if the given `head` points to an invalid header, which requires a specific response
    /// to a forkchoice update.
    fn check_invalid_ancestor(&mut self, head: B256) -> Option<PayloadStatus> {
        // check if the head was previously marked as invalid
        let header = self.invalid_headers.get(&head)?;
        // populate the latest valid hash field
        Some(self.prepare_invalid_response(header.parent))
    }

    /// Prepares the invalid payload response for the given hash, checking the
    /// database for the parent hash and populating the payload status with the latest valid hash
    /// according to the engine api spec.
    fn prepare_invalid_response(&mut self, mut parent_hash: B256) -> PayloadStatus {
        // Edge case: the `latestValid` field is the zero hash if the parent block is the terminal
        // PoW block, which we need to identify by looking at the parent's block difficulty
        if let Some(parent) = self.provider.storage.header(parent_hash) {
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

        self.consensus.validate_header(block.sealed_header()).inspect_err(|error| {
            error!(target: "ress::engine", %error, "Failed to validate header");
        })?;

        self.consensus.validate_block_pre_execution(block).inspect_err(|error| {
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
        if self.provider.storage.header(parent_hash).is_some() {
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
            if current_block.is_none() && self.provider.storage.header(current_hash).is_some() {
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

        // If the error was due to an invalid payload, the payload is added to the invalid headers
        // cache and `Ok` with [PayloadStatusEnum::Invalid] is returned.
        warn!(target: "ress::engine", invalid_hash = %block.hash(), invalid_number = block.number, %validation_err, "Invalid block error on new payload");
        let latest_valid_hash = self.latest_valid_hash_for_invalid_payload(block.parent_hash);

        // keep track of the invalid header
        self.invalid_headers.insert(block.block_with_parent());
        Ok(PayloadStatus::new(
            PayloadStatusEnum::Invalid { validation_error: validation_err.to_string() },
            latest_valid_hash,
        ))
    }
}

/// Non-empty chain of blocks.
#[derive(Debug)]
pub enum NewCanonicalChain {
    /// A simple append to the current canonical head.
    Commit {
        /// All blocks that lead back to the canonical head
        new: Vec<SealedHeader>,
    },
    /// A reorged chain consists of two chains that trace back to a shared ancestor block at which.
    /// point they diverge.
    Reorg {
        /// All blocks of the _new_ chain.
        new: Vec<SealedHeader>,
        /// All blocks of the _old_ chain.
        old: Vec<SealedHeader>,
    },
}

impl NewCanonicalChain {
    /// Returns the new tip of the chain.
    ///
    /// Returns the new tip for [`Self::Reorg`] and [`Self::Commit`] variants which commit at least
    /// 1 new block.
    pub fn tip(&self) -> &SealedHeader {
        match self {
            Self::Commit { new } | Self::Reorg { new, .. } => new.last().expect("non empty blocks"),
        }
    }

    /// Returns the length of the new chain.
    pub fn new_block_count(&self) -> usize {
        match self {
            Self::Commit { new } | Self::Reorg { new, .. } => new.len(),
        }
    }

    /// Returns the length of the reorged chain.
    pub fn reorged_block_count(&self) -> usize {
        match self {
            Self::Commit { .. } => 0,
            Self::Reorg { old, .. } => old.len(),
        }
    }
}
