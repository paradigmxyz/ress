use crate::{NodeType, StateWitnessNet};
use alloy_primitives::{
    bytes::{Buf, BufMut},
    BlockHash, Bytes, B256,
};
use alloy_rlp::{BytesMut, Decodable, Encodable};
use reth_eth_wire::{protocol::Protocol, Capability};

/// Represents message IDs for `ress` protocol messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RessMessageType {
    /// Disconnect message.
    Disconnect = 0x00,
    /// Node type message.
    NodeType = 0x01,

    /// Witness request message.
    GetWitness = 0x02,
    /// Witness response message.
    Witness = 0x03,

    /// Bytecode request message.
    GetBytecode = 0x04,
    /// Bytecode response message.
    Bytecode = 0x05,
}

impl Encodable for RessMessageType {
    fn encode(&self, out: &mut dyn BufMut) {
        out.put_u8(*self as u8);
    }

    fn length(&self) -> usize {
        1
    }
}

impl Decodable for RessMessageType {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let id = match buf.first().ok_or(alloy_rlp::Error::InputTooShort)? {
            0x00 => Self::Disconnect,
            0x01 => Self::NodeType,
            0x02 => Self::GetWitness,
            0x03 => Self::Witness,
            0x04 => Self::GetBytecode,
            0x05 => Self::Bytecode,
            _ => return Err(alloy_rlp::Error::Custom("Invalid message type")),
        };
        buf.advance(1);
        Ok(id)
    }
}

/// Represents a message in the ress protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RessMessageKind {
    /// Represents disconnect message.
    Disconnect,

    /// Represents a node type message required for handshake.
    NodeType(NodeType),

    /// Represents a witness request message.
    GetWitness(BlockHash),
    /// Represents a witness response message.
    Witness(StateWitnessNet),

    /// Represents a bytecode request message.
    GetBytecode(B256),
    /// Represents a bytecode response message.
    Bytecode(Bytes),
}

impl Encodable for RessMessageKind {
    fn encode(&self, out: &mut dyn BufMut) {
        match self {
            Self::Disconnect => (),
            Self::NodeType(node_type) => node_type.encode(out),
            Self::GetWitness(request) => request.encode(out),
            Self::Witness(witness) => witness.encode(out),
            Self::GetBytecode(request) => request.encode(out),
            Self::Bytecode(bytecode) => bytecode.encode(out),
        }
    }

    fn length(&self) -> usize {
        match self {
            Self::Disconnect => 0,
            Self::NodeType(node_type) => node_type.length(),
            Self::GetWitness(request) => request.length(),
            Self::Witness(witness) => witness.length(),
            Self::GetBytecode(request) => request.length(),
            Self::Bytecode(bytecode) => bytecode.length(),
        }
    }
}

/// Ress RLPx message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RessProtocolMessage {
    /// Message type.
    pub message_type: RessMessageType,
    /// Message data.
    pub message: RessMessageKind,
}

impl RessProtocolMessage {
    /// Returns the capability for the `ress` protocol.
    pub fn capability() -> Capability {
        Capability::new_static("ress", 1)
    }

    /// Returns the protocol for the `ress` protocol.
    pub fn protocol() -> Protocol {
        Protocol::new(Self::capability(), 6)
    }

    /// Disconnect.
    pub fn disconnect() -> Self {
        Self {
            message_type: RessMessageType::Disconnect,
            message: RessMessageKind::Disconnect,
        }
    }

    /// Create node type message.
    pub fn node_type(node_type: NodeType) -> Self {
        Self {
            message_type: RessMessageType::NodeType,
            message: RessMessageKind::NodeType(node_type),
        }
    }

    /// Execution witness request.
    pub fn get_witness(block_hash: BlockHash) -> Self {
        Self {
            message_type: RessMessageType::GetWitness,
            message: RessMessageKind::GetWitness(block_hash),
        }
    }

    /// Execution witness response.
    pub fn witness(witness: StateWitnessNet) -> Self {
        Self {
            message_type: RessMessageType::Witness,
            message: RessMessageKind::Witness(witness),
        }
    }

    /// Bytecode request.
    pub fn get_bytecode(code_hash: B256) -> Self {
        Self {
            message_type: RessMessageType::GetBytecode,
            message: RessMessageKind::GetBytecode(code_hash),
        }
    }

    /// Bytecode response.
    pub fn bytecode(bytecode: Bytes) -> Self {
        Self {
            message_type: RessMessageType::Bytecode,
            message: RessMessageKind::Bytecode(bytecode),
        }
    }

    /// Return RLP encoded message.
    pub fn encoded(&self) -> BytesMut {
        let mut buf = BytesMut::with_capacity(self.length());
        self.encode(&mut buf);
        buf
    }

    /// Decodes a `RessProtocolMessage` from the given message buffer.
    pub fn decode_message(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let message_type = RessMessageType::decode(buf)?;
        let message = match message_type {
            RessMessageType::Disconnect => RessMessageKind::Disconnect,
            RessMessageType::NodeType => RessMessageKind::NodeType(NodeType::decode(buf)?),
            RessMessageType::GetWitness => RessMessageKind::GetWitness(BlockHash::decode(buf)?),
            RessMessageType::Witness => RessMessageKind::Witness(StateWitnessNet::decode(buf)?),
            RessMessageType::GetBytecode => RessMessageKind::GetBytecode(B256::decode(buf)?),
            RessMessageType::Bytecode => RessMessageKind::Bytecode(Bytes::decode(buf)?),
        };
        Ok(Self {
            message_type,
            message,
        })
    }
}

impl Encodable for RessProtocolMessage {
    fn encode(&self, out: &mut dyn BufMut) {
        self.message_type.encode(out);
        self.message.encode(out);
    }

    fn length(&self) -> usize {
        self.message_type.length() + self.message.length()
    }
}
