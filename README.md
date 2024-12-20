# ress(reth stateless)

## component

- [stateful](./crates/stateful/)
- [stateless](./crates/stateless/)
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


