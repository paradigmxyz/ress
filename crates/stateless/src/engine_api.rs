//! most of code is from https://github.com/paradigmxyz/reth/blob/2a225352d7678c19722c55e56c2de63dceab59e5/crates/rpc/rpc-engine-api/src/engine_api.rs#L138-L316

use std::time::Instant;

use alloy_eips::eip7685::{Requests, RequestsOrHash};
use alloy_primitives::B256;
use parking_lot::Mutex;
use reth::{
    api::{
        BeaconEngineMessage, EngineApiMessageVersion, EngineTypes, EngineValidator,
        PayloadOrAttributes,
    },
    beacon_consensus::BeaconConsensusEngineHandle,
    rpc::{
        compat::engine::payload::convert_payload_input_v2_to_payload,
        types::engine::{
            CancunPayloadFields, ExecutionPayload, ExecutionPayloadInputV2,
            ExecutionPayloadSidecar, ExecutionPayloadV1, ExecutionPayloadV3, ForkchoiceState,
            ForkchoiceUpdated, PayloadStatus, PraguePayloadFields,
        },
    },
};
use reth_rpc_engine_api::EngineApiResult;
use tokio::sync::mpsc::UnboundedSender;

struct EngineApi<EngineT, Validator>
where
    EngineT: EngineTypes,
    Validator: EngineValidator<EngineT>,
{
    engine_tx: UnboundedSender<BeaconEngineMessage<EngineT>>,
    /// Engine validator.
    validator: Validator,
    /// The channel to send messages to the beacon consensus engine.
    beacon_consensus: BeaconConsensusEngineHandle<EngineT>,
    /// Start time of the latest payload request
    latest_new_payload_response: Mutex<Option<Instant>>,
}

