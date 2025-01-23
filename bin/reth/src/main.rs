use ress_subprotocol::{
    protocol::{
        handler::{CustomRlpxProtoHandler, ProtocolState},
        proto::NodeType,
    },
    utils::setup_subprotocol_network,
};
use reth_network::{protocol::IntoRlpxSubProtocol, NetworkProtocols};
use reth_network::{PeerInfo, Peers};
use reth_node_builder::NodeHandle;
use reth_node_ethereum::EthereumNode;
use tokio::sync::mpsc;
use tracing::info;

fn main() -> eyre::Result<()> {
    reth::cli::Cli::parse_args().run(|builder, _| async move {
        // launch the stateful node
        let NodeHandle {
            node,
            node_exit_future,
        } = builder.node(EthereumNode::default()).launch().await?;
        let peers: Vec<PeerInfo> = node.network.get_trusted_peers().await?;

        // add the custom network subprotocol to the launched node
        let (tx, from_peer) = mpsc::unbounded_channel();
        let state_provider = node.provider;
        let custom_rlpx_handler = CustomRlpxProtoHandler {
            state: ProtocolState { events: tx },
            node_type: NodeType::Stateful,
            state_provider: Some(state_provider),
        };
        node.network
            .add_rlpx_sub_protocol(custom_rlpx_handler.into_rlpx_sub_protocol());

        setup_subprotocol_network(from_peer, peers[0].remote_id, NodeType::Stateful).await;
        info!("gm");

        node_exit_future.await
    })
}
