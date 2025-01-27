//! Main ress executable.

use alloy_eips::BlockNumHash;
use clap::Parser;
use futures::StreamExt;
use ress_common::{
    rpc_utils::{get_latest_block, initialize_provider, parallel_latest_headers_download},
    test_utils::TestPeers,
};
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
    let rpc_provider = initialize_provider().await;
    let latest_block = get_latest_block(&rpc_provider).await?;
    // initalize necessary headers/hashes
    let maybe_rpc_adapter = if args.rpc_adapter_enabled {
        let rpc_url = std::env::var("RPC_URL").expect("`RPC_URL` env not set");
        Some(RpcAdapterProvider::new(&rpc_url)?)
    } else {
        None
    };
    let node =
        Node::launch_test_node(local_node, MAINNET.clone(), latest_block, maybe_rpc_adapter).await;
    assert!(local_node.is_ports_alive());

    // ================ PARALLEL FETCH + STORE HEADERS ================
    let start_time = std::time::Instant::now();
    let mut canonical_block_hashes = HashMap::new();
    // Parallel download
    let headers = parallel_latest_headers_download(&rpc_provider, None, Some(255)).await?;
    let tree_state = &node.provider.storage;
    for header in headers {
        canonical_block_hashes.insert(header.number, header.hash_slow());
        tree_state.insert_executed(header);
    }
    // fill the gap with latency
    let headers =
        parallel_latest_headers_download(&rpc_provider, Some(latest_block.number), None).await?;
    for header in headers {
        canonical_block_hashes.insert(header.number, header.hash_slow());
        tree_state.set_canonical_head(BlockNumHash::new(header.number, header.hash_slow()));
        tree_state.insert_executed(header);
    }

    let cloned_hashes = canonical_block_hashes.clone();
    let min = cloned_hashes.keys().min().unwrap();
    let max = cloned_hashes.keys().clone().max().unwrap();
    node.provider
        .storage
        .overwrite_block_hashes(canonical_block_hashes);

    info!(
        elapsed = ?start_time.elapsed(), "✨ prefetched block from {} to {}..",
        min,
        max
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
