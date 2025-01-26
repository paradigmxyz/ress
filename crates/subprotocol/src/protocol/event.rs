//! RLPx subprotocol event.

use crate::connection::CustomCommand;
use reth_network::Direction;
use reth_network_api::PeerId;
use tokio::sync::mpsc;

/// The events that can be emitted by our custom protocol.
#[derive(Debug)]
pub enum ProtocolEvent {
    /// Connection established.
    Established {
        /// Connection direction.
        direction: Direction,
        /// Peer ID.
        peer_id: PeerId,
        /// Sender part for forwarding commands.
        to_connection: mpsc::UnboundedSender<CustomCommand>,
    },
}
