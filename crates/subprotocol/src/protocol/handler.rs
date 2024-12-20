use crate::connection::handler::CustomRlpxConnectionHandler;

use super::{event::ProtocolEvent, proto::NodeType};
use reth_network::protocol::ProtocolHandler;
use reth_network_api::PeerId;
use std::net::SocketAddr;
use tokio::sync::mpsc;

/// Protocol state is an helper struct to store the protocol events.
#[derive(Clone, Debug)]
pub struct ProtocolState {
    pub events: mpsc::UnboundedSender<ProtocolEvent>,
}

/// The protocol handler takes care of incoming and outgoing connections.
#[derive(Debug)]
pub struct CustomRlpxProtoHandler {
    pub state: ProtocolState,
    pub node_type: NodeType,
}

impl ProtocolHandler for CustomRlpxProtoHandler {
    type ConnectionHandler = CustomRlpxConnectionHandler;

    fn on_incoming(&self, _socket_addr: SocketAddr) -> Option<Self::ConnectionHandler> {
        Some(CustomRlpxConnectionHandler {
            state: self.state.clone(),
            node_type: self.node_type.clone(),
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
        })
    }
}