impl<EngineT, Validator> EngineApi<EngineT, Validator>
where
    EngineT: EngineTypes,
    Validator: EngineValidator<EngineT>,
{
    pub fn new(
        engine_tx: UnboundedSender<BeaconEngineMessage<EngineT>>,
        beacon_consensus: BeaconConsensusEngineHandle<EngineT>,
        validator: Validator,
    ) -> Self {
        Self {
            engine_tx,
            validator,
            beacon_consensus,
            latest_new_payload_response: Mutex::new(None),
        }
    }

    /// Updates the timestamp for the latest new payload response.
    fn on_new_payload_response(&self) {
        self.latest_new_payload_response
            .lock()
            .replace(Instant::now());
    }

    /// See also <https://github.com/ethereum/execution-apis/blob/3d627c95a4d3510a8187dd02e0250ecb4331d27e/src/engine/paris.md#engine_newpayloadv1>
    /// Caution: This should not accept the `withdrawals` field
    pub async fn new_payload_v1(
        &self,
        payload: ExecutionPayloadV1,
    ) -> EngineApiResult<PayloadStatus> {
        let payload = ExecutionPayload::from(payload);
        let payload_or_attrs =
            PayloadOrAttributes::<'_, EngineT::PayloadAttributes>::from_execution_payload(
                &payload, None,
            );
        self.validator
            .validate_version_specific_fields(EngineApiMessageVersion::V1, payload_or_attrs)?;

        Ok(self
            .beacon_consensus
            .new_payload(payload, ExecutionPayloadSidecar::none())
            .await
            .inspect(|_| self.on_new_payload_response())?)
    }

    /// See also <https://github.com/ethereum/execution-apis/blob/584905270d8ad665718058060267061ecfd79ca5/src/engine/shanghai.md#engine_newpayloadv2>
    pub async fn new_payload_v2(
        &self,
        payload: ExecutionPayloadInputV2,
    ) -> EngineApiResult<PayloadStatus> {
        let payload = convert_payload_input_v2_to_payload(payload);
        let payload_or_attrs =
            PayloadOrAttributes::<'_, EngineT::PayloadAttributes>::from_execution_payload(
                &payload, None,
            );
        self.validator
            .validate_version_specific_fields(EngineApiMessageVersion::V2, payload_or_attrs)?;
        Ok(self
            .beacon_consensus
            .new_payload(payload, ExecutionPayloadSidecar::none())
            .await
            .inspect(|_| self.on_new_payload_response())?)
    }

    /// See also <https://github.com/ethereum/execution-apis/blob/fe8e13c288c592ec154ce25c534e26cb7ce0530d/src/engine/cancun.md#engine_newpayloadv3>
    pub async fn new_payload_v3(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
    ) -> EngineApiResult<PayloadStatus> {
        let payload = ExecutionPayload::from(payload);
        let payload_or_attrs =
            PayloadOrAttributes::<'_, EngineT::PayloadAttributes>::from_execution_payload(
                &payload,
                Some(parent_beacon_block_root),
            );
        self.validator
            .validate_version_specific_fields(EngineApiMessageVersion::V3, payload_or_attrs)?;

        Ok(self
            .beacon_consensus
            .new_payload(
                payload,
                ExecutionPayloadSidecar::v3(CancunPayloadFields {
                    versioned_hashes,
                    parent_beacon_block_root,
                }),
            )
            .await
            .inspect(|_| self.on_new_payload_response())?)
    }

    /// See also <https://github.com/ethereum/execution-apis/blob/7907424db935b93c2fe6a3c0faab943adebe8557/src/engine/prague.md#engine_newpayloadv4>
    pub async fn new_payload_v4(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
        execution_requests: Requests,
    ) -> EngineApiResult<PayloadStatus> {
        let payload = ExecutionPayload::from(payload);
        let payload_or_attrs =
            PayloadOrAttributes::<'_, EngineT::PayloadAttributes>::from_execution_payload(
                &payload,
                Some(parent_beacon_block_root),
            );
        self.validator
            .validate_version_specific_fields(EngineApiMessageVersion::V4, payload_or_attrs)?;

        Ok(self
            .beacon_consensus
            .new_payload(
                payload,
                ExecutionPayloadSidecar::v4(
                    CancunPayloadFields {
                        versioned_hashes,
                        parent_beacon_block_root,
                    },
                    PraguePayloadFields {
                        requests: RequestsOrHash::Requests(execution_requests),
                        // TODO: add as an argument and handle in `try_into_block`
                        target_blobs_per_block: 0,
                    },
                ),
            )
            .await
            .inspect(|_| self.on_new_payload_response())?)
    }

    /// Sends a message to the beacon consensus engine to update the fork choice _without_
    /// withdrawals.
    ///
    /// See also <https://github.com/ethereum/execution-apis/blob/3d627c95a4d3510a8187dd02e0250ecb4331d27e/src/engine/paris.md#engine_forkchoiceUpdatedV1>
    ///
    /// Caution: This should not accept the `withdrawals` field
    pub async fn fork_choice_updated_v1(
        &self,
        state: ForkchoiceState,
        payload_attrs: Option<EngineT::PayloadAttributes>,
    ) -> EngineApiResult<ForkchoiceUpdated> {
        self.validate_and_execute_forkchoice(EngineApiMessageVersion::V1, state, payload_attrs)
            .await
    }

    /// Sends a message to the beacon consensus engine to update the fork choice _with_ withdrawals,
    /// but only _after_ shanghai.
    ///
    /// See also <https://github.com/ethereum/execution-apis/blob/3d627c95a4d3510a8187dd02e0250ecb4331d27e/src/engine/shanghai.md#engine_forkchoiceupdatedv2>
    pub async fn fork_choice_updated_v2(
        &self,
        state: ForkchoiceState,
        payload_attrs: Option<EngineT::PayloadAttributes>,
    ) -> EngineApiResult<ForkchoiceUpdated> {
        self.validate_and_execute_forkchoice(EngineApiMessageVersion::V2, state, payload_attrs)
            .await
    }

    /// Sends a message to the beacon consensus engine to update the fork choice _with_ withdrawals,
    /// but only _after_ cancun.
    ///
    /// See also  <https://github.com/ethereum/execution-apis/blob/main/src/engine/cancun.md#engine_forkchoiceupdatedv3>
    pub async fn fork_choice_updated_v3(
        &self,
        state: ForkchoiceState,
        payload_attrs: Option<EngineT::PayloadAttributes>,
    ) -> EngineApiResult<ForkchoiceUpdated> {
        self.validate_and_execute_forkchoice(EngineApiMessageVersion::V3, state, payload_attrs)
            .await
    }

    /// Validates the `engine_forkchoiceUpdated` payload attributes and executes the forkchoice
    /// update.
    ///
    /// The payload attributes will be validated according to the engine API rules for the given
    /// message version:
    /// * If the version is [`EngineApiMessageVersion::V1`], then the payload attributes will be
    ///   validated according to the Paris rules.
    /// * If the version is [`EngineApiMessageVersion::V2`], then the payload attributes will be
    ///   validated according to the Shanghai rules, as well as the validity changes from cancun:
    ///   <https://github.com/ethereum/execution-apis/blob/584905270d8ad665718058060267061ecfd79ca5/src/engine/cancun.md#update-the-methods-of-previous-forks>
    ///
    /// * If the version above [`EngineApiMessageVersion::V3`], then the payload attributes will be
    ///   validated according to the Cancun rules.
    async fn validate_and_execute_forkchoice(
        &self,
        version: EngineApiMessageVersion,
        state: ForkchoiceState,
        payload_attrs: Option<EngineT::PayloadAttributes>,
    ) -> EngineApiResult<ForkchoiceUpdated> {
        if let Some(ref attrs) = payload_attrs {
            let attr_validation_res = self.validator.ensure_well_formed_attributes(version, attrs);

            // From the engine API spec:
            //
            // Client software MUST ensure that payloadAttributes.timestamp is greater than
            // timestamp of a block referenced by forkchoiceState.headBlockHash. If this condition
            // isn't held client software MUST respond with -38003: Invalid payload attributes and
            // MUST NOT begin a payload build process. In such an event, the forkchoiceState
            // update MUST NOT be rolled back.
            //
            // NOTE: This will also apply to the validation result for the cancun or
            // shanghai-specific fields provided in the payload attributes.
            //
            // To do this, we set the payload attrs to `None` if attribute validation failed, but
            // we still apply the forkchoice update.
            if let Err(err) = attr_validation_res {
                let fcu_res = self
                    .beacon_consensus
                    .fork_choice_updated(state, None, version)
                    .await?;
                // TODO: decide if we want this branch - the FCU INVALID response might be more
                // useful than the payload attributes INVALID response
                if fcu_res.is_invalid() {
                    return Ok(fcu_res);
                }
                return Err(err.into());
            }
        }

        Ok(self
            .beacon_consensus
            .fork_choice_updated(state, payload_attrs, version)
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_matches::assert_matches;
    use std::sync::Arc;

    use reth::{
        beacon_consensus::BeaconConsensusEngineEvent,
        chainspec::{ChainSpec, MAINNET},
        primitives::SealedBlock,
        rpc::compat::engine::payload::execution_payload_from_sealed_block,
    };
    use reth_node_ethereum::{node::EthereumEngineValidator, EthEngineTypes};
    use reth_tokio_util::EventSender;
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};

    struct EngineApiTestHandle {
        chain_spec: Arc<ChainSpec>,
        from_api: UnboundedReceiver<BeaconEngineMessage<EthEngineTypes>>,
    }

    fn setup_engine_api() -> (
        EngineApiTestHandle,
        EngineApi<EthEngineTypes, EthereumEngineValidator>,
    ) {
        let chain_spec: Arc<ChainSpec> = MAINNET.clone();

        let (to_engine, engine_rx) = unbounded_channel();
        let event_sender: EventSender<BeaconConsensusEngineEvent> = Default::default();
        let api = EngineApi::new(
            to_engine.clone(),
            BeaconConsensusEngineHandle::new(to_engine.clone(), event_sender),
            EthereumEngineValidator::new(chain_spec.clone()),
        );
        let handle = EngineApiTestHandle {
            chain_spec,
            from_api: engine_rx,
        };

        (handle, api)
    }

    #[tokio::test]
    async fn forwards_responses_to_consensus_engine() {
        let (mut handle, api) = setup_engine_api();

        tokio::spawn(async move {
            api.new_payload_v1(execution_payload_from_sealed_block(SealedBlock::default()))
                .await
                .unwrap();
        });
        assert_matches!(
            handle.from_api.recv().await,
            Some(BeaconEngineMessage::NewPayload { .. })
        );
    }
}
