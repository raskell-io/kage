//! Embedded storage using redb
//!
//! All state is stored in local files - no external database required.

use std::path::PathBuf;

use anyhow::Result;
use redb::{Database, ReadableTable, TableDefinition};

// Table definitions
const AGENTS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("agents");
const TASKS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("tasks");
const NAMESPACES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("namespaces");
const CONFIG_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("config");
const CONTEXT_REFS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("context_refs");

/// Known table names
#[derive(Debug, Clone, Copy)]
pub enum Table {
    Agents,
    Tasks,
    Namespaces,
    Config,
    ContextRefs,
}

impl Table {
    fn definition(&self) -> TableDefinition<'static, &'static str, &'static [u8]> {
        match self {
            Table::Agents => AGENTS_TABLE,
            Table::Tasks => TASKS_TABLE,
            Table::Namespaces => NAMESPACES_TABLE,
            Table::Config => CONFIG_TABLE,
            Table::ContextRefs => CONTEXT_REFS_TABLE,
        }
    }
}

/// Embedded key-value store
pub struct Store {
    db: Database,
}

impl Store {
    /// Open or create a store at the given path
    pub fn open(path: PathBuf) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let db = Database::create(&path)?;

        // Initialize tables
        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(AGENTS_TABLE)?;
            let _ = write_txn.open_table(TASKS_TABLE)?;
            let _ = write_txn.open_table(NAMESPACES_TABLE)?;
            let _ = write_txn.open_table(CONFIG_TABLE)?;
            let _ = write_txn.open_table(CONTEXT_REFS_TABLE)?;
        }
        write_txn.commit()?;

        Ok(Self { db })
    }

    /// Store a value
    pub fn put(&self, table: Table, key: &str, value: &[u8]) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut t = write_txn.open_table(table.definition())?;
            t.insert(key, value)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Get a value
    pub fn get(&self, table: Table, key: &str) -> Result<Option<Vec<u8>>> {
        let read_txn = self.db.begin_read()?;
        let t = read_txn.open_table(table.definition())?;

        match t.get(key)? {
            Some(value) => Ok(Some(value.value().to_vec())),
            None => Ok(None),
        }
    }

    /// Delete a value
    pub fn delete(&self, table: Table, key: &str) -> Result<bool> {
        let write_txn = self.db.begin_write()?;
        let deleted = {
            let mut t = write_txn.open_table(table.definition())?;
            let result = t.remove(key)?;
            result.is_some()
        };
        write_txn.commit()?;
        Ok(deleted)
    }

    /// List all keys in a table
    pub fn keys(&self, table: Table) -> Result<Vec<String>> {
        let read_txn = self.db.begin_read()?;
        let t = read_txn.open_table(table.definition())?;

        let mut keys = vec![];
        for entry in t.iter()? {
            let (key, _) = entry?;
            keys.push(key.value().to_string());
        }
        Ok(keys)
    }

    /// List all key-value pairs in a table
    pub fn list_all(&self, table: Table) -> Result<Vec<(String, Vec<u8>)>> {
        let read_txn = self.db.begin_read()?;
        let t = read_txn.open_table(table.definition())?;

        let mut pairs = vec![];
        for entry in t.iter()? {
            let (key, value) = entry?;
            pairs.push((key.value().to_string(), value.value().to_vec()));
        }
        Ok(pairs)
    }

    /// Batch insert multiple entries (transactional)
    pub fn put_batch(&self, table: Table, entries: &[(&str, &[u8])]) -> Result<()> {
        let write_txn = self.db.begin_write()?;
        {
            let mut t = write_txn.open_table(table.definition())?;
            for (key, value) in entries {
                t.insert(*key, *value)?;
            }
        }
        write_txn.commit()?;
        Ok(())
    }
}

/// Helper trait for serializing/deserializing stored values
pub trait Storable: serde::Serialize + serde::de::DeserializeOwned {
    fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(rmp_serde::to_vec(self)?)
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Ok(rmp_serde::from_slice(bytes)?)
    }
}

// Blanket impl
impl<T: serde::Serialize + serde::de::DeserializeOwned> Storable for T {}
