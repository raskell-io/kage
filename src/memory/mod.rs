//! Two-tier memory system for context sharing
//!
//! - Working Memory: Fast, in-memory, session-scoped
//! - Long-Term Memory: Persistent append-only logs

mod bus;
mod entry;
mod longterm;
mod query;
mod working;

pub use bus::ContextBus;
pub use entry::{MemoryEntry, MemoryScope};
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
        // TODO: Implement query across both tiers
        vec![]
    }
}
