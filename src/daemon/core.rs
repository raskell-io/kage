//! Core request handlers (transport-agnostic)
//!
//! This module provides the business logic for handling daemon requests.
//! Both Unix socket and gRPC transports use these handlers.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Result};
use tokio::sync::RwLock;

use crate::agent::AgentId;
use crate::memory::{MemoryEntry, MemoryQuery, MemoryScope, MemorySystem, PersistentContextRefStore};
use crate::task::{ApprovalId, Task, TaskConfig, TaskId, TaskScheduler, TaskStatus};

use super::protocol::{AgentInfo, ApprovalInfo, MemoryInfo, OutputLine, TaskInfo};
use super::Supervisor;

/// Shared handler state for all transports
pub struct HandlerState {
    pub supervisor: Arc<RwLock<Supervisor>>,
    pub scheduler: Arc<TaskScheduler>,
    pub memory: Arc<MemorySystem>,
    pub refs: Arc<RwLock<PersistentContextRefStore>>,
    pub started_at: Instant,
}

impl HandlerState {
    /// Create a new handler state
    pub fn new(
        supervisor: Arc<RwLock<Supervisor>>,
        scheduler: Arc<TaskScheduler>,
        memory: Arc<MemorySystem>,
        refs: Arc<RwLock<PersistentContextRefStore>>,
        started_at: Instant,
    ) -> Self {
        Self {
            supervisor,
            scheduler,
            memory,
            refs,
            started_at,
        }
    }
}

// ============================================================================
// System handlers
// ============================================================================

/// Handle ping request
pub fn handle_ping(state: &HandlerState) -> (String, u64) {
    (
        env!("CARGO_PKG_VERSION").to_string(),
        state.started_at.elapsed().as_secs(),
    )
}

/// Handle status request
pub async fn handle_status(state: &HandlerState) -> (String, u64, usize, usize, usize) {
    let sup = state.supervisor.read().await;
    let agents = sup.list_all().await;
    let active = agents.iter().filter(|a| a.status == "running").count();

    let counts = state.scheduler.registry().counts();

    (
        env!("CARGO_PKG_VERSION").to_string(),
        state.started_at.elapsed().as_secs(),
        active,
        counts.pending,
        counts.running,
    )
}

// ============================================================================
// Agent handlers
// ============================================================================

/// Handle spawn agent request
pub async fn handle_spawn_agent(
    state: &HandlerState,
    working_dir: std::path::PathBuf,
    namespace: Option<String>,
    prompt: Option<String>,
    model: Option<String>,
    max_iterations: Option<u32>,
    pty_rows: Option<u16>,
    pty_cols: Option<u16>,
) -> Result<AgentId> {
    let mut sup = state.supervisor.write().await;
    sup.spawn(working_dir, namespace, prompt, model, max_iterations, pty_rows, pty_cols)
        .await
}

/// Handle kill agent request
pub async fn handle_kill_agent(state: &HandlerState, id: AgentId, force: bool) -> Result<()> {
    let mut sup = state.supervisor.write().await;
    sup.kill(id, force).await
}

/// Handle list agents request
pub async fn handle_list_agents(
    state: &HandlerState,
    namespace: Option<String>,
    include_stopped: bool,
) -> Vec<AgentInfo> {
    let sup = state.supervisor.read().await;
    sup.list(namespace, include_stopped).await
}

/// Handle get agent request
pub async fn handle_get_agent(state: &HandlerState, id: AgentId) -> Option<AgentInfo> {
    let sup = state.supervisor.read().await;
    sup.get_info(id).await
}

/// Handle pause agent request
pub async fn handle_pause_agent(state: &HandlerState, id: AgentId) -> Result<()> {
    let mut sup = state.supervisor.write().await;
    sup.pause(id).await
}

/// Handle resume agent request
pub async fn handle_resume_agent(state: &HandlerState, id: AgentId) -> Result<()> {
    let mut sup = state.supervisor.write().await;
    sup.resume(id).await
}

// ============================================================================
// Agent I/O handlers
// ============================================================================

/// Handle send input request
pub async fn handle_send_input(state: &HandlerState, id: AgentId, input: &str) -> Result<()> {
    let sup = state.supervisor.read().await;
    sup.send_input(id, input).await
}

/// Handle get output request
pub async fn handle_get_output(
    state: &HandlerState,
    id: AgentId,
    lines: usize,
) -> Result<(Vec<OutputLine>, bool)> {
    let sup = state.supervisor.read().await;
    sup.get_output(id, lines).await
}

