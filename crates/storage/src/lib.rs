use std::sync::Arc;

use alloy_primitives::{Address, BlockNumber, B256, U256};
use backends::{disk::DiskStorage, memory::MemoryStorage, network::NetworkStorage};
use errors::StorageError;
use ress_subprotocol::connection::CustomCommand;
use reth::{
    chainspec::ChainSpec,
    primitives::{Header, SealedHeader},
    revm::primitives::{AccountInfo, Bytecode},
};
use reth_trie_sparse::SparseStateTrie;
use tokio::sync::mpsc::UnboundedSender;

pub mod backends;
pub mod errors;

/// orchestract 3 different type of backends (in memory, disk, network)
pub struct Storage {
    memory: Arc<MemoryStorage>,
    disk: Arc<DiskStorage>,
    network: Arc<NetworkStorage>,
}

impl Storage {
    pub fn new(network_peer_conn: UnboundedSender<CustomCommand>) -> Self {
        let memory = Arc::new(MemoryStorage::new());
        let disk = Arc::new(DiskStorage::new("test.db"));
        let network = Arc::new(NetworkStorage::new(network_peer_conn));
        Self {
            memory,
            disk,
            network,
        }
    }

    pub fn get_account_info_by_hash(
        &self,
        block_hash: B256,
        address: Address,
    ) -> Result<Option<AccountInfo>, StorageError> {
        todo!()
    }

    /// get bytecode from disk -> fallback network
    pub fn get_account_code(&self, code_hash: B256) -> Result<Option<Bytecode>, StorageError> {
        if let Some(bytecode) = self.disk.get_account_code(code_hash)? {
            return Ok(Some(bytecode));
        }
        if let Some(bytecode) = self.network.get_account_code(code_hash)? {
            self.disk.update_account_code(code_hash, bytecode.clone())?;
            return Ok(Some(bytecode));
        }
        Ok(None)
    }

    // get storge value from a
    pub fn get_storage_at_hash(
        &self,
        block_hash: B256,
        address: Address,
        storage_key: B256,
    ) -> Result<Option<U256>, StorageError> {
        // let Some(storage_trie) = self.storage_trie(block_hash, address)? else {
        //     return Ok(None);
        // };
        // let hashed_key = Keccak256::new(&storage_key).finalize().to_vec();
        // storage_trie
        //     .get(&hashed_key)?
        //     .map(|rlp| U256::decode(&rlp).map_err(StorageError::RLPDecode))
        //     .transpose();
        todo!()
    }

    pub fn get_block_header(
        &self,
        block_number: BlockNumber,
    ) -> Result<Option<Header>, StorageError> {
        todo!()
    }

    pub fn get_chain_config(&self) -> Result<ChainSpec, StorageError> {
        todo!()
    }

    pub fn storage_trie(
        &self,
        block_hash: B256,
        address: Address,
    ) -> Result<Option<SparseStateTrie>, StorageError> {
        // Fetch Account from state_trie
        // let Some(state_trie) = self.state_trie(block_hash)? else {
        //     return Ok(None);
        // };
        // let hashed_address = hash_address(&address);
        // let Some(encoded_account) = state_trie.get(&hashed_address)? else {
        //     return Ok(None);
        // };
        // let account = AccountState::decode(&encoded_account)?;
        // // Open storage_trie
        // let storage_root = account.storage_root;
        // Ok(Some(self.engine.open_storage_trie(
        //     H256::from_slice(&hashed_address),
        //     storage_root,
        // )))
        todo!()
    }

    pub fn state_trie(&self, block_hash: B256) -> Result<Option<SparseStateTrie>, StorageError> {
        // let Some(header) = self.get_block_header_by_hash(block_hash)? else {
        //     return Ok(None);
        // };
        // Ok(Some(self.engine.open_state_trie(header.state_root)))
        todo!()
    }

    pub fn get_block_header_by_hash(
        &self,
        block_hash: B256,
    ) -> Result<Option<SealedHeader>, StorageError> {
        // todo: get header from memeory
        // self.engine.get_block_header_by_hash(block_hash)
        return Ok(Some(SealedHeader::default()));
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
