//! Types for the `debug` API.

use alloy_primitives::{map::B256HashMap, Bytes};

/// Represents the execution witness of a block. Contains an optional map of state preimages.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RpcExecutionWitness {
    /// Map of all hashed trie nodes to their preimages that were required during the execution of
    /// the block, including during state root recomputation.
    ///
    /// `keccak(rlp(node)) => rlp(node)`
    pub state: B256HashMap<Bytes>,
    /// Map of all contract codes (created / accessed) to their preimages that were required during
    /// the execution of the block, including during state root recomputation.
    ///
    /// `keccak(bytecodes) => bytecodes`
    pub codes: B256HashMap<Bytes>,
}

impl RpcExecutionWitness {
    // / Create new RPC execution witness from state and bytecodes.
    // pub fn new(state: B256HashMap<Bytes>, codes: B256HashMap<Bytes>) -> Self {
    //     Self { state, codes }
    // }
}
