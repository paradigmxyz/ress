//! RLPx protocol A. type B. witness C. bytecode
//! following [RLPx specs](https://github.com/ethereum/devp2p/blob/master/rlpx.md)

use alloy_primitives::{
    bytes::{Buf, BufMut, BytesMut},
    map::HashMap,
    BlockHash, Bytes, B256,
};
use reth_eth_wire::{protocol::Protocol, Capability};

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CustomRlpxProtoMessageId {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CustomRlpxProtoMessageKind {
    // TODO: unsure how to disconnect after connection had established
    Disconnect,

    // A. node type
    NodeType(NodeType),

    // B. witness
    WitnessReq(BlockHash),
    WitnessRes(StateWitness),

    // C. bytecode
    BytecodeReq(B256),
    BytecodeRes(Bytes),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeType {
    Stateful,
    Stateless,
}

/// hashmap representation of multiple mpt state proof
/// `keccak(rlp(node)) -> rlp(nod)`
pub type StateWitness = HashMap<B256, Bytes>;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CustomRlpxProtoMessage {
    pub message_type: CustomRlpxProtoMessageId,
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
            message_type: CustomRlpxProtoMessageId::NodeType,
            message: CustomRlpxProtoMessageKind::NodeType(msg),
        }
    }

    /// Disconnect
    pub fn disconnect() -> Self {
        Self {
            message_type: CustomRlpxProtoMessageId::Disconnect,
            message: CustomRlpxProtoMessageKind::Disconnect,
        }
    }

    /// Request Witness
    pub fn witness_req(msg: BlockHash) -> Self {
        Self {
            message_type: CustomRlpxProtoMessageId::WitnessReq,
            message: CustomRlpxProtoMessageKind::WitnessReq(msg),
        }
    }

    /// Response Witness
    pub fn witness_res(msg: StateWitness) -> Self {
        Self {
            message_type: CustomRlpxProtoMessageId::WitnessRes,
            message: CustomRlpxProtoMessageKind::WitnessRes(msg),
        }
    }

    /// Request Bytecode
    pub fn bytecode_req(msg: B256) -> Self {
        Self {
            message_type: CustomRlpxProtoMessageId::BytecodeReq,
            message: CustomRlpxProtoMessageKind::BytecodeReq(msg),
        }
    }

    /// Response Bytecode
    pub fn bytecode_res(msg: Bytes) -> Self {
        Self {
            message_type: CustomRlpxProtoMessageId::BytecodeRes,
            message: CustomRlpxProtoMessageKind::BytecodeRes(msg),
        }
    }

    /// Creates a new `CustomRlpxProtoMessage` with the given message ID and payload.
    pub fn encoded(&self) -> BytesMut {
        let mut buf = BytesMut::new();
        buf.put_u8(self.message_type as u8);
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
                buf.put(&msg.0[..]);
            }
            CustomRlpxProtoMessageKind::BytecodeRes(msg) => {
                buf.put(&msg.0[..]);
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
            0x00 => CustomRlpxProtoMessageId::Disconnect,
            0x01 => CustomRlpxProtoMessageId::NodeType,
            0x02 => CustomRlpxProtoMessageId::WitnessReq,
            0x03 => CustomRlpxProtoMessageId::WitnessRes,
            0x04 => CustomRlpxProtoMessageId::BytecodeReq,
            0x05 => CustomRlpxProtoMessageId::BytecodeRes,
            _ => return None,
        };
        let message = match message_type {
            CustomRlpxProtoMessageId::NodeType => {
                CustomRlpxProtoMessageKind::NodeType(NodeType::from_bytes(&buf[..]))
            }
            CustomRlpxProtoMessageId::WitnessReq => {
                CustomRlpxProtoMessageKind::WitnessReq(B256::from_slice(&buf[..]))
            }
            CustomRlpxProtoMessageId::WitnessRes => {
                let deserialize: StateWitness =
                    bincode::deserialize(&buf[..]).expect("Failed to serialize message");
                CustomRlpxProtoMessageKind::WitnessRes(deserialize)
            }
            CustomRlpxProtoMessageId::BytecodeReq => {
                CustomRlpxProtoMessageKind::BytecodeReq(B256::from_slice(&buf[..]))
            }
            CustomRlpxProtoMessageId::BytecodeRes => {
                CustomRlpxProtoMessageKind::BytecodeRes(Bytes::copy_from_slice(&buf[..]))
            }
            CustomRlpxProtoMessageId::Disconnect => CustomRlpxProtoMessageKind::Disconnect,
        };

        Some(Self {
            message_type,
            message,
        })
    }
}
