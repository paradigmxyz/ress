use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_network::Ethereum;
use alloy_primitives::{Address, Bytes, B256, U256, U64};
use alloy_rpc_types_eth::{
    state::StateOverride, BlockOverrides, EIP1186AccountProofResponse, Filter, Log, SyncStatus,
    TransactionRequest,
};
use alloy_serde::JsonStorageKey;
use jsonrpsee::core::RpcResult as Result;
use ress_provider::RessProvider;
use reth_rpc_api::{
    eth::{RpcBlock, RpcReceipt},
    EngineEthApiServer,
};
use reth_rpc_eth_types::EthApiError;

/// Implementation of minimal eth RPC interface for Engine API.
#[derive(Debug)]
pub struct RessEthRpc(RessProvider);

impl RessEthRpc {
    /// Creates new ress RPC provider.
    pub fn new(provider: RessProvider) -> Self {
        Self(provider)
    }
}

/// Minimal eth RPC interface for Engine API.
/// Ref: <https://github.com/ethereum/execution-apis/blob/main/src/engine/common.md#underlying-protocol>
#[async_trait::async_trait]
impl EngineEthApiServer<RpcBlock<Ethereum>, RpcReceipt<Ethereum>> for RessEthRpc {
    /// Handler for: `eth_syncing`
    fn syncing(&self) -> Result<SyncStatus> {
        Ok(SyncStatus::None)
    }

    /// Handler for: `eth_chainId`
    async fn chain_id(&self) -> Result<Option<U64>> {
        Ok(Some(U64::from(self.0.chain_spec().chain.id())))
    }

    /// Handler for: `eth_blockNumber`
    fn block_number(&self) -> Result<U256> {
        Err(EthApiError::Unsupported("method not supported").into())
    }

    /// Handler for: `eth_call`
    async fn call(
        &self,
        _request: TransactionRequest,
        _block_id: Option<BlockId>,
        _state_overrides: Option<StateOverride>,
        _block_overrides: Option<Box<BlockOverrides>>,
    ) -> Result<Bytes> {
        Err(EthApiError::Unsupported("method not supported").into())
    }

    /// Handler for: `eth_getCode`
    async fn get_code(&self, _address: Address, _block_id: Option<BlockId>) -> Result<Bytes> {
        Err(EthApiError::Unsupported("method not supported").into())
    }

    /// Handler for: `eth_getBlockByHash`
    async fn block_by_hash(&self, _hash: B256, _full: bool) -> Result<Option<RpcBlock<Ethereum>>> {
        Err(EthApiError::Unsupported("method not supported").into())
    }

    /// Handler for: `eth_getBlockByNumber`
    async fn block_by_number(
        &self,
        _number: BlockNumberOrTag,
        _full: bool,
    ) -> Result<Option<RpcBlock<Ethereum>>> {
        Err(EthApiError::Unsupported("method not supported").into())
    }

    async fn block_receipts(
        &self,
        _block_id: BlockId,
    ) -> Result<Option<Vec<RpcReceipt<Ethereum>>>> {
        Err(EthApiError::Unsupported("method not supported").into())
    }

    /// Handler for: `eth_sendRawTransaction`
    async fn send_raw_transaction(&self, _bytes: Bytes) -> Result<B256> {
        Err(EthApiError::Unsupported("method not supported").into())
    }

    async fn transaction_receipt(&self, _hash: B256) -> Result<Option<RpcReceipt<Ethereum>>> {
        Err(EthApiError::Unsupported("method not supported").into())
    }

    /// Handler for `eth_getLogs`
    async fn logs(&self, _filter: Filter) -> Result<Vec<Log>> {
        Err(EthApiError::Unsupported("method not supported").into())
    }

    /// Handler for `eth_getProof`
    async fn get_proof(
        &self,
        _address: Address,
        _keys: Vec<JsonStorageKey>,
        _block_number: Option<BlockId>,
    ) -> Result<EIP1186AccountProofResponse> {
        Err(EthApiError::Unsupported("method not supported").into())
    }
}
