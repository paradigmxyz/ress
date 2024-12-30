use std::collections::HashMap;

use alloy_primitives::{keccak256, Address, BlockNumber, B256, U256};
use alloy_rlp::Decodable;
use alloy_trie::{nodes::TrieNode, Nibbles, TrieAccount};
use ress_subprotocol::protocol::proto::StateWitness;
use reth::revm::{
    primitives::{AccountInfo, Bytecode},
    Database,
};
use tracing::info;

use crate::bytecode_provider::{BytecodeProviderError, BytecodeProviderTrait};

/// ress will construct this `WitnessStateProvider` after getting `StateWitness` thru subprotocol communication.
pub struct WitnessStateProvider<B>
where
    B: BytecodeProviderTrait,
    B::Error: Into<WitnessStateProviderError>,
{
    pub state_witness: StateWitness,
    block_hashes: HashMap<BlockNumber, B256>,
    pub bytecode_provider: B,
}

impl<B> WitnessStateProvider<B>
where
    B: BytecodeProviderTrait,
    B::Error: Into<WitnessStateProviderError>,
{
    /// Create a new witness state provider.
    pub fn new(
        state_witness: StateWitness,
        block_hashes: HashMap<BlockNumber, B256>,
        bytecode_provider: B,
    ) -> Self {
        Self {
            state_witness,
            block_hashes,
            bytecode_provider,
        }
    }
}

/// Database error type.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WitnessStateProviderError {
    /// Block hash not found.
    #[error("block hash not found")]
    BlockHashNotFound,

    /// Error from bytecode provider.
    #[error(transparent)]
    BytecodeProviderError(#[from] BytecodeProviderError),

    /// Error when decoding RLP or trie nodes
    #[error("failed to decode data")]
    DecodingError,
}

impl<B> Database for WitnessStateProvider<B>
where
    B: BytecodeProviderTrait,
    B::Error: Into<WitnessStateProviderError>,
{
    #[doc = " The witness state provider error type."]
    type Error = WitnessStateProviderError;

    #[doc = " Get basic account information."]
    fn basic(&mut self, address: Address) -> Result<Option<AccountInfo>, Self::Error> {
        let hashed_address = &keccak256(address);
        let nibbles = Nibbles::unpack(hashed_address);

        for encoded in self.state_witness.values() {
            let node = TrieNode::decode(&mut &encoded[..]);
            match node {
                Ok(trie_node) => {
                    if let TrieNode::Leaf(leaf) = trie_node {
                        if nibbles.ends_with(&leaf.key) {
                            let account = TrieAccount::decode(&mut &leaf.value[..]);
                            match account {
                                Ok(account_node) => {
                                    info!(
                                        "storage root from witness:{}",
                                        account_node.storage_root
                                    );
                                    return Ok(Some(AccountInfo {
                                        balance: account_node.balance,
                                        nonce: account_node.nonce,
                                        code_hash: account_node.code_hash,
                                        code: None,
                                    }));
                                }
                                Err(_) => continue,
                            }
                        }

                        continue;
                    }
                }
                Err(_) => continue,
            }
        }

        Ok(None)
    }

    #[doc = " Get account code by its hash."]
    fn code_by_hash(&mut self, code_hash: B256) -> Result<Bytecode, Self::Error> {
        self.bytecode_provider
            .code_by_hash(code_hash)
            .map_err(Into::into)
    }

    #[doc = " Get storage value of address at index."]
    fn storage(&mut self, _address: Address, _index: U256) -> Result<U256, Self::Error> {
        // TODO/QUESTION: how i get this - from account's code?
        todo!()
    }

    #[doc = " Get block hash by block number."]
    fn block_hash(&mut self, number: u64) -> Result<B256, Self::Error> {
        Ok(*self
            .block_hashes
            .get(&number)
            .ok_or(WitnessStateProviderError::BlockHashNotFound)?)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use alloy_primitives::map::HashMap;
    use alloy_primitives::Bytes;
    use tokio::sync::mpsc::unbounded_channel;

    use crate::bytecode_provider::BytecodeProvider;

    use super::*;

    // *test case from reth `correctly_decodes_branch_node_values`*
    // address:0xfef955f3c66c14d005d5dd719dc3c838eb5232be
    // hashed_address:0x35f8e0fb36d119637a1f9b03ca5c35ce5640413aa9d321b5fd836dd5afd764bc
    // account:Account { nonce: 0, balance: 0, bytecode_hash: None }
    // witness:{0xc8ed2e88eb4f392010421e1279bc6daf555783bd0dcf8fcc64cf2b2da99f191a: 0xd580c22001c220018080808080808080808080808080, 0xce8c4b060e961e285a1c2d6af956fae96986f946102f23b71506524eea9e2450: 0xc22001, 0x5655f0253ad63e4f18d39fc2bfbf96f445184f547391df04bf1e40a47603aae6: 0xf86aa12035f8e0fb36d119637a1f9b03ca5c35ce5640413aa9d321b5fd836dd5afd764bcb846f8448080a0359525f4e6e459e5619b726371e527549a1bc34d3ebd535fb881691399224dffa0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470, 0x359525f4e6e459e5619b726371e527549a1bc34d3ebd535fb881691399224dff: 0xf7a01000000000000000000000000000000000000000000000000000000000000000d580c22001c220018080808080808080808080808080}
    // witness key:0x5655f0253ad63e4f18d39fc2bfbf96f445184f547391df04bf1e40a47603aae6

    #[test]
    fn test_basic() {
        let (tx, _) = unbounded_channel();

        let state_witness = HashMap::from_iter(vec![(
            B256::from_str("0xc8ed2e88eb4f392010421e1279bc6daf555783bd0dcf8fcc64cf2b2da99f191a")
                .unwrap(),
            Bytes::from_str("0xd580c22001c220018080808080808080808080808080").unwrap(),
        ),(
            B256::from_str("0xce8c4b060e961e285a1c2d6af956fae96986f946102f23b71506524eea9e2450")
                .unwrap(),
            Bytes::from_str("0xc22001").unwrap(),
        ),(
            B256::from_str("0x5655f0253ad63e4f18d39fc2bfbf96f445184f547391df04bf1e40a47603aae6")
                .unwrap(),
          Bytes::from_str("0xf86aa12035f8e0fb36d119637a1f9b03ca5c35ce5640413aa9d321b5fd836dd5afd764bcb846f8448080a0359525f4e6e459e5619b726371e527549a1bc34d3ebd535fb881691399224dffa0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470").unwrap(),
        ),(
            B256::from_str("0x359525f4e6e459e5619b726371e527549a1bc34d3ebd535fb881691399224dff")
                .unwrap(),
          Bytes::from_str("0xf7a01000000000000000000000000000000000000000000000000000000000000000d580c22001c220018080808080808080808080808080").unwrap(),
        )]);

        let mut witness_state_provider =
            WitnessStateProvider::new(state_witness, HashMap::new(), BytecodeProvider::new(tx));

        let basic_account = witness_state_provider
            .basic(Address::from_str("0xfef955f3c66c14d005d5dd719dc3c838eb5232be").unwrap())
            .unwrap();

        assert_eq!(basic_account.unwrap(), AccountInfo::default())
    }
}
