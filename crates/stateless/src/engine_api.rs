//! most of code is from https://github.com/paradigmxyz/reth/blob/2a225352d7678c19722c55e56c2de63dceab59e5/crates/rpc/rpc-engine-api/src/engine_api.rs#L138-L316

use std::time::Instant;

use alloy_eips::eip7685::{Requests, RequestsOrHash};
use alloy_primitives::B256;
use parking_lot::Mutex;
use reth::{
    api::{EngineApiMessageVersion, EngineTypes, EngineValidator, PayloadOrAttributes},
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

struct EngineApi<EngineT, Validator>
where
    EngineT: EngineTypes,
    Validator: EngineValidator<EngineT>,
{
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
        beacon_consensus: BeaconConsensusEngineHandle<EngineT>,
        validator: Validator,
    ) -> Self {
        Self {
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
    use alloy_primitives::{b256, hex, Bytes, U256};
    use assert_matches::assert_matches;
    use std::sync::Arc;

    use reth::{
        api::BeaconEngineMessage,
        beacon_consensus::BeaconConsensusEngineEvent,
        chainspec::{ChainSpec, DEV},
        rpc::types::engine::ExecutionPayloadV2,
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
        let chain_spec: Arc<ChainSpec> = DEV.clone();

        let (to_engine, engine_rx) = unbounded_channel();
        let event_sender: EventSender<BeaconConsensusEngineEvent> = Default::default();
        let api = EngineApi::new(
            BeaconConsensusEngineHandle::new(to_engine, event_sender),
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
        // TODO: but why engine_new_payload_v3 is not working? i think it's valid payload from devnet (https://github.com/paradigmxyz/reth/blob/934fd1f7f07c42ea49b92fb15694209ee0b9530f/crates/rpc/rpc-types-compat/src/engine/payload.rs#L377)
        let first_transaction_raw = Bytes::from_static(&hex!("02f9017a8501a1f0ff438211cc85012a05f2008512a05f2000830249f094d5409474fd5a725eab2ac9a8b26ca6fb51af37ef80b901040cc7326300000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000001bdd2ed4b616c800000000000000000000000000001e9ee781dd4b97bdef92e5d1785f73a1f931daa20000000000000000000000007a40026a3b9a41754a95eec8c92c6b99886f440c000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000020000000000000000000000009ae80eb647dd09968488fa1d7e412bf8558a0b7a0000000000000000000000000f9815537d361cb02befd9918c95c97d4d8a4a2bc001a0ba8f1928bb0efc3fcd01524a2039a9a2588fa567cd9a7cc18217e05c615e9d69a0544bfd11425ac7748e76b3795b57a5563e2b0eff47b5428744c62ff19ccfc305")[..]);
        let second_transaction_raw = Bytes::from_static(&hex!("03f901388501a1f0ff430c843b9aca00843b9aca0082520894e7249813d8ccf6fa95a2203f46a64166073d58878080c005f8c6a00195f6dff17753fc89b60eac6477026a805116962c9e412de8015c0484e661c1a001aae314061d4f5bbf158f15d9417a238f9589783f58762cd39d05966b3ba2fba0013f5be9b12e7da06f0dd11a7bdc4e0db8ef33832acc23b183bd0a2c1408a757a0019d9ac55ea1a615d92965e04d960cb3be7bff121a381424f1f22865bd582e09a001def04412e76df26fefe7b0ed5e10580918ae4f355b074c0cfe5d0259157869a0011c11a415db57e43db07aef0de9280b591d65ca0cce36c7002507f8191e5d4a80a0c89b59970b119187d97ad70539f1624bbede92648e2dc007890f9658a88756c5a06fb2e3d4ce2c438c0856c2de34948b7032b1aadc4642a9666228ea8cdc7786b7")[..]);
        let new_payload = ExecutionPayloadV3 {
            payload_inner: ExecutionPayloadV2 {
                payload_inner: ExecutionPayloadV1 {
                    base_fee_per_gas:  U256::from(7u64),
                    block_number: 0xa946u64,
                    block_hash: hex!("a5ddd3f286f429458a39cafc13ffe89295a7efa8eb363cf89a1a4887dbcf272b").into(),
                    logs_bloom: hex!("00200004000000000000000080000000000200000000000000000000000000000000200000000000000000000000000000000000800000000200000000000000000000000000000000000008000000200000000000000000000001000000000000000000000000000000800000000000000000000100000000000030000000000000000040000000000000000000000000000000000800080080404000000000000008000000000008200000000000200000000000000000000000000000000000000002000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000100000000000000000000").into(),
                    extra_data: hex!("d883010d03846765746888676f312e32312e31856c696e7578").into(),
                    gas_limit: 0x1c9c380,
                    gas_used: 0x1f4a9,
                    timestamp: 0x651f35b8,
                    fee_recipient: hex!("f97e180c050e5ab072211ad2c213eb5aee4df134").into(),
                    parent_hash: hex!("d829192799c73ef28a7332313b3c03af1f2d5da2c36f8ecfafe7a83a3bfb8d1e").into(),
                    prev_randao: hex!("753888cc4adfbeb9e24e01c84233f9d204f4a9e1273f0e29b43c4c148b2b8b7e").into(),
                    receipts_root: hex!("4cbc48e87389399a0ea0b382b1c46962c4b8e398014bf0cc610f9c672bee3155").into(),
                    state_root: hex!("017d7fa2b5adb480f5e05b2c95cb4186e12062eed893fc8822798eed134329d1").into(),
                    transactions: vec![first_transaction_raw, second_transaction_raw],
                },
                withdrawals: vec![],
            },
            blob_gas_used: 0xc0000,
            excess_blob_gas: 0x580000,
        };
        let versioned_hashes = vec![];
        let parent_beacon_block_root =
            b256!("531cd53b8e68deef0ea65edfa3cda927a846c307b0907657af34bc3f313b5871");
        tokio::spawn(async move {
            api.new_payload_v3(new_payload, versioned_hashes, parent_beacon_block_root)
                .await
                .unwrap();
        });
        assert_matches!(
            handle.from_api.recv().await,
            Some(BeaconEngineMessage::NewPayload { .. })
        );
    }

    // async fn run(mut rx :  UnboundedReceiver<BeaconEngineMessage<Engine>>, statelessproto: ()) {
    //         loop {

    //             match rx.recv().await {

    //             }
    // }
}
