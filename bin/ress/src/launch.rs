use alloy_rpc_types_engine::{ClientCode, ClientVersionV1, JwtSecret};
use ress_engine::engine::ConsensusEngine;
use ress_network::{RessNetworkHandle, RessNetworkManager};
use ress_protocol::{NodeType, ProtocolState, RessProtocolHandler, RessProtocolProvider};
use ress_provider::{RessDatabase, RessProvider};
use ress_testing::rpc_adapter::RpcAdapterProvider;
use reth_chainspec::ChainSpec;
use reth_consensus_debug_client::{DebugConsensusClient, RpcBlockProvider};
use reth_engine_tree::tree::error::InsertBlockFatalError;
use reth_ethereum_primitives::EthPrimitives;
use reth_network::{
    config::SecretKey, protocol::IntoRlpxSubProtocol, EthNetworkPrimitives, NetworkConfig,
    NetworkInfo, NetworkManager,
};
use reth_network_peers::TrustedPeer;
use reth_node_api::{BeaconConsensusEngineHandle, BeaconEngineMessage};
use reth_node_core::primitives::{RecoveredBlock, SealedBlock};
use reth_node_ethereum::{
    consensus::EthBeaconConsensus, node::EthereumEngineValidator, EthEngineTypes,
};
use reth_payload_builder::{noop::NoopPayloadBuilderService, PayloadStore};
use reth_rpc_builder::auth::{AuthRpcModule, AuthServerConfig, AuthServerHandle};
use reth_rpc_engine_api::{capabilities::EngineCapabilities, EngineApi};
use reth_storage_api::noop::NoopProvider;
use reth_tasks::TokioTaskExecutor;
use reth_transaction_pool::noop::NoopTransactionPool;
use std::sync::Arc;
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::*;

use crate::cli::RessArgs;

/// Ress node components.
#[derive(Debug)]
pub struct Node {
    /// Ress data provider.
    pub provider: RessProvider,
    /// P2P handle.
    pub network_handle: RessNetworkHandle,
    /// Auth RPC server handle.
    pub auth_server_handle: AuthServerHandle,
    /// Consensus engine handle.
    pub consensus_engine_handle: JoinHandle<Result<(), InsertBlockFatalError>>,
}

/// Ress node launcher
#[derive(Debug)]
pub struct NodeLauncher {
    /// Ress configuration.
    args: RessArgs,
}

impl NodeLauncher {
    /// Create new node launcher
    pub fn new(args: RessArgs) -> Self {
        Self { args }
    }
}

impl NodeLauncher {
    /// Launch ress node.
    pub async fn launch(self) -> eyre::Result<Node> {
        let data_dir = self.args.datadir.unwrap_or_chain_default(self.args.chain.chain());

        // Open database.
        let db_path = data_dir.db();
        debug!(target: "ress", path = %db_path.display(), "Opening database");
        let database = RessDatabase::new(&db_path)?;
        info!(target: "ress", path = %db_path.display(), "Database opened");
        let provider = RessProvider::new(self.args.chain.clone(), database);

        // Insert genesis block.
        let genesis_hash = self.args.chain.genesis_hash();
        let genesis_header = self.args.chain.genesis_header().clone();
        provider.insert_block(RecoveredBlock::new_sealed(
            SealedBlock::from_parts_unchecked(genesis_header, Default::default(), genesis_hash),
            Vec::new(),
        ));
        provider.insert_canonical_hash(0, genesis_hash);
        info!(target: "ress", %genesis_hash, "Inserted genesis block");

        // TODO: insert bytecodes from genesis

        // Launch network.
        let network_secret_path = self.args.network.network_secret_path(&data_dir);
        let network_secret = reth_cli_util::get_secret_key(&network_secret_path)?;

        let remote_peer = self.args.remote_peer.clone();
        let network_handle = if let Some(url) = self.args.debug.rpc_adapter_url.clone() {
            info!(target: "ress", %url, "Using RPC adapter");
            let rpc_adapter = RpcAdapterProvider::new(&url)?;
            self.launch_network(rpc_adapter, network_secret, remote_peer).await?
        } else {
            self.launch_network(provider.clone(), network_secret, remote_peer).await?
        };
        info!(target: "ress", peer_id = %network_handle.inner().peer_id(), addr = %network_handle.inner().local_addr(), "Network launched");

        // Spawn consensus engine.
        let (to_engine, from_auth_rpc) = mpsc::unbounded_channel();
        let engine_validator = EthereumEngineValidator::new(self.args.chain.clone());
        let consensus_engine = ConsensusEngine::new(
            provider.clone(),
            EthBeaconConsensus::new(self.args.chain.clone()),
            engine_validator.clone(),
            network_handle.clone(),
            from_auth_rpc,
        );
        let consensus_engine_handle = tokio::spawn(consensus_engine);
        info!(target: "ress", "Consensus engine spawned");

        // Start auth RPC server.
        let jwt_key = self.args.rpc.auth_jwt_secret(data_dir.jwt())?;
        let auth_server_handle =
            self.start_auth_server(jwt_key, engine_validator, to_engine).await?;
        info!(target: "ress", addr = %auth_server_handle.local_addr(), "Auth RPC server started");

        // Start debug consensus.
        if let Some(url) = self.args.debug.debug_consensus_url {
            let provider = Arc::new(RpcBlockProvider::new(url.clone()));
            tokio::spawn(
                DebugConsensusClient::new(auth_server_handle.clone(), provider)
                    .run::<EthEngineTypes>(),
            );
            info!(target: "ress", %url, "Debug consensus started");
        }

        Ok(Node { provider, network_handle, auth_server_handle, consensus_engine_handle })
    }

