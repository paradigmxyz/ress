use crate::{NodeType, RessMessage, RessProtocolMessage, RessProtocolProvider, StateWitnessNet};
use alloy_primitives::{bytes::BytesMut, keccak256, BlockHash, Bytes, B256};
use futures::{Stream, StreamExt};
use reth_eth_wire::multiplex::ProtocolConnection;
use reth_primitives::{BlockBody, Header};
use std::{
    collections::HashMap,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::sync::oneshot;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tracing::*;

/// Ress peer request.
#[derive(Debug)]
pub enum RessPeerRequest {
    /// Get header for specific block hash.
    GetHeader {
        /// Target block hash that we want to get header for.
        block_hash: BlockHash,
        /// The sender for the response.
        tx: oneshot::Sender<Header>,
    },
    /// Get block body for specific block hash.
    GetBlockBody {
        /// Target block hash that we want to get block body for
        block_hash: BlockHash,
        /// The sender for the response.
        tx: oneshot::Sender<BlockBody>,
    },
    /// Get bytecode for specific code hash
    GetBytecode {
        /// Target code hash that we want to get bytecode for.
        code_hash: B256,
        /// The sender for the response.
        tx: oneshot::Sender<Bytes>,
    },
    /// Get witness for specific block.
    GetWitness {
        /// Target block hash that we want to get witness for.
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
            RessPeerRequest::GetHeader { block_hash, .. } => {
                RessProtocolMessage::get_header(next_id, *block_hash)
            }
            RessPeerRequest::GetBlockBody { block_hash, .. } => {
                RessProtocolMessage::get_block_body(next_id, *block_hash)
            }
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
    fn on_header_request(&self, block_hash: B256) -> Header {
        match self.provider.header(block_hash) {
            Ok(Some(header)) => header,
            Ok(None) => {
                trace!(target: "ress::net::connection", %block_hash, "header not found");
                Default::default()
            }
            Err(error) => {
                trace!(target: "ress::net::connection", %block_hash, %error, "error retrieving header");
                Default::default()
            }
        }
    }

    fn on_block_body_request(&self, block_hash: B256) -> BlockBody {
        match self.provider.block_body(block_hash) {
            Ok(Some(body)) => body,
            Ok(None) => {
                trace!(target: "ress::net::connection", %block_hash, "block body not found");
                Default::default()
            }
            Err(error) => {
                trace!(target: "ress::net::connection", %block_hash, %error, "error retrieving block body");
                Default::default()
            }
        }
    }

    fn on_bytecode_request(&self, code_hash: B256) -> Bytes {
        match self.provider.bytecode(code_hash) {
            Ok(Some(bytecode)) => bytecode,
            Ok(None) => {
                trace!(target: "ress::net::connection", %code_hash, "bytecode not found");
                Default::default()
            }
            Err(error) => {
                trace!(target: "ress::net::connection", %code_hash, %error, "error retrieving bytecode");
                Default::default()
            }
        }
    }

    fn on_witness_request(&self, block_hash: B256) -> StateWitnessNet {
        match self.provider.witness(block_hash) {
            Ok(Some(witness)) => {
                trace!(target: "ress::net::connection", %block_hash, "witness found");
                StateWitnessNet::from_iter(witness)
            }
            Ok(None) => {
                trace!(target: "ress::net::connection", %block_hash, "witness not found");
                Default::default()
            }
            Err(error) => {
                trace!(target: "ress::net::connection", %block_hash, %error, "error retrieving witness");
                Default::default()
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
                let encoded = message.encoded();
                trace!(target: "ress::net::connection", ?message, encoded = alloy_primitives::hex::encode(&encoded), "Sending peer command");
                return Poll::Ready(Some(encoded));
            }

            if let Poll::Ready(Some(next)) = this.conn.poll_next_unpin(cx) {
                let msg = match RessProtocolMessage::decode_message(&mut &next[..]) {
                    Ok(msg) => {
                        trace!(target: "ress::net::connection", message = ?msg.message_type, "Processing message");
                        msg
                    }
                    Err(error) => {
                        trace!(target: "ress::net::connection", %error, "Error decoding peer message");
                        // TODO: report bad message
                        continue;
                    }
                };

                match msg.message {
                    RessMessage::NodeType(node_type) => {
                        if !this.node_type.is_valid_connection(&node_type) {
                            // Terminating the stream disconnects the peer.
                            return Poll::Ready(None);
                        }
                    }
                    RessMessage::GetHeader(req) => {
                        let block_hash = req.message;
                        trace!(target: "ress::net::connection", %block_hash, "serving header");
                        let header = this.on_header_request(block_hash);
                        let response = RessProtocolMessage::header(req.request_id, header);
                        return Poll::Ready(Some(response.encoded()));
                    }
                    RessMessage::GetBlockBody(req) => {
                        let block_hash = req.message;
                        trace!(target: "ress::net::connection", %block_hash, "serving block body");
                        let block_body = this.on_block_body_request(block_hash);
                        let response = RessProtocolMessage::block_body(req.request_id, block_body);
                        return Poll::Ready(Some(response.encoded()));
                    }
                    RessMessage::GetBytecode(req) => {
                        let code_hash = req.message;
                        trace!(target: "ress::net::connection", %code_hash, "serving bytecode");
                        let bytecode = this.on_bytecode_request(code_hash);
                        let response = RessProtocolMessage::bytecode(req.request_id, bytecode);
                        return Poll::Ready(Some(response.encoded()));
                    }
                    RessMessage::GetWitness(req) => {
                        let block_hash = req.message;
                        trace!(target: "ress::net::connection", %block_hash, "serving witness");
                        let witness = this.on_witness_request(block_hash);
                        let response = RessProtocolMessage::witness(req.request_id, witness);
                        return Poll::Ready(Some(response.encoded()));
                    }
                    RessMessage::Header(res) => {
                        if let Some(RessPeerRequest::GetHeader { tx, .. }) =
                            this.inflight_requests.remove(&res.request_id)
                        {
                            if res.message == Header::default() {
                                warn!(target: "ress::net::connection", "header is default");
                            }
                            // TODO: validate the header.
                            let _ = tx.send(res.message);
                        } else {
                            // TODO: report bad message
                        }
                    }
                    RessMessage::BlockBody(res) => {
                        if let Some(RessPeerRequest::GetBlockBody { tx, .. }) =
                            this.inflight_requests.remove(&res.request_id)
                        {
                            let _ = tx.send(res.message);
                        } else {
                            // TODO: report bad message
                        }
                    }
                    RessMessage::Bytecode(res) => {
                        if let Some(RessPeerRequest::GetBytecode { tx, code_hash }) =
                            this.inflight_requests.remove(&res.request_id)
                        {
                            if res.message == Bytes::default() {
                                warn!(target: "ress::net::connection", "bytes is default");
                            } else if keccak256(res.message.clone()) != code_hash {
                                error!(target: "ress::net::connection", "invalid bytes");
                            }
                            // TODO: validate the bytecode.
                            let _ = tx.send(res.message);
                        } else {
                            // TODO: report bad message
                        }
                    }
                    RessMessage::Witness(res) => {
                        if let Some(RessPeerRequest::GetWitness { tx, .. }) =
                            this.inflight_requests.remove(&res.request_id)
                        {
                            if res.message == StateWitnessNet::default() {
                                warn!(target: "ress::net::connection", "witness is default");
                            }
                            // TODO: validate the witness.
                            let _ = tx.send(res.message);
                        } else {
                            // TODO: report bad message
                        }
                    }
                };

                continue;
            }

            return Poll::Pending;
        }
    }
}
