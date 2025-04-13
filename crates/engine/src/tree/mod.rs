use alloy_consensus::BlockHeader;
use alloy_eips::BlockNumHash;
use alloy_primitives::{B256, U256};
use alloy_rpc_types_engine::{
    ExecutionData, ForkchoiceState, PayloadAttributes, PayloadStatus, PayloadStatusEnum,
    PayloadValidationError,
};
use metrics::Gauge;
use ress_provider::RessProvider;
use reth_chain_state::ExecutedBlockWithTrieUpdates;
use reth_chainspec::ChainSpec;
use reth_consensus::{Consensus, ConsensusError, HeaderValidator};
use reth_engine_tree::tree::{
    error::{InsertBlockError, InsertBlockErrorKind, InsertBlockFatalError},
    InvalidHeaderCache,
};
use reth_errors::{ProviderError, RethResult};
use reth_ethereum_engine_primitives::{EthEngineTypes, EthPayloadBuilderAttributes};
use reth_metrics::Metrics;
use reth_node_api::{
    BeaconConsensusEngineEvent, EngineApiMessageVersion, EngineValidator, ForkchoiceStateTracker,
    OnForkChoiceUpdated, PayloadBuilderAttributes, PayloadValidator,
};
use reth_node_ethereum::{consensus::EthBeaconConsensus, EthereumEngineValidator};
use reth_primitives::{Block, Header, RecoveredBlock, SealedBlock};
use reth_primitives_traits::SealedHeader;
use reth_provider::ExecutionOutcome;
use reth_ress_protocol::RLPExecutionWitness;
use stateless_validate::stateless_block_validation;
use std::{sync::Arc, time::Instant};
use tokio::sync::mpsc;
use tracing::*;

mod outcome;
pub use outcome::*;

mod block_buffer;
pub use block_buffer::BlockBuffer;

/// State root computation.
pub mod root;

mod stateless_validate;

/// Consensus engine tree for storing and validating blocks as well as advancing the chain.
#[derive(Debug)]
pub struct EngineTree {
    /// Ress provider.
    pub(crate) provider: RessProvider,
    /// Consensus.
    pub(crate) consensus: EthBeaconConsensus<ChainSpec>,
    /// Engine validator.
    pub(crate) engine_validator: EthereumEngineValidator,

    /// Current canonical head.
    pub(crate) canonical_head: BlockNumHash,
    /// Tracks the forkchoice state updates received by the CL.
    pub(crate) forkchoice_state_tracker: ForkchoiceStateTracker,
    /// Pending block buffer.
    pub(crate) block_buffer: BlockBuffer<Block>,
    /// Invalid headers.
    pub(crate) invalid_headers: InvalidHeaderCache,

    /// Outgoing events that are emitted to the handler.
    events_sender: mpsc::UnboundedSender<BeaconConsensusEngineEvent>,
    /// Metrics.
    metrics: EngineTreeMetrics,
}

impl EngineTree {
    /// Create new engine tree.
    pub fn new(
        provider: RessProvider,
        consensus: EthBeaconConsensus<ChainSpec>,
        engine_validator: EthereumEngineValidator,
        events_sender: mpsc::UnboundedSender<BeaconConsensusEngineEvent>,
    ) -> Self {
        let canonical_head = provider.chain_spec().genesis_header().num_hash_slow();
        Self {
            provider,
            consensus,
            engine_validator,
            canonical_head,
            forkchoice_state_tracker: ForkchoiceStateTracker::default(),
            block_buffer: BlockBuffer::new(256),
            invalid_headers: InvalidHeaderCache::new(256),
            events_sender,
            metrics: EngineTreeMetrics::default(),
        }
    }

    /// Returns true if the given hash is the last received sync target block.
    ///
    /// See [`ForkchoiceStateTracker::sync_target_state`]
    fn is_sync_target_head(&self, block_hash: B256) -> bool {
        if let Some(target) = self.forkchoice_state_tracker.sync_target_state() {
            return target.head_block_hash == block_hash
        }
        false
    }

    /// Returns true if the block is either persisted or buffered.
    fn is_block_persisted_or_buffered(&self, block_hash: &B256) -> bool {
        self.provider.sealed_header(block_hash).is_some() ||
            self.block_buffer.blocks.contains_key(block_hash)
    }

