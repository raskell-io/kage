//! Task scheduler for assigning tasks to agents
//!
//! The scheduler manages the task queue, handles dependencies,
//! and assigns tasks to available agents based on priority and
//! namespace affinity.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use super::registry::TaskRegistry;
use super::{Task, TaskId, TaskStatus};
use crate::agent::AgentId;

/// Scheduling strategy
#[derive(Debug, Clone, Copy, Default)]
pub enum SchedulingStrategy {
    /// First-in-first-out (respecting dependencies)
    #[default]
    Fifo,
    /// Highest priority first
    Priority,
    /// Fair scheduling across namespaces
    FairShare,
}

/// Task scheduler
pub struct TaskScheduler {
    /// Task registry
    registry: Arc<TaskRegistry>,
    /// Agent availability
    available_agents: RwLock<Vec<AgentId>>,
    /// Agent to namespace affinity
    agent_affinity: RwLock<HashMap<AgentId, String>>,
    /// Scheduling strategy
    strategy: SchedulingStrategy,
    /// Maximum concurrent tasks per namespace
    max_concurrent_per_namespace: usize,
}

impl TaskScheduler {
    /// Create a new scheduler
    pub fn new(registry: Arc<TaskRegistry>) -> Self {
        Self {
            registry,
            available_agents: RwLock::new(Vec::new()),
            agent_affinity: RwLock::new(HashMap::new()),
            strategy: SchedulingStrategy::default(),
            max_concurrent_per_namespace: 5,
        }
    }

    /// Set scheduling strategy
    pub fn with_strategy(mut self, strategy: SchedulingStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Set max concurrent tasks per namespace
    pub fn with_max_concurrent(mut self, max: usize) -> Self {
        self.max_concurrent_per_namespace = max;
        self
    }

    /// Register an agent as available
    pub async fn register_agent(&self, agent_id: AgentId, namespace: Option<&str>) {
        let mut agents = self.available_agents.write().await;
        if !agents.contains(&agent_id) {
            agents.push(agent_id);
        }

        if let Some(ns) = namespace {
            let mut affinity = self.agent_affinity.write().await;
            affinity.insert(agent_id, ns.to_string());
        }

        tracing::debug!("Agent {} registered as available", agent_id);
    }

    /// Unregister an agent
    pub async fn unregister_agent(&self, agent_id: AgentId) {
        let mut agents = self.available_agents.write().await;
        agents.retain(|&id| id != agent_id);

        let mut affinity = self.agent_affinity.write().await;
        affinity.remove(&agent_id);

        tracing::debug!("Agent {} unregistered", agent_id);
    }

    /// Mark an agent as busy (working on a task)
    pub async fn mark_agent_busy(&self, agent_id: AgentId) {
        let mut agents = self.available_agents.write().await;
        agents.retain(|&id| id != agent_id);
    }

    /// Mark an agent as available (done with task)
    pub async fn mark_agent_available(&self, agent_id: AgentId) {
        let mut agents = self.available_agents.write().await;
        if !agents.contains(&agent_id) {
            agents.push(agent_id);
        }
    }

    /// Get next task to assign
    pub async fn next_task(&self) -> Option<Task> {
        let ready_tasks = self.registry.get_ready_tasks();

        if ready_tasks.is_empty() {
            return None;
        }

        // Check namespace concurrency limits
        let running_tasks = self.registry.list_by_status(TaskStatus::Running);
        let mut namespace_counts: HashMap<Option<String>, usize> = HashMap::new();

        for task in &running_tasks {
            *namespace_counts.entry(task.namespace.clone()).or_insert(0) += 1;
        }

        // Find first task that doesn't exceed namespace limit
        for task in ready_tasks {
            let count = namespace_counts
                .get(&task.namespace)
                .copied()
                .unwrap_or(0);

            if count < self.max_concurrent_per_namespace {
                return Some(task);
            }
        }

        None
    }

    /// Try to assign a task to an available agent
    pub async fn try_assign(&self) -> Option<(Task, AgentId)> {
        // Check if we have available agents
        let agents = self.available_agents.read().await;
        if agents.is_empty() {
            return None;
        }
        drop(agents);

        // Get next task
        let task = self.next_task().await?;

        // Find best agent for this task
        let agent_id = self.select_agent_for_task(&task).await?;

        // Assign task to agent
        if let Err(e) = self.registry.assign_to_agent(task.id, agent_id) {
            tracing::error!("Failed to assign task {} to agent {}: {}", task.id, agent_id, e);
            return None;
        }

        // Mark agent as busy
        self.mark_agent_busy(agent_id).await;

        tracing::info!("Assigned task {} to agent {}", task.id, agent_id);
        Some((task, agent_id))
    }

    /// Select the best agent for a task
    async fn select_agent_for_task(&self, task: &Task) -> Option<AgentId> {
        let agents = self.available_agents.read().await;
        if agents.is_empty() {
            return None;
        }

        // If task has a namespace, prefer agents with affinity
        if let Some(ref namespace) = task.namespace {
            let affinity = self.agent_affinity.read().await;

            for &agent_id in agents.iter() {
                if affinity.get(&agent_id) == Some(namespace) {
                    return Some(agent_id);
                }
            }
        }

        // Otherwise, pick the first available
        agents.first().copied()
    }

    /// Run the scheduler loop
    ///
    /// This should be called in a background task
    pub async fn run(&self, mut shutdown_rx: tokio::sync::broadcast::Receiver<()>) {
        tracing::info!("Task scheduler started");

        loop {
            tokio::select! {
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {
                    // Try to assign pending tasks
                    while let Some((task, agent_id)) = self.try_assign().await {
                        tracing::info!(
                            "Scheduler assigned task '{}' (id: {}) to agent {}",
                            task.goal,
                            task.id,
                            agent_id
                        );
                    }
                }
                _ = shutdown_rx.recv() => {
                    tracing::info!("Task scheduler shutting down");
                    break;
                }
            }
        }
    }

    /// Get scheduler status
    pub fn status(&self) -> SchedulerStatus {
        let counts = self.registry.counts();
        SchedulerStatus {
            pending_tasks: counts.pending,
            running_tasks: counts.running,
            paused_tasks: counts.paused,
            completed_tasks: counts.completed,
            failed_tasks: counts.failed,
        }
    }

    /// Get the task registry
    pub fn registry(&self) -> &Arc<TaskRegistry> {
        &self.registry
    }
}

/// Scheduler status summary
#[derive(Debug, Clone)]
pub struct SchedulerStatus {
    pub pending_tasks: usize,
    pub running_tasks: usize,
    pub paused_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
}

/// Events emitted by the scheduler
#[derive(Debug, Clone)]
pub enum SchedulerEvent {
    /// Task was assigned to an agent
    TaskAssigned { task_id: TaskId, agent_id: AgentId },
    /// Task completed
    TaskCompleted { task_id: TaskId },
    /// Task failed
    TaskFailed { task_id: TaskId, error: String },
    /// Task paused (iteration limit, needs approval)
    TaskPaused { task_id: TaskId, reason: PauseReason },
    /// No tasks available
    QueueEmpty,
}

/// Reason for task pause
#[derive(Debug, Clone)]
pub enum PauseReason {
    /// Iteration limit reached
    IterationLimit,
    /// Needs approval
    AwaitingApproval,
    /// Manual pause
    Manual,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_scheduler() -> (TaskScheduler, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tasks.redb");
        let registry = Arc::new(TaskRegistry::open(path).unwrap());
        let scheduler = TaskScheduler::new(registry);
        (scheduler, dir)
    }

