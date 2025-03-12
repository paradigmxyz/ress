use alloy_rpc_types_engine::{JwtError, JwtSecret};
use clap::{Args, Parser};
use reth_chainspec::{Chain, ChainSpec};
use reth_cli::chainspec::ChainSpecParser;
use reth_cli_util::parse_socket_address;
use reth_discv4::{DEFAULT_DISCOVERY_ADDR, DEFAULT_DISCOVERY_PORT};
use reth_ethereum_cli::chainspec::EthereumChainSpecParser;
use reth_network_peers::TrustedPeer;
use reth_node_core::{
    dirs::{ChainPath, PlatformPath, XdgPath},
    utils::get_or_create_jwt_secret_from_path,
};
use reth_rpc_builder::constants::DEFAULT_AUTH_PORT;
use std::{
    env::VarError,
    fmt,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    str::FromStr,
    sync::Arc,
};

/// Ress CLI interface.
#[derive(Clone, Debug, Parser)]
#[command(author, version, about = "Ress", long_about = None)]
pub struct RessArgs {
    /// The chain this node is running.
    ///
    /// Possible values are either a built-in chain or the path to a chain specification file.
    #[arg(
        long,
        value_name = "CHAIN_OR_PATH",
        long_help = EthereumChainSpecParser::help_message(),
        default_value = EthereumChainSpecParser::SUPPORTED_CHAINS[0],
        value_parser = EthereumChainSpecParser::parser()
    )]
    pub chain: Arc<ChainSpec>,

    /// The path to the data dir for all ress files and subdirectories.
    ///
    /// Defaults to the OS-specific data directory:
    ///
    /// - Linux: `$XDG_DATA_HOME/ress/` or `$HOME/.local/share/ress/`
    /// - Windows: `{FOLDERID_RoamingAppData}/ress/`
    /// - macOS: `$HOME/Library/Application Support/ress/`
    #[arg(long, value_name = "DATA_DIR", verbatim_doc_comment, default_value_t)]
    pub datadir: MaybePlatformPath<DataDirPath>,

    /// Network args.
    #[clap(flatten)]
    pub network: RessNetworkArgs,

    /// RPC args.
    #[clap(flatten)]
    pub rpc: RessRpcArgs,

    /// Debug args.
    #[clap(flatten)]
    pub debug: DebugArgs,

    /// Enable Prometheus metrics.
    ///
    /// The metrics will be served at the given interface and port.
    #[arg(long, value_name = "SOCKET", value_parser = parse_socket_address, help_heading = "Metrics")]
    pub metrics: Option<SocketAddr>,
}

/// Ress networking args.
#[derive(Clone, Debug, Args)]
pub struct RessNetworkArgs {
    /// Network listening address
    #[arg(long = "addr", value_name = "ADDR", default_value_t = DEFAULT_DISCOVERY_ADDR)]
    pub addr: IpAddr,

    /// Network listening port
    #[arg(long = "port", value_name = "PORT", default_value_t = DEFAULT_DISCOVERY_PORT)]
    pub port: u16,

    /// Secret key to use for this node.
    ///
    /// This will also deterministically set the peer ID. If not specified, it will be set in the
    /// data dir for the chain being used.
    #[arg(long, value_name = "PATH")]
    pub p2p_secret_key: Option<PathBuf>,

    /// Maximum active connections for `ress` subprotocol.
    #[arg(long, default_value_t = 256)]
    pub max_active_connections: u64,

    #[allow(clippy::doc_markdown)]
    /// Comma separated enode URLs of trusted peers for P2P connections.
    ///
    /// --remote-peer enode://abcd@192.168.0.1:30303
    #[arg(long, value_delimiter = ',')]
    pub trusted_peers: Vec<TrustedPeer>,
}

impl RessNetworkArgs {
    /// Returns network socket address.
    pub fn listener_addr(&self) -> SocketAddr {
        SocketAddr::new(self.addr, self.port)
    }

    /// Returns path to network secret.
    pub fn network_secret_path(&self, data_dir: &ChainPath<DataDirPath>) -> PathBuf {
        self.p2p_secret_key.clone().unwrap_or_else(|| data_dir.p2p_secret())
    }
}

/// Ress RPC args.
#[derive(Clone, Debug, Args)]
pub struct RessRpcArgs {
    /// Auth server address to listen on
    #[arg(long = "authrpc.addr", default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
    pub auth_addr: IpAddr,

