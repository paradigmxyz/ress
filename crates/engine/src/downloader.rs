use alloy_primitives::{keccak256, Bytes, B256};
use futures::FutureExt;
use ress_network::{NetworkStorageError, RessNetworkHandle};
use ress_primitives::witness::{ExecutionWitness, StateWitness};
use ress_protocol::{StateWitnessEntry, StateWitnessNet};
use reth_chainspec::ChainSpec;
use reth_consensus::Consensus;
use reth_node_ethereum::consensus::EthBeaconConsensus;
use reth_primitives::{Block, BlockBody, Header, SealedBlock, SealedHeader};
use reth_revm::primitives::Bytecode;
use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    task::{ready, Context, Poll},
    time::Duration,
};
use tracing::*;

/// Struct for downloading chain data from the network.
pub struct EngineDownloader {
    /// Handle to the network for making requests.
    network: RessNetworkHandle,
    /// Consensus engine for validation.
    consensus: EthBeaconConsensus<ChainSpec>,
    /// Delay between retry attempts for failed requests.
    retry_delay: Duration,
    /// List of ongoing requests for full blocks.
    inflight_full_block_requests: Vec<FetchFullBlockFuture>,
    /// List of ongoing requests for bytecode.
    inflight_bytecode_requests: Vec<FetchBytecodeFuture>,
    /// List of ongoing requests for witnesses.
    inflight_witness_requests: Vec<FetchWitnessFuture>,
    /// Queue of outcomes from completed download requests.
    outcomes: VecDeque<DownloadOutcome>,
}

impl EngineDownloader {
    /// Create new engine downloader.
    pub fn new(network: RessNetworkHandle, consensus: EthBeaconConsensus<ChainSpec>) -> Self {
        Self {
            network,
            consensus,
            retry_delay: Duration::from_millis(50),
            inflight_full_block_requests: Vec::new(),
            inflight_witness_requests: Vec::new(),
            inflight_bytecode_requests: Vec::new(),
            outcomes: VecDeque::new(),
        }
    }

    /// Download full block by block hash.
    pub fn download_block(&mut self, block_hash: B256) {
        if self.inflight_full_block_requests.iter().any(|req| req.block_hash == block_hash) {
            return
        }

        trace!(target: "ress::engine::downloader", %block_hash, "Downloading full block");
        let mut fut = FetchFullBlockFuture {
            network: self.network.clone(),
            consensus: self.consensus.clone(),
            block_hash,
            retry_delay: self.retry_delay,
            pending_header_request: None,
            pending_body_request: None,
            header: None,
            body: None,
        };
        fut.pending_header_request = Some(fut.header_request(Duration::default()));
        fut.pending_body_request = Some(fut.body_request(Duration::default()));
        self.inflight_full_block_requests.push(fut);
    }

    /// Download bytecode by code hash.
    pub fn download_bytecode(&mut self, code_hash: B256) {
        if self.inflight_bytecode_requests.iter().any(|req| req.code_hash == code_hash) {
            return
        }

        trace!(target: "ress::engine::downloader", %code_hash, "Downloading bytecode");
        let network = self.network.clone();
        let fut = FetchBytecodeFuture {
            network: self.network.clone(),
            code_hash,
            retry_delay: self.retry_delay,
            pending: Box::pin(async move { network.fetch_bytecode(code_hash).await }),
        };
        self.inflight_bytecode_requests.push(fut);
    }

    /// Download witness by block hash.
    pub fn download_witness(&mut self, block_hash: B256) {
        if self.inflight_witness_requests.iter().any(|req| req.block_hash == block_hash) {
            return
        }

        trace!(target: "ress::engine::downloader", %block_hash, "Downloading witness");
        let network = self.network.clone();
        let fut = FetchWitnessFuture {
            network: self.network.clone(),
            block_hash,
            retry_delay: self.retry_delay,
            pending: Box::pin(async move { network.fetch_witness(block_hash).await }),
        };
        self.inflight_witness_requests.push(fut);
    }

