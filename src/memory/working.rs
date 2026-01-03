//! Working memory - fast in-memory session context

use std::collections::HashMap;

use super::entry::{MemoryContent, MemoryEntry, MemoryId, MemoryScope};
use super::query::MemoryQuery;

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

    /// Query entries matching criteria
    pub fn query(&self, query: &MemoryQuery) -> Vec<MemoryEntry> {
        self.entries
            .values()
            .filter(|(entry, scope)| {
                // Filter by scope
                if let Some(ref query_scope) = query.scope {
                    if scope != query_scope {
                        return false;
                    }
                }

                // Filter by time range
                if let Some(since) = query.since {
                    if entry.created_at < since {
                        return false;
                    }
                }
                if let Some(until) = query.until {
                    if entry.created_at > until {
                        return false;
                    }
                }

                // Filter by memory type
                if let Some(ref memory_type) = query.memory_type {
                    let entry_type = content_type_name(&entry.content);
                    if entry_type != memory_type {
                        return false;
                    }
                }

                // Filter by tags (all must match)
                for tag in &query.tags {
                    if !entry.tags.contains(tag) {
                        return false;
                    }
                }

                // Filter by text (simple substring match)
                if let Some(ref text) = query.text {
                    let text_lower = text.to_lowercase();
                    let content_str = format!("{:?}", entry.content).to_lowercase();
                    let id_str = entry.id.to_string().to_lowercase();

                    if !content_str.contains(&text_lower) && !id_str.contains(&text_lower) {
                        return false;
                    }
                }

                true
            })
            .map(|(entry, _)| entry.clone())
            .collect()
    }
}

/// Get the type name for a MemoryContent variant
fn content_type_name(content: &MemoryContent) -> &'static str {
    match content {
        MemoryContent::FileDiscovered { .. } => "file_discovered",
        MemoryContent::PatternLearned { .. } => "pattern_learned",
        MemoryContent::DependencyMapped { .. } => "dependency_mapped",
        MemoryContent::ErrorEncountered { .. } => "error_encountered",
        MemoryContent::DecisionMade { .. } => "decision_made",
        MemoryContent::TaskCompleted { .. } => "task_completed",
        MemoryContent::InsightShared { .. } => "insight_shared",
        MemoryContent::QuestionAsked { .. } => "question_asked",
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
