use crate::{
    downloader::{DownloadOutcome, EngineDownloader},
    tree::{DownloadRequest, EngineTree, TreeAction, TreeEvent},
};
use futures::StreamExt;
use ress_network::RessNetworkHandle;
use ress_provider::provider::RessProvider;
use reth_chainspec::ChainSpec;
use reth_ethereum_engine_primitives::EthereumEngineValidator;
use reth_node_api::{BeaconEngineMessage, BeaconOnNewPayloadError};
use reth_node_ethereum::{consensus::EthBeaconConsensus, EthEngineTypes};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::*;

/// Ress consensus engine.
#[allow(missing_debug_implementations)]
pub struct ConsensusEngine {
    tree: EngineTree,
    downloader: EngineDownloader,
    from_beacon_engine: UnboundedReceiverStream<BeaconEngineMessage<EthEngineTypes>>,
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
        }
    }

    fn on_engine_message(&mut self, message: BeaconEngineMessage<EthEngineTypes>) {
        match message {
            BeaconEngineMessage::NewPayload { payload, tx } => {
                // TODO: consider parking payload on missing witness
                let mut result =
                    self.tree.on_new_payload(payload).map_err(BeaconOnNewPayloadError::internal);
                if let Ok(outcome) = &mut result {
                    self.on_maybe_tree_event(outcome.event.take());
                }
                if let Err(error) = tx.send(result.map(|o| o.outcome)) {
                    error!(target: "ress::engine", ?error, "Failed to send payload status");
                }
            }
            BeaconEngineMessage::ForkchoiceUpdated { state, payload_attrs, tx, version } => {
                let mut result = self.tree.on_forkchoice_updated(state, payload_attrs, version);
                if let Ok(outcome) = &mut result {
                    // track last received forkchoice state
                    self.tree
                        .forkchoice_state_tracker
                        .set_latest(state, outcome.outcome.forkchoice_status());
                    self.on_maybe_tree_event(outcome.event.take());
                }
                if let Err(error) = tx.send(result.map(|o| o.outcome)) {
                    error!(target: "ress::engine", ?error, "Failed to send forkchoice outcome");
                }
            }
            BeaconEngineMessage::TransitionConfigurationExchanged => {
                warn!(target: "ress::engine", "Received unsupported `TransitionConfigurationExchanged` message");
            }
        }
    }

    fn on_download_outcome(&mut self, outcome: DownloadOutcome) {
        let mut blocks = Vec::new();
        match outcome {
            DownloadOutcome::Block(block) => {
                let block_num_hash = block.num_hash();
                let recovered = match block.try_recover() {
                    Ok(block) => block,
                    Err(_error) => {
                        debug!(target: "ress::engine", ?block_num_hash, "Error recovering downloaded block");
                        return
                    }
                };
                self.tree.block_buffer.insert_block(recovered);
                blocks = self.tree.block_buffer.remove_block_with_children(block_num_hash.hash);
            }
            DownloadOutcome::Witness(block_hash, witness) => {
                let mut bytecodes = witness.bytecode_hashes().clone();
                bytecodes
                    .retain(|code_hash| !self.tree.provider.storage.bytecode_exists(code_hash));
                self.tree.block_buffer.insert_witness(block_hash, witness, bytecodes.clone());
                if bytecodes.is_empty() {
                    blocks = self.tree.block_buffer.remove_block_with_children(block_hash);
                } else {
                    for code_hash in bytecodes {
                        self.downloader.download_bytecode(code_hash);
                    }
                }
            }
            DownloadOutcome::Bytecode(code_hash, bytecode) => {
                match self.tree.provider.storage.insert_bytecode(code_hash, bytecode) {
                    Ok(()) => {
                        blocks =
                            self.tree.block_buffer.remove_blocks_with_received_bytecode(code_hash);
                    }
                    Err(error) => {
                        error!(target: "ress::engine", %error, "Failed to insert the bytecode");
                    }
                }
            }
        };
        for (block, witness) in blocks {
            match self.tree.on_downloaded_block(block, witness) {
                Ok(maybe_event) => {
                    self.on_maybe_tree_event(maybe_event);
                }
                Err(error) => {
                    error!(target: "ress::engine", %error, "Error inserting downloaded block");
                }
            }
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
                self.downloader.download_block(block_hash);
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
}

impl Future for ConsensusEngine {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            if let Poll::Ready(outcome) = this.downloader.poll(cx) {
                this.on_download_outcome(outcome);
                continue;
            }

            if let Poll::Ready(Some(message)) = this.from_beacon_engine.poll_next_unpin(cx) {
                this.on_engine_message(message);
                continue;
            }

            return Poll::Pending
        }
    }
}
