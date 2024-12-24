# stateless node

This is stateless ethereum execution node implementation that doesn't store full state, instead using rlpx network to communicate with other stateful node and stateless node to get necessary data.

## example

this will enable running stateless <> stateless node

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


## idea

- spawn a `AuthRpcModule` https://github.com/paradigmxyz/reth/blob/main/crates/rpc/rpc-builder/src/auth.rs#L197
- reimplement handler that handles receiver of the channel from the message event from the consensus layer
  - receiver handler will match message type and get the new payload and call network subprotocol etc