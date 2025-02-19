use alloy_primitives::{map::B256HashSet, B256};
use itertools::Itertools;
use metrics::Label;
use reth_db::{
    create_db, database_metrics::DatabaseMetrics, mdbx::DatabaseArguments, Database, DatabaseEnv,
    DatabaseError,
};
use reth_db_api::{
    cursor::DbCursorRO,
    transaction::{DbTx, DbTxMut},
};
use reth_primitives::Bytecode;
use std::{path::Path, sync::Arc};
use tracing::*;

mod tables {
    use alloy_primitives::B256;
    use reth_db::{tables, TableSet, TableType, TableViewer};
    use reth_db_api::table::TableInfo;
    use reth_primitives::Bytecode;
    use std::fmt;

    tables! {
        /// Stores all contract bytecodes.
        table Bytecodes {
            type Key = B256;
            type Value = Bytecode;
        }
    }
}
use tables::{Bytecodes, Tables};

/// Ress persisted database for storing bytecodes.
#[derive(Clone, Debug)]
pub struct RessDatabase {
    database: Arc<DatabaseEnv>,
}

impl RessDatabase {
    /// Create new database at path.
    pub fn new<P: AsRef<Path>>(path: P) -> eyre::Result<Self> {
        Self::new_with_args(
            path,
            DatabaseArguments::default()
                .with_growth_step(Some(1024 * 1024 * 1024))
                .with_geometry_max_size(Some(64 * 1024 * 1024 * 1024)),
        )
    }

    /// Create new database at path with arguments.
    pub fn new_with_args<P: AsRef<Path>>(path: P, args: DatabaseArguments) -> eyre::Result<Self> {
        let database = create_db(path, args)?;
        database.create_tables_for::<Tables>()?;
        Ok(Self { database: Arc::new(database) })
    }

    /// Check if bytecode exists in the database.
    /// NOTE: find a better way to check this.
    pub fn bytecode_exists(&self, code_hash: B256) -> Result<bool, DatabaseError> {
        Ok(self.get_bytecode(code_hash)?.is_some())
    }

    /// Get bytecode by code hash.
    pub fn get_bytecode(&self, code_hash: B256) -> Result<Option<Bytecode>, DatabaseError> {
        self.database.tx()?.get::<Bytecodes>(code_hash)
    }

    /// Insert bytecode into the database.
    pub fn insert_bytecode(
        &self,
        code_hash: B256,
        bytecode: Bytecode,
    ) -> Result<(), DatabaseError> {
        let tx_mut = self.database.tx_mut()?;
        tx_mut.put::<Bytecodes>(code_hash, bytecode)?;
        tx_mut.commit()?;
        Ok(())
    }

    /// Filter the collection of code hashes for the ones that are missing from the database.
    pub fn missing_code_hashes(
        &self,
        code_hashes: B256HashSet,
    ) -> Result<B256HashSet, DatabaseError> {
        let mut missing = B256HashSet::default();
        let tx = self.database.tx()?;
        let mut cursor = tx.cursor_read::<Bytecodes>()?;
        for code_hash in code_hashes.into_iter().sorted_unstable() {
            if cursor.seek_exact(code_hash)?.is_none() {
                missing.insert(code_hash);
            }
        }
        Ok(missing)
    }

    #[allow(clippy::type_complexity)]
    fn collect_metrics(&self) -> Result<Vec<(&'static str, f64, Vec<Label>)>, DatabaseError> {
        self.database
            .view(|tx| -> reth_db::mdbx::Result<_> {
                let table = Tables::Bytecodes.name();
                let table_label = Label::new("table", table);
                let table_db = tx.inner.open_db(Some(table))?;
                let stats = tx.inner.db_stat(&table_db)?;

                let page_size = stats.page_size() as usize;
                let leaf_pages = stats.leaf_pages();
                let branch_pages = stats.branch_pages();
                let overflow_pages = stats.overflow_pages();
                let num_pages = leaf_pages + branch_pages + overflow_pages;
                let table_size = page_size * num_pages;
                let entries = stats.entries();

                let metrics = Vec::from([
                    ("db.table_size", table_size as f64, Vec::from([table_label.clone()])),
                    (
                        "db.table_pages",
                        leaf_pages as f64,
                        Vec::from([table_label.clone(), Label::new("type", "leaf")]),
                    ),
                    (
                        "db.table_pages",
                        branch_pages as f64,
                        Vec::from([table_label.clone(), Label::new("type", "branch")]),
                    ),
                    (
                        "db.table_pages",
                        overflow_pages as f64,
                        Vec::from([table_label.clone(), Label::new("type", "overflow")]),
                    ),
                    ("db.table_entries", entries as f64, Vec::from([table_label])),
                ]);
                Ok(metrics)
            })?
            .map_err(|e| DatabaseError::Read(e.into()))
    }
}

impl DatabaseMetrics for RessDatabase {
    fn report_metrics(&self) {
        for (name, value, labels) in self.gauge_metrics() {
            metrics::gauge!(name, labels).set(value);
        }
    }

