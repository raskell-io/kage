//! Task registry with persistent storage
//!
//! Stores tasks in redb for crash-safe persistence.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;

use anyhow::{Context, Result};
use redb::{Database, ReadableTable, TableDefinition};

use super::{Task, TaskId, TaskStatus};
use crate::agent::AgentId;

const TASKS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("tasks");

/// Task registry with persistent storage
pub struct TaskRegistry {
    /// In-memory cache
    tasks: RwLock<HashMap<TaskId, Task>>,
    /// Persistent database
    db: Database,
}

impl TaskRegistry {
    /// Open or create a task registry
    pub fn open(path: PathBuf) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let db = Database::create(&path).context("Failed to open task database")?;

        // Create table if it doesn't exist
        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(TASKS_TABLE)?;
        }
        write_txn.commit()?;

        // Load existing tasks
        let tasks = Self::load_all(&db)?;

        Ok(Self {
            tasks: RwLock::new(tasks),
            db,
        })
    }

    /// Load all tasks from database
    fn load_all(db: &Database) -> Result<HashMap<TaskId, Task>> {
        let mut tasks = HashMap::new();
        let read_txn = db.begin_read()?;
        let table = read_txn.open_table(TASKS_TABLE)?;

        for result in table.iter()? {
            let (_, value) = result?;
            let task: Task = rmp_serde::from_slice(value.value())?;
            tasks.insert(task.id, task);
        }

        Ok(tasks)
    }

    /// Persist a task to database
    fn persist(&self, task: &Task) -> Result<()> {
        let data = rmp_serde::to_vec(task)?;
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TASKS_TABLE)?;
            table.insert(task.id.to_string().as_str(), data.as_slice())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    /// Add a new task
    pub fn add(&self, task: Task) -> Result<TaskId> {
        let id = task.id;

        // Persist first
        self.persist(&task)?;

        // Then update cache
        let mut tasks = self.tasks.write().unwrap();
        tasks.insert(id, task);

        tracing::debug!("Task {} added", id);
        Ok(id)
    }

    /// Get a task by ID
    pub fn get(&self, id: TaskId) -> Option<Task> {
        let tasks = self.tasks.read().unwrap();
        tasks.get(&id).cloned()
    }

    /// Update a task
    pub fn update(&self, task: Task) -> Result<()> {
        let id = task.id;

        // Persist first
        self.persist(&task)?;

        // Then update cache
        let mut tasks = self.tasks.write().unwrap();
        tasks.insert(id, task);

        tracing::debug!("Task {} updated", id);
        Ok(())
    }

    /// Remove a task
    pub fn remove(&self, id: TaskId) -> Result<bool> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(TASKS_TABLE)?;
            table.remove(id.to_string().as_str())?;
        }
        write_txn.commit()?;

        let mut tasks = self.tasks.write().unwrap();
        let removed = tasks.remove(&id).is_some();

        if removed {
            tracing::debug!("Task {} removed", id);
        }

        Ok(removed)
    }

    /// List all tasks
    pub fn list(&self) -> Vec<Task> {
        let tasks = self.tasks.read().unwrap();
        let mut list: Vec<_> = tasks.values().cloned().collect();
        // Sort by priority (descending) then created_at (ascending)
        list.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });
        list
    }

    /// List tasks by status
    pub fn list_by_status(&self, status: TaskStatus) -> Vec<Task> {
        let tasks = self.tasks.read().unwrap();
        let mut list: Vec<_> = tasks
            .values()
            .filter(|t| t.status == status)
            .cloned()
            .collect();
        list.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });
        list
    }

    /// List tasks by namespace
    pub fn list_by_namespace(&self, namespace: &str) -> Vec<Task> {
        let tasks = self.tasks.read().unwrap();
        let mut list: Vec<_> = tasks
            .values()
            .filter(|t| t.namespace.as_deref() == Some(namespace))
            .cloned()
            .collect();
        list.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });
        list
    }

    /// Get tasks assigned to an agent
    pub fn get_by_agent(&self, agent_id: AgentId) -> Vec<Task> {
        let tasks = self.tasks.read().unwrap();
        tasks
            .values()
            .filter(|t| t.agent == Some(agent_id))
            .cloned()
            .collect()
    }

    /// Get pending tasks that are ready to run
    pub fn get_ready_tasks(&self) -> Vec<Task> {
        let tasks = self.tasks.read().unwrap();

        // Collect completed task IDs
        let completed: Vec<TaskId> = tasks
            .values()
            .filter(|t| t.status == TaskStatus::Completed)
            .map(|t| t.id)
            .collect();

        // Get pending tasks with all dependencies met
        let mut ready: Vec<_> = tasks
            .values()
            .filter(|t| t.is_ready(&completed))
            .cloned()
            .collect();

        // Sort by priority (descending) then created_at (ascending)
        ready.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });

        ready
    }

    /// Get counts by status
    pub fn counts(&self) -> TaskCounts {
        let tasks = self.tasks.read().unwrap();
        let mut counts = TaskCounts::default();

        for task in tasks.values() {
            match task.status {
                TaskStatus::Pending => counts.pending += 1,
                TaskStatus::Running => counts.running += 1,
                TaskStatus::Paused => counts.paused += 1,
                TaskStatus::Completed => counts.completed += 1,
                TaskStatus::Failed => counts.failed += 1,
                TaskStatus::Cancelled => counts.cancelled += 1,
            }
        }

        counts
    }

    /// Mark a task as running with an agent
    pub fn assign_to_agent(&self, task_id: TaskId, agent_id: AgentId) -> Result<()> {
        let mut tasks = self.tasks.write().unwrap();

        if let Some(task) = tasks.get_mut(&task_id) {
            task.assign(agent_id);
            let task_clone = task.clone();
            drop(tasks);
            self.persist(&task_clone)?;
            tracing::info!("Task {} assigned to agent {}", task_id, agent_id);
            Ok(())
        } else {
            anyhow::bail!("Task {} not found", task_id)
        }
    }

    /// Mark a task as completed
    pub fn complete_task(&self, task_id: TaskId) -> Result<()> {
        let mut tasks = self.tasks.write().unwrap();

        if let Some(task) = tasks.get_mut(&task_id) {
            task.complete();
            let task_clone = task.clone();
            drop(tasks);
            self.persist(&task_clone)?;
            tracing::info!("Task {} completed", task_id);
            Ok(())
        } else {
            anyhow::bail!("Task {} not found", task_id)
        }
    }

    /// Mark a task as failed
    pub fn fail_task(&self, task_id: TaskId, error: Option<&str>) -> Result<()> {
        let mut tasks = self.tasks.write().unwrap();

        if let Some(task) = tasks.get_mut(&task_id) {
            task.fail(error);
            let task_clone = task.clone();
            drop(tasks);
            self.persist(&task_clone)?;
            tracing::info!("Task {} failed: {:?}", task_id, error);
            Ok(())
        } else {
            anyhow::bail!("Task {} not found", task_id)
        }
    }

    /// Pause a task
    pub fn pause_task(&self, task_id: TaskId) -> Result<()> {
        let mut tasks = self.tasks.write().unwrap();

        if let Some(task) = tasks.get_mut(&task_id) {
            task.pause();
            let task_clone = task.clone();
            drop(tasks);
            self.persist(&task_clone)?;
            tracing::info!("Task {} paused", task_id);
            Ok(())
        } else {
            anyhow::bail!("Task {} not found", task_id)
        }
    }

    /// Resume a task
    pub fn resume_task(
        &self,
        task_id: TaskId,
        guidance: Option<&str>,
        extend_iterations: u32,
    ) -> Result<()> {
        let mut tasks = self.tasks.write().unwrap();

        if let Some(task) = tasks.get_mut(&task_id) {
            task.resume(guidance, extend_iterations);
            let task_clone = task.clone();
            drop(tasks);
            self.persist(&task_clone)?;
            tracing::info!(
                "Task {} resumed with {} more iterations",
                task_id,
                extend_iterations
            );
            Ok(())
        } else {
            anyhow::bail!("Task {} not found", task_id)
        }
    }

    /// Cancel a task
    pub fn cancel_task(&self, task_id: TaskId) -> Result<()> {
        let mut tasks = self.tasks.write().unwrap();

        if let Some(task) = tasks.get_mut(&task_id) {
            task.cancel();
            let task_clone = task.clone();
            drop(tasks);
            self.persist(&task_clone)?;
            tracing::info!("Task {} cancelled", task_id);
            Ok(())
        } else {
            anyhow::bail!("Task {} not found", task_id)
        }
    }

    /// Increment iteration count for a task
    pub fn increment_iteration(&self, task_id: TaskId) -> Result<Task> {
        let mut tasks = self.tasks.write().unwrap();

        if let Some(task) = tasks.get_mut(&task_id) {
            task.increment_iteration();
            let task_clone = task.clone();
            drop(tasks);
            self.persist(&task_clone)?;
            Ok(task_clone)
        } else {
            anyhow::bail!("Task {} not found", task_id)
        }
    }
}

