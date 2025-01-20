use alloy_provider::{network::AnyNetwork, Provider, ProviderBuilder};
use alloy_rpc_types::BlockTransactionsKind;
use clap::Parser;
use futures::{StreamExt, TryStreamExt};
use ress_common::test_utils::TestPeers;
use ress_node::Node;
use reth_chainspec::MAINNET;
use reth_consensus_debug_client::{DebugConsensusClient, RpcBlockProvider};
use reth_network::NetworkEventListenerProvider;
use reth_node_ethereum::EthEngineTypes;
use std::sync::Arc;
use tracing::info;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Peer number (1 or 2)
    #[arg(value_parser = clap::value_parser!(u8).range(1..=2))]
    peer_number: u8,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
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

    let node = Node::launch_test_node(local_node, MAINNET.clone()).await;
    assert!(local_node.is_ports_alive());

    // ============================== DEMO ==========================================

    // initalize necessary headers/hashes
    // todo: there could be gap between new payload and this prefetching latest block number
    let rpc_block_provider = ProviderBuilder::new()
        .network::<AnyNetwork>()
        .on_http(std::env::var("RPC_URL").expect("need rpc").parse()?);
    let latest_block_number = rpc_block_provider.get_block_number().await?;
    info!(
        "✨ prefetching 256 block number from {} to {}..",
        latest_block_number - 255,
        latest_block_number
    );

    // ================ PARALLEL FETCH + STORE HEADERS ================
    let range = (latest_block_number - 255)..=latest_block_number;

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
    let storage = node.storage;
    let parent_header = headers.last().unwrap().clone();
    info!("latest header: {}", parent_header.number);
    storage.overwrite_block_hashes_by_headers(headers);
    storage.set_block_header(parent_header);
    assert!(storage.is_canonical_hashes_exist(latest_block_number));

    // ================ CONSENSUS CLIENT ================

    let ws_block_provider =
        RpcBlockProvider::new(std::env::var("WS_RPC_URL").expect("need ws rpc").parse()?);
    let rpc_consensus_client =
        DebugConsensusClient::new(node.authserver_handler, Arc::new(ws_block_provider));
    tokio::spawn(async move {
        info!("💨 running debug consensus client");
        rpc_consensus_client.run::<EthEngineTypes>().await;
    });

    // =================================================================

    let mut events = node.p2p_handler.network_handle.event_listener();
    while let Some(event) = events.next().await {
        info!(target: "ress","Received event: {:?}", event);
    }

    Ok(())
}
