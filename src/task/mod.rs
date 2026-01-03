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

    /// Check success criteria and return the first one that's met
    pub fn check_success_criteria(&self, ctx: &CriteriaContext) -> Option<CriterionResult> {
        for criterion in &self.config.success_criteria {
            match criterion.evaluate(ctx) {
                CriterionResult::Met { criterion, details } => {
                    return Some(CriterionResult::Met { criterion, details });
                }
                CriterionResult::Error { message } => {
                    tracing::warn!("Error checking success criterion: {}", message);
                }
                CriterionResult::NotMet => {}
            }
        }
        None
    }

    /// Check abort criteria and return the first one that's met
    pub fn check_abort_criteria(&self, ctx: &CriteriaContext) -> Option<CriterionResult> {
        for criterion in &self.config.abort_criteria {
            match criterion.evaluate(ctx) {
                CriterionResult::Met { criterion, details } => {
                    return Some(CriterionResult::Met { criterion, details });
                }
                CriterionResult::Error { message } => {
                    tracing::warn!("Error checking abort criterion: {}", message);
                }
                CriterionResult::NotMet => {}
            }
        }
        None
    }

    /// Check all criteria and return what action should be taken
    pub fn evaluate_criteria(&self, ctx: &CriteriaContext) -> CriteriaAction {
        // Check success criteria first
        if let Some(CriterionResult::Met { criterion, details }) = self.check_success_criteria(ctx) {
            return CriteriaAction::Complete { criterion, details };
        }

        // Check abort criteria
        if let Some(CriterionResult::Met { criterion, details }) = self.check_abort_criteria(ctx) {
            return CriteriaAction::Abort { criterion, details };
        }

        CriteriaAction::Continue
    }
}

/// Action to take based on criteria evaluation
#[derive(Debug, Clone)]
pub enum CriteriaAction {
    /// Continue running the task
    Continue,
    /// Complete the task (success criterion met)
    Complete { criterion: String, details: String },
    /// Abort the task (abort criterion met)
    Abort { criterion: String, details: String },
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
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Criterion {
    /// Match a pattern in output (regex)
    PatternMatch {
        /// Regex pattern to match
        pattern: String,
        /// Match in last N lines (0 = all output)
        #[serde(default)]
        last_n_lines: usize,
    },

    /// Check if a file exists
    FileExists {
        /// Path to check (relative to working dir)
        path: PathBuf,
    },

    /// Check if a file contains a pattern
    FileContains {
        /// Path to check
        path: PathBuf,
        /// Pattern to search for
        pattern: String,
    },

    /// Run a command and check exit code
    CommandSucceeds {
        /// Command to run
        command: String,
        /// Working directory (relative to task repo)
        #[serde(default)]
        working_dir: Option<PathBuf>,
    },

    /// Check if tests pass (auto-detects test runner)
    TestsPass,

    /// Error count threshold (for abort)
    ErrorCount {
        /// Pattern that indicates an error
        pattern: String,
        /// Maximum allowed occurrences
        max_count: usize,
    },

    /// Repeated output detection (stuck in loop)
    RepeatedOutput {
        /// Minimum times output must repeat
        min_repeats: usize,
        /// Number of recent lines to check
        window_size: usize,
    },

    /// Output contains specific phrases (for abort)
    OutputContains {
        /// Phrases to check for
        phrases: Vec<String>,
    },

    /// No progress for N iterations
    NoProgress {
        /// Number of iterations without file changes
        iterations: u32,
    },
}

/// Result of evaluating a criterion
#[derive(Debug, Clone)]
pub enum CriterionResult {
    /// Criterion was met
    Met { criterion: String, details: String },
    /// Criterion was not met
    NotMet,
    /// Error evaluating criterion
    Error { message: String },
}

/// Context for evaluating criteria
#[derive(Debug, Clone, Default)]
pub struct CriteriaContext {
    /// Recent agent output lines
    pub output_lines: Vec<String>,
    /// Working directory
    pub working_dir: Option<PathBuf>,
    /// Files modified in this session
    pub modified_files: Vec<PathBuf>,
    /// Current iteration
    pub iteration: u32,
    /// Last iteration with file changes
    pub last_change_iteration: u32,
}

impl Criterion {
    /// Evaluate this criterion
    pub fn evaluate(&self, ctx: &CriteriaContext) -> CriterionResult {
        match self {
            Criterion::PatternMatch { pattern, last_n_lines } => {
                self.eval_pattern_match(pattern, *last_n_lines, ctx)
            }
            Criterion::FileExists { path } => {
                self.eval_file_exists(path, ctx)
            }
            Criterion::FileContains { path, pattern } => {
                self.eval_file_contains(path, pattern, ctx)
            }
            Criterion::CommandSucceeds { command, working_dir } => {
                self.eval_command_succeeds(command, working_dir.as_ref(), ctx)
            }
            Criterion::TestsPass => {
                self.eval_tests_pass(ctx)
            }
            Criterion::ErrorCount { pattern, max_count } => {
                self.eval_error_count(pattern, *max_count, ctx)
            }
            Criterion::RepeatedOutput { min_repeats, window_size } => {
                self.eval_repeated_output(*min_repeats, *window_size, ctx)
            }
            Criterion::OutputContains { phrases } => {
                self.eval_output_contains(phrases, ctx)
            }
            Criterion::NoProgress { iterations } => {
                self.eval_no_progress(*iterations, ctx)
            }
        }
    }

