use crate::{
    downloader::{DownloadOutcome, EngineDownloader},
    tree::{DownloadRequest, EngineTree, TreeAction, TreeEvent},
};
use alloy_primitives::B256;
use alloy_rpc_types_engine::{PayloadStatus, PayloadStatusEnum};
use futures::{FutureExt, StreamExt};
use ress_network::RessNetworkHandle;
use ress_provider::RessProvider;
use reth_chainspec::ChainSpec;
use reth_engine_tree::tree::error::InsertBlockFatalError;
use reth_errors::ProviderError;
use reth_ethereum_engine_primitives::EthereumEngineValidator;
use reth_node_api::{BeaconEngineMessage, BeaconOnNewPayloadError, ExecutionPayload};
use reth_node_ethereum::{consensus::EthBeaconConsensus, EthEngineTypes};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::{
    sync::{mpsc::UnboundedReceiver, oneshot},
    time::Sleep,
};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::*;

/// Ress consensus engine.
#[allow(missing_debug_implementations)]
pub struct ConsensusEngine {
    tree: EngineTree,
    downloader: EngineDownloader,
    from_beacon_engine: UnboundedReceiverStream<BeaconEngineMessage<EthEngineTypes>>,
    parked_payload_timeout: Duration,
    parked_payload: Option<ParkedPayload>,
}

impl ConsensusEngine {
    /// Initialize consensus engine.
    pub fn new(
        provider: RessProvider,
        consensus: EthBeaconConsensus<ChainSpec>,
        engine_validator: EthereumEngineValidator,
        network: RessNetworkHandle,
        from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
    ) -> Self {
        Self {
            tree: EngineTree::new(provider, consensus.clone(), engine_validator),
            downloader: EngineDownloader::new(network, consensus),
            from_beacon_engine: UnboundedReceiverStream::from(from_beacon_engine),
            parked_payload_timeout: Duration::from_secs(1),
            parked_payload: None,
        }
    }

    fn on_maybe_tree_event(&mut self, maybe_event: Option<TreeEvent>) {
        if let Some(event) = maybe_event {
            self.on_tree_event(event);
        }
    }

    fn on_tree_event(&mut self, event: TreeEvent) {
        match event {
            TreeEvent::Download(DownloadRequest::Block { block_hash }) => {
                self.downloader.download_full_block(block_hash);
                if !self.tree.block_buffer.witnesses.contains_key(&block_hash) {
                    self.downloader.download_witness(block_hash);
                }
            }
            TreeEvent::Download(DownloadRequest::Witness { block_hash }) => {
                self.downloader.download_witness(block_hash);
            }
            TreeEvent::TreeAction(TreeAction::MakeCanonical { sync_target_head }) => {
                self.tree.make_canonical(sync_target_head);
            }
        }
    }

    fn on_download_outcome(
        &mut self,
        outcome: DownloadOutcome,
    ) -> Result<(), InsertBlockFatalError> {
        let mut blocks = Vec::new();
        match outcome {
            DownloadOutcome::FullBlocksRange(blocks) => {
                trace!(target: "ress::engine", first = ?blocks.last().map(|b| b.num_hash()), last = ?blocks.first().map(|b| b.num_hash()), "Downloaded block range");
                // TODO:
            }
            DownloadOutcome::HeadersRange(headers) => {
                trace!(target: "ress::engine", first = ?headers.last().map(|b| b.num_hash()), last = ?headers.first().map(|b| b.num_hash()), "Downloaded block range");
                // TODO:
            }
            DownloadOutcome::FullBlock(block) => {
                let block_num_hash = block.num_hash();
                trace!(target: "ress::engine", ?block_num_hash, "Downloaded block");
                let recovered = match block.try_recover() {
                    Ok(block) => block,
                    Err(_error) => {
                        debug!(target: "ress::engine", ?block_num_hash, "Error recovering downloaded block");
                        return Ok(())
                    }
                };
                self.tree.block_buffer.insert_block(recovered);
                blocks = self.tree.block_buffer.remove_block_with_children(block_num_hash.hash);
            }
            DownloadOutcome::Witness(block_hash, witness) => {
                let code_hashes = witness.bytecode_hashes().clone();
                let missing_code_hashes =
                    self.tree.provider.missing_code_hashes(code_hashes).map_err(|error| {
                        InsertBlockFatalError::Provider(ProviderError::Database(error))
                    })?;
                self.tree.block_buffer.insert_witness(
                    block_hash,
                    witness,
                    missing_code_hashes.clone(),
                );
                trace!(target: "ress::engine", %block_hash, missing_bytecodes_len = missing_code_hashes.len(), "Downloaded witness");
                if missing_code_hashes.is_empty() {
                    blocks = self.tree.block_buffer.remove_block_with_children(block_hash);
                } else {
                    for code_hash in missing_code_hashes {
                        self.downloader.download_bytecode(code_hash);
                    }
                }
            }
            DownloadOutcome::Bytecode(code_hash, bytecode) => {
                trace!(target: "ress::engine", %code_hash, "Downloaded bytecode");
                match self.tree.provider.insert_bytecode(code_hash, bytecode) {
                    Ok(()) => {
                        blocks =
                            self.tree.block_buffer.remove_blocks_with_received_bytecode(code_hash);
                    }
                    Err(error) => {
                        error!(target: "ress::engine", %error, "Failed to insert the bytecode");
                    }
                };
            }
        };
        for (block, witness) in blocks {
            let block_number = block.number;
            let block_hash = block.hash();
            trace!(target: "ress::engine", block_number, %block_hash, "Inserting block after download");
            let mut result = self
                .tree
                .on_downloaded_block(block, witness)
                .map_err(BeaconOnNewPayloadError::internal);
            match &mut result {
                Ok(outcome) => {
                    self.on_maybe_tree_event(outcome.event.take());
                }
                Err(error) => {
                    error!(target: "ress::engine", block_number, %block_hash, %error, "Error inserting downloaded block");
                }
            };
            if self.parked_payload.as_ref().is_some_and(|parked| parked.block_hash == block_hash) {
                let parked = self.parked_payload.take().unwrap();
                trace!(target: "ress::engine", block_number, %block_hash, elapsed = ?parked.parked_at.elapsed(), "Sending response for parked payload");
                if let Err(error) = parked.tx.send(result.map(|o| o.outcome)) {
                    error!(target: "ress::engine", block_number, %block_hash, ?error, "Failed to send payload status");
                }
            }
        }
        Ok(())
    }

