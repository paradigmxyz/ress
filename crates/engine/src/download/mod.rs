use alloy_primitives::{map::HashMap, B256};
use futures::FutureExt;
use metrics::{Counter, Gauge, Histogram};
use ress_network::RessNetworkHandle;
use reth_chainspec::ChainSpec;
use reth_metrics::Metrics;
use reth_node_ethereum::consensus::EthBeaconConsensus;
use reth_primitives::{Bytecode, SealedBlock, SealedHeader};
use reth_ress_protocol::RLPExecutionWitness;
use std::{
    collections::VecDeque,
    task::{Context, Poll},
    time::Duration,
};
use tracing::*;

/// Futures for fetching and validating blockchain data.
#[allow(missing_debug_implementations)]
pub mod futs;
use futs::*;

/// Struct for downloading chain data from the network.
#[allow(missing_debug_implementations)]
pub struct EngineDownloader {
    network: RessNetworkHandle,
    consensus: EthBeaconConsensus<ChainSpec>,
    retry_delay: Duration,

    inflight_full_block_requests: Vec<FetchFullBlockFuture>,
    inflight_bytecode_requests: Vec<FetchBytecodeFuture>,
    inflight_witness_requests: Vec<FetchWitnessFuture>,
    inflight_finalized_block_requests: Vec<FetchFullBlockWithAncestorsFuture>,
    outcomes: VecDeque<DownloadOutcome>,

    metrics: EngineDownloaderMetrics,
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
            inflight_finalized_block_requests: Vec::new(),
            outcomes: VecDeque::new(),
            metrics: EngineDownloaderMetrics::default(),
        }
    }

    /// Download full block by block hash.
    pub fn download_full_block(&mut self, block_hash: B256) {
        if self.inflight_full_block_requests.iter().any(|req| req.block_hash() == block_hash) {
            return;
        }

        debug!(target: "ress::engine::downloader", %block_hash, "Downloading full block");
        let fut = FetchFullBlockFuture::new(
            self.network.clone(),
            self.consensus.clone(),
            self.retry_delay,
            block_hash,
        );
        self.inflight_full_block_requests.push(fut);
        self.metrics.inc_total(RequestMetricTy::FullBlock);
        self.metrics.set_inflight(RequestMetricTy::FullBlock, self.inflight_witness_requests.len());
    }

    /// Download bytecode by code hash.
    pub fn download_bytecode(&mut self, code_hash: B256) {
        if self.inflight_bytecode_requests.iter().any(|req| req.code_hash() == code_hash) {
            return;
        }

        debug!(target: "ress::engine::downloader", %code_hash, "Downloading bytecode");
        let fut = FetchBytecodeFuture::new(self.network.clone(), self.retry_delay, code_hash);
        self.inflight_bytecode_requests.push(fut);
        self.metrics.inc_total(RequestMetricTy::Bytecode);
        self.metrics.set_inflight(RequestMetricTy::Bytecode, self.inflight_bytecode_requests.len());
    }

    /// Download witness by block hash.
    pub fn download_witness(&mut self, block_hash: B256) {
        if self.inflight_witness_requests.iter().any(|req| req.block_hash() == block_hash) {
            return;
        }

        debug!(target: "ress::engine::downloader", %block_hash, "Downloading witness");
        let fut = FetchWitnessFuture::new(self.network.clone(), self.retry_delay, block_hash);
        self.inflight_witness_requests.push(fut);
        self.metrics.inc_total(RequestMetricTy::Witness);
        self.metrics.set_inflight(RequestMetricTy::Witness, self.inflight_witness_requests.len());
    }

    /// Download finalized block with 256 ancestors.
    pub fn download_finalized_with_ancestors(&mut self, block_hash: B256) {
        if self.inflight_finalized_block_requests.iter().any(|req| req.block_hash() == block_hash) {
            return;
        }

        debug!(target: "ress::engine::downloader", %block_hash, "Downloading finalized");
        let fut = FetchFullBlockWithAncestorsFuture::new(
            self.network.clone(),
            self.consensus.clone(),
            self.retry_delay,
            block_hash,
            256,
        );
        self.inflight_finalized_block_requests.push(fut);
        self.metrics.inc_total(RequestMetricTy::Finalized);
        self.metrics
            .set_inflight(RequestMetricTy::Finalized, self.inflight_finalized_block_requests.len());
    }

    /// Poll downloader.
    pub fn poll(&mut self, cx: &mut Context<'_>) -> Poll<DownloadOutcome> {
        if let Some(outcome) = self.outcomes.pop_front() {
            return Poll::Ready(outcome);
        }

        // advance all full block range requests
        for idx in (0..self.inflight_finalized_block_requests.len()).rev() {
            let mut request = self.inflight_finalized_block_requests.swap_remove(idx);
            if let Poll::Ready((block, ancestors)) = request.poll_unpin(cx) {
                let elapsed = request.elapsed();
                self.metrics.record_elapsed(RequestMetricTy::Finalized, elapsed);
                trace!(target: "ress::engine::downloader", block=?block.num_hash(), ancestors_len = ancestors.len(), ?elapsed, "Received finalized block");
                self.outcomes.push_back(DownloadOutcome::new(
                    DownloadData::FinalizedBlock(block, ancestors),
                    elapsed,
                ));
            } else {
                self.inflight_finalized_block_requests.push(request);
            }
        }
        self.metrics
            .set_inflight(RequestMetricTy::Finalized, self.inflight_finalized_block_requests.len());

        // advance all full block requests
        for idx in (0..self.inflight_full_block_requests.len()).rev() {
            let mut request = self.inflight_full_block_requests.swap_remove(idx);
            if let Poll::Ready(block) = request.poll_unpin(cx) {
                let elapsed = request.elapsed();
                self.metrics.record_elapsed(RequestMetricTy::FullBlock, elapsed);
                trace!(target: "ress::engine::downloader", block = ?block.num_hash(), ?elapsed, "Received single full block");
                self.outcomes
                    .push_back(DownloadOutcome::new(DownloadData::FullBlock(block), elapsed));
            } else {
                self.inflight_full_block_requests.push(request);
            }
        }
        self.metrics
            .set_inflight(RequestMetricTy::FullBlock, self.inflight_full_block_requests.len());

        // advance all witness requests
        for idx in (0..self.inflight_witness_requests.len()).rev() {
            let mut request = self.inflight_witness_requests.swap_remove(idx);
            if let Poll::Ready(witness) = request.poll_unpin(cx) {
                let elapsed = request.elapsed();
                self.metrics.record_elapsed(RequestMetricTy::Witness, elapsed);
                trace!(target: "ress::engine::downloader", block_hash = %request.block_hash(), ?elapsed, "Received witness");
                self.outcomes.push_back(DownloadOutcome::new(
                    DownloadData::Witness(request.block_hash(), witness),
                    elapsed,
                ));
            } else {
                self.inflight_witness_requests.push(request);
            }
        }
        self.metrics.set_inflight(RequestMetricTy::Witness, self.inflight_witness_requests.len());

        // advance all bytecode requests
        for idx in (0..self.inflight_bytecode_requests.len()).rev() {
            let mut request = self.inflight_bytecode_requests.swap_remove(idx);
            if let Poll::Ready(bytecode) = request.poll_unpin(cx) {
                let elapsed = request.elapsed();
                self.metrics.record_elapsed(RequestMetricTy::Bytecode, elapsed);
                trace!(target: "ress::engine::downloader", code_hash = %request.code_hash(), ?elapsed, "Received bytecode");
                self.outcomes.push_back(DownloadOutcome::new(
                    DownloadData::Bytecode(request.code_hash(), bytecode),
                    elapsed,
                ));
            } else {
                self.inflight_bytecode_requests.push(request);
            }
        }
        self.metrics.set_inflight(RequestMetricTy::Bytecode, self.inflight_bytecode_requests.len());

        if let Some(outcome) = self.outcomes.pop_front() {
            return Poll::Ready(outcome);
        }

        Poll::Pending
    }
}

