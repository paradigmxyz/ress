use crate::GetHeaders;
use alloy_primitives::{map::B256HashMap, Bytes, B256};
use alloy_rlp::Encodable;
use reth_primitives::{BlockBody, Header};
use reth_storage_errors::provider::ProviderResult;
use std::future::Future;

/// Maximum number of block headers to serve.
///
/// Used to limit lookups.
pub const MAX_HEADERS_SERVE: usize = 1024;

/// Maximum number of block headers to serve.
///
/// Used to limit lookups.
pub const MAX_BODIES_SERVE: usize = 1024;

/// Maximum size of replies to data retrievals: 2MB
pub const SOFT_RESPONSE_LIMIT: usize = 2 * 1024 * 1024;

/// A provider trait for ress protocol.
pub trait RessProtocolProvider: Send + Sync {
    /// Return block header by hash.
    fn header(&self, block_hash: B256) -> ProviderResult<Option<Header>>;

    /// Return block headers.
    fn headers(&self, request: GetHeaders) -> ProviderResult<Vec<Header>> {
        let mut total_bytes = 0;
        let mut block_hash = request.start_hash;
        let mut headers = Vec::new();
        while let Some(header) = self.header(block_hash)? {
            block_hash = header.parent_hash;
            total_bytes += header.length();
            headers.push(header);
            if headers.len() >= request.limit as usize ||
                headers.len() >= MAX_HEADERS_SERVE ||
                total_bytes > SOFT_RESPONSE_LIMIT
            {
                break
            }
        }
        Ok(headers)
    }

    /// Return block body by hash.
    fn block_body(&self, block_hash: B256) -> ProviderResult<Option<BlockBody>>;

    /// Return block bodies.
    fn block_bodies(&self, block_hashes: Vec<B256>) -> ProviderResult<Vec<BlockBody>> {
        let mut total_bytes = 0;
        let mut bodies = Vec::new();
        for block_hash in block_hashes {
            if let Some(body) = self.block_body(block_hash)? {
                total_bytes += body.length();
                bodies.push(body);
                if bodies.len() >= MAX_BODIES_SERVE || total_bytes > SOFT_RESPONSE_LIMIT {
                    break
                }
            } else {
                break
            }
        }
        Ok(bodies)
    }

    /// Return bytecode by code hash.
    fn bytecode(&self, code_hash: B256) -> ProviderResult<Option<Bytes>>;

    /// Return witness by block hash.
    fn witness(
        &self,
        block_hash: B256,
    ) -> impl Future<Output = ProviderResult<Option<B256HashMap<Bytes>>>> + Send;
}
