use reth_network::Direction;
use reth_network_api::PeerId;
use tokio::sync::mpsc;

use crate::connection::CustomCommand;

/// The events that can be emitted by our custom protocol.
#[derive(Debug)]
pub enum ProtocolEvent {
    Established {
        #[allow(dead_code)]
        direction: Direction,
        peer_id: PeerId,
        to_connection: mpsc::UnboundedSender<CustomCommand>,
    },
}
