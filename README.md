# stateless reth 

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