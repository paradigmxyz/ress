use super::CustomRlpxConnection;
use crate::protocol::{
    event::ProtocolEvent,
    handler::ProtocolState,
    proto::{CustomRlpxProtoMessage, NodeType},
};
use reth_db::DatabaseEnv;
use reth_eth_wire::{
    capability::SharedCapabilities, multiplex::ProtocolConnection, protocol::Protocol,
};
use reth_network::protocol::{ConnectionHandler, OnNotSupported};
use reth_network_api::{Direction, PeerId};
use reth_node_api::NodeTypesWithDBAdapter;
use reth_node_ethereum::EthereumNode;
use reth_provider::providers::BlockchainProvider;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;

/// The connection handler for the custom RLPx protocol.
#[derive(Debug)]
pub struct CustomRlpxConnectionHandler {
    pub(crate) state: ProtocolState,
    pub(crate) node_type: NodeType,
    pub(crate) state_provider:
        Option<BlockchainProvider<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>>,
}

impl ConnectionHandler for CustomRlpxConnectionHandler {
    type Connection = CustomRlpxConnection;

    fn protocol(&self) -> Protocol {
        CustomRlpxProtoMessage::protocol()
    }

    fn on_unsupported_by_peer(
        self,
        _supported: &SharedCapabilities,
        _direction: Direction,
        _peer_id: PeerId,
    ) -> OnNotSupported {
        OnNotSupported::Disconnect
    }

    fn into_connection(
        self,
        direction: Direction,
        peer_id: PeerId,
        conn: ProtocolConnection,
    ) -> Self::Connection {
        let (tx, rx) = mpsc::unbounded_channel();

        self.state
            .events
            .send(ProtocolEvent::Established {
                direction,
                peer_id,
                to_connection: tx,
            })
            .ok();

        CustomRlpxConnection {
            conn,
            commands: UnboundedReceiverStream::new(rx),
            original_node_type: self.node_type,
            peer_node_type: None,
            pending_is_valid_connection: None,
            pending_bytecode: None,
            pending_witness: None,
            state_provider: self.state_provider,
        }
    }
}
