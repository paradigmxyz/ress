use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    str::FromStr,
    thread, time,
};

use clap::Parser;
use futures::StreamExt;
use ress_subprotocol::{
    connection::CustomCommand,
    protocol::{
        event::ProtocolEvent,
        handler::{CustomRlpxProtoHandler, ProtocolState},
        proto::NodeType,
    },
};
use reth::{providers::noop::NoopProvider, revm::primitives::alloy_primitives::B512};
use reth_network::{
    config::SecretKey, protocol::IntoRlpxSubProtocol, EthNetworkPrimitives, NetworkConfig,
    NetworkEventListenerProvider, NetworkManager,
};
use reth_network_api::PeerId;
use tokio::sync::mpsc;
use tracing::info;

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
async fn main() {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let local_node = match args.peer_number {
        1 => TestPeers::Peer1,
        2 => TestPeers::Peer2,
        _ => unreachable!(),
    };

    // This block provider implementation is used for testing purposes.
    let client = NoopProvider::default();

    let (tx, mut from_peer) = mpsc::unbounded_channel();
    let custom_rlpx_handler = CustomRlpxProtoHandler {
        state: ProtocolState { events: tx },
    };

    // Configure the network
    let config = NetworkConfig::builder(local_node.get_key())
        .listener_addr(local_node.get_addr())
        .disable_discovery()
        .add_rlpx_sub_protocol(custom_rlpx_handler.into_rlpx_sub_protocol())
        .build(client);

    // create the network instance
    let subnetwork = NetworkManager::<EthNetworkPrimitives>::new(config)
        .await
        .unwrap();

    let subnetwork_peer_id = *subnetwork.peer_id();
    let subnet_secret = subnetwork.secret_key();
    let subnetwork_peer_addr = subnetwork.local_addr();
    let subnet_all_peers: Vec<_> = subnetwork.all_peers().collect();
    let subnetwork_handle = subnetwork.peers_handle();

    info!("subnetwork_peer_id {}", subnetwork_peer_id);
    info!("subnetwork_peer_addr {}", subnetwork_peer_addr);
    info!("subnet_all_peers {:?}", subnet_all_peers);
    info!("subnet_secret {:?}", subnet_secret);

    // peer 1 should wait to have another peer to be spawn
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
    info!("rlpx-subprotocol Connection established!");

    // Step 1. Type message subprotocol
    peer_conn
        .send(CustomCommand::NodeType {
            node_type: NodeType::Stateful,
        })
        .unwrap();

    info!(target:"rlpx-subprotocol", "sent type");

    // interact with the network
    let mut events = handle.event_listener();
    while let Some(event) = events.next().await {
        println!("Received event: {:?}", event);
    }
}