/// Handle attach request - returns a broadcast receiver for streaming
pub async fn handle_attach(
    state: &HandlerState,
    id: AgentId,
) -> Result<tokio::sync::broadcast::Receiver<OutputLine>> {
    let sup = state.supervisor.read().await;

    // First verify agent exists
    if sup.get_info(id).await.is_none() {
        return Err(anyhow!("Agent {} not found", id));
    }

    sup.attach(id).await
}

// ============================================================================
// Task handlers
// ============================================================================

/// Handle add task request
pub fn handle_add_task(
    state: &HandlerState,
    goal: String,
    namespace: Option<String>,
    repository: Option<std::path::PathBuf>,
    max_iterations: Option<u32>,
) -> Result<TaskId> {
    let config = TaskConfig {
        max_iterations: max_iterations.unwrap_or(10),
        ..TaskConfig::default()
    };

    let mut task = Task::new(&goal).with_config(config);

    if let Some(ns) = namespace {
        task = task.with_namespace(&ns);
    }

    if let Some(repo) = repository {
        task = task.with_repository(repo);
    }

    let id = state.scheduler.registry().add(task)?;
    tracing::info!("Task {} added: {}", id, goal);
    Ok(id)
}

/// Handle list tasks request
pub fn handle_list_tasks(state: &HandlerState, status: Option<String>) -> Result<Vec<TaskInfo>> {
    let tasks = if let Some(status_str) = status {
        let task_status = match status_str.to_lowercase().as_str() {
            "pending" => TaskStatus::Pending,
            "running" => TaskStatus::Running,
            "paused" => TaskStatus::Paused,
            "completed" => TaskStatus::Completed,
            "failed" => TaskStatus::Failed,
            "cancelled" => TaskStatus::Cancelled,
            _ => return Err(anyhow!("Invalid status: {}", status_str)),
        };
        state.scheduler.registry().list_by_status(task_status)
    } else {
        state.scheduler.registry().list()
    };

    let task_infos: Vec<TaskInfo> = tasks
        .into_iter()
        .map(|t| TaskInfo {
            id: t.id,
            goal: t.goal,
            status: format!("{:?}", t.status).to_lowercase(),
            agent: t.agent,
            namespace: t.namespace,
            iterations: t.iterations,
            max_iterations: t.config.max_iterations,
            created_at: t.created_at.timestamp(),
        })
        .collect();

    Ok(task_infos)
}

/// Handle cancel task request
pub fn handle_cancel_task(state: &HandlerState, id: TaskId) -> Result<()> {
    state.scheduler.registry().cancel_task(id)?;
    tracing::info!("Task {} cancelled", id);
    Ok(())
}

// ============================================================================
// Approval handlers
// ============================================================================

/// Handle list approvals request
pub async fn handle_list_approvals(state: &HandlerState) -> Vec<ApprovalInfo> {
    let sup = state.supervisor.read().await;
    sup.list_approvals()
}

/// Handle approve request
pub async fn handle_approve(state: &HandlerState, id: ApprovalId) -> Result<AgentId> {
    let mut sup = state.supervisor.write().await;
    let agent_id = sup.approve(id)?;
    // Resume the agent after approval
    if let Err(e) = sup.resume(agent_id).await {
        tracing::warn!("Failed to resume agent {} after approval: {}", agent_id, e);
    }
    tracing::info!("Approved action {} for agent {}", id, agent_id);
    Ok(agent_id)
}

/// Handle reject request
pub async fn handle_reject(
    state: &HandlerState,
    id: ApprovalId,
    reason: Option<String>,
) -> Result<AgentId> {
    let mut sup = state.supervisor.write().await;
    let agent_id = sup.reject(id, reason)?;
    tracing::info!("Rejected action {} for agent {}", id, agent_id);
    Ok(agent_id)
}

// ============================================================================
// Memory handlers
// ============================================================================

