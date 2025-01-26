use crate::{
    NodeType, RessMessageKind, RessProtocolMessage, RessProtocolProvider, StateWitnessNet,
};
use alloy_primitives::{bytes::BytesMut, BlockHash, Bytes, B256};
use futures::{Stream, StreamExt};
use reth_eth_wire::multiplex::ProtocolConnection;
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::oneshot;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::*;

/// Ress protocol command.
#[derive(Debug)]
pub enum RessProtocolCommand {
    /// Sends a node type message to the peer
    NodeType {
        /// Node type.
        node_type: NodeType,
        /// The response will be sent to this channel.
        response: oneshot::Sender<bool>,
    },
    /// Get witness for specific block
    Witness {
        /// target block hash that we want to get witness from
        block_hash: BlockHash,
        /// The response will be sent to this channel.
        response: oneshot::Sender<StateWitnessNet>,
    },
    /// Get bytecode for specific code hash
    Bytecode {
        /// Target code hash that we want to get bytecode for.
        code_hash: B256,
        /// The response will be sent to this channel.
        response: oneshot::Sender<Bytes>,
    },
}

/// The connection handler for the custom RLPx protocol.
#[derive(Debug)]
pub struct RessProtocolConnection<P> {
    /// Provider.
    provider: P,
    /// Node type.
    node_type: NodeType,

    conn: ProtocolConnection,
    commands: UnboundedReceiverStream<RessProtocolCommand>,

    // below two type decides connection type
    peer_node_type: Option<NodeType>,

    pending_is_valid_connection: Option<oneshot::Sender<bool>>,
    pending_witness: Option<oneshot::Sender<StateWitnessNet>>,
    pending_bytecode: Option<oneshot::Sender<Bytes>>,
}

impl<P> RessProtocolConnection<P> {
    /// Create new connection.
    pub fn new(
        provider: P,
        node_type: NodeType,
        conn: ProtocolConnection,
        commands: UnboundedReceiverStream<RessProtocolCommand>,
    ) -> Self {
        Self {
            provider,
            conn,
            commands,
            node_type,
            peer_node_type: None,
            pending_is_valid_connection: None,
            pending_bytecode: None,
            pending_witness: None,
        }
    }

    fn on_command(&mut self, command: RessProtocolCommand) -> RessProtocolMessage {
        match command {
            RessProtocolCommand::NodeType {
                node_type,
                response,
            } => {
                self.peer_node_type = Some(node_type);
                self.pending_is_valid_connection = Some(response);
                RessProtocolMessage::node_type(node_type)
            }
            RessProtocolCommand::Witness {
                block_hash,
                response,
            } => {
                self.pending_witness = Some(response);
                RessProtocolMessage::get_witness(block_hash)
            }
            RessProtocolCommand::Bytecode {
                code_hash,
                response,
            } => {
                self.pending_bytecode = Some(response);
                RessProtocolMessage::get_bytecode(code_hash)
            }
        }
    }
}

impl<P: RessProtocolProvider> RessProtocolConnection<P> {
    fn on_bytecode_request(&self, code_hash: B256) -> Bytes {
        match self.provider.bytecode(code_hash) {
            Ok(Some(bytecode)) => bytecode,
            Ok(None) => {
                trace!(target: "ress::net::connection", %code_hash, "bytecode not found");
                Bytes::default()
            }
            Err(error) => {
                trace!(target: "ress::net::connection", %code_hash, %error, "error retrieving bytecode");
                Bytes::default()
            }
        }
    }

    fn on_witness_request(&self, block_hash: B256) -> StateWitnessNet {
        match self.provider.witness(block_hash) {
            Ok(Some(witness)) => StateWitnessNet::from_iter(witness),
            Ok(None) => {
                trace!(target: "ress::net::connection", %block_hash, "witness not found");
                StateWitnessNet::default()
            }
            Err(error) => {
                trace!(target: "ress::net::connection", %block_hash, %error, "error retrieving witness");
                StateWitnessNet::default()
            }
        }
    }
}

impl<P> Stream for RessProtocolConnection<P>
where
    P: RessProtocolProvider + Unpin,
{
    type Item = BytesMut;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            if let Poll::Ready(Some(cmd)) = this.commands.poll_next_unpin(cx) {
                let message = this.on_command(cmd);
                return Poll::Ready(Some(message.encoded()));
            }

            if let Poll::Ready(Some(next)) = this.conn.poll_next_unpin(cx) {
                // TODO: handle error
                let msg = RessProtocolMessage::decode_message(&mut &next[..]).unwrap();

                match msg.message {
                    RessMessageKind::Disconnect => {
                        // TODO: this actually doesn't disconnecting the channel. How can i gracefully stop
                        return Poll::Ready(None);
                    }
                    RessMessageKind::NodeType(node_type) => {
                        if !this.node_type.is_valid_connection(&node_type) {
                            return Poll::Ready(Some(RessProtocolMessage::disconnect().encoded()));
                        }

                        if let Some(sender) = this.pending_is_valid_connection.take() {
                            sender.send(true).ok();
                        }
                    }
                    RessMessageKind::Bytecode(bytes) => {
                        if let Some(sender) = this.pending_bytecode.take() {
                            sender.send(bytes).ok();
                        }
                    }
                    RessMessageKind::Witness(msg) => {
                        if let Some(sender) = this.pending_witness.take() {
                            sender.send(msg).ok();
                        }
                    }
                    RessMessageKind::GetBytecode(code_hash) => {
                        debug!(target: "ress::net::connection", %code_hash, "requesting bytecode");
                        let bytecode = this.on_bytecode_request(code_hash);
                        let response = RessProtocolMessage::bytecode(bytecode);
                        return Poll::Ready(Some(response.encoded()));
                    }
                    RessMessageKind::GetWitness(block_hash) => {
                        debug!(target: "ress::net::connection", %block_hash, "requesting witness");
                        let witness = this.on_witness_request(block_hash);
                        let response = RessProtocolMessage::witness(witness);
                        return Poll::Ready(Some(response.encoded()));
                    }
                };

                continue;
            }
        }
    }
}
