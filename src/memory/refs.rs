//! Context references for linking memory entries to agents
//!
//! ContextRef allows agents to reference specific memory entries with different
//! relationship types:
//! - **Attached**: Auto-loads when agent starts
//! - **Bookmarked**: Quick retrieval for agent
//! - **Inherited**: Copied from parent when agent was forked
//! - **Pinned**: Always visible regardless of scope

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::agent::AgentId;
use super::entry::MemoryId;

/// Unique identifier for a context reference
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContextRefId(Ulid);

impl ContextRefId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }

    pub fn from_ulid(ulid: Ulid) -> Self {
        Self(ulid)
    }
}

impl Default for ContextRefId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ContextRefId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for ContextRefId {
    type Err = ulid::DecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Ulid::from_string(s)?))
    }
}

/// Type of context reference
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContextRefType {
    /// Attached context - auto-loads when agent starts
    Attached,

    /// Bookmarked - quick retrieval for agent
    Bookmarked,

    /// Inherited from parent agent (when forked)
    Inherited {
        /// The parent agent this was inherited from
        parent_agent: AgentId,
    },

    /// Pinned - always visible regardless of scope
    Pinned,
}

impl ContextRefType {
    pub fn name(&self) -> &'static str {
        match self {
            ContextRefType::Attached => "attached",
            ContextRefType::Bookmarked => "bookmarked",
            ContextRefType::Inherited { .. } => "inherited",
            ContextRefType::Pinned => "pinned",
        }
    }
}

/// A reference from an agent to a specific memory entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRef {
    /// Unique identifier for this reference
    pub id: ContextRefId,

    /// The agent that owns this reference
    pub agent_id: AgentId,

    /// The memory entry being referenced
    pub memory_id: MemoryId,

    /// Type of reference
    pub ref_type: ContextRefType,

    /// When this reference was created
    pub created_at: DateTime<Utc>,

    /// Optional metadata (labels, notes, etc.)
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl ContextRef {
    /// Create a new context reference
    pub fn new(agent_id: AgentId, memory_id: MemoryId, ref_type: ContextRefType) -> Self {
        Self {
            id: ContextRefId::new(),
            agent_id,
            memory_id,
            ref_type,
            created_at: Utc::now(),
            metadata: HashMap::new(),
        }
    }

    /// Create an attached reference
    pub fn attached(agent_id: AgentId, memory_id: MemoryId) -> Self {
        Self::new(agent_id, memory_id, ContextRefType::Attached)
    }

    /// Create a bookmarked reference
    pub fn bookmarked(agent_id: AgentId, memory_id: MemoryId) -> Self {
        Self::new(agent_id, memory_id, ContextRefType::Bookmarked)
    }

    /// Create an inherited reference
    pub fn inherited(agent_id: AgentId, memory_id: MemoryId, parent_agent: AgentId) -> Self {
        Self::new(agent_id, memory_id, ContextRefType::Inherited { parent_agent })
    }

    /// Create a pinned reference
    pub fn pinned(agent_id: AgentId, memory_id: MemoryId) -> Self {
        Self::new(agent_id, memory_id, ContextRefType::Pinned)
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Check if this is an auto-load reference type
    pub fn is_auto_load(&self) -> bool {
        matches!(self.ref_type, ContextRefType::Attached | ContextRefType::Inherited { .. })
    }
}

/// Store for context references
///
/// Provides CRUD operations and indexing by agent.
#[derive(Debug, Default)]
pub struct ContextRefStore {
    /// All refs by ID
    refs: HashMap<ContextRefId, ContextRef>,
    /// Index: agent_id -> list of ref IDs
    by_agent: HashMap<AgentId, Vec<ContextRefId>>,
    /// Index: memory_id -> list of ref IDs
    by_memory: HashMap<MemoryId, Vec<ContextRefId>>,
}

impl ContextRefStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a context reference
    pub fn add(&mut self, ctx_ref: ContextRef) {
        let id = ctx_ref.id;
        let agent_id = ctx_ref.agent_id;
        let memory_id = ctx_ref.memory_id;

        // Add to main store
        self.refs.insert(id, ctx_ref);

        // Update agent index
        self.by_agent.entry(agent_id).or_default().push(id);

        // Update memory index
        self.by_memory.entry(memory_id).or_default().push(id);
    }

    /// Remove a context reference
    pub fn remove(&mut self, id: ContextRefId) -> Option<ContextRef> {
        if let Some(ctx_ref) = self.refs.remove(&id) {
            // Update agent index
            if let Some(refs) = self.by_agent.get_mut(&ctx_ref.agent_id) {
                refs.retain(|&r| r != id);
            }

            // Update memory index
            if let Some(refs) = self.by_memory.get_mut(&ctx_ref.memory_id) {
                refs.retain(|&r| r != id);
            }

            Some(ctx_ref)
        } else {
            None
        }
    }

    /// Get a context reference by ID
    pub fn get(&self, id: ContextRefId) -> Option<&ContextRef> {
        self.refs.get(&id)
    }

    /// Get all refs for an agent
    pub fn by_agent(&self, agent_id: AgentId) -> Vec<&ContextRef> {
        self.by_agent
            .get(&agent_id)
            .map(|ids| ids.iter().filter_map(|id| self.refs.get(id)).collect())
            .unwrap_or_default()
    }

    /// Get all refs for an agent filtered by type
    pub fn by_agent_and_type(&self, agent_id: AgentId, ref_type: &ContextRefType) -> Vec<&ContextRef> {
        self.by_agent(agent_id)
            .into_iter()
            .filter(|r| std::mem::discriminant(&r.ref_type) == std::mem::discriminant(ref_type))
            .collect()
    }

    /// Get auto-load refs for an agent (Attached + Inherited)
    pub fn auto_load_refs(&self, agent_id: AgentId) -> Vec<&ContextRef> {
        self.by_agent(agent_id)
            .into_iter()
            .filter(|r| r.is_auto_load())
            .collect()
    }

    /// Get all refs pointing to a memory entry
    pub fn by_memory(&self, memory_id: MemoryId) -> Vec<&ContextRef> {
        self.by_memory
            .get(&memory_id)
            .map(|ids| ids.iter().filter_map(|id| self.refs.get(id)).collect())
            .unwrap_or_default()
    }

    /// Check if an agent has a specific memory ref
    pub fn has_ref(&self, agent_id: AgentId, memory_id: MemoryId) -> bool {
        self.by_agent(agent_id)
            .iter()
            .any(|r| r.memory_id == memory_id)
    }

    /// Remove a ref by agent and memory (if exists)
    pub fn remove_by_agent_memory(&mut self, agent_id: AgentId, memory_id: MemoryId) -> Option<ContextRef> {
        let id = self.by_agent(agent_id)
            .iter()
            .find(|r| r.memory_id == memory_id)
            .map(|r| r.id)?;
        self.remove(id)
    }

    /// Get all refs
    pub fn all(&self) -> impl Iterator<Item = &ContextRef> {
        self.refs.values()
    }

    /// Count of refs
    pub fn len(&self) -> usize {
        self.refs.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.refs.is_empty()
    }

    /// Copy refs from parent agent to child (for forking)
    /// Returns the number of refs copied
    pub fn inherit_from_parent(
        &mut self,
        parent_id: AgentId,
        child_id: AgentId,
        memory_ids: Option<&[MemoryId]>,
    ) -> usize {
        let parent_refs: Vec<_> = self.by_agent(parent_id)
            .into_iter()
            .filter(|r| {
                // Only inherit pinned refs by default, or specific ones if provided
                if let Some(ids) = memory_ids {
                    ids.contains(&r.memory_id)
                } else {
                    matches!(r.ref_type, ContextRefType::Pinned)
                }
            })
            .map(|r| r.memory_id)
            .collect();

        let count = parent_refs.len();

        for memory_id in parent_refs {
            let new_ref = ContextRef::inherited(child_id, memory_id, parent_id);
            self.add(new_ref);
        }

        count
    }
}

