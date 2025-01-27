//! Main ress executable.

use alloy_eips::{BlockId, BlockNumHash, NumHash};
use alloy_provider::{network::AnyNetwork, Provider, ProviderBuilder};
use alloy_rpc_types::BlockTransactionsKind;
use clap::Parser;
use futures::{StreamExt, TryStreamExt};
use ress_common::test_utils::TestPeers;
use ress_node::Node;
use ress_testing::rpc_adapter::RpcAdapterProvider;
use reth_chainspec::MAINNET;
use reth_consensus_debug_client::{DebugConsensusClient, RpcBlockProvider};
use reth_network::NetworkEventListenerProvider;
use reth_node_ethereum::EthEngineTypes;
use std::{collections::HashMap, sync::Arc};
use tracing::info;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Peer number (1 or 2)
    #[arg(value_parser = clap::value_parser!(u8).range(1..=2))]
    peer_number: u8,

    #[arg(long = "enable-rpc-adapter")]
    rpc_adapter_enabled: bool,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        orig_hook(panic_info);
        std::process::exit(1);
    }));

    tracing_subscriber::fmt::init();
    dotenvy::dotenv()?;

    // =================================================================

    // <for testing purpose>
    let args = Args::parse();
    let local_node = match args.peer_number {
        1 => TestPeers::Peer1,
        2 => TestPeers::Peer2,
        _ => unreachable!(),
    };

    // =============================== Launch Node ==================================

    // initalize necessary headers/hashes
    // todo: there could be gap between new payload and this prefetching latest block number
    let rpc_block_provider = ProviderBuilder::new()
        .network::<AnyNetwork>()
        .on_http(std::env::var("RPC_URL").expect("need rpc").parse()?);
    let latest_block = rpc_block_provider
        .get_block(BlockId::latest(), false.into())
        .await?
        .expect("no latest block");
    let latest_block_number = latest_block.inner.header.number;
    let latest_block_hash = latest_block.inner.header.hash;

    let maybe_rpc_adapter = if args.rpc_adapter_enabled {
        let rpc_url = std::env::var("RPC_URL").expect("`RPC_URL` env not set");
        Some(RpcAdapterProvider::new(&rpc_url)?)
    } else {
        None
    };
    let node = Node::launch_test_node(
        local_node,
        MAINNET.clone(),
        NumHash::new(latest_block_number, latest_block_hash),
        maybe_rpc_adapter,
    )
    .await;
    assert!(local_node.is_ports_alive());

    // ================ PARALLEL FETCH + STORE HEADERS ================
    let start_time = std::time::Instant::now();
    let range = (latest_block_number - 255)..=latest_block_number;

    let mut canonical_block_hashes = HashMap::new();

    // Parallel download
    let headers = futures::stream::iter(range)
        .map(|block_number| {
            let provider = rpc_block_provider.clone();
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
        .await?;
    let tree_state = &node.provider.storage;
    for header in headers {
        canonical_block_hashes.insert(header.number, header.hash_slow());
        tree_state.insert_header(header);
    }
    let latest_block_number_updated = rpc_block_provider.get_block_number().await?;
    let range = latest_block_number..=latest_block_number_updated;
    // Parallel download
    let headers = futures::stream::iter(range)
        .map(|block_number| {
            let provider = rpc_block_provider.clone();
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
        .await?;
    for header in headers {
        canonical_block_hashes.insert(header.number, header.hash_slow());
        tree_state.set_canonical_head(BlockNumHash::new(header.number, header.hash_slow()));
        tree_state.insert_header(header);
    }
    node.provider
        .storage
        .overwrite_block_hashes(canonical_block_hashes);
    info!(
        elapsed = ?start_time.elapsed(), "✨ prefetched block from {} to {}..",
        latest_block_number - 255,
        latest_block_number_updated
    );
    let head = node.provider.storage.get_canonical_head();
    info!("head: {:#?}", head);

    // ================ CONSENSUS CLIENT ================

    let ws_block_provider =
        RpcBlockProvider::new(std::env::var("WS_RPC_URL").expect("need ws rpc").parse()?);
    let rpc_consensus_client =
        DebugConsensusClient::new(node.authserver_handle, Arc::new(ws_block_provider));
    tokio::spawn(async move {
        info!("💨 running debug consensus client");
        rpc_consensus_client.run::<EthEngineTypes>().await;
    });

    // =================================================================

    let mut events = node.network_handle.network_handle.event_listener();
    while let Some(event) = events.next().await {
        info!(target: "ress", ?event, "Received network event");
    }

    Ok(())
}
