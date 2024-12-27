use std::net::TcpListener;

use alloy_primitives::{b256, hex, U256};
use clap::Parser;
use futures::StreamExt;
use ress_core::{node::Node, test_utils::TestPeers};
use ress_subprotocol::{connection::CustomCommand, protocol::proto::StateWitness};
use reth::rpc::types::engine::ExecutionPayloadV3;
use reth::{
    revm::primitives::{Bytes, B256},
    rpc::{
        api::EngineApiClient,
        types::engine::{ExecutionPayloadV1, ExecutionPayloadV2},
    },
};
use reth_network::NetworkEventListenerProvider;

use reth_node_ethereum::EthEngineTypes;

use tokio::sync::oneshot;
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
    // =================================================================

    tracing_subscriber::fmt::init();

    // <for testing purpose>
    let args = Args::parse();
    let local_node = match args.peer_number {
        1 => TestPeers::Peer1,
        2 => TestPeers::Peer2,
        _ => unreachable!(),
    };

    // =================================================================
    // spin up node

    let node = Node::launch_test_node(&local_node).await;

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

    let first_transaction_raw = Bytes::from_static(&hex!("02f9017a8501a1f0ff438211cc85012a05f2008512a05f2000830249f094d5409474fd5a725eab2ac9a8b26ca6fb51af37ef80b901040cc7326300000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000001bdd2ed4b616c800000000000000000000000000001e9ee781dd4b97bdef92e5d1785f73a1f931daa20000000000000000000000007a40026a3b9a41754a95eec8c92c6b99886f440c000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000020000000000000000000000009ae80eb647dd09968488fa1d7e412bf8558a0b7a0000000000000000000000000f9815537d361cb02befd9918c95c97d4d8a4a2bc001a0ba8f1928bb0efc3fcd01524a2039a9a2588fa567cd9a7cc18217e05c615e9d69a0544bfd11425ac7748e76b3795b57a5563e2b0eff47b5428744c62ff19ccfc305")[..]);
    let second_transaction_raw = Bytes::from_static(&hex!("03f901388501a1f0ff430c843b9aca00843b9aca0082520894e7249813d8ccf6fa95a2203f46a64166073d58878080c005f8c6a00195f6dff17753fc89b60eac6477026a805116962c9e412de8015c0484e661c1a001aae314061d4f5bbf158f15d9417a238f9589783f58762cd39d05966b3ba2fba0013f5be9b12e7da06f0dd11a7bdc4e0db8ef33832acc23b183bd0a2c1408a757a0019d9ac55ea1a615d92965e04d960cb3be7bff121a381424f1f22865bd582e09a001def04412e76df26fefe7b0ed5e10580918ae4f355b074c0cfe5d0259157869a0011c11a415db57e43db07aef0de9280b591d65ca0cce36c7002507f8191e5d4a80a0c89b59970b119187d97ad70539f1624bbede92648e2dc007890f9658a88756c5a06fb2e3d4ce2c438c0856c2de34948b7032b1aadc4642a9666228ea8cdc7786b7")[..]);
    let new_payload = ExecutionPayloadV3 {
        payload_inner: ExecutionPayloadV2 {
            payload_inner: ExecutionPayloadV1 {
                base_fee_per_gas:  U256::from(7u64),
                block_number: 0xa946u64,
                block_hash: hex!("a5ddd3f286f429458a39cafc13ffe89295a7efa8eb363cf89a1a4887dbcf272b").into(),
                logs_bloom: hex!("00200004000000000000000080000000000200000000000000000000000000000000200000000000000000000000000000000000800000000200000000000000000000000000000000000008000000200000000000000000000001000000000000000000000000000000800000000000000000000100000000000030000000000000000040000000000000000000000000000000000800080080404000000000000008000000000008200000000000200000000000000000000000000000000000000002000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000100000000000000000000").into(),
                extra_data: hex!("d883010d03846765746888676f312e32312e31856c696e7578").into(),
                gas_limit: 0x1c9c380,
                gas_used: 0x1f4a9,
                timestamp: 0x651f35b8,
                fee_recipient: hex!("f97e180c050e5ab072211ad2c213eb5aee4df134").into(),
                parent_hash: hex!("d829192799c73ef28a7332313b3c03af1f2d5da2c36f8ecfafe7a83a3bfb8d1e").into(),
                prev_randao: hex!("753888cc4adfbeb9e24e01c84233f9d204f4a9e1273f0e29b43c4c148b2b8b7e").into(),
                receipts_root: hex!("4cbc48e87389399a0ea0b382b1c46962c4b8e398014bf0cc610f9c672bee3155").into(),
                state_root: hex!("017d7fa2b5adb480f5e05b2c95cb4186e12062eed893fc8822798eed134329d1").into(),
                transactions: vec![first_transaction_raw, second_transaction_raw],
            },
            withdrawals: vec![],
        },
        blob_gas_used: 0xc0000,
        excess_blob_gas: 0x580000,
    };
    let versioned_hashes = vec![];
    let parent_beacon_block_root =
        b256!("531cd53b8e68deef0ea65edfa3cda927a846c307b0907657af34bc3f313b5871");
    // Send new events to execution client -> called `Result::unwrap()` on an `Err` value: RequestTimeout
    tokio::spawn(async move {
        let _ = EngineApiClient::<EthEngineTypes>::new_payload_v3(
            &node.authserve_handle.http_client(),
            new_payload,
            versioned_hashes,
            parent_beacon_block_root,
        )
        .await;
    });

    // =================================================================

    // interact with the network
    let mut events = node.network_handle.event_listener();
    while let Some(event) = events.next().await {
        info!("Received event: {:?}", event);
    }

    Ok(())
}
