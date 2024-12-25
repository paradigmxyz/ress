# stateless node

This is stateless ethereum execution node implementation that doesn't store full state, instead using rlpx network to communicate with other stateful node and stateless node to get necessary data.


## components
- rpc: engine API 
- network: spin up rplx network 
- evm: use evm crate
- storage: bytecode storage
- consensus(`EthBeaconConsensus`): 
- network(`NetworkManager`): handle network that add 