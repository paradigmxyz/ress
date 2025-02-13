use alloy_primitives::{Bytes, B256};
use alloy_provider::{Provider, RootProvider};
use alloy_rlp::Encodable;
use alloy_rpc_client::ClientBuilder;
use alloy_rpc_types_debug::ExecutionWitness;
use alloy_rpc_types_eth::{Block, BlockNumberOrTag, BlockTransactionsKind};
use futures::StreamExt;
use ress_protocol::{
    GetHeaders, RessPeerRequest, StateWitnessNet, MAX_BODIES_SERVE, MAX_HEADERS_SERVE,
    SOFT_RESPONSE_LIMIT,
};
use reth_primitives::{BlockBody, Header, TransactionSigned};
use std::collections::{hash_map, HashMap};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::*;

/// RPC adapter that can substitute `RessNetworkManager`.
#[derive(Clone, Debug)]
pub struct RpcNetworkAdapter {
    provider: RootProvider,
    bytecodes: HashMap<B256, Bytes>,
}

impl RpcNetworkAdapter {
    /// Create new RPC adapter.
    pub fn new(url: &str) -> eyre::Result<Self> {
        let client = ClientBuilder::default().http(url.parse()?);
        Ok(Self { provider: RootProvider::new(client), bytecodes: Default::default() })
    }

    async fn block(
        &self,
        block_hash: B256,
        transactions_kind: BlockTransactionsKind,
    ) -> Option<Block> {
        let result =
            self.provider.get_block_by_hash(block_hash, transactions_kind).await.inspect_err(
                |error| {
                    debug!(target: "ress::rpc_adapter", %block_hash, %error, "Failed to request block from provider");
                },
            );
        result.ok().flatten()
    }

    async fn on_headers_request(&self, request: GetHeaders) -> Vec<Header> {
        let mut total_bytes = 0;
        let mut block_hash = request.start_hash;
        let mut headers = Vec::new();
        while let Some(block) = self.block(block_hash, BlockTransactionsKind::Hashes).await {
            let header = block.header.into_consensus();
            block_hash = header.parent_hash;
            total_bytes += header.length();
            headers.push(header);
            if headers.len() >= MAX_HEADERS_SERVE || total_bytes > SOFT_RESPONSE_LIMIT {
                break
            }
        }
        headers
    }

    async fn on_block_bodies_request(&self, request: Vec<B256>) -> Vec<BlockBody> {
        let mut total_bytes = 0;
        let mut bodies = Vec::new();
        for block_hash in request {
            let Some(block) = self.block(block_hash, BlockTransactionsKind::Full).await else {
                break;
            };
            let block = block.map_transactions(|tx| TransactionSigned::from(tx.inner));
            let body = BlockBody {
                transactions: block.transactions.into_transactions().collect(),
                withdrawals: block.withdrawals.map(|w| w.into_inner().into()),
                ommers: Default::default(),
            };
            total_bytes += body.length();
            bodies.push(body);
            if bodies.len() >= MAX_BODIES_SERVE || total_bytes > SOFT_RESPONSE_LIMIT {
                break
            }
        }
        bodies
    }

    /// Run RPC network adapter to respond to peer requests.
    pub async fn run(mut self, mut request_stream: UnboundedReceiverStream<RessPeerRequest>) {
        while let Some(request) = request_stream.next().await {
            match request {
                RessPeerRequest::GetHeaders { request, tx } => {
                    let headers = self.on_headers_request(request).await;
                    if tx.send(headers).is_err() {
                        debug!(target: "ress::rpc_adapter", ?request, "Failed to send header");
                    }
                }
                RessPeerRequest::GetBlockBodies { request, tx } => {
                    let bodies = self.on_block_bodies_request(request.clone()).await;
                    if tx.send(bodies).is_err() {
                        debug!(target: "ress::rpc_adapter", ?request, "Failed to send block body");
                    }
                }
                RessPeerRequest::GetBytecode { code_hash, tx } => {
                    let maybe_bytecode = self.bytecodes.get(&code_hash).cloned();
                    if tx.send(maybe_bytecode.unwrap_or_default()).is_err() {
                        debug!(target: "ress::rpc_adapter", %code_hash, "Failed to send bytecode");
                    }
                }
                RessPeerRequest::GetWitness { block_hash, tx } => {
                    let maybe_block = self.block(block_hash, BlockTransactionsKind::Hashes).await;

                    let maybe_witness = if let Some(block) = maybe_block {
                        let tag: BlockNumberOrTag = block.header.number.into();
                        let maybe_witness =  self.provider
                            .client()
                            .request::<_, ExecutionWitness>("debug_executionWitness", [tag])
                            .await
                            .map_err(|error| {
                                debug!(target: "ress::rpc_adapter", %block_hash, %error, "Failed to request witness from provider");
                            })
                            .ok();
                        if let Some(witness) = &maybe_witness {
                            for (code_hash, bytecode) in &witness.codes {
                                if let hash_map::Entry::Vacant(entry) =
                                    self.bytecodes.entry(*code_hash)
                                {
                                    entry.insert(bytecode.clone());
                                }
                            }
                        }
                        maybe_witness.map(|witness| StateWitnessNet::from_iter(witness.state))
                    } else {
                        None
                    };

                    if tx.send(maybe_witness.unwrap_or_default()).is_err() {
                        debug!(target: "ress::rpc_adapter", %block_hash, "Failed to send witness");
                    }
                }
            }
        }
    }
}
