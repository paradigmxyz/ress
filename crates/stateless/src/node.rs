use jsonrpsee_http_client::transport::HttpBackend;
use reth::beacon_consensus::BeaconConsensusEngineEvent;
use reth::beacon_consensus::BeaconConsensusEngineHandle;
use reth::chainspec::MAINNET;
use reth::rpc::builder::auth::AuthServerConfig;
use reth::rpc::builder::auth::AuthServerConfigBuilder;
use reth::rpc::builder::auth::AuthServerHandle;
use reth::rpc::types::engine::ClientCode;
use reth::rpc::types::engine::ClientVersionV1;
use reth::rpc::types::engine::JwtSecret;
use reth::tasks::TokioTaskExecutor;
use reth::{
    chainspec::ChainSpec, rpc::builder::auth::AuthRpcModule,
    transaction_pool::noop::NoopTransactionPool,
};
use reth_node_ethereum::node::EthereumEngineValidator;
use reth_node_ethereum::EthEngineTypes;
use reth_payload_builder::test_utils::spawn_test_payload_service;
use reth_payload_builder::PayloadBuilderHandle;
use reth_provider::noop::NoopProvider;
use reth_provider::test_utils::MockEthProvider;
use reth_rpc_engine_api::capabilities::EngineCapabilities;
use reth_rpc_engine_api::EngineApi;
use reth_rpc_layer::AuthClientService;
use reth_tokio_util::EventSender;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::net::SocketAddrV4;
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;

pub struct Ress {}

impl Ress {
    pub async fn run() -> jsonrpsee::http_client::HttpClient<AuthClientService<HttpBackend>> {
        let secret = JwtSecret::random();
        let handle = Self::launch_auth(secret).await;
        let client = handle.http_client();
        client
    }

    pub async fn launch_auth(secret: JwtSecret) -> AuthServerHandle {
        let config = AuthServerConfig::builder(secret)
            .socket_addr(SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::LOCALHOST,
                61248,
            )))
            .build();
        let (tx, _rx) = unbounded_channel();
        let beacon_engine_handle =
            BeaconConsensusEngineHandle::<EthEngineTypes>::new(tx, Default::default());
        let client = ClientVersionV1 {
            code: ClientCode::RH,
            name: "Ress".to_string(),
            version: "v0.1.0".to_string(),
            commit: "defa64b2".to_string(),
        };

        let engine_api = EngineApi::new(
            NoopProvider::default(),
            MAINNET.clone(),
            beacon_engine_handle,
            spawn_test_payload_service().into(),
            NoopTransactionPool::default(),
            Box::<TokioTaskExecutor>::default(),
            client,
            EngineCapabilities::default(),
            EthereumEngineValidator::new(MAINNET.clone()),
        );
        let module = AuthRpcModule::new(engine_api);
        module.start_server(config).await.unwrap()
    }
}

#[cfg(test)]
mod tests {
    use jsonrpsee_core::client::ClientT;
    use reth::primitives::{Block, BlockExt, PooledTransaction};
    use reth_rpc_types_compat::engine::payload::block_to_payload_v1;

    use super::*;

    #[tokio::test]
    async fn test_run() {
        reth_tracing::init_test_tracing();
        let client = Ress::run().await;
        println!("client :{:?}", client);

        let block = Block::default().seal_slow();
        let payload = block_to_payload_v1::<PooledTransaction>(block.clone());
        println!("{:?}", payload);
        let params = (payload,);
        // Example RPC call: `engine_newPayloadV1`
        let result: bool = client
            .request("engine_newPayloadV1", params)
            .await
            .expect("Failed to send new payload");

        println!("Payload accepted: {}", result);
    }
}
