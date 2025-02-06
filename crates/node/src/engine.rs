use alloy_eips::NumHash;
use alloy_primitives::map::B256HashSet;
use alloy_primitives::B256;
use alloy_primitives::U256;
use alloy_rpc_types_engine::ExecutionPayload;
use alloy_rpc_types_engine::ExecutionPayloadSidecar;
use alloy_rpc_types_engine::ForkchoiceState;
use alloy_rpc_types_engine::PayloadStatus;
use alloy_rpc_types_engine::PayloadStatusEnum;
use rayon::iter::IntoParallelRefIterator;
use ress_primitives::chain_state::NewCanonicalChain;
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
use reth_errors::ProviderError;
use reth_errors::ProviderResult;
use reth_errors::RethError;
use reth_errors::RethResult;
use reth_node_api::BeaconEngineMessage;
use reth_node_api::EngineApiMessageVersion;
use reth_node_api::EngineValidator;
use reth_node_api::ForkchoiceStateTracker;
use reth_node_api::OnForkChoiceUpdated;
use reth_node_api::PayloadBuilder;
use reth_node_api::PayloadBuilderAttributes;
use reth_node_api::PayloadTypes;
use reth_node_api::PayloadValidator;
use reth_node_ethereum::consensus::EthBeaconConsensus;
use reth_node_ethereum::node::EthereumEngineValidator;
use reth_node_ethereum::EthEngineTypes;
use reth_payload_builder::PayloadBuilderHandle;
use reth_primitives::EthPrimitives;
use reth_primitives::GotExpected;
use reth_primitives::Header;
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
    /// Tracks the forkchoice state updates received by the CL.
    forkchoice_state_tracker: ForkchoiceStateTracker,
    /// Handle to the payload builder that will receive payload attributes for valid forkchoice
    /// updates
    pub payload_builder: PayloadBuilderHandle<EthEngineTypes>,
}

impl ConsensusEngine {
    /// Initialize consensus engine.
    pub fn new(
        provider: RessProvider,
        from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
        payload_builder: PayloadBuilderHandle<EthEngineTypes>,
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
            forkchoice_state_tracker: ForkchoiceStateTracker::default(),
            payload_builder,
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
                version,
            } => {
                let outcome = self.on_forkchoice_update(state, payload_attrs, version);
                if let Ok(res) = &outcome {
                    self.forkchoice_state_tracker
                        .set_latest(state, res.forkchoice_status());
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
        payload_attrs: Option<<EthEngineTypes as PayloadTypes>::PayloadAttributes>,
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

        if state.head_block_hash.is_zero() {
            return Ok(OnForkChoiceUpdated::invalid_state());
        }
        // todo: invalid_ancestors check

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
            trace!(target: "engine::tree", "fcu head hash is already canonical");

            // check finalized bock hash and safe block hash canonical
            if let Err(outcome) = self.ensure_consistent_forkchoice_state(state) {
                return Ok(outcome);
            }

            // we still need to process payload attributes if the head is already canonical
            if let Some(attr) = payload_attrs {
                let tip = self
                    .provider
                    .storage
                    .header_by_hash(self.provider.storage.get_canonical_head().hash)
                    .ok_or_else(|| {
                        // If we can't find the canonical block, then something is wrong and we need
                        // to return an error
                        RethError::Provider(ProviderError::HeaderNotFound(
                            state.head_block_hash.into(),
                        ))
                    })?;

                let updated = self.process_payload_attributes(attr, tip, state, version);
                return Ok(updated);
            }
        }

        // 2. ensure we can apply a new chain update for the head block
        if let Some(chain_update) = self.on_new_head(state.head_block_hash)? {
            let tip = chain_update.tip();
            self.on_canonical_chain_update(chain_update.clone());

            // update the safe and finalized blocks and ensure their values are valid
            if let Err(outcome) = self.ensure_consistent_forkchoice_state(state) {
                // safe or finalized hashes are invalid
                return Ok(outcome);
            }

            if let Some(attr) = payload_attrs {
                let updated = self.process_payload_attributes(attr, tip.clone(), state, version);
                return Ok(updated);
            }

            return Ok(OnForkChoiceUpdated::valid(
                PayloadStatus::from_status(PayloadStatusEnum::Valid)
                    .with_latest_valid_hash(state.head_block_hash),
            ));
        }

        // 3. check if the head is already part of the canonical chain
        if let Some(canonical_header) = self.provider.storage.header_by_hash(state.head_block_hash)
        {
            debug!(target: "engine::tree", head = canonical_header.number, "fcu head block is already canonical");

            // 2. Client software MAY skip an update of the forkchoice state and MUST NOT begin a
            //    payload build process if `forkchoiceState.headBlockHash` references a `VALID`
            //    ancestor of the head of canonical chain, i.e. the ancestor passed payload
            //    validation process and deemed `VALID`. In the case of such an event, client
            //    software MUST return `{payloadStatus: {status: VALID, latestValidHash:
            //    forkchoiceState.headBlockHash, validationError: null}, payloadId: null}`

            // the head block is already canonical, so we're not triggering a payload job and can
            // return right away
            return Ok(OnForkChoiceUpdated::valid(
                PayloadStatus::from_status(PayloadStatusEnum::Valid)
                    .with_latest_valid_hash(state.head_block_hash),
            ));
        }

        // 4. we don't have the block to perform the update
        // we assume the FCU is valid and at least the head is missing,
        // so we need to start syncing to it
        //
        // find the appropriate target to sync to, if we don't have the safe block hash then we
        // start syncing to the safe block via backfill first
        let target = if self.forkchoice_state_tracker.is_empty() &&
            // check that safe block is valid and missing
            !state.safe_block_hash.is_zero() &&
            self.provider.storage.header_by_hash(state.safe_block_hash).is_none()
        {
            debug!(target: "engine::tree", "missing safe block on initial FCU, downloading safe block");
            state.safe_block_hash
        } else {
            state.head_block_hash
        };

        // todo: it's not downloading rn
        trace!(target: "engine::tree", %target, "downloading missing block");

        Ok(OnForkChoiceUpdated::valid(PayloadStatus::from_status(
            PayloadStatusEnum::Syncing,
        )))
    }