    async fn launch_network<P>(
        &self,
        protocol_provider: P,
        secret_key: SecretKey,
        remote_peer: Option<TrustedPeer>,
    ) -> eyre::Result<RessNetworkHandle>
    where
        P: RessProtocolProvider + Clone + Unpin + 'static,
    {
        // Configure and instantiate the network
        let (events_sender, protocol_events) = mpsc::unbounded_channel();
        let protocol_handler = RessProtocolHandler {
            provider: protocol_provider,
            node_type: NodeType::Stateless,
            state: ProtocolState { events_sender },
        };
        let config = NetworkConfig::builder(secret_key)
            .listener_addr(self.args.network.listener_addr())
            .disable_discovery()
            .add_rlpx_sub_protocol(protocol_handler.into_rlpx_sub_protocol())
            .build_with_noop_provider(self.args.chain.clone());
        let manager = NetworkManager::<EthNetworkPrimitives>::new(config).await?;

        if let Some(remote_peer) = remote_peer {
            let remote_addr = remote_peer.resolve_blocking()?.tcp_addr();
            manager.peers_handle().add_peer(remote_peer.id, remote_addr);
        }

        // get a handle to the network to interact with it
        let network_handle = manager.handle().clone();
        // spawn the network
        tokio::spawn(manager);

        let (peer_requests_tx, peer_requests_rx) = mpsc::unbounded_channel();
        // spawn ress network manager
        tokio::spawn(RessNetworkManager::new(
            UnboundedReceiverStream::from(protocol_events),
            UnboundedReceiverStream::from(peer_requests_rx),
        ));
        Ok(RessNetworkHandle::new(network_handle, peer_requests_tx))
    }

    async fn start_auth_server(
        &self,
        jwt_key: JwtSecret,
        engine_validator: EthereumEngineValidator,
        to_engine: mpsc::UnboundedSender<BeaconEngineMessage<EthEngineTypes>>,
    ) -> eyre::Result<AuthServerHandle> {
        let (_, payload_builder_handle) = NoopPayloadBuilderService::<EthEngineTypes>::new();
        let client_version = ClientVersionV1 {
            code: ClientCode::RH,
            name: "Ress".to_string(),
            version: "".to_string(),
            commit: "".to_string(),
        };
        let engine_api = EngineApi::new(
            NoopProvider::<ChainSpec, EthPrimitives>::new(self.args.chain.clone()),
            self.args.chain.clone(),
            BeaconConsensusEngineHandle::<EthEngineTypes>::new(to_engine),
            PayloadStore::new(payload_builder_handle),
            NoopTransactionPool::default(),
            Box::<TokioTaskExecutor>::default(),
            client_version,
            EngineCapabilities::default(),
            engine_validator,
        );
        let auth_socket = self.args.rpc.auth_rpc_addr();
        let config = AuthServerConfig::builder(jwt_key).socket_addr(auth_socket).build();
        Ok(AuthRpcModule::new(engine_api).start_server(config).await?)
    }
}
