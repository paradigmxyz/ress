//! RLPx subprotocol handler.

use super::{event::ProtocolEvent, proto::NodeType};
use crate::connection::handler::CustomRlpxConnectionHandler;
use reth_db::DatabaseEnv;
use reth_network::protocol::ProtocolHandler;
use reth_network_api::PeerId;
use reth_node_api::NodeTypesWithDBAdapter;
use reth_node_ethereum::EthereumNode;
use reth_provider::providers::BlockchainProvider;
use std::{fmt, net::SocketAddr, sync::Arc};
use tokio::sync::mpsc;

/// Protocol state is an helper struct to store the protocol events.
#[derive(Clone, Debug)]
pub struct ProtocolState {
    /// Protocol event sender.
    pub events: mpsc::UnboundedSender<ProtocolEvent>,
}

/// The protocol handler takes care of incoming and outgoing connections.
pub struct CustomRlpxProtoHandler {
    /// Current state of the protocol.
    pub state: ProtocolState,
    /// Node type.
    pub node_type: NodeType,
    /// Blockchain provider.
    pub state_provider:
        Option<BlockchainProvider<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>>,
}

impl fmt::Debug for CustomRlpxProtoHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CustomRlpxProtoHandler")
            .field("state", &self.state)
            .field("node_type", &self.node_type)
            .finish_non_exhaustive()
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
