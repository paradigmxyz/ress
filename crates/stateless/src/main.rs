use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener},
    str::FromStr,
    thread, time,
};

use alloy_primitives::{b256, hex, U256};
use alloy_rpc_types_eth::Block;
use clap::Parser;
use futures::StreamExt;
use jsonrpsee_core::client::ClientT;
use node::Ress;
use ress_subprotocol::{
    connection::CustomCommand,
    protocol::{
        event::ProtocolEvent,
        handler::{CustomRlpxProtoHandler, ProtocolState},
        proto::{NodeType, StateWitness},
    },
};
use reth::{
    api::EngineTypes,
    beacon_consensus::BeaconConsensusEngineHandle,
    chainspec::{DEV, MAINNET},
    primitives::{BlockExt, PooledTransaction, TransactionSigned},
    providers::noop::NoopProvider,
    revm::primitives::{alloy_primitives::B512, Bytes, B256},
    rpc::{
        api::EngineApiClient,
        builder::auth::{AuthRpcModule, AuthServerConfig},
        types::engine::{
            CancunPayloadFields, ClientCode, ClientVersionV1, ExecutionPayload,
            ExecutionPayloadSidecar, ExecutionPayloadV1, ExecutionPayloadV2,
        },
    },
    tasks::TokioTaskExecutor,
    transaction_pool::noop::NoopTransactionPool,
};
use reth::{beacon_consensus::BeaconConsensusEngineEvent, rpc::types::engine::ExecutionPayloadV3};
use reth_consensus_debug_client::block_to_execution_payload_v3;
use reth_network::{
    config::SecretKey, protocol::IntoRlpxSubProtocol, EthNetworkPrimitives, NetworkConfig,
    NetworkEventListenerProvider, NetworkManager,
};
use reth_network_api::PeerId;
use reth_node_ethereum::{node::EthereumEngineValidator, EthEngineTypes};
use reth_payload_builder::test_utils::spawn_test_payload_service;
use reth_rpc_engine_api::{capabilities::EngineCapabilities, EngineApi};
use reth_rpc_layer::JwtSecret;
use reth_rpc_types_compat::engine::{block_to_payload_v1, payload::try_into_block};
use reth_tokio_util::EventSender;
use tokio::sync::{
    mpsc::{self, unbounded_channel},
    oneshot,
};
use tracing::{info, warn};

pub mod engine_api;
pub mod node;

//==============================================
// testing utils for testing with 2 stateless node peers conenction
//==============================================

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Peer number (1 or 2)
    #[arg(value_parser = clap::value_parser!(u8).range(1..=2))]
    peer_number: u8,
}

#[derive(PartialEq, Eq)]
pub enum TestPeers {
    Peer1,
    Peer2,
}

impl TestPeers {
    pub fn get_key(&self) -> SecretKey {
        match self {
            TestPeers::Peer1 => SecretKey::from_slice(&[0x01; 32]).expect("32 bytes"),
            TestPeers::Peer2 => SecretKey::from_slice(&[0x02; 32]).expect("32 bytes"),
        }
    }