/// Persistent context ref store backed by redb
///
/// Wraps an in-memory ContextRefStore with redb persistence.
/// All mutations are synced to disk.
pub struct PersistentContextRefStore {
    store: ContextRefStore,
    db: std::sync::Arc<crate::storage::Store>,
}

impl PersistentContextRefStore {
    /// Create a new persistent store, loading existing refs from redb
    pub fn new(db: std::sync::Arc<crate::storage::Store>) -> anyhow::Result<Self> {
        use crate::storage::{Storable, Table};

        let mut store = ContextRefStore::new();

        // Load existing refs from database
        let entries = db.list_all(Table::ContextRefs)?;
        for (_key, bytes) in entries {
            if let Ok(ctx_ref) = ContextRef::from_bytes(&bytes) {
                store.add(ctx_ref);
            }
        }

        Ok(Self { store, db })
    }

    /// Add a context reference (persisted)
    pub fn add(&mut self, ctx_ref: ContextRef) -> anyhow::Result<()> {
        use crate::storage::{Storable, Table};

        let key = ctx_ref.id.to_string();
        let bytes = ctx_ref.to_bytes()?;
        self.db.put(Table::ContextRefs, &key, &bytes)?;
        self.store.add(ctx_ref);
        Ok(())
    }

    /// Remove a context reference (persisted)
    pub fn remove(&mut self, id: ContextRefId) -> anyhow::Result<Option<ContextRef>> {
        use crate::storage::Table;

        let key = id.to_string();
        self.db.delete(Table::ContextRefs, &key)?;
        Ok(self.store.remove(id))
    }

    /// Get a context reference by ID
    pub fn get(&self, id: ContextRefId) -> Option<&ContextRef> {
        self.store.get(id)
    }

    /// Get all refs for an agent
    pub fn by_agent(&self, agent_id: AgentId) -> Vec<&ContextRef> {
        self.store.by_agent(agent_id)
    }

