use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    str::FromStr,
};

use alloy_primitives::B512;
use alloy_rpc_types_engine::JwtSecret;
use reth_network::config::SecretKey;
use reth_transaction_pool::PeerId;

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum TestPeers {
    Peer1,
    Peer2,
}

impl TestPeers {
    pub fn get_jwt_key(&self) -> JwtSecret {
        match self {
            TestPeers::Peer1 => JwtSecret::from_hex(
                "0x4cbc48e87389399a0ea0b382b1c46962c4b8e398014bf0cc610f9c672bee3155",
            )
            .expect("32 bytes"),
            TestPeers::Peer2 => JwtSecret::from_hex(
                "0xd829192799c73ef28a7332313b3c03af1f2d5da2c36f8ecfafe7a83a3bfb8d1e",
            )
            .expect("32 bytes"),
        }
    }

    pub fn get_key(&self) -> SecretKey {
        match self {
            TestPeers::Peer1 => SecretKey::from_slice(&[0x01; 32]).expect("32 bytes"),
            TestPeers::Peer2 => SecretKey::from_slice(&[0x02; 32]).expect("32 bytes"),
        }
    }

    pub fn get_network_addr(&self) -> SocketAddr {
        match self {
            TestPeers::Peer1 => SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 61397)),
            TestPeers::Peer2 => SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 61398)),
        }
    }

    pub fn get_authserver_addr(&self) -> SocketAddr {
        match self {
            TestPeers::Peer1 => SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 8551)),
            TestPeers::Peer2 => SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 8552)),
        }
    }

    pub fn get_peer_id(&self) -> PeerId {
        match self {
            TestPeers::Peer1 => B512::from_str("0x1b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f70beaf8f588b541507fed6a642c5ab42dfdf8120a7f639de5122d47a69a8e8d1").expect("not b512"),
            TestPeers::Peer2 => B512::from_str("0x4d4b6cd1361032ca9bd2aeb9d900aa4d45d9ead80ac9423374c451a7254d07662a3eada2d0fe208b6d257ceb0f064284662e857f57b66b54c198bd310ded36d0").expect("not b512"),
        }
    }

    pub fn get_peer(&self) -> Self {
        match self {
            TestPeers::Peer1 => TestPeers::Peer2,
            TestPeers::Peer2 => TestPeers::Peer1,
        }
    }

    pub fn get_etherscan_api(&self) -> String {
        match self {
            TestPeers::Peer1 => std::env::var("ETHERSCAN_API_KEY1").expect("need api key"),
            TestPeers::Peer2 => std::env::var("ETHERSCAN_API_KEY2").expect("need api key"),
        }
    }
}
