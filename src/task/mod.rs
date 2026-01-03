//! Task scheduling and checkpoint management
//!
//! The task system enables agents to work autonomously toward goals with:
//! - Iteration limits and checkpoints
//! - Resume capability with guidance injection
//! - Success/abort criteria detection
//! - Approval workflows

pub mod checkpoint;
pub mod registry;
pub mod scheduler;

pub use checkpoint::CheckpointStore;
pub use registry::TaskRegistry;
pub use scheduler::TaskScheduler;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::agent::AgentId;

/// Unique identifier for a task
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(Ulid);

impl TaskId {
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for TaskId {
    type Err = ulid::DecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Ulid::from_string(s)?))
    }
}

/// Task status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Waiting to be assigned
    Pending,

    /// Currently being worked on
    Running,

    /// Paused (iteration limit, needs approval, etc.)
    Paused,

    /// Successfully completed
    Completed,

    /// Failed
    Failed,

    /// Cancelled by user
    Cancelled,
}

/// A task represents a goal for an agent to accomplish
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Unique ID
    pub id: TaskId,

    /// Natural language goal
    pub goal: String,

    /// Current status
    pub status: TaskStatus,

    /// Assigned agent (if any)
    pub agent: Option<AgentId>,

    /// Namespace this task belongs to
    pub namespace: Option<String>,

    /// Repository path
    pub repository: Option<PathBuf>,

    /// Task configuration
    pub config: TaskConfig,

    /// Current iteration count
    pub iterations: u32,

    /// Priority (higher = more urgent, 0-255)
    pub priority: u8,

    /// Dependencies (tasks that must complete first)
    pub depends_on: Vec<TaskId>,

    /// User-provided guidance for resume
    pub guidance: Option<String>,

    /// Error message if failed
    pub error: Option<String>,

    /// When this was created
    pub created_at: chrono::DateTime<chrono::Utc>,

    /// When this was last updated
    pub updated_at: chrono::DateTime<chrono::Utc>,

    /// When this was started (assigned to agent)
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,

    /// When this was completed (if applicable)
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Task {
    /// Create a new task
    pub fn new(goal: &str) -> Self {
        let now = chrono::Utc::now();
        Self {
            id: TaskId::new(),
            goal: goal.to_string(),
            status: TaskStatus::Pending,
            agent: None,
            namespace: None,
            repository: None,
            config: TaskConfig::default(),
            iterations: 0,
            priority: 100, // Default middle priority
            depends_on: Vec::new(),
            guidance: None,
            error: None,
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
        }
    }

    /// Set namespace
    pub fn with_namespace(mut self, namespace: &str) -> Self {
        self.namespace = Some(namespace.to_string());
        self
    }

    /// Set repository
    pub fn with_repository(mut self, path: PathBuf) -> Self {
        self.repository = Some(path);
        self
    }

    /// Set config
    pub fn with_config(mut self, config: TaskConfig) -> Self {
        self.config = config;
        self
    }

    /// Set priority
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    /// Add a dependency
    pub fn with_dependency(mut self, task_id: TaskId) -> Self {
        self.depends_on.push(task_id);
        self
    }

    /// Set guidance
    pub fn with_guidance(mut self, guidance: &str) -> Self {
        self.guidance = Some(guidance.to_string());
        self
    }

    /// Check if task is ready to run (no unmet dependencies)
    pub fn is_ready(&self, completed_tasks: &[TaskId]) -> bool {
        self.status == TaskStatus::Pending
            && self
                .depends_on
                .iter()
                .all(|dep| completed_tasks.contains(dep))
    }

    /// Increment iteration count
    pub fn increment_iteration(&mut self) {
        self.iterations += 1;
        self.updated_at = chrono::Utc::now();
    }

    /// Check if iteration limit reached
    pub fn is_iteration_limit_reached(&self) -> bool {
        self.iterations >= self.config.max_iterations
    }

    /// Check if checkpoint is due
    pub fn is_checkpoint_due(&self) -> bool {
        self.config.checkpoint_every > 0 && self.iterations % self.config.checkpoint_every == 0
    }

    /// Assign to an agent
    pub fn assign(&mut self, agent_id: AgentId) {
        self.agent = Some(agent_id);
        self.status = TaskStatus::Running;
        self.started_at = Some(chrono::Utc::now());
        self.updated_at = chrono::Utc::now();
    }

    /// Unassign from agent
    pub fn unassign(&mut self) {
        self.agent = None;
        self.status = TaskStatus::Pending;
        self.updated_at = chrono::Utc::now();
    }

    /// Mark as completed
    pub fn complete(&mut self) {
        self.status = TaskStatus::Completed;
        self.completed_at = Some(chrono::Utc::now());
        self.updated_at = chrono::Utc::now();
    }

    /// Mark as failed
    pub fn fail(&mut self, error: Option<&str>) {
        self.status = TaskStatus::Failed;
        self.error = error.map(|s| s.to_string());
        self.updated_at = chrono::Utc::now();
    }

    /// Pause the task
    pub fn pause(&mut self) {
        self.status = TaskStatus::Paused;
        self.updated_at = chrono::Utc::now();
    }

    /// Resume the task with optional guidance
    pub fn resume(&mut self, guidance: Option<&str>, extend_iterations: u32) {
        self.status = TaskStatus::Pending;
        self.config.max_iterations = self.iterations + extend_iterations;
        if let Some(g) = guidance {
            self.guidance = Some(g.to_string());
        }
        self.updated_at = chrono::Utc::now();
    }

    /// Cancel the task
    pub fn cancel(&mut self) {
        self.status = TaskStatus::Cancelled;
        self.updated_at = chrono::Utc::now();
    }

    /// Get duration if completed
    pub fn duration(&self) -> Option<chrono::Duration> {
        match (self.started_at, self.completed_at) {
            (Some(start), Some(end)) => Some(end - start),
            _ => None,
        }
    }
}

