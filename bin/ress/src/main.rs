use clap::Parser;
use futures::StreamExt;
use ress_common::test_utils::TestPeers;
use ress_node::Node;
use reth_chainspec::MAINNET;
use reth_consensus_debug_client::{DebugConsensusClient, RpcBlockProvider};
use reth_network::NetworkEventListenerProvider;
use reth_node_ethereum::EthEngineTypes;
use std::net::TcpListener;
use std::sync::Arc;
use tracing::{debug, info};

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
    is_ports_alive(local_node);
    // ============================== DEMO ==========================================

    // todo : we need to probably have logic to fill necessary headers before running consensus client
    // 1. rpc call latest number
    //2.  rpc 256 https :
    // 3. storage.set(headers)
    let block_provider =
        RpcBlockProvider::new(std::env::var("WS_RPC_URL").expect("need `WS_RPC_URL` env"));
    let rpc_consensus_client =
        DebugConsensusClient::new(node.authserver_handler.clone(), Arc::new(block_provider));
    tokio::spawn(async move {
        info!("rpc consensus client run");
        rpc_consensus_client.run::<EthEngineTypes>().await;
    });

    // =================================================================

    // interact with the network
    let mut events = node.p2p_handler.network_handle.event_listener();
    while let Some(event) = events.next().await {
        info!(target: "ress","Received event: {:?}", event);
    }

    Ok(())
}

fn is_ports_alive(local_node: TestPeers) {
    let is_alive = match TcpListener::bind(("0.0.0.0", local_node.get_authserver_addr().port())) {
        Ok(_listener) => false,
        Err(_) => true,
    };
    debug!(target: "ress","auth server is_alive: {:?}", is_alive);

    let is_alive = match TcpListener::bind(("0.0.0.0", local_node.get_network_addr().port())) {
        Ok(_listener) => false,
        Err(_) => true,
    };
    debug!(target: "ress","network is_alive: {:?}", is_alive);
}