/// Handle query memory request
pub async fn handle_query_memory(
    state: &HandlerState,
    text: Option<String>,
    scope: Option<String>,
    memory_type: Option<String>,
    tags: Vec<String>,
    since: Option<i64>,
    limit: Option<usize>,
) -> (Vec<MemoryInfo>, usize) {
    let mut query = MemoryQuery::new();

    if let Some(t) = text {
        query = query.text(&t);
    }
    if let Some(s) = scope {
        if let Some(parsed_scope) = parse_scope_string(&s) {
            query = query.scope(parsed_scope);
        }
    }
    if let Some(mt) = memory_type {
        query = query.memory_type(&mt);
    }
    for tag in tags {
        query = query.tag(&tag);
    }
    if let Some(s) = since {
        if let Some(dt) = chrono::DateTime::from_timestamp(s, 0) {
            query = query.since(dt);
        }
    }
    if let Some(l) = limit {
        query = query.limit(l);
    }

    let entries = state.memory.query(query).await;
    let total = entries.len();

    let infos: Vec<MemoryInfo> = entries
        .into_iter()
        .map(|e| memory_entry_to_info(&e))
        .collect();

    (infos, total)
}

/// Handle get memory request
pub async fn handle_get_memory(state: &HandlerState, id: String) -> Result<MemoryInfo> {
    let query = MemoryQuery::new().text(&id).limit(1);
    let entries = state.memory.query(query).await;

    if let Some(entry) = entries.first() {
        Ok(memory_entry_to_info(entry))
    } else {
        Err(anyhow!("Memory entry {} not found", id))
    }
}

/// Handle store memory request
pub async fn handle_store_memory(
    state: &HandlerState,
    entry: MemoryEntry,
    scope: MemoryScope,
) -> Result<String> {
    let id = entry.id.to_string();
    state.memory.store(entry, scope).await?;
    Ok(id)
}

/// Handle prune memory request
pub fn handle_prune_memory(
    state: &HandlerState,
    older_than_days: u32,
    dry_run: bool,
) -> Result<(usize, u64)> {
    state.memory.longterm.prune(older_than_days, dry_run)
}

// ============================================================================
// Helper functions
// ============================================================================

/// Parse a scope string like "global", "namespace:backend", "agent:xxx"
fn parse_scope_string(s: &str) -> Option<MemoryScope> {
    if s == "global" {
        return Some(MemoryScope::Global);
    }

    if let Some(ns) = s.strip_prefix("namespace:") {
        return Some(MemoryScope::Namespace(ns.to_string()));
    }

    if let Some(agent_str) = s.strip_prefix("agent:") {
        if let Ok(agent_id) = agent_str.parse() {
            return Some(MemoryScope::Agent(agent_id));
        }
    }

    None
}

/// Convert a MemoryEntry to MemoryInfo for wire format
fn memory_entry_to_info(entry: &MemoryEntry) -> MemoryInfo {
    use crate::memory::entry::MemoryContent;

    let (content_type, content_summary) = match &entry.content {
        MemoryContent::FileDiscovered { path, summary, .. } => (
            "file_discovered".to_string(),
            format!("{}: {}", path.display(), summary),
        ),
        MemoryContent::PatternLearned {
            pattern,
            confidence,
            ..
        } => (
            "pattern_learned".to_string(),
            format!("{} (confidence: {:.0}%)", pattern, confidence * 100.0),
        ),
        MemoryContent::DependencyMapped {
            from,
            to,
            relationship,
        } => (
            "dependency_mapped".to_string(),
            format!("{} {} {}", from, relationship, to),
        ),
        MemoryContent::ErrorEncountered { error, worked, .. } => {
            let status = if *worked { "resolved" } else { "unresolved" };
            (
                "error_encountered".to_string(),
                format!("[{}] {}", status, error),
            )
        }
        MemoryContent::DecisionMade { decision, .. } => {
            ("decision_made".to_string(), decision.clone())
        }
        MemoryContent::TaskCompleted { task_id, summary, .. } => (
            "task_completed".to_string(),
            format!("{}: {}", task_id, summary),
        ),
        MemoryContent::InsightShared { topic, content } => (
            "insight_shared".to_string(),
            format!("{}: {}", topic, content),
        ),
        MemoryContent::QuestionAsked { question, answer } => {
            let status = if answer.is_some() {
                "answered"
            } else {
                "unanswered"
            };
            (
                "question_asked".to_string(),
                format!("[{}] {}", status, question),
            )
        }
    };

    // Serialize full content to JSON for storage
    let content_json = serde_json::to_string(&entry.content).unwrap_or_default();

    MemoryInfo {
        id: entry.id.to_string(),
        created_at: entry.created_at.timestamp(),
        created_by: entry.created_by.to_string(),
        content_type,
        content_summary,
        content: content_json,
        tags: entry.tags.clone(),
        scope: String::new(), // Will be set by caller if needed
    }
}
