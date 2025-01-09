//! Execution witness type.

use alloy_primitives::{map::B256HashMap, BlockNumber, Bytes, B256};
use std::collections::HashMap;

/// Alias type representing execution state witness.
/// Execution state witness is a mapping of hashes of encoded
/// trie nodes to their preimage:
/// `keccak(rlp(node)): rlp(node)`
pub type StateWitness = B256HashMap<Bytes>;

/// Execution witness contains all data necessary to execute the block (except for bytecodes).
/// That includes:
///     - state witness - collection of all touched trie nodes which is used for state retrieval and state root computation.
///     - block hashes - mapping of block number to block hash necessary to execute the [`BLOCKHASH`](https://www.evm.codes/?fork=cancun#40) opcode.
#[derive(PartialEq, Eq, Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExecutionWitness {
    /// The state witness with touched trie nodes.
    pub state_witness: StateWitness,
    /// Mapping of block numbers to block hashes.
    pub block_hashes: HashMap<BlockNumber, B256>,
}

impl ExecutionWitness {
    /// Create new [`ExecutionWitness`].
    pub fn new(state_witness: StateWitness, block_hashes: HashMap<BlockNumber, B256>) -> Self {
        Self {
            state_witness,
            block_hashes,
        }
    }
}
