use reth_network_api::PeerId;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tracing::info;

use crate::{
    connection::CustomCommand,
    protocol::{event::ProtocolEvent, proto::NodeType},
};

/// Establish connection and send type checking
pub async fn setup_subprotocol_network(
    mut from_peer: UnboundedReceiver<ProtocolEvent>,
    peer_id: PeerId,
    node_type: NodeType,
) -> UnboundedSender<CustomCommand> {
    info!("gm ? peer id");
    // Establish connection between peer0 and peer1
    let peer_to_peer = from_peer.recv().await.expect("peer connecting");
    let peer_conn = match peer_to_peer {
        ProtocolEvent::Established {
            direction: _,
            peer_id: received_peer_id,
            to_connection,
        } => {
            assert_eq!(received_peer_id, peer_id);
            to_connection
        }
    };
    info!("🟢 connection established with peer_id: {} ", peer_id);

    // =================================================================

    //  Type message subprotocol
    let (tx, rx) = tokio::sync::oneshot::channel();
    peer_conn
        .send(CustomCommand::NodeType {
            node_type,
            response: tx,
        })
        .unwrap();
    info!("🟢 awaiting response");
    let response = rx.await.unwrap();
    assert!(response);
    info!(?response, "🟢 connection type valid");
    peer_conn
}