    /// Poll downloader.
    pub fn poll(&mut self, cx: &mut Context<'_>) -> Poll<DownloadOutcome> {
        if let Some(outcome) = self.outcomes.pop_front() {
            return Poll::Ready(outcome)
        }

        // advance all full block requests
        for idx in (0..self.inflight_full_block_requests.len()).rev() {
            let mut request = self.inflight_full_block_requests.swap_remove(idx);
            if let Poll::Ready(block) = request.poll_unpin(cx) {
                trace!(target: "ress::engine::downloader", block=?block.num_hash(), "Received single full block");
                self.outcomes.push_back(DownloadOutcome::Block(block));
            } else {
                self.inflight_full_block_requests.push(request);
            }
        }

        // advance all witness requests
        for idx in (0..self.inflight_witness_requests.len()).rev() {
            let mut request = self.inflight_witness_requests.swap_remove(idx);
            if let Poll::Ready(witness) = request.poll_unpin(cx) {
                trace!(target: "ress::engine::downloader", block_hash = %request.block_hash, "Received witness");
                self.outcomes.push_back(DownloadOutcome::Witness(request.block_hash, witness));
            } else {
                self.inflight_witness_requests.push(request);
            }
        }

        // advance all bytecode requests
        for idx in (0..self.inflight_bytecode_requests.len()).rev() {
            let mut request = self.inflight_bytecode_requests.swap_remove(idx);
            if let Poll::Ready(bytecode) = request.poll_unpin(cx) {
                trace!(target: "ress::engine::downloader", code_hash = %request.code_hash, "Received bytecode");
                self.outcomes.push_back(DownloadOutcome::Bytecode(request.code_hash, bytecode));
            } else {
                self.inflight_bytecode_requests.push(request);
            }
        }

        if let Some(outcome) = self.outcomes.pop_front() {
            return Poll::Ready(outcome)
        }

        Poll::Pending
    }
}

/// Download outcome.

#[derive(Debug)]
pub enum DownloadOutcome {
    /// Downloaded block.
    Block(SealedBlock),
    /// Downloaded bytecode.
    Bytecode(B256, Bytecode),
    /// Downloaded execution witness.
    Witness(B256, ExecutionWitness),
}

type DownloadFut<Ok> = Pin<Box<dyn Future<Output = Result<Ok, NetworkStorageError>> + Send + Sync>>;

/// A future that downloads a full block from the network.
///
/// This will attempt to fetch both the header and body for the given block hash at the same time.
/// When both requests succeed, the future will yield the full block.
#[must_use = "futures do nothing unless polled"]
pub struct FetchFullBlockFuture {
    network: RessNetworkHandle,
    consensus: EthBeaconConsensus<ChainSpec>,
    retry_delay: Duration,
    block_hash: B256,
    pending_header_request: Option<DownloadFut<Header>>,
    pending_body_request: Option<DownloadFut<BlockBody>>,
    header: Option<SealedHeader>,
    body: Option<BlockBody>,
}

impl FetchFullBlockFuture {
    /// Returns the hash of the block being requested.
    pub const fn block_hash(&self) -> &B256 {
        &self.block_hash
    }

    fn header_request(&self, delay: Duration) -> DownloadFut<Header> {
        let network = self.network.clone();
        let hash = self.block_hash;
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            network.fetch_header(hash).await
        })
    }

    fn body_request(&self, delay: Duration) -> DownloadFut<BlockBody> {
        let network = self.network.clone();
        let hash = self.block_hash;
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            network.fetch_block_body(hash).await
        })
    }
}

impl Future for FetchFullBlockFuture {
    type Output = SealedBlock;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            if let Some(fut) = &mut this.pending_header_request {
                if let Poll::Ready(response) = fut.poll_unpin(cx) {
                    this.pending_header_request.take();
                    match response {
                        Ok(header) => {
                            let header = SealedHeader::seal_slow(header);
                            if header.hash() == this.block_hash {
                                this.header = Some(header);
                            } else {
                                debug!(target: "ress::engine::downloader", expected = %this.block_hash, received = %header.hash(), "Received wrong header");
                                this.pending_header_request =
                                    Some(this.header_request(this.retry_delay));
                                continue
                            }
                        }
                        Err(error) => {
                            debug!(target: "ress::engine::downloader", %error, %this.block_hash, "Header download failed");
                            this.pending_header_request =
                                Some(this.header_request(this.retry_delay));
                            continue
                        }
                    };
                }
            }