    fn eval_pattern_match(&self, pattern: &str, last_n_lines: usize, ctx: &CriteriaContext) -> CriterionResult {
        let regex = match regex::Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return CriterionResult::Error { message: format!("Invalid regex: {}", e) },
        };

        let lines_to_check: Vec<_> = if last_n_lines > 0 && ctx.output_lines.len() > last_n_lines {
            ctx.output_lines.iter().rev().take(last_n_lines).collect()
        } else {
            ctx.output_lines.iter().collect()
        };

        for line in lines_to_check {
            if regex.is_match(line) {
                return CriterionResult::Met {
                    criterion: format!("Pattern '{}' matched", pattern),
                    details: line.clone(),
                };
            }
        }

        CriterionResult::NotMet
    }

    fn eval_file_exists(&self, path: &PathBuf, ctx: &CriteriaContext) -> CriterionResult {
        let full_path = if let Some(ref wd) = ctx.working_dir {
            wd.join(path)
        } else {
            path.clone()
        };

        if full_path.exists() {
            CriterionResult::Met {
                criterion: "File exists".to_string(),
                details: full_path.display().to_string(),
            }
        } else {
            CriterionResult::NotMet
        }
    }

    fn eval_file_contains(&self, path: &PathBuf, pattern: &str, ctx: &CriteriaContext) -> CriterionResult {
        let full_path = if let Some(ref wd) = ctx.working_dir {
            wd.join(path)
        } else {
            path.clone()
        };

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(_) => return CriterionResult::NotMet,
        };

