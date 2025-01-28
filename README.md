# ress(reth stateless)

## Poc scenario

RUST_LOG=info cargo run -r --bin ress -- --remote-peer "enode://060bb5ab4a20bbb2465a4db24de7a740db00207e34044454504bf004d6396bd9b03bf08b1df3f1f468366a2c0b809dee7aa54069af94fa11bdb26b9103ee76d6@127.0.0.1:30303"  --no-debug-consensus 2 --enable-rpc-adapter

*note example payload generated from [here](https://github.com/Rjected/execution-payload-builder/tree/main)*

### 1. launch ress node

Spawning 3 process.
- authserver: `EngineApi` implemented server to listen consensus message.
- p2p network: spawn network that implemented `ress-protocol`.
- engine: `ConsensusEngine` that spawned to keep receive message


<img src=".github/images/1.png" alt="" width="500" />

### 2. new payload

Authserver received the message and validate payload scaleton. Send to `ConsensusEngine` and handle logic of further validation - against the parent header, construct Executor and run evm and post validation with receipt.

Storage is abstracted in 3 different backend, disk, memory, network.


<img src=".github/images/2.png" alt="" width="500" />

### 3. new fcu

validate message and update the state of node.

<img src=".github/images/3.png" alt="" width="300" />



## run
ress <> ress && reth(stateful reth) <> ress && reth <> reth (this is reth impl)(x)

- test_uils (peer1)
```console
RUST_LOG=info cargo run --bin ress 1
```

- test_uils (peer2)
```console
RUST_LOG=info cargo run --bin ress 2
```


## component

- binary
  - [reth](./bin/reth): run original reth client that added custom subprotocol to communicate with ress
  - [ress](./bin/ress): run resss client - stateless execution client

- crates
  - [ress-common](./crates/common): ress common
  - [ress-network](./crates/network): provide functions to spawn authserver and network.
  - [ress-node](./crates/node): provide abstraction of launch the node and inner consensus engine implementation
  - [ress-storage](./crates/storage): provide abstraction of storage that handles 3 backends (disk, in memory, network) based on request.
  - [ress-vm](./crates/vm): provide executor that can execute evm from new block
  - [subprotocol](./crates/subprotocol/)
