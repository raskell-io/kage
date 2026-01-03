//! Working memory - fast in-memory session context

use std::collections::HashMap;

use super::entry::{MemoryEntry, MemoryId, MemoryScope};

/// Fast in-memory working memory
pub struct WorkingMemory {
    /// Entries by ID
    entries: HashMap<MemoryId, (MemoryEntry, MemoryScope)>,

    /// Index by scope
    by_scope: HashMap<String, Vec<MemoryId>>,

    /// TTL tracking (when entries should expire)
    expiry: HashMap<MemoryId, chrono::DateTime<chrono::Utc>>,
}

impl WorkingMemory {
    /// Create a new working memory
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            by_scope: HashMap::new(),
            expiry: HashMap::new(),
        }
    }

    /// Store an entry
    pub fn store(&mut self, entry: MemoryEntry, scope: MemoryScope) {
        let id = entry.id;
        let scope_key = scope_to_key(&scope);

        // Add to main storage
        self.entries.insert(id, (entry, scope));

        // Add to scope index
        self.by_scope
            .entry(scope_key)
            .or_insert_with(Vec::new)
            .push(id);

        // Set expiry (default 24h)
        let expiry = chrono::Utc::now() + chrono::Duration::hours(24);
        self.expiry.insert(id, expiry);
    }

    /// Get an entry by ID
    pub fn get(&self, id: MemoryId) -> Option<&MemoryEntry> {
        self.entries.get(&id).map(|(e, _)| e)
    }

    /// Get entries by scope
    pub fn get_by_scope(&self, scope: &MemoryScope) -> Vec<&MemoryEntry> {
        let scope_key = scope_to_key(scope);
        self.by_scope
            .get(&scope_key)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.entries.get(id).map(|(e, _)| e))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Evict expired entries
    pub fn evict_expired(&mut self) {
        let now = chrono::Utc::now();
        let expired: Vec<MemoryId> = self
            .expiry
            .iter()
            .filter(|(_, exp)| **exp < now)
            .map(|(id, _)| *id)
            .collect();

        for id in expired {
            self.entries.remove(&id);
            self.expiry.remove(&id);
            // Note: We don't clean up by_scope here for efficiency
            // It will have stale references but get() filters them
        }
    }

    /// Get entry count
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for WorkingMemory {
    fn default() -> Self {
        Self::new()
    }
}

fn scope_to_key(scope: &MemoryScope) -> String {
    match scope {
        MemoryScope::Agent(id) => format!("agent:{}", id),
        MemoryScope::Namespace(name) => format!("namespace:{}", name),
        MemoryScope::Global => "global".to_string(),
    }
}