/// Task counts by status
#[derive(Debug, Clone, Default)]
pub struct TaskCounts {
    pub pending: usize,
    pub running: usize,
    pub paused: usize,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
}

impl TaskCounts {
    pub fn total(&self) -> usize {
        self.pending + self.running + self.paused + self.completed + self.failed + self.cancelled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_registry() -> (TaskRegistry, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tasks.redb");
        let registry = TaskRegistry::open(path).unwrap();
        (registry, dir)
    }

    #[test]
    fn test_add_and_get() {
        let (registry, _dir) = create_test_registry();

        let task = Task::new("Fix the bug");
        let id = registry.add(task.clone()).unwrap();

        let retrieved = registry.get(id).unwrap();
        assert_eq!(retrieved.goal, "Fix the bug");
        assert_eq!(retrieved.status, TaskStatus::Pending);
    }

    #[test]
    fn test_update() {
        let (registry, _dir) = create_test_registry();

        let task = Task::new("Fix the bug");
        let id = registry.add(task).unwrap();

        let mut task = registry.get(id).unwrap();
        task.priority = 200;
        registry.update(task).unwrap();

        let retrieved = registry.get(id).unwrap();
        assert_eq!(retrieved.priority, 200);
    }

    #[test]
    fn test_list_by_status() {
        let (registry, _dir) = create_test_registry();

        registry.add(Task::new("Task 1")).unwrap();
        let id2 = registry.add(Task::new("Task 2")).unwrap();
        registry.add(Task::new("Task 3")).unwrap();

        // Complete one task
        registry.complete_task(id2).unwrap();

        let pending = registry.list_by_status(TaskStatus::Pending);
        assert_eq!(pending.len(), 2);

        let completed = registry.list_by_status(TaskStatus::Completed);
        assert_eq!(completed.len(), 1);
    }

    #[test]
    fn test_ready_tasks_with_dependencies() {
        let (registry, _dir) = create_test_registry();

        let task1 = Task::new("Task 1");
        let id1 = registry.add(task1).unwrap();

        let task2 = Task::new("Task 2").with_dependency(id1);
        registry.add(task2).unwrap();

        // Initially only task1 is ready
        let ready = registry.get_ready_tasks();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].goal, "Task 1");

        // Complete task1
        registry.complete_task(id1).unwrap();

        // Now task2 is also ready
        let ready = registry.get_ready_tasks();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].goal, "Task 2");
    }

    #[test]
    fn test_priority_ordering() {
        let (registry, _dir) = create_test_registry();

        registry
            .add(Task::new("Low priority").with_priority(50))
            .unwrap();
        registry
            .add(Task::new("High priority").with_priority(200))
            .unwrap();
        registry
            .add(Task::new("Medium priority").with_priority(100))
            .unwrap();

        let tasks = registry.list();
        assert_eq!(tasks[0].goal, "High priority");
        assert_eq!(tasks[1].goal, "Medium priority");
        assert_eq!(tasks[2].goal, "Low priority");
    }
}
