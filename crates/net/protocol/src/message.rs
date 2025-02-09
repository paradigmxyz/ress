//! Implements Ress protocol
//! Defines structs/enums for messages, request-response pairs.
//!
//! Examples include creating, encoding, and decoding protocol messages.

use crate::{NodeType, StateWitnessNet};
use alloy_primitives::{
    bytes::{Buf, BufMut},
    BlockHash, Bytes, B256,
};
use alloy_rlp::{BytesMut, Decodable, Encodable};
use reth_eth_wire::{message::RequestPair, protocol::Protocol, Capability};
use reth_primitives::Header;

/// An Ress protocol message, containing a message ID and payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RessProtocolMessage {
    /// The unique identifier representing the type of the Ress message.
    pub message_type: RessMessageID,
    /// The content of the message, including specific data based on the message type.
    pub message: RessMessage,
}

impl RessProtocolMessage {
    /// Returns the capability for the `ress` protocol.
    pub fn capability() -> Capability {
        Capability::new_static("ress", 1)
    }

    /// Returns the protocol for the `ress` protocol.
    pub fn protocol() -> Protocol {
        Protocol::new(Self::capability(), 7)
    }

    /// Create node type message.
    pub fn node_type(node_type: NodeType) -> Self {
        Self { message_type: RessMessageID::NodeType, message: RessMessage::NodeType(node_type) }
    }

    /// Header request.
    pub fn get_header(request_id: u64, block_hash: BlockHash) -> Self {
        Self {
            message_type: RessMessageID::GetHeader,
            message: RessMessage::GetHeader(RequestPair { request_id, message: block_hash }),
        }
    }

    /// Header response.
    pub fn header(request_id: u64, header: Header) -> Self {
        Self {
            message_type: RessMessageID::Header,
            message: RessMessage::Header(RequestPair { request_id, message: header }),
        }
    }

    /// Bytecode request.
    pub fn get_bytecode(request_id: u64, code_hash: B256) -> Self {
        Self {
            message_type: RessMessageID::GetBytecode,
            message: RessMessage::GetBytecode(RequestPair { request_id, message: code_hash }),
        }
    }

    /// Bytecode response.
    pub fn bytecode(request_id: u64, bytecode: Bytes) -> Self {
        Self {
            message_type: RessMessageID::Bytecode,
            message: RessMessage::Bytecode(RequestPair { request_id, message: bytecode }),
        }
    }

    /// Execution witness request.
    pub fn get_witness(request_id: u64, block_hash: BlockHash) -> Self {
        Self {
            message_type: RessMessageID::GetWitness,
            message: RessMessage::GetWitness(RequestPair { request_id, message: block_hash }),
        }
    }

    /// Execution witness response.
    pub fn witness(request_id: u64, witness: StateWitnessNet) -> Self {
        Self {
            message_type: RessMessageID::Witness,
            message: RessMessage::Witness(RequestPair { request_id, message: witness }),
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
        let message_type = RessMessageID::decode(buf)?;
        let message = match message_type {
            RessMessageID::NodeType => RessMessage::NodeType(NodeType::decode(buf)?),
            RessMessageID::GetHeader => RessMessage::GetHeader(RequestPair::decode(buf)?),
            RessMessageID::Header => RessMessage::Header(RequestPair::decode(buf)?),
            RessMessageID::GetBytecode => RessMessage::GetBytecode(RequestPair::decode(buf)?),
            RessMessageID::Bytecode => RessMessage::Bytecode(RequestPair::decode(buf)?),
            RessMessageID::GetWitness => RessMessage::GetWitness(RequestPair::decode(buf)?),
            RessMessageID::Witness => RessMessage::Witness(RequestPair::decode(buf)?),
        };
        Ok(Self { message_type, message })
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
/// Represents message IDs for `ress` protocol messages.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "arbitrary"), derive(arbitrary::Arbitrary))]
pub enum RessMessageID {
    /// Node type message.
    NodeType = 0x00,

    /// Header request message.
    GetHeader = 0x01,
    /// Header response message.
    Header = 0x02,

    /// Bytecode request message.
    GetBytecode = 0x03,
    /// Bytecode response message.
    Bytecode = 0x04,

    /// Witness request message.
    GetWitness = 0x05,
    /// Witness response message.
    Witness = 0x06,
}

impl Encodable for RessMessageID {
    fn encode(&self, out: &mut dyn BufMut) {
        out.put_u8(*self as u8);
    }

    fn length(&self) -> usize {
        1
    }
}

impl Decodable for RessMessageID {
    fn decode(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        let id = match buf.first().ok_or(alloy_rlp::Error::InputTooShort)? {
            0x00 => Self::NodeType,
            0x01 => Self::GetHeader,
            0x02 => Self::Header,
            0x03 => Self::GetBytecode,
            0x04 => Self::Bytecode,
            0x05 => Self::GetWitness,
            0x06 => Self::Witness,
            _ => return Err(alloy_rlp::Error::Custom("Invalid message type")),
        };
        buf.advance(1);
        Ok(id)
    }
}

/// Represents a message in the ress protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RessMessage {
    /// Represents a node type message required for handshake.
    NodeType(NodeType),

    /// Represents a header request message.
    GetHeader(RequestPair<B256>),
    /// Represents a header response message.
    Header(RequestPair<Header>),

    /// Represents a bytecode request message.
    GetBytecode(RequestPair<B256>),
    /// Represents a bytecode response message.
    Bytecode(RequestPair<Bytes>),

    /// Represents a witness request message.
    GetWitness(RequestPair<BlockHash>),
    /// Represents a witness response message.
    Witness(RequestPair<StateWitnessNet>),
}

impl Encodable for RessMessage {
    fn encode(&self, out: &mut dyn BufMut) {
        match self {
            Self::NodeType(node_type) => node_type.encode(out),
            Self::GetHeader(request) => request.encode(out),
            Self::Header(header) => header.encode(out),
            Self::GetBytecode(request) => request.encode(out),
            Self::Bytecode(bytecode) => bytecode.encode(out),
            Self::GetWitness(request) => request.encode(out),
            Self::Witness(witness) => witness.encode(out),
        }
    }

    fn length(&self) -> usize {
        match self {
            Self::NodeType(node_type) => node_type.length(),
            Self::GetHeader(request) => request.length(),
            Self::Header(header) => header.length(),
            Self::GetBytecode(request) => request.length(),
            Self::Bytecode(bytecode) => bytecode.length(),
            Self::GetWitness(request) => request.length(),
            Self::Witness(witness) => witness.length(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use proptest_arbitrary_interop::arb;
    use std::fmt;

    fn rlp_roundtrip<V>(value: V)
    where
        V: Encodable + Decodable + PartialEq + fmt::Debug,
    {
        let encoded = alloy_rlp::encode(&value);
        let decoded = V::decode(&mut &encoded[..]);
        assert_eq!(Ok(value), decoded);
    }

    proptest! {
        #[test]
        fn message_type_roundtrip(message_type in arb::<RessMessageID>()) {
            rlp_roundtrip(message_type);
        }
    }
}