/// Download outcome.
#[derive(Debug)]
pub struct DownloadOutcome {
    /// Downloaded data.
    pub data: DownloadData,
    /// Time elapsed since download started.
    pub elapsed: Duration,
}

impl DownloadOutcome {
    /// Create new download outcome.
    pub fn new(data: DownloadData, elapsed: Duration) -> Self {
        Self { data, elapsed }
    }
}

/// Download data.
#[derive(Debug)]
pub enum DownloadData {
    /// Downloaded full block.
    FullBlock(SealedBlock),
    /// Downloaded bytecode.
    Bytecode(B256, Bytecode),
    /// Downloaded execution witness.
    Witness(B256, RLPExecutionWitness),
    /// Downloaded full block with ancestors.
    FinalizedBlock(SealedBlock, Vec<SealedHeader>),
}

#[derive(Default, Debug)]
struct EngineDownloaderMetrics {
    by_type: HashMap<RequestMetricTy, DownloadRequestTypeMetrics>,
}

impl EngineDownloaderMetrics {
    fn for_type(&mut self, ty: RequestMetricTy) -> &DownloadRequestTypeMetrics {
        self.by_type.entry(ty).or_insert_with(|| {
            DownloadRequestTypeMetrics::new_with_labels(&[("type", ty.to_string())])
        })
    }

    fn inc_total(&mut self, ty: RequestMetricTy) {
        self.for_type(ty).total.increment(1);
    }

    fn set_inflight(&mut self, ty: RequestMetricTy, count: usize) {
        self.for_type(ty).inflight.set(count as f64);
    }

    fn record_elapsed(&mut self, ty: RequestMetricTy, elapsed: Duration) {
        self.for_type(ty).elapsed.record(elapsed.as_secs_f64());
    }
}

#[derive(Metrics)]
#[metrics(scope = "engine.downloader")]
struct DownloadRequestTypeMetrics {
    /// The total number of requests.
    total: Counter,
    /// The number of inflight requests.
    inflight: Gauge,
    /// The number of seconds request took to complete.
    elapsed: Histogram,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, strum_macros::Display)]
#[strum(serialize_all = "snake_case")]
enum RequestMetricTy {
    FullBlock,
    Bytecode,
    Witness,
    Finalized,
}
