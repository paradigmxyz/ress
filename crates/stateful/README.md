# stateful node

This is a regular `reth` node that contains the full state. The main difference from a standard node is the addition of a custom `CustomRlpxProtoHandler` that enables communication with stateless nodes via a subprotocol network.

