use alloy_primitives::{map::B256HashMap, Bytes, B256};
use alloy_provider::{Provider, ProviderBuilder, RootProvider};
use alloy_rpc_types_debug::ExecutionWitness;
use alloy_rpc_types_eth::{BlockNumberOrTag, BlockTransactionsKind};
use alloy_transport_http::Http;
use parking_lot::RwLock;
use reqwest::Client;
use ress_protocol::RessProtocolProvider;
use reth_storage_errors::provider::{ProviderError, ProviderResult};
use std::{collections::HashMap, sync::Arc};

/// RPC adapter that implements [`RessProtocolProvider`].
#[derive(Clone, Debug)]
pub struct RpcAdapterProvider {
    provider: RootProvider<Http<reqwest::Client>>,
    bytecodes: Arc<RwLock<HashMap<B256, Bytes>>>,
}

impl RpcAdapterProvider {
    /// Create new RPC adapter.
    pub fn new(url: &str) -> eyre::Result<Self> {
        let provider = ProviderBuilder::new().on_http(url.parse()?);
        Ok(Self {
            provider,
            bytecodes: Arc::new(RwLock::default()),
        })
    }
}

impl RessProtocolProvider for RpcAdapterProvider {
    fn bytecode(&self, code_hash: B256) -> ProviderResult<Option<Bytes>> {
        Ok(self.bytecodes.read().get(&code_hash).cloned())
    }

    fn witness(&self, block_hash: B256) -> ProviderResult<Option<B256HashMap<Bytes>>> {
        let provider = self.provider.clone();
        // Use `block_in_place` to safely block
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async { get_witness_by_hash(provider, block_hash).await })
        })
        .map_err(|_error| ProviderError::BlockHashNotFound(block_hash));
        if let Ok(response) = &result {
            // update bytecode cache
            let mut bytecodes = self.bytecodes.write();
            for (code_hash, bytecode) in &response.codes {
                bytecodes.insert(*code_hash, bytecode.clone());
            }
        }
        result.map(|witness| Some(witness.state))
    }
}

async fn get_witness_by_hash(
    provider: RootProvider<Http<Client>>,
    block_hash: B256,
) -> eyre::Result<ExecutionWitness> {
    let block = provider
        .get_block_by_hash(block_hash, BlockTransactionsKind::Hashes)
        .await?
        .ok_or(ProviderError::BlockHashNotFound(block_hash))?;

    let tag: BlockNumberOrTag = block.header.number.into();
    let witness: ExecutionWitness = provider
        .client()
        .request("debug_executionWitness", [tag])
        .await?;

    Ok(witness)
}