        let regex = match regex::Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return CriterionResult::Error { message: format!("Invalid regex: {}", e) },
        };

        if regex.is_match(&content) {
            CriterionResult::Met {
                criterion: format!("File contains pattern '{}'", pattern),
                details: full_path.display().to_string(),
            }
        } else {
            CriterionResult::NotMet
        }
    }

    fn eval_command_succeeds(&self, command: &str, working_dir: Option<&PathBuf>, ctx: &CriteriaContext) -> CriterionResult {
        let wd = working_dir
            .or(ctx.working_dir.as_ref())
            .cloned()
            .unwrap_or_else(|| PathBuf::from("."));

        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&wd)
            .output();

        match output {
            Ok(out) if out.status.success() => CriterionResult::Met {
                criterion: format!("Command succeeded: {}", command),
                details: String::from_utf8_lossy(&out.stdout).to_string(),
            },
            Ok(_) => CriterionResult::NotMet,
            Err(e) => CriterionResult::Error { message: format!("Command failed: {}", e) },
        }
    }

    fn eval_tests_pass(&self, ctx: &CriteriaContext) -> CriterionResult {
        let wd = ctx.working_dir.clone().unwrap_or_else(|| PathBuf::from("."));

        // Try to detect and run the appropriate test command
        let test_commands = [
            ("Cargo.toml", "cargo test --no-fail-fast 2>&1 | tail -5"),
            ("package.json", "npm test 2>&1 | tail -10"),
            ("go.mod", "go test ./... 2>&1 | tail -5"),
            ("pytest.ini", "pytest 2>&1 | tail -5"),
            ("setup.py", "python -m pytest 2>&1 | tail -5"),
            ("Makefile", "make test 2>&1 | tail -5"),
        ];

        for (marker, cmd) in test_commands {
            if wd.join(marker).exists() {
                return self.eval_command_succeeds(cmd, Some(&wd), ctx);
            }
        }

        CriterionResult::Error { message: "No test runner detected".to_string() }
    }

    fn eval_error_count(&self, pattern: &str, max_count: usize, ctx: &CriteriaContext) -> CriterionResult {
        let regex = match regex::Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return CriterionResult::Error { message: format!("Invalid regex: {}", e) },
        };

        let count = ctx.output_lines.iter().filter(|line| regex.is_match(line)).count();

        if count > max_count {
            CriterionResult::Met {
                criterion: format!("Error count exceeded: {} > {}", count, max_count),
                details: format!("Pattern '{}' found {} times", pattern, count),
            }
        } else {
            CriterionResult::NotMet
        }
    }

    fn eval_repeated_output(&self, min_repeats: usize, window_size: usize, ctx: &CriteriaContext) -> CriterionResult {
        if ctx.output_lines.len() < window_size * min_repeats {
            return CriterionResult::NotMet;
        }

        // Get the last window_size lines
        let last_window: Vec<_> = ctx.output_lines.iter().rev().take(window_size).collect();

        // Check previous windows for matches
        let mut repeat_count = 1;
        for i in 1..min_repeats {
            let start = window_size * i;
            let end = start + window_size;

            if end > ctx.output_lines.len() {
                break;
            }

            let prev_window: Vec<_> = ctx.output_lines.iter().rev().skip(start).take(window_size).collect();

            if last_window == prev_window {
                repeat_count += 1;
            }
        }

        if repeat_count >= min_repeats {
            CriterionResult::Met {
                criterion: format!("Output repeated {} times", repeat_count),
                details: "Agent may be stuck in a loop".to_string(),
            }
        } else {
            CriterionResult::NotMet
        }
    }

    fn eval_output_contains(&self, phrases: &[String], ctx: &CriteriaContext) -> CriterionResult {
        let output = ctx.output_lines.join("\n").to_lowercase();

        for phrase in phrases {
            if output.contains(&phrase.to_lowercase()) {
                return CriterionResult::Met {
                    criterion: format!("Output contains '{}'", phrase),
                    details: "Abort phrase detected".to_string(),
                };
            }
        }

        CriterionResult::NotMet
    }

    fn eval_no_progress(&self, max_iterations: u32, ctx: &CriteriaContext) -> CriterionResult {
        let iterations_since_change = ctx.iteration.saturating_sub(ctx.last_change_iteration);

        if iterations_since_change >= max_iterations {
            CriterionResult::Met {
                criterion: format!("No progress for {} iterations", iterations_since_change),
                details: "No file changes detected".to_string(),
            }
        } else {
            CriterionResult::NotMet
        }
    }

    /// Get a human-readable description
    pub fn description(&self) -> String {
        match self {
            Criterion::PatternMatch { pattern, .. } => format!("Output matches '{}'", pattern),
            Criterion::FileExists { path } => format!("File exists: {}", path.display()),
            Criterion::FileContains { path, pattern } => format!("{} contains '{}'", path.display(), pattern),
            Criterion::CommandSucceeds { command, .. } => format!("Command succeeds: {}", command),
            Criterion::TestsPass => "Tests pass".to_string(),
            Criterion::ErrorCount { pattern, max_count } => format!("Error '{}' > {} times", pattern, max_count),
            Criterion::RepeatedOutput { min_repeats, .. } => format!("Output repeats {} times", min_repeats),
            Criterion::OutputContains { phrases } => format!("Output contains: {:?}", phrases),
            Criterion::NoProgress { iterations } => format!("No progress for {} iterations", iterations),
        }
    }
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
