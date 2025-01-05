use std::net::TcpListener;

use alloy_primitives::b256;
use clap::Parser;
use futures::StreamExt;
use ress_common::test_utils::TestPeers;
use ress_common::utils::read_example_payload;
use ress_node::Node;
use reth::chainspec::MAINNET;
use reth::rpc::api::EngineApiClient;
use reth::rpc::types::engine::ExecutionPayloadV3;
use reth_network::NetworkEventListenerProvider;

use reth_node_ethereum::EthEngineTypes;

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
    // =================================================================

    // <for testing purpose>
    let args = Args::parse();
    let local_node = match args.peer_number {
        1 => TestPeers::Peer1,
        2 => TestPeers::Peer2,
        _ => unreachable!(),
    };

    // =================================================================

    let node = Node::launch_test_node(local_node, MAINNET.clone()).await;

    // =================================================================
    // debugging for port liveness of auth server and network

    let is_alive = match TcpListener::bind(("0.0.0.0", local_node.get_authserver_addr().port())) {
        Ok(_listener) => false,
        Err(_) => true,
    };
    info!("auth server is_alive: {:?}", is_alive);

    let is_alive = match TcpListener::bind(("0.0.0.0", local_node.get_network_addr().port())) {
        Ok(_listener) => false,
        Err(_) => true,
    };
    info!("network is_alive: {:?}", is_alive);

    // =================================================================
    // I'm trying to send some rpc request to Engine API

    // example block 21555422
    let new_payload: ExecutionPayloadV3 = read_example_payload("./fixtures/mainnet-21555422.json")?;
    let versioned_hashes = vec![];
    let parent_beacon_block_root =
        b256!("e8e81982655244a28f4419613b2812c7615bed7b8dcf605c00793bb5f89d1c2c");
    // Send new events to execution client -> called `Result::unwrap()` on an `Err` value: RequestTimeout
    tokio::spawn(async move {
        let _ = EngineApiClient::<EthEngineTypes>::new_payload_v3(
            &node.authserver_handler.http_client(),
            new_payload,
            versioned_hashes,
            parent_beacon_block_root,
        )
        .await;
    });

    info!("i sent");

    // =================================================================

    // interact with the network
    let mut events = node.p2p_handler.network_handle.event_listener();
    while let Some(event) = events.next().await {
        info!("Received event: {:?}", event);
    }

    Ok(())
}
