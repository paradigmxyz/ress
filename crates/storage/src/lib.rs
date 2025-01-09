use std::sync::{Arc, Mutex};

use alloy_primitives::{keccak256, Address, BlockNumber, B256, U256};
use alloy_rlp::Decodable;
use backends::{disk::DiskStorage, memory::MemoryStorage, network::NetworkStorage};
use errors::StorageError;
use ress_subprotocol::{connection::CustomCommand, protocol::proto::StateWitness};
use reth_chainspec::ChainSpec;
use reth_primitives::{Header, SealedHeader};
use reth_revm::primitives::{AccountInfo, Bytecode};
use reth_trie::{Nibbles, TrieAccount, TrieNode};
use tokio::sync::mpsc::UnboundedSender;
use tracing::info;

pub mod backends;
pub mod errors;

/// orchestrate 3 different type of backends (in-memory, disk, network)
#[derive(Debug, Clone)]
pub struct Storage {
    pub chain_spec: Arc<ChainSpec>,
    pub memory: Arc<MemoryStorage>,
    pub disk: Arc<Mutex<DiskStorage>>,
    pub network: Arc<NetworkStorage>,
}

impl Storage {
    pub fn new(
        network_peer_conn: UnboundedSender<CustomCommand>,
        chain_spec: Arc<ChainSpec>,
    ) -> Self {
        let memory = Arc::new(MemoryStorage::new());
        let disk = Arc::new(Mutex::new(DiskStorage::new("test.db")));
        let network = Arc::new(NetworkStorage::new(network_peer_conn));
        Self {
            chain_spec,
            memory,
            disk,
            network,
        }
    }

    /// set block hash and set block header
    pub fn set_block(&self, block_hash: B256, header: Header) {
        self.memory.set_block_hash(block_hash, header.number);
        self.memory.set_block_header(block_hash, header);
    }

    /// get account info by block hash from witness
    pub fn get_account_info_by_hash(
        &self,
        block_hash: B256,
        address: Address,
    ) -> Result<Option<AccountInfo>, StorageError> {
        info!(
            target = "storage",
            "request witness for block hash: {:?}, address: {:?}", block_hash, address
        );
        // 1. first check if info in memory
        if let Some(account_info) = self.memory.get_account_info_by_hash(block_hash, address)? {
            Ok(Some(account_info))
        } else {
            let witness = self.network.get_witness(block_hash).unwrap();
            // 2. get account info from witness
            // todo: use sparse merkle tree
            let acc_info = self.get_account_info_from_witness(witness, address);
            info!(target = "storage", "decoded account info: {:?}", acc_info);
            acc_info
        }
    }

    /// get bytecode from disk -> fallback network
    pub fn get_account_code(&self, code_hash: B256) -> Result<Option<Bytecode>, StorageError> {
        let disk = self.disk.lock().unwrap();
        if let Some(bytecode) = disk.get_account_code(code_hash)? {
            return Ok(Some(bytecode));
        }

        if let Some(bytecode) = self.network.get_account_code(code_hash)? {
            disk.update_account_code(code_hash, bytecode.clone())?;
            return Ok(Some(bytecode));
        }
        Ok(None)
    }

    pub fn get_storage_at_hash(
        &self,
        _block_hash: B256,
        _address: Address,
        _storage_key: U256,
    ) -> Result<Option<U256>, StorageError> {
        todo!()
    }

    pub fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Header>, StorageError> {
        self.memory.get_block_header(block_number)
    }

    pub fn get_chain_config(&self) -> Arc<ChainSpec> {
        self.chain_spec.clone()
    }

    pub fn get_block_header_by_hash(
        &self,
        block_hash: B256,
    ) -> Result<Option<SealedHeader>, StorageError> {
        self.memory.get_block_header_by_hash(block_hash)
    }

