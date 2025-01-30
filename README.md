# ress(reth stateless)

## Hive test

Ress is testing with [hive](https://github.com/ethereum/hive). Simulator send request to `adapter`, which proxy `engine/*` request to `ress`, and other requests to `reth`. 

- `reth`: auth port(8651), rpc port(8544), subnetwork(30303)
- `ress`: auth port(8552), rpc port(-), subnetwork(61398)


## run hive

- reth with subprotocol
```console
RUST_LOG=info cargo run -r --bin reth node --authrpc.port 8651 --http.port 8544 -d --trusted-peers enode://4d4b6cd1361032ca9bd2aeb9d900aa4d45d9ead80ac9423374c451a7254d07662a3eada2d0fe208b6d257ceb0f064284662e857f57b66b54c198bd310ded36d0@127.0.0.1:61398
```

- ress
```console
RUST_LOG=info cargo run -r --bin ress -- --remote-peer "enode://cc25edb29102f8307282b9217971563801e3db4fb424e61ff39162d7659675426a4f5710e56de9857c2f666283bc04fc009a3cfccf20ef6827a660317ecb18e0@127.0.0.1:30303" 2
```

## run dev env

- ress 1
```console
RUST_LOG=info cargo run --bin ress 1 --enable-debug-consensus --enable-rpc-adapter
```

- ress 2
```console
RUST_LOG=info cargo run --bin ress 2 --enable-debug-consensus --enable-rpc-adapter
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