    fn on_engine_message(&mut self, message: BeaconEngineMessage<EthEngineTypes>) {
        match message {
            BeaconEngineMessage::NewPayload { payload, tx } => {
                let block_hash = payload.block_hash();
                let block_number = payload.block_number();
                let maybe_witness = self.tree.block_buffer.remove_witness(&payload.block_hash());
                debug!(target: "ress::engine", block_number, %block_hash, has_witness = maybe_witness.is_some(), "Inserting new payload");
                let mut result = self
                    .tree
                    .on_new_payload(payload, maybe_witness)
                    .map_err(BeaconOnNewPayloadError::internal);
                if let Ok(outcome) = &mut result {
                    if let Some(event) = outcome.event.take() {
                        self.on_tree_event(event.clone());
                        if let Some(block_hash) =
                            event.as_witness_download().filter(|_| outcome.outcome.is_syncing())
                        {
                            debug!(target: "ress::engine", block_number, %block_hash, "Parking payload due to missing witness");
                            self.parked_payload = Some(ParkedPayload::new(
                                block_hash,
                                tx,
                                self.parked_payload_timeout,
                            ));
                            return
                        }
                    }
                    self.on_maybe_tree_event(outcome.event.take());
                }
                let outcome_result = result.map(|o| o.outcome);
                debug!(target: "ress::engine", block_number, %block_hash, result = ?outcome_result, "Returning payload result");
                if let Err(error) = tx.send(outcome_result) {
                    error!(target: "ress::engine", ?error, "Failed to send payload status");
                }
            }
            BeaconEngineMessage::ForkchoiceUpdated { state, payload_attrs, tx, version } => {
                debug!(
                    target: "ress::engine",
                    head = %state.head_block_hash,
                    safe = %state.safe_block_hash,
                    finalized = %state.finalized_block_hash,
                    "Updating forkchoice state"
                );
                let mut result = self.tree.on_forkchoice_updated(state, payload_attrs, version);
                if let Ok(outcome) = &mut result {
                    // track last received forkchoice state
                    self.tree
                        .forkchoice_state_tracker
                        .set_latest(state, outcome.outcome.forkchoice_status());
                    self.on_maybe_tree_event(outcome.event.take());
                }
                let outcome_result = result.map(|o| o.outcome);
                debug!(target: "ress::engine", ?state, result = ?outcome_result, "Returning forkchoice update result");
                if let Err(error) = tx.send(outcome_result) {
                    error!(target: "ress::engine", ?error, "Failed to send forkchoice outcome");
                }
            }
            BeaconEngineMessage::TransitionConfigurationExchanged => {
                warn!(target: "ress::engine", "Received unsupported `TransitionConfigurationExchanged` message");
            }
        }
    }
}

impl Future for ConsensusEngine {
    type Output = Result<(), InsertBlockFatalError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            if let Poll::Ready(outcome) = this.downloader.poll(cx) {
                this.on_download_outcome(outcome)?;
                continue;
            }

            if let Some(parked) = &mut this.parked_payload {
                if parked.timeout.poll_unpin(cx).is_ready() {
                    let parked = this.parked_payload.take().unwrap();
                    warn!(target: "ress::engine", block_hash = %parked.block_hash, "Could not download missing payload data in time");
                    let status = PayloadStatus::from_status(PayloadStatusEnum::Syncing);
                    if let Err(error) = parked.tx.send(Ok(status)) {
                        error!(target: "ress::engine", ?error, "Failed to send parked payload status");
                    }
                } else {
                    return Poll::Pending
                }
            }

            if let Poll::Ready(Some(message)) = this.from_beacon_engine.poll_next_unpin(cx) {
                this.on_engine_message(message);
                continue;
            }

            return Poll::Pending
        }
    }
}

struct ParkedPayload {
    block_hash: B256,
    tx: oneshot::Sender<Result<PayloadStatus, BeaconOnNewPayloadError>>,
    parked_at: Instant,
    timeout: Pin<Box<Sleep>>,
}

impl ParkedPayload {
    fn new(
        block_hash: B256,
        tx: oneshot::Sender<Result<PayloadStatus, BeaconOnNewPayloadError>>,
        timeout: Duration,
    ) -> Self {
        Self {
            block_hash,
            tx,
            parked_at: Instant::now(),
            timeout: Box::pin(tokio::time::sleep(timeout)),
        }
    }
}
