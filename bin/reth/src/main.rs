use ress_subprotocol::protocol::{
    handler::{CustomRlpxProtoHandler, ProtocolState},
    proto::NodeType,
};
use reth_network::{protocol::IntoRlpxSubProtocol, NetworkProtocols};
use reth_node_builder::NodeHandle;
use reth_node_ethereum::EthereumNode;
use tokio::sync::mpsc;

fn main() -> eyre::Result<()> {
    reth::cli::Cli::parse_args().run(|builder, _args| async move {
        // launch the stateful node
        let NodeHandle {
            node,
            node_exit_future,
        } = builder.node(EthereumNode::default()).launch().await?;

        // add the custom network subprotocol to the launched node
        let (tx, mut _from_peer0) = mpsc::unbounded_channel();
        let state_provider = node.provider;
        let custom_rlpx_handler = CustomRlpxProtoHandler {
            state: ProtocolState { events: tx },
            node_type: NodeType::Stateful,
            state_provider: Some(state_provider),
        };
        node.network
            .add_rlpx_sub_protocol(custom_rlpx_handler.into_rlpx_sub_protocol());

        node_exit_future.await
    })
}