    #[tokio::test]
    async fn test_register_agent() {
        let (scheduler, _dir) = create_test_scheduler();

        let agent_id = AgentId::new();
        scheduler.register_agent(agent_id, Some("backend")).await;

        let agents = scheduler.available_agents.read().await;
        assert!(agents.contains(&agent_id));
    }

    #[tokio::test]
    async fn test_next_task() {
        let (scheduler, _dir) = create_test_scheduler();

        // No tasks initially
        assert!(scheduler.next_task().await.is_none());

        // Add a task
        let task = Task::new("Test task");
        scheduler.registry.add(task).unwrap();

        // Now we should get the task
        let next = scheduler.next_task().await;
        assert!(next.is_some());
        assert_eq!(next.unwrap().goal, "Test task");
    }

    #[tokio::test]
    async fn test_try_assign() {
        let (scheduler, _dir) = create_test_scheduler();

        // Add task and agent
        let task = Task::new("Test task");
        scheduler.registry.add(task).unwrap();

        let agent_id = AgentId::new();
        scheduler.register_agent(agent_id, None).await;

        // Should assign
        let result = scheduler.try_assign().await;
        assert!(result.is_some());

        let (assigned_task, assigned_agent) = result.unwrap();
        assert_eq!(assigned_task.goal, "Test task");
        assert_eq!(assigned_agent, agent_id);

        // Agent should be marked busy
        let agents = scheduler.available_agents.read().await;
        assert!(!agents.contains(&agent_id));
    }

    #[tokio::test]
    async fn test_namespace_affinity() {
        let (scheduler, _dir) = create_test_scheduler();

        // Add backend and frontend tasks
        let backend_task = Task::new("Backend task").with_namespace("backend");
        scheduler.registry.add(backend_task).unwrap();

        // Add agents with different affinities
        let backend_agent = AgentId::new();
        let frontend_agent = AgentId::new();

        scheduler
            .register_agent(frontend_agent, Some("frontend"))
            .await;
        scheduler
            .register_agent(backend_agent, Some("backend"))
            .await;

        // Backend task should go to backend agent
        let result = scheduler.try_assign().await;
        assert!(result.is_some());

        let (_, assigned_agent) = result.unwrap();
        assert_eq!(assigned_agent, backend_agent);
    }
}
