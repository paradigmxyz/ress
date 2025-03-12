//! Reth node that supports ress subprotocol.

use clap::Parser;
use reth::{args::RessArgs, chainspec::EthereumChainSpecParser};
use reth_node_builder::NodeHandle;

fn main() -> eyre::Result<()> {
    reth::cli::Cli::<EthereumChainSpecParser, RessArgs>::parse().run(
        |builder, ress_args| async move {
            // launch the stateful node
            let NodeHandle { node, node_exit_future } =
                builder.node(reth_node_ethereum::EthereumNode::default()).launch().await?;

            // Install ress subprotocol.
            if ress_args.enabled {
                reth::ress::install_ress_subprotocol(
                    ress_args,
                    node.provider,
                    node.block_executor,
                    node.network,
                    node.task_executor,
                    node.add_ons_handle.engine_events.new_listener(),
                )?;
            }

            node_exit_future.await
        },
    )
}
