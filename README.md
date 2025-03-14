# ress(reth stateless)

Ress (shorthand for: Reth Stateless) is a fully validating stateless Ethereum Execution Layer with 14GB disk requirements. Stateless nodes matter not only for improving Ethereum’s decentralization, but also to scale the L1 gas limit, scaling optimistic L2s, and for implementing Native Rollups to improve the L2 ecosystem’s security & interoperability.

> [!CAUTION]
> Ress is an experimental software that has not been fully tested in production grade settings.
> At this moment, we do not recommend using ress nodes in validator setups.

## Run

To run ress you need to either start a reth stateful node with `--ress.enable` to enable support for `ress` subprotocol or connect to a node which already has support enabled. You will need to provide the trusted peer manually:

```bash
ress --trusted-peers <peers>
```

To fetch the enode from a reth node you can do 
```bash
cast rpc admin_nodeInfo -r http://localhost:9545 | jq .enode
```

## How it works

Live sync works like any other stateful node: Ress receives a new payload from the consensus client and fetches necessary state data (witness, block, bytecodes) from a stateful reth client via [RLPx subprotocol dedicated to Ress](https://github.com/paradigmxyz/reth/tree/main/crates/ress/protocol). It verfies payload and calculates the new state root all in memory.

```mermaid
sequenceDiagram
    CL->>Ress: NewPayload
    Ress->>Reth: GetWitness
    Reth-->>Ress: Witness
    Ress->>Reth: GetBytecode
    Note over Ress,Reth: (only missing)
    Reth-->>Ress: Bytecode
    Ress->>Ress: Validate Payload
    Ress-->>CL: PayloadStatus
```

### Components

To run a ress node successfully you need the following components:
1. Ress node (stateless) connected to a Consensus Client
2. Reth node (stateful) connected to a Consensus Client

```mermaid
flowchart TB
    subgraph Stateful["Stateful Client Pair"]
        SFCL["Consensus Layer"]
        Reth["Reth Node"]
        SFCL -->|Engine API| Reth
    end
    
    subgraph Stateless["Stateless Client Pair"]
        SLCL["Consensus Layer"]
        Ress["Ress Node"]
        SLCL -->|Engine API| Ress
    end

    Ress -->|ress RLPx subprotocol| Reth
```

## How it was tested

### Hive Tests 

Ress was tested with [hive](https://github.com/ethereum/hive). A simulator sends requests to `adapter`, which proxies `engine/*` request to `ress`, and other requests to `reth`. 

- `reth`: auth port(8651), rpc port(8544), subnetwork(30303)
- `ress`: auth port(8552), rpc port(-), subnetwork(61398)

We ran the cancun test suite and from the 226 tests, ress passes `206` hive tests successfully:
```
Mar 13 09:46:28.033 INF simulation ethereum/engine finished suites=1 tests=226 failed=20
```

### Holesky validators
For our proof of concept, we successfully [ran Ress-backed Ethereum validators](https://light-holesky.beaconcha.in/validator/1919380?v=attestations) on Holesky testnet and correctly attested to block validity.