    /// Returns the new chain for the given head.
    ///
    /// This also handles reorgs.
    ///
    /// Note: This does not update the tracked state and instead returns the new chain based on the
    /// given head.
    fn on_new_head(&self, new_head: B256) -> ProviderResult<Option<NewCanonicalChain>> {
        // get the executed new head block
        let Some(new_head_block) = self.provider.storage.header_by_hash(new_head) else {
            return Ok(None);
        };

        let new_head_number = new_head_block.number;
        let mut current_canonical_number = self.provider.storage.get_canonical_head().number;

        let mut new_chain = vec![new_head_block.clone()];
        let mut current_hash = new_head_block.parent_hash;
        let mut current_number = new_head_number - 1;

        // Walk back the new chain until we reach a block we know about
        //
        // This is only done for in-memory blocks, because we should not have persisted any blocks
        // that are _above_ the current canonical head.
        while current_number > current_canonical_number {
            if let Some(block) = self.provider.storage.header_by_hash(current_hash) {
                current_hash = block.parent_hash;
                current_number -= 1;
                new_chain.push(block);
            } else {
                warn!(target: "engine::tree", current_hash=?current_hash, "Sidechain block not found in TreeState");
                // This should never happen as we're walking back a chain that should connect to
                // the canonical chain
                return Ok(None);
            }
        }

        // If we have reached the current canonical head by walking back from the target, then we
        // know this represents an extension of the canonical chain.
        if current_hash == self.provider.storage.get_canonical_head().hash {
            new_chain.reverse();

            // Simple extension of the current chain
            return Ok(Some(NewCanonicalChain::Commit { new: new_chain }));
        }

        // We have a reorg. Walk back both chains to find the fork point.
        let mut old_chain = Vec::new();
        let mut old_hash = self.provider.storage.get_canonical_head().hash;
        // If the canonical chain is ahead of the new chain,
        // gather all blocks until new head number.
        while current_canonical_number > current_number {
            if let Some(block) = self.provider.storage.header_by_hash(old_hash) {
                old_chain.push(block.clone());
                old_hash = block.parent_hash;
                current_canonical_number -= 1;
            } else {
                // This shouldn't happen as we're walking back the canonical chain
                warn!(target: "engine::tree", current_hash=?old_hash, "Canonical block not found in TreeState");
                return Ok(None);
            }
        }

        // Both new and old chain pointers are now at the same height.
        debug_assert_eq!(current_number, current_canonical_number);

        // Walk both chains from specified hashes at same height until
        // a common ancestor (fork block) is reached.
        while old_hash != current_hash {
            if let Some(block) = self.provider.storage.header_by_hash(old_hash) {
                old_hash = block.parent_hash;
                old_chain.push(block);
            } else {
                // This shouldn't happen as we're walking back the canonical chain
                warn!(target: "engine::tree", current_hash=?old_hash, "Canonical block not found in TreeState");
                return Ok(None);
            }

            if let Some(block) = self.provider.storage.header_by_hash(current_hash) {
                current_hash = block.parent_hash;
                new_chain.push(block);
            } else {
                // This shouldn't happen as we've already walked this path
                warn!(target: "engine::tree", invalid_hash=?current_hash, "New chain block not found in TreeState");
                return Ok(None);
            }
        }
        new_chain.reverse();
        old_chain.reverse();

        Ok(Some(NewCanonicalChain::Reorg {
            new: new_chain,
            old: old_chain,
        }))
    }

