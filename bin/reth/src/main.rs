//! Reth node that supports ress subprotocol.

use reth_node_builder::NodeHandle;
use reth_node_ethereum::EthereumNode;

fn main() -> eyre::Result<()> {
    reth::cli::Cli::parse_args().run(|builder, _args| async move {
        // launch the stateful node
        let NodeHandle {
            node: _,
            node_exit_future,
        } = builder.node(EthereumNode::default()).launch().await?;

        // TODO: implement `RessProtocolProvider`
        // add the custom network subprotocol to the launched node
        // let (tx, mut _from_peer0) = mpsc::unbounded_channel();
        // let protocol_handler = RessProtocolHandler {
        //     provider: (),
        //     state: ProtocolState { events: tx },
        //     node_type: NodeType::Stateful,
        // };
        // node.network
        //     .add_rlpx_sub_protocol(protocol_handler.into_rlpx_sub_protocol());

        node_exit_future.await
    })
}
