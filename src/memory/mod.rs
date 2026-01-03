//! Two-tier memory system for context sharing
//!
//! - Working Memory: Fast, in-memory, session-scoped
//! - Long-Term Memory: Persistent storage (filesystem, S3, or Azure Blob)

pub mod backend;
pub mod backends;
mod bus;
pub mod entry;
mod longterm;
mod query;
mod working;

pub use backend::{MemoryBackend, MemoryBackendConfig};
pub use bus::ContextBus;
pub use entry::{MemoryContent, MemoryEntry, MemoryId, MemoryScope};
pub use longterm::LongTermMemory;
pub use query::MemoryQuery;
pub use working::WorkingMemory;

use std::sync::Arc;

use tokio::sync::RwLock;

/// Unified memory system
pub struct MemorySystem {
    /// Fast in-memory working memory
    pub working: Arc<RwLock<WorkingMemory>>,

    /// Persistent long-term memory
    pub longterm: Arc<LongTermMemory>,

    /// Real-time pub/sub bus
    pub bus: Arc<ContextBus>,
}

impl MemorySystem {
    /// Create a new memory system
    pub fn new(state_dir: std::path::PathBuf) -> anyhow::Result<Self> {
        Ok(Self {
            working: Arc::new(RwLock::new(WorkingMemory::new())),
            longterm: Arc::new(LongTermMemory::new(state_dir)?),
            bus: Arc::new(ContextBus::new()),
        })
    }

    /// Store a memory entry
    pub async fn store(&self, entry: MemoryEntry, scope: MemoryScope) -> anyhow::Result<()> {
        // Store in working memory
        {
            let mut working = self.working.write().await;
            working.store(entry.clone(), scope.clone());
        }

        // Persist to long-term if not agent-scoped
        if !matches!(scope, MemoryScope::Agent(_)) {
            self.longterm.append(entry.clone(), scope.clone())?;
        }

        // Broadcast to subscribers
        self.bus.publish(entry, scope).await;

        Ok(())
    }

    /// Query memory
    pub async fn query(&self, query: MemoryQuery) -> Vec<MemoryEntry> {
        use std::collections::HashSet;

        let mut results = Vec::new();
        let mut seen_ids: HashSet<String> = HashSet::new();

        // 1. Query working memory first (fast, recent)
        {
            let working = self.working.read().await;
            let working_entries = working.query(&query);
            for entry in working_entries {
                let id_str = entry.id.to_string();
                if !seen_ids.contains(&id_str) {
                    seen_ids.insert(id_str);
                    results.push(entry);
                }
            }
        }

        // 2. Query long-term memory (slower, historical)
        if let Ok(longterm_entries) = self.longterm.query(&query) {
            for entry in longterm_entries {
                let id_str = entry.id.to_string();
                if !seen_ids.contains(&id_str) {
                    seen_ids.insert(id_str);
                    results.push(entry);
                }
            }
        }

        // 3. Sort by created_at descending (newest first)
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        // 4. Apply final limit
        if let Some(limit) = query.limit {
            results.truncate(limit);
        }

        results
    }
}