    /// Invoked when we the canonical chain has been updated.
    ///
    /// This is invoked on a valid forkchoice update, or if we can make the target block canonical.
    fn on_canonical_chain_update(&mut self, chain_update: NewCanonicalChain) {
        trace!(target: "engine::tree", new_blocks = %chain_update.new_block_count(), reorged_blocks =  %chain_update.reorged_block_count(), "applying new chain update");

        // update the tracked canonical head
        self.provider.storage.set_canonical_head(NumHash {
            number: chain_update.tip().number,
            hash: chain_update.tip().hash_slow(),
        });

        let chain = chain_update.clone();

        // reinsert any missing reorged blocks
        if let NewCanonicalChain::Reorg { new, old } = &chain_update {
            let new_first = new.first().map(|first| NumHash {
                number: first.number,
                hash: first.hash_slow(),
            });
            let old_first = old.first().map(|first| NumHash {
                number: first.number,
                hash: first.hash_slow(),
            });
            trace!(target: "engine::tree", ?new_first, ?old_first, "Reorg detected, new and old first blocks");

            self.reinsert_reorged_blocks(new.clone());
            // Try reinserting the reorged canonical chain. This is only possible if we have
            // `persisted_trie_updatess` for those blocks.
            self.reinsert_reorged_blocks(old.to_vec());
        }

        // update the tracked in-memory state with the new chain
        self.provider.storage.update_chain(chain_update);
        self.provider.storage.set_canonical_head(NumHash {
            number: chain.tip().number,
            hash: chain.tip().hash_slow(),
        });
    }

    /// This reinserts any blocks in the new chain that do not already exist in the tree
    fn reinsert_reorged_blocks(&mut self, new_chain: Vec<Header>) {
        for header in new_chain {
            if self
                .provider
                .storage
                .header_by_hash(header.hash_slow())
                .is_none()
            {
                trace!(target: "engine::tree", num=?header.number, hash=?header.hash_slow(), "Reinserting header into tree state");
                self.provider.storage.insert_header(header);
            }
        }
    }

    /// Validates the payload attributes with respect to the header and fork choice state.
    ///
    /// Note: At this point, the fork choice update is considered to be VALID, however, we can still
    /// return an error if the payload attributes are invalid.
    fn process_payload_attributes(
        &self,
        attrs: <EthEngineTypes as PayloadTypes>::PayloadAttributes,
        head: Header,
        state: ForkchoiceState,
        version: EngineApiMessageVersion,
    ) -> OnForkChoiceUpdated {
        if let Err(err) =
            EngineValidator::<EthEngineTypes>::validate_payload_attributes_against_header(
                &self.engine_validator,
                &attrs,
                &head,
            )
        {
            warn!(target: "engine::tree", %err, ?head, "Invalid payload attributes");
            return OnForkChoiceUpdated::invalid_payload_attributes();
        }

        // 8. Client software MUST begin a payload build process building on top of
        //    forkchoiceState.headBlockHash and identified via buildProcessId value if
        //    payloadAttributes is not null and the forkchoice state has been updated successfully.
        //    The build process is specified in the Payload building section.
        match PayloadBuilderAttributes::try_new(state.head_block_hash, attrs, version as u8) {
            Ok(attributes) => {
                // send the payload to the builder and return the receiver for the pending payload
                // id, initiating payload job is handled asynchronously
                let pending_payload_id = self.payload_builder.send_new_payload(attributes);

                // Client software MUST respond to this method call in the following way:
                // {
                //      payloadStatus: {
                //          status: VALID,
                //          latestValidHash: forkchoiceState.headBlockHash,
                //          validationError: null
                //      },
                //      payloadId: buildProcessId
                // }
                //
                // if the payload is deemed VALID and the build process has begun.
                OnForkChoiceUpdated::updated_with_pending_payload_id(
                    PayloadStatus::new(PayloadStatusEnum::Valid, Some(state.head_block_hash)),
                    pending_payload_id,
                )
            }
            Err(_) => OnForkChoiceUpdated::invalid_payload_attributes(),
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
        payload: ExecutionPayload,
        sidecar: ExecutionPayloadSidecar,
    ) -> Result<PayloadStatus, EngineError> {
        let block_number = payload.block_number();
        let block_hash = payload.block_hash();
        info!(target: "ress::engine", %block_hash, block_number, "👋 new payload");

        // ===================== Validation =====================
        // todo: invalid_ancestors check
        let parent_hash = payload.parent_hash();
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
