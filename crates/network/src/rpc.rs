use std::{net::SocketAddr, sync::Arc};

use ress_common::test_utils::TestPeers;
use reth::{
    api::BeaconEngineMessage,
    beacon_consensus::{BeaconConsensusEngineEvent, BeaconConsensusEngineHandle},
    chainspec::ChainSpec,
    payload::test_utils::spawn_test_payload_service,
    primitives::EthPrimitives,
    providers::noop::NoopProvider,
    rpc::{
        builder::auth::{AuthRpcModule, AuthServerConfig, AuthServerHandle},
        types::engine::{ClientCode, ClientVersionV1, JwtSecret},
    },
    tasks::TokioTaskExecutor,
    transaction_pool::noop::NoopTransactionPool,
};
use reth_node_ethereum::{node::EthereumEngineValidator, EthEngineTypes};
use reth_rpc_engine_api::{capabilities::EngineCapabilities, EngineApi};
use reth_tokio_util::EventSender;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

// todo: add execution rpc later
pub struct RpcHandler {
    //  auth server handler
    pub authserver_handle: Arc<AuthServerHandle>,

    // beacon engine receiver
    pub from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
}

impl RpcHandler {
    pub async fn start_server(id: TestPeers, chain_spec: Arc<ChainSpec>) -> Self {
        let (authserver_handle, from_beacon_engine) =
            Self::launch_auth_server(id.get_jwt_key(), id.get_authserver_addr(), chain_spec).await;

        Self {
            authserver_handle: Arc::new(authserver_handle),
            from_beacon_engine,
        }
    }

    async fn launch_auth_server(
        jwt_key: JwtSecret,
        socket: SocketAddr,
        chain_spec: Arc<ChainSpec>,
    ) -> (
        AuthServerHandle,
        UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
    ) {
        let config = AuthServerConfig::builder(jwt_key)
            .socket_addr(socket)
            .build();
        let (tx, rx) = unbounded_channel();
        let event_handler: EventSender<BeaconConsensusEngineEvent> = Default::default();
        // Create a listener for the events
        let mut _listener = event_handler.new_listener();
        let beacon_engine_handle =
            BeaconConsensusEngineHandle::<EthEngineTypes>::new(tx, event_handler);
        let client = ClientVersionV1 {
            code: ClientCode::RH,
            name: "Ress".to_string(),
            version: "".to_string(),
            commit: "".to_string(),
        };

        let engine_api = EngineApi::new(
            NoopProvider::<ChainSpec, EthPrimitives>::new(chain_spec.clone()),
            chain_spec.clone(),
            beacon_engine_handle,
            spawn_test_payload_service().into(),
            NoopTransactionPool::default(),
            Box::<TokioTaskExecutor>::default(),
            client,
            EngineCapabilities::default(),
            EthereumEngineValidator::new(chain_spec.clone().into()),
        );
        let module = AuthRpcModule::new(engine_api);
        let handle = module.start_server(config).await.unwrap();
        (handle, rx)
    }
}