    /// Get all refs for an agent filtered by type
    pub fn by_agent_and_type(&self, agent_id: AgentId, ref_type: &ContextRefType) -> Vec<&ContextRef> {
        self.store.by_agent_and_type(agent_id, ref_type)
    }

    /// Get auto-load refs for an agent (Attached + Inherited)
    pub fn auto_load_refs(&self, agent_id: AgentId) -> Vec<&ContextRef> {
        self.store.auto_load_refs(agent_id)
    }

    /// Get all refs pointing to a memory entry
    pub fn by_memory(&self, memory_id: MemoryId) -> Vec<&ContextRef> {
        self.store.by_memory(memory_id)
    }

    /// Check if an agent has a specific memory ref
    pub fn has_ref(&self, agent_id: AgentId, memory_id: MemoryId) -> bool {
        self.store.has_ref(agent_id, memory_id)
    }

    /// Remove a ref by agent and memory (if exists)
    pub fn remove_by_agent_memory(&mut self, agent_id: AgentId, memory_id: MemoryId) -> anyhow::Result<Option<ContextRef>> {
        if let Some(ctx_ref) = self.store.by_agent(agent_id).iter().find(|r| r.memory_id == memory_id) {
            let id = ctx_ref.id;
            return self.remove(id);
        }
        Ok(None)
    }

    /// Get all refs
    pub fn all(&self) -> impl Iterator<Item = &ContextRef> {
        self.store.all()
    }

    /// Count of refs
    pub fn len(&self) -> usize {
        self.store.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }

    /// Copy refs from parent agent to child (for forking)
    /// Returns the number of refs copied
    pub fn inherit_from_parent(
        &mut self,
        parent_id: AgentId,
        child_id: AgentId,
        memory_ids: Option<&[MemoryId]>,
    ) -> anyhow::Result<usize> {
        use crate::storage::{Storable, Table};

        let parent_refs: Vec<_> = self.store.by_agent(parent_id)
            .into_iter()
            .filter(|r| {
                if let Some(ids) = memory_ids {
                    ids.contains(&r.memory_id)
                } else {
                    matches!(r.ref_type, ContextRefType::Pinned)
                }
            })
            .map(|r| r.memory_id)
            .collect();

        let count = parent_refs.len();

        for memory_id in parent_refs {
            let new_ref = ContextRef::inherited(child_id, memory_id, parent_id);
            let key = new_ref.id.to_string();
            let bytes = new_ref.to_bytes()?;
            self.db.put(Table::ContextRefs, &key, &bytes)?;
            self.store.add(new_ref);
        }

        Ok(count)
    }

    /// Remove all refs for an agent (cleanup on agent termination)
    pub fn remove_by_agent(&mut self, agent_id: AgentId) -> anyhow::Result<usize> {
        let refs_to_remove: Vec<_> = self.store.by_agent(agent_id).iter().map(|r| r.id).collect();
        let count = refs_to_remove.len();

        for id in refs_to_remove {
            self.remove(id)?;
        }

        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentId;

    #[test]
    fn test_context_ref_creation() {
        let agent_id = AgentId::new();
        let memory_id = MemoryId::new();

        let attached = ContextRef::attached(agent_id, memory_id);
        assert!(matches!(attached.ref_type, ContextRefType::Attached));
        assert!(attached.is_auto_load());

        let bookmarked = ContextRef::bookmarked(agent_id, memory_id);
        assert!(matches!(bookmarked.ref_type, ContextRefType::Bookmarked));
        assert!(!bookmarked.is_auto_load());

        let parent_id = AgentId::new();
        let inherited = ContextRef::inherited(agent_id, memory_id, parent_id);
        assert!(matches!(inherited.ref_type, ContextRefType::Inherited { .. }));
        assert!(inherited.is_auto_load());
    }

    #[test]
    fn test_context_ref_store() {
        let mut store = ContextRefStore::new();
        let agent_id = AgentId::new();
        let memory_id = MemoryId::new();

        let ctx_ref = ContextRef::attached(agent_id, memory_id);
        let ref_id = ctx_ref.id;
        store.add(ctx_ref);

        assert_eq!(store.len(), 1);
        assert!(store.get(ref_id).is_some());
        assert_eq!(store.by_agent(agent_id).len(), 1);
        assert!(store.has_ref(agent_id, memory_id));

        store.remove(ref_id);
        assert!(store.is_empty());
    }

    #[test]
    fn test_inherit_from_parent() {
        let mut store = ContextRefStore::new();
        let parent_id = AgentId::new();
        let child_id = AgentId::new();
        let memory_id = MemoryId::new();

        // Add a pinned ref to parent
        store.add(ContextRef::pinned(parent_id, memory_id));

        // Inherit to child
        let count = store.inherit_from_parent(parent_id, child_id, None);
        assert_eq!(count, 1);

        // Child should have inherited ref
        let child_refs = store.by_agent(child_id);
        assert_eq!(child_refs.len(), 1);
        assert!(matches!(child_refs[0].ref_type, ContextRefType::Inherited { .. }));
    }
}
