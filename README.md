# ress(reth stateless)

## Poc seniaro

I join network (statless - ress)
- connect 1 ress 
- connect 1 reth (i wanna get witness / bytecode + small...etc)


## run: ress <> ress && reth(stateful reth) <> ress && reth <> reth (this is reth impl)(x)

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
  - [ress-core](./crates/ress): ress core 
  - [subprotocol](./crates/subprotocol/)


## general flow

setup stage
- 1) stateful node launch + add rlpx protocol bytescode & witness
- 2) sateless node launch + add rlpx protocol bytescode & witness
- 3) [Type handshake] rlpx connection: stateful <> statefull (revert) | statefull <> stateless | stateless <> stateless
- 4) stateless gets block(new payload) from consensus 
  - engine api
- 5) stateless send rlpx msg to stateful for get witness/bytecode of current new payload to validate 
  - consensus engine coordinates this 
- 6) execute on evm
- 7) response back to CL -> FCU req/res


