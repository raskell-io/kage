//! Memory entry types

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::agent::AgentId;
use crate::task::TaskId;

/// Unique identifier for a memory entry
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MemoryId(Ulid);

impl MemoryId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for MemoryId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MemoryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for MemoryId {
    type Err = ulid::DecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Ulid::from_string(s)?))
    }
}

/// Memory scope controls visibility
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryScope {
    /// Private to one agent
    Agent(AgentId),

    /// Shared within a namespace
    Namespace(String),

    /// Available to all agents (opt-in)
    Global,
}

/// What agents can remember and share
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// Unique ID
    pub id: MemoryId,

    /// When this was created
    pub created_at: chrono::DateTime<chrono::Utc>,

    /// Which agent created this
    pub created_by: AgentId,

    /// The actual content
    pub content: MemoryContent,

    /// Tags for filtering
    pub tags: Vec<String>,
}

impl MemoryEntry {
    /// Create a new memory entry
    pub fn new(agent: AgentId, content: MemoryContent) -> Self {
        Self {
            id: MemoryId::new(),
            created_at: chrono::Utc::now(),
            created_by: agent,
            content,
            tags: vec![],
        }
    }

    /// Add tags
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

/// Types of memory content
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MemoryContent {
    /// Discovered a file and its structure
    FileDiscovered {
        path: PathBuf,
        summary: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        structure: Option<String>,
    },

    /// Learned a reusable pattern
    PatternLearned {
        pattern: String,
        examples: Vec<String>,
        confidence: f32,
    },

    /// Mapped a dependency relationship
    DependencyMapped {
        from: String,
        to: String,
        relationship: String,
    },

    /// Encountered an error and how it was resolved
    ErrorEncountered {
        error: String,
        resolution: Option<String>,
        worked: bool,
    },

    /// Made an architectural decision
    DecisionMade {
        decision: String,
        reasoning: String,
        alternatives: Vec<String>,
    },

    /// Completed a task
    TaskCompleted {
        task_id: TaskId,
        summary: String,
        artifacts: Vec<PathBuf>,
    },

    /// Explicitly shared an insight
    InsightShared {
        topic: String,
        content: String,
    },

    /// Asked a question (possibly with answer)
    QuestionAsked {
        question: String,
        answer: Option<String>,
    },
}
