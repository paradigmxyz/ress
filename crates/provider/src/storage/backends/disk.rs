use std::sync::Arc;

use alloy_primitives::{Bytes, B256};
use parking_lot::Mutex;
use reth_revm::primitives::Bytecode;
use rusqlite::{Connection, OptionalExtension, Result};

use crate::errors::DiskStorageError;

// todo: for now for simplicity using sqlite, mb later move kv storage like libmbdx
/// On disk storage.
#[derive(Clone, Debug)]
pub struct DiskStorage {
    conn: Arc<Mutex<Connection>>,
}

impl DiskStorage {
    /// Return new disk storage.
    pub fn new(path: &str) -> Self {
        let conn = Arc::new(Mutex::new(Connection::open(path).unwrap()));
        conn.lock()
            .execute(
                "CREATE TABLE IF NOT EXISTS account_code (
            id   INTEGER PRIMARY KEY,
            codehash STRING NOT NULL UNIQUE,
            bytecode BLOB
        )",
                (),
            )
            .unwrap();
        Self { conn }
    }

    // TODO: remove
    pub(crate) fn filter_code_hashes(&self, code_hashes: Vec<B256>) -> Vec<B256> {
        code_hashes
            .into_iter()
            .filter(|code_hash| !self.code_hash_exists_in_db(code_hash))
            .collect()
    }

    pub(crate) fn code_hash_exists_in_db(&self, code_hash: &B256) -> bool {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT COUNT(*) FROM account_code WHERE codehash = ?1")
            .unwrap();
        let count: i64 = stmt
            .query_row([code_hash.to_string()], |row| row.get(0))
            .unwrap_or(0);
        count > 0
    }

    /// get bytecode from disk
    pub(crate) fn get_bytecode(
        &self,
        code_hash: B256,
    ) -> Result<Option<Bytecode>, DiskStorageError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare("SELECT bytecode FROM account_code WHERE codehash = ?1")?;
        let bytecode: Option<Vec<u8>> = stmt
            .query_row([code_hash.to_string()], |row| {
                let bytes: Vec<u8> = row.get(0)?;
                Ok(bytes)
            })
            .optional()?;

        if let Some(bytes) = bytecode {
            let bytecode: Bytecode = Bytecode::LegacyRaw(Bytes::copy_from_slice(&bytes));
            Ok(Some(bytecode))
        } else {
            Ok(None)
        }
    }

    /// Update bytecode in the database
    pub(crate) fn update_bytecode(
        &self,
        code_hash: B256,
        bytecode: Bytecode,
    ) -> Result<(), DiskStorageError> {
        let conn = self.conn.lock();
        let result = conn.execute(
            "INSERT INTO account_code (codehash, bytecode) VALUES (?1, ?2)
            ON CONFLICT(codehash) DO UPDATE SET bytecode = excluded.bytecode",
            rusqlite::params![code_hash.to_string(), bytecode.bytes_slice()],
        );

        match result {
            Ok(_) => Ok(()),
            Err(e) => Err(DiskStorageError::Database(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use tempfile::tempdir;

    #[test]
    fn test_update_and_get_account_code() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let storage = DiskStorage::new(db_path.to_str().unwrap());

        let code_hash = B256::random();
        let bytecode = Bytecode::LegacyRaw(Bytes::from_str("0xabcdef").unwrap());

        let result = storage.update_bytecode(code_hash, bytecode.clone());
        assert!(result.is_ok(), "Failed to update account code");

        let retrieved_bytecode = storage.get_bytecode(code_hash).unwrap();
        assert!(
            retrieved_bytecode.is_some(),
            "Expected bytecode to be found"
        );

        assert_eq!(
            retrieved_bytecode.unwrap(),
            bytecode,
            "Retrieved bytecode does not match the original"
        );
    }

    #[test]
    fn test_get_account_code_not_exist() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let storage = DiskStorage::new(db_path.to_str().unwrap());

        let code_hash = B256::random();
        let retrieved_bytecode = storage.get_bytecode(code_hash).unwrap();
        assert!(
            retrieved_bytecode.is_none(),
            "Expected bytecode to be None for non-existent code hash"
        );
    }
}