            if let Some(fut) = &mut this.pending_body_request {
                if let Poll::Ready(response) = fut.poll_unpin(cx) {
                    this.pending_body_request.take();
                    match response {
                        Ok(body) => {
                            this.body = Some(body);
                        }
                        Err(error) => {
                            debug!(target: "ress::engine::downloader", %error, %this.block_hash, "Body download failed");
                            this.pending_body_request = Some(this.body_request(this.retry_delay));
                            continue
                        }
                    };
                }
            }

            if let (Some(header), Some(body)) = (this.header.take(), this.body.take()) {
                // ensure the block is valid, else retry
                if let Err(error) = <EthBeaconConsensus<ChainSpec> as Consensus<Block>>::validate_body_against_header(&this.consensus, &body, &header) {
                    debug!(target: "ress::engine::downloader", %error, hash = %header.hash(), "Received wrong body");
                    this.header = Some(header);
                    this.pending_body_request = Some(this.body_request(this.retry_delay));
                    continue
                }

                return Poll::Ready(SealedBlock::from_sealed_parts(header, body))
            }

            return Poll::Pending
        }
    }
}

/// A future that downloads a bytecode from the network.
#[must_use = "futures do nothing unless polled"]
pub struct FetchBytecodeFuture {
    network: RessNetworkHandle,
    code_hash: B256,
    retry_delay: Duration,
    pending: DownloadFut<Bytes>,
}

impl FetchBytecodeFuture {
    fn bytecode_request(&self) -> DownloadFut<Bytes> {
        let network = self.network.clone();
        let hash = self.code_hash;
        let delay = self.retry_delay;
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            network.fetch_bytecode(hash).await
        })
    }
}

impl Future for FetchBytecodeFuture {
    type Output = Bytecode;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            match ready!(this.pending.poll_unpin(cx)) {
                Ok(bytecode) => {
                    let bytecode = Bytecode::new_raw(bytecode);
                    let code_hash = bytecode.hash_slow();
                    if code_hash == this.code_hash {
                        return Poll::Ready(bytecode)
                    } else {
                        debug!(target: "ress::engine::downloader", expected = %this.code_hash, received = %code_hash, "Received wrong bytecode");
                    }
                }
                Err(error) => {
                    debug!(target: "ress::engine::downloader", %error, %this.code_hash, "Bytecode download failed");
                }
            };
            this.pending = this.bytecode_request();
        }
    }
}

/// A future that downloads a witness from the network.
#[must_use = "futures do nothing unless polled"]
pub struct FetchWitnessFuture {
    network: RessNetworkHandle,
    block_hash: B256,
    retry_delay: Duration,
    pending: DownloadFut<StateWitnessNet>,
}

impl FetchWitnessFuture {
    fn witness_request(&self) -> DownloadFut<StateWitnessNet> {
        let network = self.network.clone();
        let hash = self.block_hash;
        let delay = self.retry_delay;
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            network.fetch_witness(hash).await
        })
    }
}

impl Future for FetchWitnessFuture {
    type Output = ExecutionWitness;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();

        loop {
            match ready!(this.pending.poll_unpin(cx)) {
                Ok(witness) => {
                    if witness.0.is_empty() {
                        debug!(target: "ress::engine::downloader", block_hash = %this.block_hash, "Received empty witness");
                    } else {
                        let mut state_witness = StateWitness::default();
                        let valid = 'witness: {
                            for StateWitnessEntry { hash, bytes } in witness.0 {
                                let entry_hash = keccak256(&bytes);
                                if hash == entry_hash {
                                    state_witness.insert(hash, bytes);
                                } else {
                                    debug!(target: "ress::engine::downloader", block_hash = %this.block_hash, expected = %entry_hash, received = %hash, "Invalid witness entry");
                                    break 'witness false
                                }
                            }
                            true
                        };
                        if valid {
                            return Poll::Ready(ExecutionWitness::new(state_witness))
                        }
                    }
                }
                Err(error) => {
                    debug!(target: "ress::engine::downloader", %error, %this.block_hash, "Witness download failed");
                }
            };
            this.pending = this.witness_request();
        }
    }
}
