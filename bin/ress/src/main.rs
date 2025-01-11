use alloy_primitives::{b256, B256};
use alloy_rpc_types::engine::ExecutionPayloadV3;
use clap::Parser;
use futures::StreamExt;
use ress_common::test_utils::TestPeers;
use ress_common::utils::{read_example_header, read_example_payload};
use ress_node::Node;
use reth_chainspec::MAINNET;
use reth_network::NetworkEventListenerProvider;
use reth_node_ethereum::EthEngineTypes;
use reth_rpc_api::EngineApiClient;
use std::net::TcpListener;
use std::str::FromStr;
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

    // 1. new payload
    // for demo, we first need to dump 21592411 - 256 ~ 21592411 blocks to storage before send msg
    let parent_block_hash =
        B256::from_str("77b8cb14ead0df5a77367c14c9f0ed7248e26bbc43145443877266cdbb86e332").unwrap();
    let header = read_example_header("./fixtures/header/mainnet-21592410.json")?;
    node.storage.set_block(parent_block_hash, header);

    // for demo, we imagine consensus client send block 21592411 payload
    let new_payload: ExecutionPayloadV3 =
        read_example_payload("./fixtures/payload/mainnet-21592411.json")?;
    let versioned_hashes = vec![];
    let parent_beacon_block_root =
        b256!("b4f0f62dd56d57c266332be9a87eb3332be4e22198f8124ce44660b1454dab25");
    // Send new events to execution client -> called `Result::unwrap()` on an `Err` value: RequestTimeout
    let new_payload_handle = {
        let execution_client = node.authserver_handler.http_client();
        tokio::spawn(async move {
            // Send `new_payload_v3`
            let res = EngineApiClient::<EthEngineTypes>::new_payload_v3(
                &execution_client,
                new_payload,
                versioned_hashes,
                parent_beacon_block_root,
            )
            .await;
            println!("NewPayload response: {:?}", res);
        })
    };

    // Wait for the spawned task to complete:
    let _ = new_payload_handle.await;

    // 2. new payload

    let head_block_hash = b256!("b4f0f62dd56d57c266332be9a87eb3332be4e22198f8124ce44660b1454dab25");
    let safe_block_hash = b256!("b4f0f62dd56d57c266332be9a87eb3332be4e22198f8124ce44660b1454dab25");
    let finalized_block_hash =
        b256!("b4f0f62dd56d57c266332be9a87eb3332be4e22198f8124ce44660b1454dab25");

    let state = alloy_rpc_types_engine::ForkchoiceState {
        head_block_hash,
        safe_block_hash,
        finalized_block_hash,
    };

    let fork_choice_update_handle = {
        let execution_client = node.authserver_handler.http_client();
        tokio::spawn(async move {
            // Send `fork_choice_update_v3`
            let res = EngineApiClient::<EthEngineTypes>::fork_choice_updated_v3(
                &execution_client,
                state,
                None,
            )
            .await;
            println!("Forkchoiceupdate response: {:?}", res);
        })
    };

    // Wait for the spawned task to complete:
    let _ = fork_choice_update_handle.await;

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
