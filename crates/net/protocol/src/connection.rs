use crate::{
    NodeType, RessMessageKind, RessProtocolMessage, RessProtocolProvider, StateWitnessNet,
};
use alloy_primitives::{bytes::BytesMut, BlockHash, Bytes, B256};
use futures::{Stream, StreamExt};
use reth_eth_wire::multiplex::ProtocolConnection;
use std::{
    collections::HashMap,
    pin::Pin,
    task::{ready, Context, Poll},
};
use tokio::sync::oneshot;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::*;

/// Ress peer request.
#[derive(Debug)]
pub enum RessPeerRequest {
    /// Get bytecode for specific code hash
    GetBytecode {
        /// Target code hash that we want to get bytecode for.
        code_hash: B256,
        /// The sender for the response.
        tx: oneshot::Sender<Bytes>,
    },
    /// Get witness for specific block
    GetWitness {
        /// target block hash that we want to get witness from
        block_hash: BlockHash,
        /// The sender for the response.
        tx: oneshot::Sender<StateWitnessNet>,
    },
}

/// The connection handler for the custom RLPx protocol.
#[derive(Debug)]
pub struct RessProtocolConnection<P> {
    /// Provider.
    provider: P,
    /// Node type.
    node_type: NodeType,
    /// Protocol connection.
    conn: ProtocolConnection,
    /// Stream of incoming commands.
    commands: UnboundedReceiverStream<RessPeerRequest>,
    /// Incremental counter for request ids.
    next_id: u64,
    /// Collection of inflight requests.
    inflight_requests: HashMap<u64, RessPeerRequest>,
}

impl<P> RessProtocolConnection<P> {
    /// Create new connection.
    pub fn new(
        provider: P,
        node_type: NodeType,
        conn: ProtocolConnection,
        commands: UnboundedReceiverStream<RessPeerRequest>,
    ) -> Self {
        Self {
            provider,
            conn,
            commands,
            node_type,
            next_id: 0,
            inflight_requests: HashMap::default(),
        }
    }

    /// Returns the next request id
    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn on_command(&mut self, command: RessPeerRequest) -> RessProtocolMessage {
        let next_id = self.next_id();
        let message = match &command {
            RessPeerRequest::GetWitness { block_hash, .. } => {
                RessProtocolMessage::get_witness(next_id, *block_hash)
            }
            RessPeerRequest::GetBytecode { code_hash, .. } => {
                RessProtocolMessage::get_bytecode(next_id, *code_hash)
            }
        };
        self.inflight_requests.insert(next_id, command);
        message
    }
}

impl<P: RessProtocolProvider> RessProtocolConnection<P> {
    fn on_bytecode_request(&self, code_hash: B256) -> Bytes {
        match self.provider.bytecode(code_hash) {
            Ok(Some(bytecode)) => bytecode,
            Ok(None) => {
                trace!(target: "ress::network::connection", %code_hash, "bytecode not found");
                Bytes::default()
            }
            Err(error) => {
                trace!(target: "ress::network::connection", %code_hash, %error, "error retrieving bytecode");
                Bytes::default()
            }
        }
    }

    fn on_witness_request(&self, block_hash: B256) -> StateWitnessNet {
        match self.provider.witness(block_hash) {
            Ok(Some(witness)) => StateWitnessNet::from_iter(witness),
            Ok(None) => {
                trace!(target: "ress::network::connection", %block_hash, "witness not found");
                StateWitnessNet::default()
            }
            Err(error) => {
                trace!(target: "ress::network::connection", %block_hash, %error, "error retrieving witness");
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

            let Some(msg) = ready!(this.conn.poll_next_unpin(cx)) else {
                return Poll::Ready(None);
            };

            match RessProtocolMessage::decode_message(&mut &msg[..]) {
                Ok(msg) => {
                    match msg.message {
                        RessMessageKind::NodeType(node_type) => {
                            if !this.node_type.is_valid_connection(&node_type) {
                                // Terminating the stream disconnects the peer.
                                return Poll::Ready(None);
                            }
                        }
                        RessMessageKind::Bytecode(res) => {
                            if let Some(RessPeerRequest::GetBytecode { tx, .. }) =
                                this.inflight_requests.remove(&res.request_id)
                            {
                                // TODO: validate the bytecode.
                                let _ = tx.send(res.message);
                            } else {
                                // TODO: report bad message
                                continue;
                            }
                        }
                        RessMessageKind::Witness(res) => {
                            if let Some(RessPeerRequest::GetWitness { tx, .. }) =
                                this.inflight_requests.remove(&res.request_id)
                            {
                                // TODO: validate the witness.
                                let _ = tx.send(res.message);
                            } else {
                                // TODO: report bad message
                                continue;
                            }
                        }
                        RessMessageKind::GetBytecode(req) => {
                            let code_hash = req.message;
                            debug!(target: "ress::network::connection", %code_hash, "serving bytecode");
                            let bytecode = this.on_bytecode_request(code_hash);
                            let response = RessProtocolMessage::bytecode(req.request_id, bytecode);
                            return Poll::Ready(Some(response.encoded()));
                        }
                        RessMessageKind::GetWitness(req) => {
                            let block_hash = req.message;
                            debug!(target: "ress::network::connection", %block_hash, "serving witness");
                            let witness = this.on_witness_request(block_hash);
                            let response = RessProtocolMessage::witness(req.request_id, witness);
                            return Poll::Ready(Some(response.encoded()));
                        }
                    };
                }
                Err(e) => {
                    error!("{}", e);
                    continue;
                }
            };

            continue;
        }
    }
}
