# hive-adapter

Acts as a proxy/mitm between hive and reth/ress:
* if it receives a non-engine request: forwards it to reth, gets its response and sends it back to hive.
* if it receives an engine request: forwards it to reth, get its response, THEN sends that same request to ress and gets its response. Only then does it send the reth response back to hive

### How to run hive

0. install docker
1. `git clone https://github.com/ethereum/hive && cd hive && go build .`
2. apply [hive.patch](./hive.patch) to hive repo.
3. build `reth`, `ress` and `adapter` and copy them into  `hive/clients/reth/`
4. from inside the hive repo run: `rm -rf workspace/ && ./hive --sim ethereum/engine --sim.limit api --client reth`

### Tips
* logs can be checked under the workspace folder created after a hive run. Example: `hive/workspace/logs/reth/client-fc9f9ef80a74664e877fd59008e59e24fba35c43d02bbc30e2530952cc07a907.log`
* tracing levels can be changed on the entrypoint: `hive/clients/reth/reth.sh`
