use alloy_primitives::{Bytes, B256};
use reth::revm::primitives::Bytecode;
use rusqlite::{Connection, OptionalExtension, Result};

use crate::errors::StorageError;

// todo: for now for simplicity using sqlite, mb later move kv storage like libmbdx
pub struct DiskStorage {
    conn: Connection,
}

impl DiskStorage {
    pub fn new(path: &str) -> Self {
        let conn = Connection::open(path).unwrap();
        conn.execute(
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

    /// get bytecode from disk -> fall back network
    pub fn get_account_code(&self, code_hash: B256) -> Result<Option<Bytecode>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT bytecode FROM account_code WHERE codehash = ?1")
            .unwrap();
        let bytecode: Option<Vec<u8>> = stmt
            .query_row([code_hash.to_string()], |row| {
                let bytes: Vec<u8> = row.get(0)?;
                Ok(bytes)
            })
            .optional()
            .unwrap();

        if let Some(bytes) = bytecode {
            let bytecode: Bytecode = Bytecode::LegacyRaw(Bytes::copy_from_slice(&bytes));
            Ok(Some(bytecode))
        } else {
            Ok(None)
        }
    }

    /// update bytecode in the database
    pub fn update_account_code(
        &self,
        code_hash: B256,
        bytecode: Vec<u8>,
    ) -> Result<(), StorageError> {
        self.conn
            .execute(
                "INSERT INTO account_code (codehash, bytecode) VALUES (?1, ?2)
            ON CONFLICT(codehash) DO UPDATE SET bytecode = excluded.bytecode",
                rusqlite::params![code_hash.to_string(), bytecode],
            )
            .unwrap();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_update_and_get_account_code() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let storage = DiskStorage::new(db_path.to_str().unwrap());

        let code_hash = B256::random();
        let bytecode = vec![0x01, 0x02, 0x03, 0x04];

        let result = storage.update_account_code(code_hash.clone(), bytecode.clone());
        assert!(result.is_ok(), "Failed to update account code");

        let retrieved_bytecode = storage.get_account_code(code_hash.clone()).unwrap();
        assert!(
            retrieved_bytecode.is_some(),
            "Expected bytecode to be found"
        );

        assert_eq!(
            retrieved_bytecode.unwrap(),
            Bytecode::LegacyRaw(Bytes::copy_from_slice(&bytecode)),
            "Retrieved bytecode does not match the original"
        );
    }

    #[test]
    fn test_get_account_code_not_exist() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let storage = DiskStorage::new(db_path.to_str().unwrap());

        let code_hash = B256::random();
        let retrieved_bytecode = storage.get_account_code(code_hash).unwrap();
        assert!(
            retrieved_bytecode.is_none(),
            "Expected bytecode to be None for non-existent code hash"
        );
    }
}
