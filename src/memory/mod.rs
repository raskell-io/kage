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

    /// Persistent long-term memory backend
    longterm_backend: Arc<dyn MemoryBackend>,

    /// Real-time pub/sub bus
    pub bus: Arc<ContextBus>,

    /// Legacy accessor for direct LongTermMemory access (prune, etc.)
    /// TODO: Remove once all callers use the backend trait
    pub longterm: Arc<LongTermMemory>,
}

impl MemorySystem {
    /// Create a new memory system with default filesystem backend
    pub fn new(state_dir: std::path::PathBuf) -> anyhow::Result<Self> {
        let longterm = Arc::new(LongTermMemory::new(state_dir.clone())?);
        let backend = backends::FilesystemBackend::new(state_dir.join("events"))?;

        Ok(Self {
            working: Arc::new(RwLock::new(WorkingMemory::new())),
            longterm_backend: Arc::new(backend),
            bus: Arc::new(ContextBus::new()),
            longterm,
        })
    }

    /// Create a new memory system with configurable backend
    pub async fn with_config(
        config: &MemoryBackendConfig,
        state_dir: std::path::PathBuf,
    ) -> anyhow::Result<Self> {
        let longterm = Arc::new(LongTermMemory::new(state_dir.clone())?);
        let backend: Arc<dyn MemoryBackend> = Arc::from(
            backend::create_backend(config, state_dir).await?
        );

        Ok(Self {
            working: Arc::new(RwLock::new(WorkingMemory::new())),
            longterm_backend: backend,
            bus: Arc::new(ContextBus::new()),
            longterm,
        })
    }

    /// Create a new memory system with a specific backend
    pub fn with_backend(
        backend: Box<dyn MemoryBackend>,
        state_dir: std::path::PathBuf,
    ) -> anyhow::Result<Self> {
        let longterm = Arc::new(LongTermMemory::new(state_dir)?);

        Ok(Self {
            working: Arc::new(RwLock::new(WorkingMemory::new())),
            longterm_backend: Arc::from(backend),
            bus: Arc::new(ContextBus::new()),
            longterm,
        })
    }

    /// Get the backend name
    pub fn backend_name(&self) -> &'static str {
        self.longterm_backend.name()
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
            self.longterm_backend.append(entry.clone(), scope.clone()).await?;
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

        // 2. Query long-term backend (slower, historical)
        if let Ok(longterm_entries) = self.longterm_backend.query(&query).await {
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

    /// Prune old entries from long-term storage
    pub async fn prune(&self, older_than_days: u32, dry_run: bool) -> anyhow::Result<(usize, u64)> {
        self.longterm_backend.prune(older_than_days, dry_run).await
    }

    /// Check backend health
    pub async fn health_check(&self) -> anyhow::Result<()> {
        self.longterm_backend.health_check().await
    }
}