/// Task configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskConfig {
    /// Maximum iterations before checkpoint
    pub max_iterations: u32,

    /// Checkpoint interval
    pub checkpoint_every: u32,

    /// Approval level
    pub approval: ApprovalLevel,

    /// Success criteria
    pub success_criteria: Vec<Criterion>,

    /// Abort criteria
    pub abort_criteria: Vec<Criterion>,
}

impl Default for TaskConfig {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            checkpoint_every: 2,
            approval: ApprovalLevel::OnCommit,
            success_criteria: vec![],
            abort_criteria: vec![],
        }
    }
}

/// Approval level for agent actions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalLevel {
    /// No approval needed
    None,

    /// Approve file writes
    OnWrite,

    /// Approve git commits
    OnCommit,

    /// Approve everything
    Always,
}

/// Criteria for success/abort detection
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Criterion {
    /// Match a pattern in output
    PatternMatch { pattern: String },

    /// Check if a file exists
    FileExists { path: PathBuf },

    /// Run a command and check exit code
    CommandSucceeds { command: String },

    /// Check if tests pass
    TestsPass,
}

/// Task checkpoint for resume capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Task ID
    pub task_id: TaskId,

    /// Checkpoint ID
    pub checkpoint_id: Ulid,

    /// Iteration at checkpoint
    pub iteration: u32,

    /// Task state at checkpoint
    pub task_state: Task,

    /// Conversation context (if available)
    pub context: Option<String>,

    /// When this checkpoint was created
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Checkpoint {
    /// Create a new checkpoint
    pub fn new(task: &Task) -> Self {
        Self {
            task_id: task.id,
            checkpoint_id: Ulid::new(),
            iteration: task.iterations,
            task_state: task.clone(),
            context: None,
            created_at: chrono::Utc::now(),
        }
    }

    /// Add conversation context
    pub fn with_context(mut self, context: &str) -> Self {
        self.context = Some(context.to_string());
        self
    }
}
