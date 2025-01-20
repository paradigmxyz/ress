use crate::connection::handler::CustomRlpxConnectionHandler;

use super::{event::ProtocolEvent, proto::NodeType};
use reth_db::DatabaseEnv;
use reth_network::protocol::ProtocolHandler;
use reth_network_api::PeerId;
use reth_node_api::NodeTypesWithDBAdapter;
use reth_node_ethereum::EthereumNode;
use reth_provider::providers::BlockchainProvider;
use std::{fmt::Debug, net::SocketAddr, sync::Arc};
use tokio::sync::mpsc;

/// Protocol state is an helper struct to store the protocol events.
#[derive(Clone, Debug)]
pub struct ProtocolState {
    pub events: mpsc::UnboundedSender<ProtocolEvent>,
}

/// The protocol handler takes care of incoming and outgoing connections.
pub struct CustomRlpxProtoHandler {
    pub state: ProtocolState,
    pub node_type: NodeType,
    pub state_provider:
        Option<BlockchainProvider<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>>,
}

impl Debug for CustomRlpxProtoHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CustomRlpxProtoHandler")
            .field("state", &self.state)
            .field("node_type", &self.node_type)
            .field("state_provider", &"stateprovider")
            .finish()
    }
}

impl ProtocolHandler for CustomRlpxProtoHandler {
    type ConnectionHandler = CustomRlpxConnectionHandler;

    fn on_incoming(&self, _socket_addr: SocketAddr) -> Option<Self::ConnectionHandler> {
        Some(CustomRlpxConnectionHandler {
            state: self.state.clone(),
            node_type: self.node_type.clone(),
            state_provider: self.state_provider.clone(),
        })
    }

    fn on_outgoing(
        &self,
        _socket_addr: SocketAddr,
        _peer_id: PeerId,
    ) -> Option<Self::ConnectionHandler> {
        Some(CustomRlpxConnectionHandler {
            state: self.state.clone(),
            node_type: self.node_type.clone(),
            state_provider: self.state_provider.clone(),
        })
    }
}
