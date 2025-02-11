//! Main ress executable.

use clap::Parser;
use futures::StreamExt;
use ress::{cli::RessArgs, launch::NodeLauncher};
use reth_network::NetworkEventListenerProvider;
use tracing::{info, level_filters::LevelFilter};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        orig_hook(panic_info);
        std::process::exit(1);
    }));

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder().with_default_directive(LevelFilter::INFO.into()).from_env_lossy(),
        )
        .init();

    let node = NodeLauncher::new(RessArgs::parse()).launch().await?;
    let mut events = node.network_handle.inner().event_listener();
    while let Some(event) = events.next().await {
        info!(target: "ress", ?event, "Received network event");
    }

    Ok(())
}
