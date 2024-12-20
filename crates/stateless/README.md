# stateless 

## demo

### peer 1
This will spawn network that support rlpx subprotocol and wait for 5 seconds for peer 2 to be spawn and connect with peer 2
```console
RUST_LOG=info cargo run -p stateless 1
```

### peer 2
This will spawn network that support rlpx subprotocol and connect with peer 1
```console
RUST_LOG=info cargo run -p stateless 2
```


## components
- rpc: engine API 
- network: spin up rplx network 
- evm: use evm crate
- storage: bytecode storage
- consensus(`EthBeaconConsensus`): 
- network(`NetworkManager`): handle network that add 