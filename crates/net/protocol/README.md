# Ress Protocol (RESS)

The `ress` protocol runs on top of [RLPx], allowing a stateless full node to fetch necessary state data (witness, block, bytecode) from a stateful full node. The protocol is an optional extension for peers that support (or are interested in) stateless Ethereum full nodes.

The current version is `ress/0`.

## Overview

The `ress` protocol is designed to provide support for stateless full nodes. Its goal is to enable the exchange of necessary block execution state from a stateful node to a stateless node so that the latter can store in disk only the minimal required state (such as bytecode) and lazily fetch other state data. It supports retrieving a state witness for a target new payload from a stateful peer, as well as contract bytecode and full block data (headers and bodies). The `ress` protocol is intended to be run alongside other protocols (e.g., `eth`), rather than as a standalone protocol.

## Basic operation

Once a connection is established, a [NodeType] message must be sent. After the peer's node type is validated according to the connection rules, any other protocol messages may be sent. The `ress` session will be terminated if the peer combination is invalid (e.g., stateful-to-stateful connections are not needed).

Within a session, four types of messages can be exchanged: header, body, bytecode, and witness.

During the startup phase, a stateless node downloads the necessary ancestor blocks (header and body) via the header and body messages. When the stateless node receives a new payload through the engine API, it requests a witness—a compressed multi Merkle proof of state—using the witness message. From this witness, the stateless node can determine if any contract bytecode is missing by comparing it with its disk. It then requests any missing bytecode. All requests are sent to the connected stateful peer and occur synchronously.

## Protocol Messages

In most messages, the first element of the message data list is the request-id. For requests, this is a 64-bit integer value chosen by the requesting peer. The responding peer must mirror the value in the request-id element of the response message.

### NodeType (0x00)

`[nodetype]`

Informs a peer of its node type. This message should be sent immediately after the connection is established and before any other RESS protocol messages.

There are two types of nodes in the network:

| ID  | Node Type |
| --- | --------- |
| 0   | Stateless |
| 1   | Stateful  |


The following table shows which connections between node types are valid:

|           | stateless | stateful |
| --------- | --------- | -------- |
| stateless | true      | true     |
| stateful  | true      | false    |


### GetHeader (0x01)

`[request-id: P, [blockhash: B_32]]`

Require peer to return a Header message containing the header of the given block hash. 

### Header (0x02)

`[request-id: P, [header]]`

This is the response to GetHeader, providing the requested block header. Response corresponds to a block hash of the GetHeader request.

### GetBlockBody (0x03)

`[request-id: P, [blockhash: B_32]]`

Require peer to return a block body message containing the body of the given block hash. 

### BlockBody (0x04)

`[request-id: P, [blockbody]]`

This is the response to GetBlockBody, providing the requested block body. Response corresponds to a block hash of the GetBlockBody request.

### GetBytecode (0x05)

`[request-id: P, [codehash: B_32]]`

Require peer to return a bytecode message containing the bytecode of the given code hash. 

### Bytecode (0x06)

`[request-id: P, [bytes]]`

This is the response to GetBytecode, providing the requested bytecode. Response corresponds to a code hash of the GetBytecode request.

### GetWitness (0x07)

`[request-id: P, [blockhash: B_32]]`

Require peer to return a state witness message containing the witness of the given block hash. 

### Witness (0x08)

`[request-id: P, [statewitness]]`

This is the response to GetWitness, providing the requested state witness. Response corresponds to a block hash of the GetWitness request.

[NodeType]: #NodeType-0x00
[RLPx]: https://github.com/ethereum/devp2p/blob/master/rlpx.md