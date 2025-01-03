use alloy_primitives::B256;
use reth::primitives::Bytecode;

use crate::errors::StorageError;

pub struct DiskStorage {
    // Some DB handle, e.g. Arc<rocksdb::DB>
}

impl DiskStorage {
    pub fn new(_path: &str) -> Self {
        Self {
            // ...
        }
    }

    /// get bytecode from disk -> fall back network
    pub fn get_account_code(&self, _code_hash: B256) -> Result<Option<Bytecode>, StorageError> {
        todo!()
    }
}
