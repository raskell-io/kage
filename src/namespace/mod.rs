//! Namespace registry - multi-repository organization

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Unique identifier for a namespace
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NamespaceId(Ulid);

impl NamespaceId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for NamespaceId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for NamespaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A namespace groups related repositories
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Namespace {
    /// Unique ID
    pub id: NamespaceId,

    /// Human-readable name
    pub name: String,

    /// Repositories in this namespace
    pub repositories: Vec<Repository>,

    /// Memory sharing mode
    pub memory_sharing: MemorySharingMode,

    /// When this was created
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Namespace {
    /// Create a new namespace
    pub fn new(name: &str) -> Self {
        Self {
            id: NamespaceId::new(),
            name: name.to_string(),
            repositories: vec![],
            memory_sharing: MemorySharingMode::Full,
            created_at: chrono::Utc::now(),
        }
    }

    /// Add a repository
    pub fn add_repository(&mut self, repo: Repository) {
        self.repositories.push(repo);
    }

    /// Remove a repository by name
    pub fn remove_repository(&mut self, name: &str) -> bool {
        let len = self.repositories.len();
        self.repositories.retain(|r| r.name != name);
        self.repositories.len() < len
    }

    /// Get repository by name
    pub fn get_repository(&self, name: &str) -> Option<&Repository> {
        self.repositories.iter().find(|r| r.name == name)
    }
}

/// A repository within a namespace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    /// Repository name (usually directory name)
    pub name: String,

    /// Local path
    pub path: PathBuf,

    /// Git remote URL (optional)
    pub remote: Option<String>,
}

impl Repository {
    /// Create a new repository reference
    pub fn new(name: &str, path: PathBuf) -> Self {
        Self {
            name: name.to_string(),
            path,
            remote: None,
        }
    }

    /// Set the git remote
    pub fn with_remote(mut self, remote: &str) -> Self {
        self.remote = Some(remote.to_string());
        self
    }
}

/// Memory sharing mode for a namespace
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemorySharingMode {
    /// All memories shared automatically
    Full,

    /// Only explicitly shared memories
    Explicit,

    /// No sharing
    None,
}

impl Default for MemorySharingMode {
    fn default() -> Self {
        Self::Full
    }
}

/// Registry of all namespaces
pub struct NamespaceRegistry {
    namespaces: HashMap<String, Namespace>,
}

impl NamespaceRegistry {
    /// Create a new registry
    pub fn new() -> Self {
        Self {
            namespaces: HashMap::new(),
        }
    }

    /// Create a namespace
    pub fn create(&mut self, name: &str) -> &Namespace {
        let ns = Namespace::new(name);
        self.namespaces.insert(name.to_string(), ns);
        self.namespaces.get(name).unwrap()
    }

    /// Get a namespace by name
    pub fn get(&self, name: &str) -> Option<&Namespace> {
        self.namespaces.get(name)
    }

    /// Get a mutable namespace by name
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Namespace> {
        self.namespaces.get_mut(name)
    }

    /// List all namespaces
    pub fn list(&self) -> Vec<&Namespace> {
        self.namespaces.values().collect()
    }

    /// Delete a namespace
    pub fn delete(&mut self, name: &str) -> bool {
        self.namespaces.remove(name).is_some()
    }
}

impl Default for NamespaceRegistry {
    fn default() -> Self {
        Self::new()
    }
}
