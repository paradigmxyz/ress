use crate::protocol::proto::StateWitness;

use super::protocol::proto::{CustomRlpxProtoMessage, CustomRlpxProtoMessageKind, NodeType};
use alloy_primitives::{bytes::BytesMut, BlockHash, B256};
use futures::{Stream, StreamExt};
use reth::revm::primitives::Bytecode;
use reth_eth_wire::multiplex::ProtocolConnection;
use std::{
    pin::Pin,
    task::{ready, Context, Poll},
};
use tokio::sync::oneshot;
use tokio_stream::wrappers::UnboundedReceiverStream;

pub(crate) mod handler;

/// Custom commands that the subprotocol supports.
pub enum CustomCommand {
    /// Sends a node type message to the peer
    NodeType {
        node_type: NodeType,
        /// The response will be sent to this channel.
        response: oneshot::Sender<bool>,
    },
    /// Get witness for specific block
    Witness {
        /// target block hash that we want to get witness from
        block_hash: BlockHash,
        /// The response will be sent to this channel.
        response: oneshot::Sender<StateWitness>,
    },
    /// Get bytecode for specific codehash
    Bytecode {
        /// target code hash that we want to get bytecode from
        code_hash: B256,
        /// The response will be sent to this channel.
        response: oneshot::Sender<Bytecode>,
    },
}

/// The connection handler for the custom RLPx protocol.
pub struct CustomRlpxConnection {
    conn: ProtocolConnection,
    commands: UnboundedReceiverStream<CustomCommand>,

    // below two type decides connection type
    original_node_type: NodeType,
    peer_node_type: Option<NodeType>,

    pending_is_valid_connection: Option<oneshot::Sender<bool>>,
    pending_witness: Option<oneshot::Sender<StateWitness>>,
    pending_bytecode: Option<oneshot::Sender<Bytecode>>,
}

/// determine whether is valid node combination or not
fn is_valid_node_type_connection(original_node: &NodeType, peer_node: &NodeType) -> bool {
    match (original_node, peer_node) {
        (NodeType::Stateless, NodeType::Stateful) => true,
        (NodeType::Stateful, NodeType::Stateless) => true,
        (NodeType::Stateful, NodeType::Stateful) => false,
        (NodeType::Stateless, NodeType::Stateless) => true,
    }
}

impl Stream for CustomRlpxConnection {
    type Item = BytesMut;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            if let Poll::Ready(Some(cmd)) = this.commands.poll_next_unpin(cx) {
                return match cmd {
                    CustomCommand::NodeType {
                        node_type,
                        response,
                    } => {
                        print!("👀");
                        this.peer_node_type = Some(node_type.clone());
                        this.pending_is_valid_connection = Some(response);
                        Poll::Ready(Some(CustomRlpxProtoMessage::node_type(node_type).encoded()))
                    }
                    CustomCommand::Witness {
                        block_hash,
                        response,
                    } => {
                        print!("⭐️");
                        this.pending_witness = Some(response);
                        Poll::Ready(Some(
                            CustomRlpxProtoMessage::witness_req(block_hash).encoded(),
                        ))
                    }
                    CustomCommand::Bytecode {
                        code_hash,
                        response,
                    } => {
                        print!("🚀");
                        this.pending_bytecode = Some(response);
                        Poll::Ready(Some(
                            CustomRlpxProtoMessage::bytecode_req(code_hash).encoded(),
                        ))
                    }
                };
            }

            let Some(msg) = ready!(this.conn.poll_next_unpin(cx)) else {
                return Poll::Ready(None);
            };

            let Some(msg) = CustomRlpxProtoMessage::decode_message(&mut &msg[..]) else {
                return Poll::Ready(None);
            };

            match msg.message {
                CustomRlpxProtoMessageKind::NodeType(node_type) => {
                    print!("👀👀👀");
                    if !is_valid_node_type_connection(&this.original_node_type, &node_type) {
                        println!("🔴 invalid conenction!");
                        return Poll::Ready(Some(CustomRlpxProtoMessage::disconnect().encoded()));
                    } else {
                        println!("🟢 valid conenction!");
                        if let Some(sender) = this.pending_is_valid_connection.take() {
                            sender.send(true).ok();
                        }
                        continue;
                    }
                }
                CustomRlpxProtoMessageKind::Disconnect => {
                    // TODO: this actually doesn't disconnecting the channel. How can i gracefully stop
                    return Poll::Ready(None);
                }
                CustomRlpxProtoMessageKind::WitnessReq(block_hash) => {
                    // TODO: get state witness from other full node peers
                    println!("🟢 requested for blockhash {}!", block_hash);

                    // [mock]
                    let mut state_witness = StateWitness::default();
                    state_witness.insert(B256::ZERO, [0x00].into());

                    return Poll::Ready(Some(
                        CustomRlpxProtoMessage::witness_res(state_witness).encoded(),
                    ));
                }
                CustomRlpxProtoMessageKind::WitnessRes(msg) => {
                    if let Some(sender) = this.pending_witness.take() {
                        sender.send(msg).ok();
                    }
                    continue;
                }
                CustomRlpxProtoMessageKind::BytecodeReq(code_hash) => {
                    // TODO: get bytecode from other full node peers
                    println!("🟢 requested for codehash {}!", code_hash);

                    // [mock]
                    let bytecode = Bytecode::default();
                    return Poll::Ready(Some(
                        CustomRlpxProtoMessage::bytecode_res(bytecode).encoded(),
                    ));
                }
                CustomRlpxProtoMessageKind::BytecodeRes(msg) => {
                    if let Some(sender) = this.pending_bytecode.take() {
                        sender.send(msg).ok();
                    }
                    continue;
                }
            };
        }
    }
}
