use reth::api::BeaconEngineMessage;
use reth::rpc::types::engine::PayloadStatus;
use reth_node_ethereum::EthEngineTypes;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::info;

pub struct ConsensusEngine {
    beacon_rx: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
}

impl ConsensusEngine {
    pub fn new(beacon_rx: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>) -> Self {
        Self { beacon_rx }
    }

    pub async fn run(mut self) {
        while let Some(beacon_msg) = self.beacon_rx.recv().await {
            self.handle_beacon_message(beacon_msg).await;
        }
    }

    async fn handle_beacon_message(&self, msg: BeaconEngineMessage<EthEngineTypes>) {
        match msg {
            BeaconEngineMessage::NewPayload {
                payload: new_payload,
                sidecar: _,
                tx,
            } => {
                info!("received new payload: {:?}", new_payload);
                let _ = tx.send(Ok(PayloadStatus::from_status(
                    reth::rpc::types::engine::PayloadStatusEnum::Accepted,
                )));
            }
            BeaconEngineMessage::ForkchoiceUpdated {
                state,
                payload_attrs,
                version,
                tx,
            } => {
                // Implement forkchoice updated handling
                todo!()
            }
            BeaconEngineMessage::TransitionConfigurationExchanged => {
                // Implement transition configuration handling
                todo!()
            }
        }
    }
}