    /// Auth server port to listen on
    #[arg(long = "authrpc.port", default_value_t = DEFAULT_AUTH_PORT)]
    pub auth_port: u16,

    /// Path to a JWT secret to use for the authenticated engine-API RPC server.
    ///
    /// This will enforce JWT authentication for all requests coming from the consensus layer.
    ///
    /// If no path is provided, a secret will be generated and stored in the datadir under
    /// `<DIR>/<CHAIN_ID>/jwt.hex`. For mainnet this would be `~/.ress/mainnet/jwt.hex` by default.
    #[arg(long = "authrpc.jwtsecret", value_name = "PATH", global = true, required = false)]
    pub auth_jwtsecret: Option<PathBuf>,
}

impl RessRpcArgs {
    /// Returns auth RPC socker address.
    pub fn auth_rpc_addr(&self) -> SocketAddr {
        SocketAddr::new(self.auth_addr, self.auth_port)
    }

    /// Reads and returns JWT secret at user provider path _or_
    /// reads or creates and returns JWT secret at default path.
    pub fn auth_jwt_secret(&self, default_jwt_path: PathBuf) -> Result<JwtSecret, JwtError> {
        match self.auth_jwtsecret.as_ref() {
            Some(fpath) => {
                tracing::debug!(target: "ress::cli", user_path=?fpath, "Reading JWT auth secret file");
                JwtSecret::from_file(fpath)
            }
            None => get_or_create_jwt_secret_from_path(&default_jwt_path),
        }
    }
}

/// Ress debug args.
#[derive(Clone, Debug, Args)]
pub struct DebugArgs {
    /// Url for debug consensus client.
    #[arg(long = "debug.debug-consensus-url")]
    pub debug_consensus_url: Option<String>,

    /// Url for RPC adapter.
    #[arg(long = "debug.rpc-network-adapter-url")]
    pub rpc_network_adapter_url: Option<String>,
}

/// Returns the path to the ress data dir.
///
/// The data dir should contain a subdirectory for each chain, and those chain directories will
/// include all information for that chain, such as the p2p secret.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct DataDirPath;

impl XdgPath for DataDirPath {
    fn resolve() -> Option<PathBuf> {
        data_dir()
    }
}

/// Returns the path to the ress data directory.
///
/// Refer to [`dirs_next::data_dir`] for cross-platform behavior.
pub fn data_dir() -> Option<PathBuf> {
    dirs_next::data_dir().map(|root| root.join("ress"))
}

/// Returns the path to the ress database.
///
/// Refer to [`dirs_next::data_dir`] for cross-platform behavior.
pub fn database_path() -> Option<PathBuf> {
    data_dir().map(|root| root.join("db"))
}

/// An Optional wrapper type around [`PlatformPath`].
///
/// This is useful for when a path is optional, such as the `--data-dir` flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaybePlatformPath<D>(Option<PlatformPath<D>>);

// === impl MaybePlatformPath ===

impl<D: XdgPath> MaybePlatformPath<D> {
    /// Returns the path if it is set, otherwise returns the default path for the given chain.
    pub fn unwrap_or_chain_default(&self, chain: Chain) -> ChainPath<D> {
        ChainPath::new(
            self.0.clone().unwrap_or_else(|| PlatformPath::default().join(chain.to_string())),
            chain,
            Default::default(),
        )
    }
}

impl<D: XdgPath> fmt::Display for MaybePlatformPath<D> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(path) = &self.0 {
            path.fmt(f)
        } else {
            // NOTE: this is a workaround for making it work with clap's `default_value_t` which
            // computes the default value via `Default -> Display -> FromStr`
            write!(f, "default")
        }
    }
}

impl<D> Default for MaybePlatformPath<D> {
    fn default() -> Self {
        Self(None)
    }
}

impl<D> FromStr for MaybePlatformPath<D> {
    type Err = shellexpand::LookupError<VarError>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let p = match s {
            "default" => {
                // NOTE: this is a workaround for making it work with clap's `default_value_t` which
                // computes the default value via `Default -> Display -> FromStr`
                None
            }
            _ => Some(PlatformPath::from_str(s)?),
        };
        Ok(Self(p))
    }
}

// impl<D> From<PathBuf> for MaybePlatformPath<D> {
//     fn from(path: PathBuf) -> Self {
//         Self(Some(PlatformPath(path, std::marker::PhantomData)))
//     }
// }
