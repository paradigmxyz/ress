# stateless reth

q. general flow: 
setup stage
- 1) statefull node launch + add rlpx protocol bytescode & witness
- 2) sateless node launch + add rlpx protocol bytescode & witness
- 3) [Type handshake] connection: stateful <> statefull (revert) | statefull <> stateless | stateless <> stateless
  - q. actaully network level there is no way to get type (peerid/ip is totally irrelevant). so does it also need to be rlpx and later disconnect? 
  - a. ask matt if possible if on network not do on protocol message
  
sync stage
- 4) stateless gets block from consensus 
  - q. where? 
- 5) stateless send rlpx msg to statefull for get witness of current block hash
  - q. does it mean will use witness provider when sending message?
- 6) while `update` witness from witness provider, also use code bytescode provider
  - q. then where does rlpx of bytescode policy is needed? isn't enough to just have provider?


------
- purpose is to have full functional execution client without storing full state.
- iteration 1: goal is to sync the block
- evm* | network (rlpx)
- stateless reth doesn't have tx pool -> q. does it mean all yield tx to stateful node? (just like light client)
- senario of syncing block : get block(list of transaction - payload) from CL -> (pre execute) execute on evm (post execute) -> `HashedPostState` -> (withness provider) -> `StateWitness`
-> why we need bytecode? maybe to execute against contract?
-> historical block hash 256 : for BLOCKHASH opcode
- block validation : `SealedBlock` (from CL) + `StateWitness` (from witness provider) => true/false
- engine api : accept payload (block that passed validation from witness above) and send it to consensus
-----


## run

```
cargo run -- node
```

## network

> base on rlpx https://github.com/ethereum/devp2p/blob/master/rlpx.md

- connect(type)
  : validate is this valid connection
- get_bytecode(codehash)
  :
- bytecode response = (codehash, bytecode)
- get_witness (blockhash)
  1. block hash -> state -> hashed post state -> witness
  2. return hash + witness
- witness response = (blockhash, witness)
  : [TrieWitness](https://github.com/paradigmxyz/reth/blob/ef033abaf9105f167a778e00411e005ba9a0271e/crates/trie/trie/src/witness.rs#L87)
  - compute([HashedPostState](https://github.com/paradigmxyz/reth/blob/ef033abaf9105f167a778e00411e005ba9a0271e/crates/trie/trie/src/state.rs#L17-L24) that can be from state (account/storage))
  - multi mpt proofs of specific block

## evm

execute evm block given the witness

##

Create a witness state provider. State provider is comprised of the witness, block hashes for the past 256 blocks and bytecode provider. BytecodeProvider is a trait capable of retrieving bytecode from "somewhere".
