//! Execution witness type.

use alloy_primitives::{
    map::{B256HashMap, B256HashSet},
    Bytes,
};
use alloy_rlp::Decodable;
use alloy_trie::{nodes::TrieNode, TrieAccount, KECCAK_EMPTY};

/// Alias type representing execution state witness.
/// Execution state witness is a mapping of hashes of encoded
/// trie nodes to their preimage:
/// `keccak(rlp(node)): rlp(node)`
pub type StateWitness = B256HashMap<Bytes>;

/// Execution witness contains all data necessary to execute the block (except for bytecodes).
/// That includes:
///     - state witness - collection of all touched trie nodes which is used for state retrieval and
///       state root computation.
#[derive(PartialEq, Eq, Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExecutionWitness {
    /// The state witness with touched trie nodes.
    pub state_witness: StateWitness,
}

impl ExecutionWitness {
    /// Create new [`ExecutionWitness`].
    pub fn new(state_witness: StateWitness) -> Self {
        Self { state_witness }
    }

    /// Returns all code hashes found in the witness.
    pub fn get_bytecode_hashes(&self) -> B256HashSet {
        let mut bytecode_hashes = B256HashSet::default();
        for encoded in self.state_witness.values() {
            if let Ok(TrieNode::Leaf(leaf)) = TrieNode::decode(&mut &encoded[..]) {
                if let Ok(account) = TrieAccount::decode(&mut &leaf.value[..]) {
                    if account.code_hash != KECCAK_EMPTY {
                        bytecode_hashes.insert(account.code_hash);
                    }
                }
            }
        }
        bytecode_hashes
    }
}
