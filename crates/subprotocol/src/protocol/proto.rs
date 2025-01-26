//! RLPx protocol A. type B. witness C. bytecode
//! following [RLPx specs](https://github.com/ethereum/devp2p/blob/master/rlpx.md)

use alloy_primitives::{
    bytes::{Buf, BufMut, BytesMut},
    BlockHash, B256,
};
use ress_primitives::witness::ExecutionWitness;
use reth_eth_wire::{protocol::Protocol, Capability};
use reth_revm::primitives::Bytecode;
use serde::{Deserialize, Serialize};

#[allow(missing_docs)]
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CustomRlpxProtoMessageType {
    Disconnect = 0x00,
    // A. node type
    NodeType = 0x01,

    // B. witness
    WitnessReq = 0x02,
    WitnessRes = 0x03,

    // C. bytecode
    BytecodeReq = 0x04,
    BytecodeRes = 0x05,
}

#[allow(missing_docs)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CustomRlpxProtoMessageKind {
    Disconnect,

    // A. node type
    NodeType(NodeType),

    // B. witness
    WitnessReq(BlockHash),
    WitnessRes(ExecutionWitness),

    // C. bytecode
    BytecodeReq(BytecodeRequest),
    BytecodeRes(Option<Bytecode>),
}

/// Node type variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeType {
    /// Stateful reth node.
    Stateful,
    /// Stateless ress node.
    Stateless,
}

impl NodeType {
    /// `NodeType` to bytes
    fn as_bytes(&self) -> &[u8] {
        match self {
            NodeType::Stateful => &[0x00],
            NodeType::Stateless => &[0x01],
        }
    }

    /// bytes to `NodeType`
    fn from_bytes(v: &[u8]) -> Self {
        match v.first() {
            Some(0x00) => NodeType::Stateful,
            Some(0x01) => NodeType::Stateless,
            _ => panic!("not supported node type"),
        }
    }
}

/// Bytecode request
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BytecodeRequest {
    /// Bytecode hash.
    pub code_hash: B256,
    /// Block hash.
    // TODO: why is this needed?
    pub block_hash: BlockHash,
}

impl BytecodeRequest {
    /// Create new bytecode request.
    pub fn new(code_hash: B256, block_hash: BlockHash) -> Self {
        Self {
            code_hash,
            block_hash,
        }
    }
}

/// Ress RLPx message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomRlpxProtoMessage {
    /// Message type.
    pub ty: CustomRlpxProtoMessageType,
    /// Message data.
    pub message: CustomRlpxProtoMessageKind,
}

impl CustomRlpxProtoMessage {
    /// Returns the capability for the `custom_rlpx` protocol.
    pub fn capability() -> Capability {
        Capability::new_static("custom_rlpx", 1)
    }

    /// Returns the protocol for the `custom_rlpx` protocol.
    pub fn protocol() -> Protocol {
        Protocol::new(Self::capability(), 6)
    }

    /// Create node type message
    pub fn node_type(msg: NodeType) -> Self {
        Self {
            ty: CustomRlpxProtoMessageType::NodeType,
            message: CustomRlpxProtoMessageKind::NodeType(msg),
        }
    }

    /// Disconnect
    pub fn disconnect() -> Self {
        Self {
            ty: CustomRlpxProtoMessageType::Disconnect,
            message: CustomRlpxProtoMessageKind::Disconnect,
        }
    }

    /// Request Witness
    pub fn witness_req(msg: BlockHash) -> Self {
        Self {
            ty: CustomRlpxProtoMessageType::WitnessReq,
            message: CustomRlpxProtoMessageKind::WitnessReq(msg),
        }
    }

    /// Response Witness
    pub fn witness_res(msg: ExecutionWitness) -> Self {
        Self {
            ty: CustomRlpxProtoMessageType::WitnessRes,
            message: CustomRlpxProtoMessageKind::WitnessRes(msg),
        }
    }

    /// Request Bytecode
    pub fn bytecode_req(msg: BytecodeRequest) -> Self {
        Self {
            ty: CustomRlpxProtoMessageType::BytecodeReq,
            message: CustomRlpxProtoMessageKind::BytecodeReq(msg),
        }
    }

    /// Response Bytecode
    pub fn bytecode_res(msg: Option<Bytecode>) -> Self {
        Self {
            ty: CustomRlpxProtoMessageType::BytecodeRes,
            message: CustomRlpxProtoMessageKind::BytecodeRes(msg),
        }
    }

    /// Creates a new `CustomRlpxProtoMessage` with the given message ID and payload.
    pub fn encoded(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        buf.put_u8(self.ty as u8);
        match &self.message {
            CustomRlpxProtoMessageKind::NodeType(msg) => {
                buf.put(msg.as_bytes());
            }
            CustomRlpxProtoMessageKind::WitnessReq(msg) => {
                buf.put(&msg.0[..]);
            }
            CustomRlpxProtoMessageKind::WitnessRes(msg) => {
                let serialized = bincode::serialize(msg).expect("Failed to serialize message");
                buf.put(&serialized[..]);
            }
            CustomRlpxProtoMessageKind::BytecodeReq(msg) => {
                let serialized = bincode::serialize(msg).expect("Failed to serialize message");
                buf.put(&serialized[..]);
            }
            CustomRlpxProtoMessageKind::BytecodeRes(msg) => {
                let serialized = bincode::serialize(msg).expect("Failed to serialize message");
                buf.put(&serialized[..]);
            }
            CustomRlpxProtoMessageKind::Disconnect => {}
        }
        buf
    }

    /// Decodes a `CustomRlpxProtoMessage` from the given message buffer.
    pub fn decode_message(buf: &mut &[u8]) -> Option<Self> {
        if buf.is_empty() {
            return None;
        }
        let id = buf[0];
        buf.advance(1);
        let message_type = match id {
            0x00 => CustomRlpxProtoMessageType::Disconnect,
            0x01 => CustomRlpxProtoMessageType::NodeType,
            0x02 => CustomRlpxProtoMessageType::WitnessReq,
            0x03 => CustomRlpxProtoMessageType::WitnessRes,
            0x04 => CustomRlpxProtoMessageType::BytecodeReq,
            0x05 => CustomRlpxProtoMessageType::BytecodeRes,
            _ => return None,
        };
        let message = match message_type {
            CustomRlpxProtoMessageType::NodeType => {
                CustomRlpxProtoMessageKind::NodeType(NodeType::from_bytes(&buf[..]))
            }
            CustomRlpxProtoMessageType::WitnessReq => {
                CustomRlpxProtoMessageKind::WitnessReq(B256::from_slice(&buf[..]))
            }
            CustomRlpxProtoMessageType::WitnessRes => {
                let deserialize: ExecutionWitness =
                    bincode::deserialize(&buf[..]).expect("Failed to serialize message");
                CustomRlpxProtoMessageKind::WitnessRes(deserialize)
            }
            CustomRlpxProtoMessageType::BytecodeReq => {
                let deserialize: BytecodeRequest =
                    bincode::deserialize(&buf[..]).expect("Failed to serialize message");
                CustomRlpxProtoMessageKind::BytecodeReq(deserialize)
            }
            CustomRlpxProtoMessageType::BytecodeRes => {
                let deserialize: Option<Bytecode> =
                    bincode::deserialize(&buf[..]).expect("Failed to serialize message");
                CustomRlpxProtoMessageKind::BytecodeRes(deserialize)
            }
            CustomRlpxProtoMessageType::Disconnect => CustomRlpxProtoMessageKind::Disconnect,
        };

        Some(Self {
            ty: message_type,
            message,
        })
    }
}