    fn gauge_metrics(&self) -> Vec<(&'static str, f64, Vec<Label>)> {
        self.collect_metrics()
            .inspect_err(
                |error| error!(target: "ress::db", ?error, "Error collecting database metrics"),
            )
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{b256, hex, Bytes};
    use derive_more::Deref;
    use tempfile::TempDir;

    const CREATE_2_DEPLOYER_CODEHASH: B256 =
        b256!("b0550b5b431e30d38000efb7107aaa0ade03d48a7198a140edda9d27134468b2");
    const CREATE_2_DEPLOYER_BYTECODE: [u8; 1584] = hex!("6080604052600436106100435760003560e01c8063076c37b21461004f578063481286e61461007157806356299481146100ba57806366cfa057146100da57600080fd5b3661004a57005b600080fd5b34801561005b57600080fd5b5061006f61006a366004610327565b6100fa565b005b34801561007d57600080fd5b5061009161008c366004610327565b61014a565b60405173ffffffffffffffffffffffffffffffffffffffff909116815260200160405180910390f35b3480156100c657600080fd5b506100916100d5366004610349565b61015d565b3480156100e657600080fd5b5061006f6100f53660046103ca565b610172565b61014582826040518060200161010f9061031a565b7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe082820381018352601f90910116604052610183565b505050565b600061015683836102e7565b9392505050565b600061016a8484846102f0565b949350505050565b61017d838383610183565b50505050565b6000834710156101f4576040517f08c379a000000000000000000000000000000000000000000000000000000000815260206004820152601d60248201527f437265617465323a20696e73756666696369656e742062616c616e636500000060448201526064015b60405180910390fd5b815160000361025f576040517f08c379a000000000000000000000000000000000000000000000000000000000815260206004820181905260248201527f437265617465323a2062797465636f6465206c656e677468206973207a65726f60448201526064016101eb565b8282516020840186f5905073ffffffffffffffffffffffffffffffffffffffff8116610156576040517f08c379a000000000000000000000000000000000000000000000000000000000815260206004820152601960248201527f437265617465323a204661696c6564206f6e206465706c6f790000000000000060448201526064016101eb565b60006101568383305b6000604051836040820152846020820152828152600b8101905060ff815360559020949350505050565b61014e806104ad83390190565b6000806040838503121561033a57600080fd5b50508035926020909101359150565b60008060006060848603121561035e57600080fd5b8335925060208401359150604084013573ffffffffffffffffffffffffffffffffffffffff8116811461039057600080fd5b809150509250925092565b7f4e487b7100000000000000000000000000000000000000000000000000000000600052604160045260246000fd5b6000806000606084860312156103df57600080fd5b8335925060208401359150604084013567ffffffffffffffff8082111561040557600080fd5b818601915086601f83011261041957600080fd5b81358181111561042b5761042b61039b565b604051601f82017fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe0908116603f011681019083821181831017156104715761047161039b565b8160405282815289602084870101111561048a57600080fd5b826020860160208301376000602084830101528095505050505050925092509256fe608060405234801561001057600080fd5b5061012e806100206000396000f3fe6080604052348015600f57600080fd5b506004361060285760003560e01c8063249cb3fa14602d575b600080fd5b603c603836600460b1565b604e565b60405190815260200160405180910390f35b60008281526020818152604080832073ffffffffffffffffffffffffffffffffffffffff8516845290915281205460ff16608857600060aa565b7fa2ef4600d742022d532d4747cb3547474667d6f13804902513b2ec01c848f4b45b9392505050565b6000806040838503121560c357600080fd5b82359150602083013573ffffffffffffffffffffffffffffffffffffffff8116811460ed57600080fd5b80915050925092905056fea26469706673582212205ffd4e6cede7d06a5daf93d48d0541fc68189eeb16608c1999a82063b666eb1164736f6c63430008130033a2646970667358221220fdc4a0fe96e3b21c108ca155438d37c9143fb01278a3c1d274948bad89c564ba64736f6c63430008130033");

    #[derive(Deref, Debug)]
    struct TempDatabase {
        #[deref]
        database: RessDatabase,
        #[allow(dead_code)]
        tempdir: TempDir,
    }

    fn create_test_db() -> eyre::Result<TempDatabase> {
        let tempdir = tempfile::Builder::new().prefix("ress-test-").rand_bytes(8).tempdir()?;
        Ok(TempDatabase { database: RessDatabase::new(tempdir.path())?, tempdir })
    }

    #[test]
    fn bytecode_persistence() {
        let database = create_test_db().unwrap();
        let bytecode = Bytecode::new_raw(Bytes::from_static(&CREATE_2_DEPLOYER_BYTECODE));
        let code_hash = bytecode.hash_slow();
        assert_eq!(code_hash, CREATE_2_DEPLOYER_CODEHASH);

        assert!(!database.bytecode_exists(code_hash).unwrap());
        assert_eq!(
            database.missing_code_hashes(B256HashSet::from_iter([code_hash])).unwrap(),
            B256HashSet::from_iter([code_hash])
        );
        database.insert_bytecode(code_hash, bytecode.clone()).unwrap();
        assert!(database.bytecode_exists(code_hash).unwrap());
        assert_eq!(
            database.missing_code_hashes(B256HashSet::from_iter([code_hash])).unwrap(),
            B256HashSet::default()
        );
        assert_eq!(database.get_bytecode(code_hash).unwrap(), Some(bytecode));
    }
}