    // todo: will be later alter somehow with logic like `SparseStateTrie::from_witness()` and look up account from trie
    pub fn get_account_info_from_witness(
        &self,
        witness: StateWitness,
        address: Address,
    ) -> Result<Option<AccountInfo>, StorageError> {
        let hashed_address = &keccak256(address);
        let nibbles = Nibbles::unpack(hashed_address);

        for encoded in witness.values() {
            let node = TrieNode::decode(&mut &encoded[..]);
            match node {
                Ok(trie_node) => {
                    if let TrieNode::Leaf(leaf) = trie_node {
                        if nibbles.ends_with(&leaf.key) {
                            let account = TrieAccount::decode(&mut &leaf.value[..]);
                            match account {
                                Ok(account_node) => {
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
}

// todo witness test fixture
// #[cfg(test)]
// mod tests {
//     use std::str::FromStr;

//     use alloy_primitives::map::HashMap;
//     use alloy_primitives::Bytes;
//     use tokio::sync::mpsc::unbounded_channel;

//     use crate::engine::provider::bytecode::BytecodeProvider;

//     use super::*;

//     // *test case from reth `correctly_decodes_branch_node_values`*
//     // address:0xfef955f3c66c14d005d5dd719dc3c838eb5232be
//     // hashed_address:0x35f8e0fb36d119637a1f9b03ca5c35ce5640413aa9d321b5fd836dd5afd764bc
//     // account:Account { nonce: 0, balance: 0, bytecode_hash: None }
//     // witness:{0xc8ed2e88eb4f392010421e1279bc6daf555783bd0dcf8fcc64cf2b2da99f191a: 0xd580c22001c220018080808080808080808080808080, 0xce8c4b060e961e285a1c2d6af956fae96986f946102f23b71506524eea9e2450: 0xc22001, 0x5655f0253ad63e4f18d39fc2bfbf96f445184f547391df04bf1e40a47603aae6: 0xf86aa12035f8e0fb36d119637a1f9b03ca5c35ce5640413aa9d321b5fd836dd5afd764bcb846f8448080a0359525f4e6e459e5619b726371e527549a1bc34d3ebd535fb881691399224dffa0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470, 0x359525f4e6e459e5619b726371e527549a1bc34d3ebd535fb881691399224dff: 0xf7a01000000000000000000000000000000000000000000000000000000000000000d580c22001c220018080808080808080808080808080}
//     // witness key:0x5655f0253ad63e4f18d39fc2bfbf96f445184f547391df04bf1e40a47603aae6

//     #[test]
//     fn test_basic() {
//         let (tx, _) = unbounded_channel();

//         let state_witness = HashMap::from_iter(vec![(
//             B256::from_str("0xc8ed2e88eb4f392010421e1279bc6daf555783bd0dcf8fcc64cf2b2da99f191a")
//                 .unwrap(),
//             Bytes::from_str("0xd580c22001c220018080808080808080808080808080").unwrap(),
//         ),(
//             B256::from_str("0xce8c4b060e961e285a1c2d6af956fae96986f946102f23b71506524eea9e2450")
//                 .unwrap(),
//             Bytes::from_str("0xc22001").unwrap(),
//         ),(
//             B256::from_str("0x5655f0253ad63e4f18d39fc2bfbf96f445184f547391df04bf1e40a47603aae6")
//                 .unwrap(),
//           Bytes::from_str("0xf86aa12035f8e0fb36d119637a1f9b03ca5c35ce5640413aa9d321b5fd836dd5afd764bcb846f8448080a0359525f4e6e459e5619b726371e527549a1bc34d3ebd535fb881691399224dffa0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470").unwrap(),
//         ),(
//             B256::from_str("0x359525f4e6e459e5619b726371e527549a1bc34d3ebd535fb881691399224dff")
//                 .unwrap(),
//           Bytes::from_str("0xf7a01000000000000000000000000000000000000000000000000000000000000000d580c22001c220018080808080808080808080808080").unwrap(),
//         )]);

//         let mut witness_state_provider =
//             WitnessStateProvider::new(state_witness, HashMap::new(), BytecodeProvider::new(tx));

//         let basic_account = witness_state_provider
//             .basic(Address::from_str("0xfef955f3c66c14d005d5dd719dc3c838eb5232be").unwrap())
//             .unwrap();

//         assert_eq!(basic_account.unwrap(), AccountInfo::default())
//     }
// }
