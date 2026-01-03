//! Memory query types

use super::entry::MemoryScope;

/// Query parameters for searching memory
#[derive(Debug, Clone, Default)]
pub struct MemoryQuery {
    /// Full-text search query
    pub text: Option<String>,

    /// Filter by scope
    pub scope: Option<MemoryScope>,

    /// Filter by memory type
    pub memory_type: Option<String>,

    /// Filter by tags
    pub tags: Vec<String>,

    /// Time range start
    pub since: Option<chrono::DateTime<chrono::Utc>>,

    /// Time range end
    pub until: Option<chrono::DateTime<chrono::Utc>>,

    /// Maximum results
    pub limit: Option<usize>,

    /// Offset for pagination
    pub offset: Option<usize>,
}

impl MemoryQuery {
    /// Create a new empty query
    pub fn new() -> Self {
        Self::default()
    }

    /// Add text search
    pub fn text(mut self, text: &str) -> Self {
        self.text = Some(text.to_string());
        self
    }

    /// Filter by scope
    pub fn scope(mut self, scope: MemoryScope) -> Self {
        self.scope = Some(scope);
        self
    }

    /// Filter by type
    pub fn memory_type(mut self, t: &str) -> Self {
        self.memory_type = Some(t.to_string());
        self
    }

    /// Add tag filter
    pub fn tag(mut self, tag: &str) -> Self {
        self.tags.push(tag.to_string());
        self
    }

    /// Set time range
    pub fn since(mut self, since: chrono::DateTime<chrono::Utc>) -> Self {
        self.since = Some(since);
        self
    }

    /// Set limit
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }
}