    pub fn get_addr(&self) -> SocketAddr {
        match self {
            TestPeers::Peer1 => SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 61397)),
            TestPeers::Peer2 => SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 61398)),
        }
    }

    pub fn get_peer_id(&self) -> PeerId {
        match self {
            TestPeers::Peer1 => B512::from_str("0x1b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f70beaf8f588b541507fed6a642c5ab42dfdf8120a7f639de5122d47a69a8e8d1").unwrap(),
            TestPeers::Peer2 => B512::from_str("0x4d4b6cd1361032ca9bd2aeb9d900aa4d45d9ead80ac9423374c451a7254d07662a3eada2d0fe208b6d257ceb0f064284662e857f57b66b54c198bd310ded36d0").unwrap(),
        }
    }

    pub fn get_peer(&self) -> Self {
        match self {
            TestPeers::Peer1 => TestPeers::Peer2,
            TestPeers::Peer2 => TestPeers::Peer1,
        }
    }
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let local_node = match args.peer_number {
        1 => TestPeers::Peer1,
        2 => TestPeers::Peer2,
        _ => unreachable!(),
    };

    // =================================================================

    // 127.0.0.1:61248 spawn auth server
    let secret = JwtSecret::random();
    let config = AuthServerConfig::builder(secret)
        .socket_addr(SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::LOCALHOST,
            61248,
        )))
        .build();
    let (tx, mut rx) = unbounded_channel();
    let event_handler: EventSender<BeaconConsensusEngineEvent> = Default::default();
    // Create a listener for the events
    let mut listener = event_handler.new_listener();
    let beacon_engine_handle =
        BeaconConsensusEngineHandle::<EthEngineTypes>::new(tx, event_handler);
    let client = ClientVersionV1 {
        code: ClientCode::RH,
        name: "Ress".to_string(),
        version: "v0.1.0".to_string(),
        commit: "defa64b2".to_string(),
    };
    let engine_api = EngineApi::new(
        NoopProvider::default(),
        DEV.clone(),
        beacon_engine_handle,
        spawn_test_payload_service().into(),
        NoopTransactionPool::default(),
        Box::<TokioTaskExecutor>::default(),
        client,
        EngineCapabilities::default(),
        EthereumEngineValidator::new(DEV.clone()),
    );
    let module = AuthRpcModule::new(engine_api);
    let handle = module.start_server(config).await.unwrap();
    let client = handle.http_client();

    // =================================================================
    // I'm trying to send some rpc request to Engine API
    // =================================================================

    // TODO: v1/v2 is not working with `EngineApiClient` (https://github.com/paradigmxyz/reth/blob/934fd1f7f07c42ea49b92fb15694209ee0b9530f/crates/rpc/rpc-builder/tests/it/auth.rs#L15-L30 failing)
    // let payload = block_to_payload_v1::<TransactionSigned>(block.clone());
    // println!("payload:{:?}", payload);

    // engine_exchange_capabilities is working (= auth server spawn correctly)
    let h = EngineApiClient::<EthEngineTypes>::exchange_capabilities(&client, vec![])
        .await
        .unwrap();
    println!("Payload accepted: {:?}", h);

    // TODO: but why engine_new_payload_v3 is not working? i think it's valid payload from devnet (https://github.com/paradigmxyz/reth/blob/934fd1f7f07c42ea49b92fb15694209ee0b9530f/crates/rpc/rpc-types-compat/src/engine/payload.rs#L377)
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
    let h = EngineApiClient::<EthEngineTypes>::new_payload_v3(
        &client,
        new_payload,
        versioned_hashes,
        parent_beacon_block_root,
    )
    .await;
    println!("Payload : {:?}", h);

    while let Some(msg) = rx.recv().await {
        // actually even tho the `Err(RequestTimeout)` happen, it got message from rx about `NewPayload`
        println!("Received message from beacon engine: {:?}", msg);
    }

    while let Some(event) = listener.next().await {
        info!("Received event: {:?}", event);
    }

    // =================================================================
    // Now setup & spin up network that implemented subprotocol
    // =================================================================

    // This block provider implementation is used for testing purposes.
    let client = NoopProvider::default();

    let (tx, mut from_peer) = mpsc::unbounded_channel();
    let custom_rlpx_handler = CustomRlpxProtoHandler {
        state: ProtocolState { events: tx },
        node_type: NodeType::Stateless,
    };

    // Configure the network
    let config = NetworkConfig::builder(local_node.get_key())
        .listener_addr(local_node.get_addr())
        .disable_discovery()
        .add_rlpx_sub_protocol(custom_rlpx_handler.into_rlpx_sub_protocol())
        .build(client);

    // create the network instance
    let subnetwork = NetworkManager::<EthNetworkPrimitives>::new(config).await?;

    let subnetwork_peer_id = *subnetwork.peer_id();
    let subnet_secret = subnetwork.secret_key();
    let subnetwork_peer_addr = subnetwork.local_addr();

    let subnetwork_handle = subnetwork.peers_handle();

    info!("subnetwork_peer_id {}", subnetwork_peer_id);
    info!("subnetwork_peer_addr {}", subnetwork_peer_addr);
    info!("subnet_secret {:?}", subnet_secret);

    // [testing] peer 1 should wait to have another peer to be spawn
    if local_node == TestPeers::Peer1 {
        let ten_millis = time::Duration::from_secs(5);
        thread::sleep(ten_millis);
        info!("waited for 5 seconds");
    }

    // connect peer to own network
    subnetwork_handle.add_peer(
        local_node.get_peer().get_peer_id(),
        local_node.get_peer().get_addr(),
    );

    info!("added peer_id: {:?}", local_node.get_peer().get_peer_id());

    // get a handle to the network to interact with it
    let handle = subnetwork.handle().clone();
    // spawn the network
    tokio::task::spawn(subnetwork);

    // let is_alive = match TcpListener::bind(("0.0.0.0", 61248)) {
    //     Ok(_listener) => false, // Successfully bound, so the port is not in use
    //     Err(_) => true,         // Failed to bind, so the port is in use
    // };
    // println!("is_alive:{:?}", is_alive);

    // Establish connection between peer0 and peer1
    let peer_to_peer = from_peer.recv().await.expect("peer connecting");
    let peer_conn = match peer_to_peer {
        ProtocolEvent::Established {
            direction: _,
            peer_id,
            to_connection,
        } => {
            assert_eq!(peer_id, local_node.get_peer().get_peer_id());
            to_connection
        }
    };

    info!(target:"rlpx-subprotocol",  "Connection established!");

    // =================================================================

    // Step 1. Type message subprotocol
    info!(target:"rlpx-subprotocol", "1️⃣ check connection valiadation");
    // TODO: for now we initiate original node type on protocol state above, but after conenction we send msg to trigger connection validation. Is there a way to explicitly mention node type one time?
    let (tx, rx) = oneshot::channel();
    peer_conn
        .send(CustomCommand::NodeType {
            node_type: NodeType::Stateless,
            response: tx,
        })
        .unwrap();
    let response = rx.await.unwrap();
    assert!(response);
    info!(target:"rlpx-subprotocol",?response, "Connection validation finished");

    // =================================================================

    // Step 2. Request witness
    // [testing] peer1 -> peer2
    // TODO: request witness whenever it get new payload from CL
    info!(target:"rlpx-subprotocol", "2️⃣ request witness");
    let (tx, rx) = oneshot::channel();
    peer_conn
        .send(CustomCommand::Witness {
            block_hash: B256::random(),
            response: tx,
        })
        .unwrap();
    let response = rx.await.unwrap();
    // [mock]
    let mut state_witness = StateWitness::default();
    state_witness.insert(B256::ZERO, [0x00].into());
    assert_eq!(response, state_witness);
    info!(target:"rlpx-subprotocol", ?response, "Witness received");

    // =================================================================

    // Step 3. Request bytecode
    // [testing] peer1 -> peer2
    // TODO: consensus engine will call this request via Bytecode Provider to get necessary bytecode when validating payload
    info!(target:"rlpx-subprotocol", "3️⃣ request bytecode");
    let (tx, rx) = oneshot::channel();
    peer_conn
        .send(CustomCommand::Bytecode {
            code_hash: B256::random(),
            response: tx,
        })
        .unwrap();
    let response = rx.await.unwrap();

    // [mock]
    let bytecode: Bytes = [0xab, 0xab].into();
    assert_eq!(response, bytecode);
    info!(target:"rlpx-subprotocol", ?response, "Bytecode received");

    // =================================================================

    // interact with the network
    let mut events = handle.event_listener();
    while let Some(event) = events.next().await {
        info!("Received event: {:?}", event);
    }

    Ok(())
}
