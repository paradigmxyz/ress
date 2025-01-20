use alloy_rpc_types_engine::{ClientCode, ClientVersionV1, JwtSecret};
use ress_common::test_utils::TestPeers;
use reth_chainspec::ChainSpec;
use reth_engine_primitives::BeaconConsensusEngineHandle;
use reth_node_api::BeaconEngineMessage;
use reth_node_ethereum::{node::EthereumEngineValidator, EthEngineTypes};
use reth_payload_builder::test_utils::spawn_test_payload_service;
use reth_primitives::EthPrimitives;
use reth_provider::noop::NoopProvider;
use reth_rpc_builder::auth::{AuthRpcModule, AuthServerConfig, AuthServerHandle};
use reth_rpc_engine_api::{capabilities::EngineCapabilities, EngineApi};
use reth_tasks::TokioTaskExecutor;
use reth_transaction_pool::noop::NoopTransactionPool;
use std::{net::SocketAddr, sync::Arc};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

// todo: add execution rpc later
pub struct RpcHandler {
    //  auth server handler
    pub authserver_handle: AuthServerHandle,

    // beacon engine receiver
    pub from_beacon_engine: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
}

impl RpcHandler {
    pub(crate) async fn start_server(id: TestPeers, chain_spec: Arc<ChainSpec>) -> Self {
        let (authserver_handle, from_beacon_engine) =
            Self::launch_auth_server(id.get_jwt_key(), id.get_authserver_addr(), chain_spec).await;

        Self {
            authserver_handle,
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
        let beacon_engine_handle = BeaconConsensusEngineHandle::<EthEngineTypes>::new(tx);
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
            EthereumEngineValidator::new(chain_spec.clone()),
        );
        let module = AuthRpcModule::new(engine_api);
        let handle = module.start_server(config).await.unwrap();
        (handle, rx)
    }
}
