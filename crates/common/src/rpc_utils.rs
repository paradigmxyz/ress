//! Ress rpc utilities.

use alloy_eips::{BlockId, BlockNumHash};
use alloy_provider::{
    network::{primitives::BlockTransactionsKind, AnyNetwork},
    Provider, ProviderBuilder, RootProvider,
};
use alloy_transport_http::{Client, Http};
use eyre::bail;
use futures::{StreamExt, TryStreamExt};
use reth_primitives::Header;

/// Initializes a new provider instance with the specified network and HTTP client.
///
/// This function expects the `RPC_URL` environment variable to be set. It parses the URL and
/// uses it to configure the provider for HTTP communication.
pub async fn initialize_provider() -> RootProvider<Http<Client>, AnyNetwork> {
    ProviderBuilder::new().network::<AnyNetwork>().on_http(
        std::env::var("RPC_URL")
            .expect("need rpc")
            .parse()
            .expect("invalid rpc url"),
    )
}

/// Retrieves the latest block from the provider and returns its number and hash.
///
/// This function fetches the latest block from the provider and extracts its number and hash.
/// It returns a `BlockNumHash` struct containing these two values.
pub async fn get_latest_block(
    provider: &RootProvider<Http<Client>, AnyNetwork>,
) -> eyre::Result<BlockNumHash> {
    let latest_block = provider
        .get_block(BlockId::latest(), false.into())
        .await?
        .expect("no latest block");
    let latest_block_number = latest_block.inner.header.number;
    let latest_block_hash = latest_block.inner.header.hash;
    Ok(BlockNumHash {
        number: latest_block_number,
        hash: latest_block_hash,
    })
}

/// download range of headers in parallel from given `start_block_number` or `range_gap`
///
/// The parameters should be passed either one to determine the range
pub async fn parallel_latest_headers_download(
    provider: &RootProvider<Http<Client>, AnyNetwork>,
    start_block_number: Option<u64>,
    range_gap: Option<u64>,
) -> eyre::Result<Vec<Header>> {
    if (range_gap.is_none() && start_block_number.is_none())
        || (range_gap.is_some() && start_block_number.is_some())
    {
        bail!("need to provide either start block or gap")
    }
    let latest_block = get_latest_block(provider).await?;
    let range = if let Some(gap) = range_gap {
        (latest_block.number - gap)..=latest_block.number
    } else {
        start_block_number.unwrap()..=latest_block.number
    };

    futures::stream::iter(range)
        .map(|block_number| {
            let provider = provider.clone();
            async move {
                let block_header = provider
                    .get_block_by_number(block_number.into(), BlockTransactionsKind::Hashes)
                    .await?
                    .expect("no block fetched from rpc")
                    .header
                    .clone()
                    .into_consensus()
                    .into_header_with_defaults();
                Ok::<_, eyre::Report>(block_header)
            }
        })
        .buffer_unordered(25)
        .try_collect::<Vec<_>>()
        .await
}
