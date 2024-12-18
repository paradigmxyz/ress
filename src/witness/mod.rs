use alloy_primitives::{map::HashMap, BlockNumber, Bytes, B256};
use reth::providers::{AccountReader, StateProvider};

/// hashmap representation of multiple mpt state proof
/// `keccak(rlp(node)) -> rlp(nod)`
type Witness = HashMap<B256, Bytes>;

struct WitnessStateProvider<B: Send + Sync> {
    state_witness: Witness,
    block_hashes: HashMap<BlockNumber, B256>,
    bytecode_provider: B,
}
