use crate::protocol::proto::StateWitness;

use super::protocol::proto::{CustomRlpxProtoMessage, CustomRlpxProtoMessageKind, NodeType};
use alloy_primitives::{bytes::BytesMut, BlockHash, Bytes, B256};
use futures::{Stream, StreamExt};
use reth_eth_wire::multiplex::ProtocolConnection;
use std::{
    pin::Pin,
    task::{ready, Context, Poll},
};
use tokio::sync::oneshot;
use tokio_stream::wrappers::UnboundedReceiverStream;

pub(crate) mod handler;

/// We define some custom commands that the subprotocol supports.
pub enum CustomCommand {
    /// Sends a node type message to the peer
    NodeType { node_type: NodeType },
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
        response: oneshot::Sender<Bytes>,
    },
}

/// The connection handler for the custom RLPx protocol.
pub struct CustomRlpxConnection {
    conn: ProtocolConnection,
    commands: UnboundedReceiverStream<CustomCommand>,

    // below two type decides connection type
    original_node_type: Option<NodeType>,
    peer_node_type: Option<NodeType>,

    pending_witness: Option<oneshot::Sender<StateWitness>>,
    pending_bytecode: Option<oneshot::Sender<Bytes>>,
}

/// determine whether is valid node combination or not
fn is_valid_node_type_connection(original_node: &NodeType, peer_node: &NodeType) -> bool {
    match (original_node, peer_node) {
        // TODO: for now i wanna only do stateless -> stateful connection
        (NodeType::Stateless, NodeType::Stateful) => true,
        (NodeType::Stateful, NodeType::Stateless) => false,
        (NodeType::Stateful, NodeType::Stateful) => false,
        (NodeType::Stateless, NodeType::Stateless) => false,
    }
}

impl Stream for CustomRlpxConnection {
    type Item = BytesMut;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            if let Poll::Ready(Some(cmd)) = this.commands.poll_next_unpin(cx) {
                return match cmd {
                    CustomCommand::NodeType { node_type } => {
                        match &this.original_node_type {
                            Some(_) => {
                                this.peer_node_type = Some(node_type.clone());
                            }
                            None => {
                                this.original_node_type = Some(node_type.clone());
                            }
                        }

                        Poll::Ready(Some(CustomRlpxProtoMessage::node_type(node_type).encoded()))
                    }
                    CustomCommand::Witness {
                        block_hash,
                        response,
                    } => {
                        this.pending_witness = Some(response);
                        Poll::Ready(Some(
                            CustomRlpxProtoMessage::witness_req(block_hash).encoded(),
                        ))
                    }
                    CustomCommand::Bytecode {
                        code_hash,
                        response,
                    } => {
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
                    match &this.original_node_type {
                        Some(original_node_type) => {
                            if !is_valid_node_type_connection(original_node_type, &node_type) {
                                return Poll::Ready(Some(
                                    CustomRlpxProtoMessage::disconnect().encoded(),
                                ));
                            }
                        }
                        None => {}
                    }
                    return Poll::Ready(Some(
                        CustomRlpxProtoMessage::node_type(node_type).encoded(),
                    ));
                }
                CustomRlpxProtoMessageKind::Disconnect => {
                    return Poll::Ready(None);
                }
                CustomRlpxProtoMessageKind::WitnessReq(block_hash) => {
                    // TODO: get state witness from other full node peers
                    let state_witness = StateWitness::default();
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
                    let bytecode = Bytes::new();
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
            }

            return Poll::Pending;
        }
    }
}