    /// Determines if the given block is part of a fork.
    fn is_fork(&self, target_hash: B256) -> bool {
        // verify that the given hash is not part of an extension of the canon chain.
        let canonical_head = self.canonical_head;
        let mut current_hash = target_hash;
        while let Some(current_block) = self.provider.sealed_header(&current_hash) {
            if current_block.hash() == canonical_head.hash {
                return false
            }
            // We already passed the canonical head
            if current_block.number <= canonical_head.number {
                break
            }
            current_hash = current_block.parent_hash;
        }

        true
    }

    /// Emits an outgoing event to the engine.
    pub fn emit_event(&mut self, event: BeaconConsensusEngineEvent) {
        let _ = self.events_sender.send(event).inspect_err(
            |err| error!(target: "engine::tree", ?err, "Failed to send internal event"),
        );
    }

    /// Set canonical head.
    pub fn set_canonical_head(&mut self, head: BlockNumHash) {
        self.canonical_head = head;
        self.metrics.head.set(head.number as f64);
    }

    /// Handle forkchoice update.
    pub fn on_forkchoice_updated(
        &mut self,
        state: ForkchoiceState,
        payload_attrs: Option<PayloadAttributes>,
        version: EngineApiMessageVersion,
    ) -> RethResult<TreeOutcome<OnForkChoiceUpdated>> {
        // ===================== Validation =====================
        if let Some(on_updated) = self.pre_validate_forkchoice_update(state) {
            return Ok(TreeOutcome::new(on_updated));
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
        if self.canonical_head.hash == state.head_block_hash {
            trace!(target: "ress::engine", "fcu head hash is already canonical");

            // update the safe and finalized blocks and ensure their values are valid
            if let Err(outcome) = self.ensure_consistent_forkchoice_state(state) {
                // safe or finalized hashes are invalid
                return Ok(TreeOutcome::new(outcome));
            }

            // we still need to process payload attributes if the head is already canonical
            if let Some(attr) = payload_attrs {
                let head = self.canonical_head;
                let tip = self.provider.sealed_header(&head.hash).ok_or_else(|| {
                    // If we can't find the canonical block, then something is wrong and we need
                    // to return an error
                    ProviderError::HeaderNotFound(state.head_block_hash.into())
                })?;
                if let Some(status) = self.validate_payload_attributes(attr, &tip, state, version) {
                    return Ok(TreeOutcome::new(status)); // payload attributes are invalid
                }
            }

            // the head block is already canonical
            return Ok(TreeOutcome::new(OnForkChoiceUpdated::valid(PayloadStatus::new(
                PayloadStatusEnum::Valid,
                Some(state.head_block_hash),
            ))));
        }

        // 2. ensure we can apply a new chain update for the head block
        if let Some(chain_update) = self.on_new_head(state.head_block_hash) {
            let tip = chain_update.tip().clone();
            self.on_canonical_chain_update(chain_update);

            // update the safe and finalized blocks and ensure their values are valid
            if let Err(outcome) = self.ensure_consistent_forkchoice_state(state) {
                // safe or finalized hashes are invalid
                return Ok(TreeOutcome::new(outcome));
            }

            // clean up blocks from in-memory state and buffer
            self.provider.on_finalized(&state.finalized_block_hash);
            if let Some(finalized_number) = self.provider.block_number(&state.finalized_block_hash)
            {
                self.block_buffer.evict_old_blocks(finalized_number);
            }

            if let Some(attr) = payload_attrs {
                if let Some(status) = self.validate_payload_attributes(attr, &tip, state, version) {
                    return Ok(TreeOutcome::new(status)); // payload attributes are invalid
                }
            }

            return Ok(TreeOutcome::new(OnForkChoiceUpdated::valid(PayloadStatus::new(
                PayloadStatusEnum::Valid,
                Some(state.head_block_hash),
            ))));
        }

        // 3. check if the head is already part of the canonical chain
        if let Some(header) = self.provider.sealed_header(&state.head_block_hash) {
            debug!(target: "ress::engine", head = header.number, "fcu head block is already canonical");

            // 2. Client software MAY skip an update of the forkchoice state and MUST NOT begin a
            //    payload build process if `forkchoiceState.headBlockHash` references a `VALID`
            //    ancestor of the head of canonical chain, i.e. the ancestor passed payload
            //    validation process and deemed `VALID`. In the case of such an event, client
            //    software MUST return `{payloadStatus: {status: VALID, latestValidHash:
            //    forkchoiceState.headBlockHash, validationError: null}, payloadId: null}`

            // the head block is already canonical, so we're not triggering a payload job and can
            // return right away
            return Ok(TreeOutcome::new(OnForkChoiceUpdated::valid(PayloadStatus::new(
                PayloadStatusEnum::Valid,
                Some(state.head_block_hash),
            ))));
        }

        // 4. we don't have the block to perform the update
        // we assume the FCU is valid and at least the head is missing,
        // so we need to start syncing to it
        //
        // find the appropriate target to sync to, if we don't have the safe block hash then we
        // start syncing to the safe block via backfill first
        let mut outcome = TreeOutcome::new(OnForkChoiceUpdated::valid(PayloadStatus::from_status(
            PayloadStatusEnum::Syncing,
        )));

        // Find the appropriate target to sync to.
        if !state.finalized_block_hash.is_zero() &&
            !self.is_block_persisted_or_buffered(&state.finalized_block_hash)
        {
            debug!(target: "ress::engine", finalized = %state.finalized_block_hash, "Missing finalized block on FCU, downloading");
            outcome = outcome.with_event(TreeEvent::download_finalized(state.finalized_block_hash));
        } else {
            let target = if !state.safe_block_hash.is_zero() &&
                !self.is_block_persisted_or_buffered(&state.safe_block_hash)
            {
                state.safe_block_hash
            } else {
                state.head_block_hash
            };
            let target = self.lowest_buffered_ancestor_or(target);
            if target != state.finalized_block_hash && !self.is_block_persisted_or_buffered(&target)
            {
                debug!(target: "ress::engine", %target, "Downloading missing ancestor on FCU");
                outcome = outcome.with_event(TreeEvent::download_block(target));
            }
        }

        Ok(outcome)
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
            !self.provider.is_hash_canonical(&state.finalized_block_hash)
        {
            return Err(OnForkChoiceUpdated::invalid_state());
        }
        if !state.safe_block_hash.is_zero() &&
            !self.provider.is_hash_canonical(&state.safe_block_hash)
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
        let new_head_block = self.provider.sealed_header(&new_head)?;

        let mut current_canonical_number = self.canonical_head.number;
        let (mut current_number, mut current_hash) =
            new_head_block.parent_num_hash().into_components();
        let mut new_chain = vec![new_head_block];

        // Walk back the new chain until we reach a block we know about
        //
        // This is only done for in-memory blocks, because we should not have persisted any blocks
        // that are _above_ the current canonical head.
        while current_number > current_canonical_number {
            if let Some(header) = self.provider.sealed_header(&current_hash) {
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
        if current_hash == self.canonical_head.hash {
            new_chain.reverse();
            return Some(NewCanonicalChain::Commit { new: new_chain });
        }

        // We have a reorg. Walk back both chains to find the fork point.
        let mut old_chain = Vec::new();
        let mut old_hash = self.canonical_head.hash;

        // If the canonical chain is ahead of the new chain,
        // gather all blocks until new head number.
        while current_canonical_number > current_number {
            if let Some(header) = self.provider.sealed_header(&old_hash) {
                old_hash = header.parent_hash;
                current_canonical_number -= 1;
                old_chain.push(header);
            } else {
                // This shouldn't happen as we're walking back the canonical chain
                warn!(target: "ress::engine", current_hash=?old_hash, "Canonical block not found in TreeState");
                return None;
            }
        }

        // Both new and old chain pointers are now at the same height.
        debug_assert_eq!(current_number, current_canonical_number);

        // Walk both chains from specified hashes at same height until
        // a common ancestor (fork block) is reached.
        while old_hash != current_hash {
            if let Some(header) = self.provider.sealed_header(&old_hash) {
                old_hash = header.parent_hash;
                old_chain.push(header);
            } else {
                // This shouldn't happen as we're walking back the canonical chain
                warn!(target: "ress::engine", current_hash=?old_hash, "Canonical block not found in TreeState");
                return None;
            }

            if let Some(header) = self.provider.sealed_header(&current_hash) {
                current_hash = header.parent_hash;
                new_chain.push(header);
            } else {
                // This shouldn't happen as we've already walked this path
                warn!(target: "ress::engine", invalid_hash=?current_hash, "New chain block not found in TreeState");
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
            warn!(target: "ress::engine", %err, ?head, "Invalid payload attributes");
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

    /// Attempts to make the given target canonical.
    ///
    /// This will update the tracked canonical in memory state and do the necessary housekeeping.
    pub fn make_canonical(&mut self, target: B256) {
        if let Some(chain_update) = self.on_new_head(target) {
            self.on_canonical_chain_update(chain_update);
        }
    }

    /// Invoked when we the canonical chain has been updated.
    ///
    /// This is invoked on a valid forkchoice update, or if we can make the target block canonical.
    fn on_canonical_chain_update(&mut self, update: NewCanonicalChain) {
        trace!(target: "ress::engine", new_blocks = %update.new_block_count(), reorged_blocks = %update.reorged_block_count(), "applying new chain update");
        let start = Instant::now();
        let tip = update.tip().clone();
        self.set_canonical_head(tip.num_hash());
        let (new, old) = match update {
            NewCanonicalChain::Commit { new } => (new, Vec::new()),
            NewCanonicalChain::Reorg { new, old } => (new, old),
        };
        self.provider.on_chain_update(new, old);
        self.emit_event(BeaconConsensusEngineEvent::CanonicalChainCommitted(
            Box::new(tip),
            start.elapsed(),
        ));
    }

    /// Handler for new payload message.
    pub fn on_new_payload(
        &mut self,
        payload: ExecutionData,
        maybe_witness: Option<RLPExecutionWitness>,
    ) -> Result<TreeOutcome<PayloadStatus>, InsertBlockFatalError> {
        let parent_hash = payload.payload.parent_hash();

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
                return Ok(TreeOutcome::new(status));
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
            return Ok(TreeOutcome::new(status));
        }

        // TODO:
        // let recovered = match block.clone().try_recover() {
        //     Ok(block) => block,
        //     Err(_error) => {
        //         return Ok(TreeOutcome::new(self.on_insert_block_error(InsertBlockError::new(
        //             block.into_sealed_block(),
        //             InsertBlockErrorKind::Provider(ProviderError::SenderRecoveryError),
        //         ))?))
        //     }
        // };

        let mut maybe_tree_event = None;
        let status = match self.insert_block(block.clone(), maybe_witness) {
            Ok(InsertPayloadOk::Inserted(BlockStatus::Valid) | InsertPayloadOk::AlreadySeen) => {
                self.try_connect_buffered_blocks(block_hash)?;
                // if the block is valid and it is the sync target head, make it canonical
                if self.is_sync_target_head(block_hash) {
                    maybe_tree_event = Some(TreeEvent::make_canonical(block_hash));
                }
                PayloadStatus::new(PayloadStatusEnum::Valid, Some(block_hash))
            }
            Ok(InsertPayloadOk::Inserted(BlockStatus::Disconnected {
                missing_ancestor, ..
            })) => {
                // not known to be invalid, but we don't know anything else
                maybe_tree_event = Some(TreeEvent::download_block(missing_ancestor.hash));
                PayloadStatus::new(PayloadStatusEnum::Syncing, None)
            }
            Ok(InsertPayloadOk::Inserted(BlockStatus::NoWitness)) => {
                // we don't have a witness to validate the payload
                maybe_tree_event = Some(TreeEvent::download_witness(block_hash));
                PayloadStatus::new(PayloadStatusEnum::Syncing, None)
            }
            Err(kind) => {
                self.on_insert_block_error(InsertBlockError::new(block.into_sealed_block(), kind))?
            }
        };

        let mut outcome = TreeOutcome::new(status);
        if let Some(event) = maybe_tree_event {
            outcome = outcome.with_event(event);
        }
        Ok(outcome)
    }

    /// Invoked with a block downloaded from the network
    ///
    /// Returns an event with the appropriate action to take, such as:
    ///  - download more missing blocks
    ///  - try to canonicalize the target if the `block` is the tracked target (head) block.
    pub fn on_downloaded_block(
        &mut self,
        block: RecoveredBlock<Block>,
        witness: RLPExecutionWitness,
    ) -> Result<TreeOutcome<PayloadStatus>, InsertBlockFatalError> {
        let block_num_hash = block.num_hash();
        let lowest_buffered_ancestor = self.lowest_buffered_ancestor_or(block_num_hash.hash);
        if let Some(status) =
            self.check_invalid_ancestor_with_head(lowest_buffered_ancestor, block_num_hash.hash)
        {
            return Ok(TreeOutcome::new(status))
        }

        // try to append the block
        let block_hash = block.hash();
        let mut maybe_tree_event = None;
        let status = match self.insert_block(block.clone(), Some(witness)) {
            Ok(InsertPayloadOk::Inserted(BlockStatus::Valid)) => {
                if self.is_sync_target_head(block_num_hash.hash) {
                    trace!(target: "ress::engine", block = ?block_num_hash, "appended downloaded sync target block");
                    // we just inserted the current sync target block, we can try to make it
                    // canonical
                    maybe_tree_event = Some(TreeEvent::make_canonical(block_num_hash.hash));
                }
                trace!(target: "ress::engine", block = ?block_num_hash, "appended downloaded block");
                self.try_connect_buffered_blocks(block_num_hash.hash)?;
                PayloadStatus::new(PayloadStatusEnum::Valid, Some(block_hash))
            }
            Ok(InsertPayloadOk::AlreadySeen) => {
                trace!(target: "ress::engine", "Downloaded block already executed");
                PayloadStatus::new(PayloadStatusEnum::Valid, Some(block_hash))
            }
            Ok(InsertPayloadOk::Inserted(BlockStatus::Disconnected {
                missing_ancestor, ..
            })) => {
                // block is not connected to the canonical head, we need to download
                // its missing branch first
                maybe_tree_event = Some(TreeEvent::download_block(missing_ancestor.hash));
                PayloadStatus::new(PayloadStatusEnum::Syncing, None)
            }
            Ok(InsertPayloadOk::Inserted(BlockStatus::NoWitness)) => {
                // we don't have a witness to validate the payload
                maybe_tree_event = Some(TreeEvent::download_witness(block_hash));
                PayloadStatus::new(PayloadStatusEnum::Syncing, None)
            }
            Err(kind) => {
                debug!(target: "ress::engine", error = %kind, "Failed to insert downloaded block");
                self.on_insert_block_error(InsertBlockError::new(block.into_sealed_block(), kind))?
            }
        };

        let mut outcome = TreeOutcome::new(status);
        if let Some(event) = maybe_tree_event {
            outcome = outcome.with_event(event);
        }
        Ok(outcome)
    }

    /// Attempts to connect any buffered blocks that are connected to the given parent hash.
    fn try_connect_buffered_blocks(
        &mut self,
        parent_hash: B256,
    ) -> Result<(), InsertBlockFatalError> {
        let blocks = self.block_buffer.remove_block_with_children(parent_hash);

        if blocks.is_empty() {
            // nothing to append
            return Ok(())
        }

        let now = Instant::now();
        let block_count = blocks.len();
        for (child, witness) in blocks {
            let child_num_hash = child.num_hash();
            match self.insert_block(child.clone(), Some(witness)) {
                Ok(result) => {
                    debug!(target: "engine::tree", child = ?child_num_hash, ?result, "connected buffered block");
                    if self.is_sync_target_head(child_num_hash.hash) &&
                        matches!(result, InsertPayloadOk::Inserted(BlockStatus::Valid))
                    {
                        self.make_canonical(child_num_hash.hash);
                    }
                }
                Err(kind) => {
                    debug!(target: "engine::tree", error = ?kind, "failed to connect buffered block to tree");
                    if let Err(fatal) = self.on_insert_block_error(InsertBlockError::new(
                        child.into_sealed_block(),
                        kind,
                    )) {
                        warn!(target: "engine::tree", %fatal, "Fatal error occurred while connecting buffered blocks");
                        return Err(fatal)
                    }
                }
            }
        }

        debug!(target: "engine::tree", elapsed = ?now.elapsed(), %block_count, "connected buffered blocks");
        Ok(())
    }

    /// Insert block into the tree.
    pub fn insert_block(
        &mut self,
        block: RecoveredBlock<Block>,
        maybe_witness: Option<RLPExecutionWitness>,
    ) -> Result<InsertPayloadOk, InsertBlockErrorKind> {
        let start = Instant::now();
        let block_num_hash = block.num_hash();
        debug!(target: "ress::engine", block=?block_num_hash, parent_hash = %block.parent_hash, state_root = %block.state_root, "Inserting new block into tree");

        if self.provider.sealed_header(block.hash_ref()).is_some() {
            return Ok(InsertPayloadOk::AlreadySeen);
        }

        trace!(target: "ress::engine", block=?block_num_hash, "Validating block consensus");
        self.validate_block(&block)?;

        let Some(parent) = self.provider.sealed_header(&block.parent_hash) else {
            // we don't have the state required to execute this block, buffering it and find the
            // missing parent block
            let missing_ancestor = self
                .block_buffer
                .lowest_ancestor(&block.parent_hash)
                .map(|block| block.parent_num_hash())
                .unwrap_or_else(|| block.parent_num_hash());
            trace!(target: "ress::engine", block=?block_num_hash, ?missing_ancestor, has_witness=maybe_witness.is_some(), "Block has missing ancestor");
            if let Some(witness) = maybe_witness {
                self.block_buffer.insert_witness(block.hash(), witness);
            }
            self.block_buffer.insert_block(block);
            return Ok(InsertPayloadOk::Inserted(BlockStatus::Disconnected {
                head: self.canonical_head,
                missing_ancestor,
            }));
        };
        let parent_header = parent.clone(); // TODO: the next line creates a SealedHeader<SealedHead> so I'm cloning it here to use
                                            // later

        let parent = SealedHeader::new(parent, block.parent_hash);
        if let Err(error) =
            self.consensus.validate_header_against_parent(block.sealed_header(), &parent)
        {
            error!(target: "ress::engine", %error, "Failed to validate header against parent");
            return Err(error.into());
        }

        let Some(execution_witness) = maybe_witness else {
            self.block_buffer.insert_block(block);
            trace!(target: "ress::engine", block = ?block_num_hash, "Block has missing witness");
            return Ok(InsertPayloadOk::Inserted(BlockStatus::NoWitness))
        };

        // Insert function here

        let output = stateless_block_validation(
            self.provider.chain_spec().clone(),
            parent_header.clone(),
            block.clone(),
            execution_witness,
        )?;

        // ===================== Update Node State =====================
        self.provider.insert_block(block.clone());

        // Emit event.
        let executed = ExecutedBlockWithTrieUpdates::new(
            Arc::new(block),
            Arc::new(ExecutionOutcome::from((output, block_num_hash.number))),
            Default::default(), // not needed
            Default::default(), // not needed
        );
        let elapsed = start.elapsed();
        let event = if self.is_fork(block_num_hash.hash) {
            BeaconConsensusEngineEvent::ForkBlockAdded(executed, elapsed)
        } else {
            BeaconConsensusEngineEvent::CanonicalBlockAdded(executed, elapsed)
        };
        self.emit_event(event);

        debug!(target: "ress::engine", block=?block_num_hash, ?elapsed, "Finished inserting block");
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
        if let Some(parent) = self.provider.sealed_header(&parent_hash) {
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
        if self.provider.sealed_header(&parent_hash).is_some() {
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
            if current_block.is_none() && self.provider.sealed_header(&current_hash).is_some() {
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

/// Block inclusion can be valid, accepted, or invalid. Invalid blocks are returned as an error
/// variant.
///
/// If we don't know the block's parent, we return `Disconnected`, as we can't claim that the block
/// is valid or not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockStatus {
    /// The block is valid and block extends canonical chain.
    Valid,
    /// The block may be valid and has an unknown missing ancestor.
    Disconnected {
        /// Current canonical head.
        head: BlockNumHash,
        /// The lowest ancestor block that is not connected to the canonical chain.
        missing_ancestor: BlockNumHash,
    },
    /// The block has no witness.
    NoWitness,
}

/// How a payload was inserted if it was valid.
///
/// If the payload was valid, but has already been seen, [`InsertPayloadOk::AlreadySeen`] is
/// returned, otherwise [`InsertPayloadOk::Inserted`] is returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertPayloadOk {
    /// The payload was valid, but we have already seen it.
    AlreadySeen,
    /// The payload was valid and inserted into the tree.
    Inserted(BlockStatus),
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

#[derive(Metrics)]
#[metrics(scope = "engine.tree")]
struct EngineTreeMetrics {
    /// Canonical head block number.
    head: Gauge,
}
