//! Checkpoint system for task state persistence
//!
//! Checkpoints allow tasks to be resumed after interruption, with
//! the ability to inject guidance and extend iteration limits.

use std::fs;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use ulid::Ulid;

use super::{Checkpoint, Task, TaskId};

/// Checkpoint store for persisting task state
pub struct CheckpointStore {
    /// Directory for checkpoint files
    checkpoint_dir: PathBuf,
}

impl CheckpointStore {
    /// Create a new checkpoint store
    pub fn new(state_dir: PathBuf) -> Result<Self> {
        let checkpoint_dir = state_dir.join("checkpoints");
        fs::create_dir_all(&checkpoint_dir)?;

        Ok(Self { checkpoint_dir })
    }

    /// Save a checkpoint for a task
    pub fn save(&self, task: &Task, context: Option<&str>) -> Result<Checkpoint> {
        let mut checkpoint = Checkpoint::new(task);
        if let Some(ctx) = context {
            checkpoint = checkpoint.with_context(ctx);
        }

        let path = self.checkpoint_path(task.id, checkpoint.checkpoint_id);

        // Ensure task directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write checkpoint as MessagePack
        let file = fs::File::create(&path).context("Failed to create checkpoint file")?;
        let mut writer = BufWriter::new(file);
        let data = rmp_serde::to_vec(&checkpoint)?;
        writer.write_all(&data)?;
        writer.flush()?;

        tracing::info!(
            "Saved checkpoint {} for task {} at iteration {}",
            checkpoint.checkpoint_id,
            task.id,
            task.iterations
        );

        Ok(checkpoint)
    }

    /// Load a checkpoint by ID
    pub fn load(&self, task_id: TaskId, checkpoint_id: Ulid) -> Result<Checkpoint> {
        let path = self.checkpoint_path(task_id, checkpoint_id);

        let file = fs::File::open(&path).context("Checkpoint not found")?;
        let mut reader = BufReader::new(file);
        let mut data = Vec::new();
        reader.read_to_end(&mut data)?;

        let checkpoint: Checkpoint = rmp_serde::from_slice(&data)?;
        Ok(checkpoint)
    }

    /// Load the latest checkpoint for a task
    pub fn load_latest(&self, task_id: TaskId) -> Result<Option<Checkpoint>> {
        let checkpoints = self.list(task_id)?;

        if checkpoints.is_empty() {
            return Ok(None);
        }

        // Checkpoints are sorted by ID (which is ULID, so chronological)
        let latest = checkpoints.last().unwrap();
        let checkpoint = self.load(task_id, *latest)?;
        Ok(Some(checkpoint))
    }

    /// List all checkpoint IDs for a task
    pub fn list(&self, task_id: TaskId) -> Result<Vec<Ulid>> {
        let task_dir = self.task_checkpoint_dir(task_id);

        if !task_dir.exists() {
            return Ok(Vec::new());
        }

        let mut checkpoints = Vec::new();

        for entry in fs::read_dir(&task_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|e| e.to_str()) == Some("checkpoint") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if let Ok(id) = Ulid::from_string(stem) {
                        checkpoints.push(id);
                    }
                }
            }
        }

        checkpoints.sort();
        Ok(checkpoints)
    }

    /// Delete a checkpoint
    pub fn delete(&self, task_id: TaskId, checkpoint_id: Ulid) -> Result<bool> {
        let path = self.checkpoint_path(task_id, checkpoint_id);

        if path.exists() {
            fs::remove_file(&path)?;
            tracing::debug!(
                "Deleted checkpoint {} for task {}",
                checkpoint_id,
                task_id
            );
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Delete all checkpoints for a task
    pub fn delete_all(&self, task_id: TaskId) -> Result<usize> {
        let task_dir = self.task_checkpoint_dir(task_id);

        if !task_dir.exists() {
            return Ok(0);
        }

        let mut count = 0;
        for entry in fs::read_dir(&task_dir)? {
            let entry = entry?;
            fs::remove_file(entry.path())?;
            count += 1;
        }

        // Remove the directory if empty
        let _ = fs::remove_dir(&task_dir);

        tracing::debug!("Deleted {} checkpoints for task {}", count, task_id);
        Ok(count)
    }

    /// Prune old checkpoints, keeping only the N most recent
    pub fn prune(&self, task_id: TaskId, keep: usize) -> Result<usize> {
        let checkpoints = self.list(task_id)?;

        if checkpoints.len() <= keep {
            return Ok(0);
        }

        let to_delete = checkpoints.len() - keep;
        let mut deleted = 0;

        for checkpoint_id in checkpoints.iter().take(to_delete) {
            if self.delete(task_id, *checkpoint_id)? {
                deleted += 1;
            }
        }

        tracing::debug!(
            "Pruned {} old checkpoints for task {}, kept {}",
            deleted,
            task_id,
            keep
        );
        Ok(deleted)
    }

    /// Get the total size of checkpoints for a task
    pub fn size(&self, task_id: TaskId) -> Result<u64> {
        let task_dir = self.task_checkpoint_dir(task_id);

        if !task_dir.exists() {
            return Ok(0);
        }

        let mut total = 0;
        for entry in fs::read_dir(&task_dir)? {
            let entry = entry?;
            total += entry.metadata()?.len();
        }

        Ok(total)
    }

    /// Get detailed info about checkpoints for a task
    pub fn info(&self, task_id: TaskId) -> Result<CheckpointInfo> {
        let checkpoints = self.list(task_id)?;
        let count = checkpoints.len();
        let size = self.size(task_id)?;

        let (oldest, newest) = if !checkpoints.is_empty() {
            (
                Some(checkpoints.first().unwrap().datetime().into()),
                Some(checkpoints.last().unwrap().datetime().into()),
            )
        } else {
            (None, None)
        };

        Ok(CheckpointInfo {
            task_id,
            count,
            size,
            oldest,
            newest,
        })
    }

    /// Get the directory for a task's checkpoints
    fn task_checkpoint_dir(&self, task_id: TaskId) -> PathBuf {
        self.checkpoint_dir.join(task_id.to_string())
    }

    /// Get the path for a specific checkpoint
    fn checkpoint_path(&self, task_id: TaskId, checkpoint_id: Ulid) -> PathBuf {
        self.task_checkpoint_dir(task_id)
            .join(format!("{}.checkpoint", checkpoint_id))
    }
}

/// Information about checkpoints for a task
#[derive(Debug, Clone)]
pub struct CheckpointInfo {
    pub task_id: TaskId,
    pub count: usize,
    pub size: u64,
    pub oldest: Option<chrono::DateTime<chrono::Utc>>,
    pub newest: Option<chrono::DateTime<chrono::Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_store() -> (CheckpointStore, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let store = CheckpointStore::new(dir.path().to_path_buf()).unwrap();
        (store, dir)
    }

    #[test]
    fn test_save_and_load() {
        let (store, _dir) = create_test_store();

        let mut task = Task::new("Test task");
        task.iterations = 5;

        let checkpoint = store.save(&task, Some("Test context")).unwrap();

        let loaded = store.load(task.id, checkpoint.checkpoint_id).unwrap();
        assert_eq!(loaded.task_id, task.id);
        assert_eq!(loaded.iteration, 5);
        assert_eq!(loaded.context, Some("Test context".to_string()));
    }

    #[test]
    fn test_load_latest() {
        let (store, _dir) = create_test_store();

        let mut task = Task::new("Test task");

        // Save multiple checkpoints
        task.iterations = 1;
        store.save(&task, None).unwrap();

        task.iterations = 2;
        store.save(&task, None).unwrap();

        task.iterations = 3;
        store.save(&task, None).unwrap();

        // Latest should be iteration 3
        let latest = store.load_latest(task.id).unwrap().unwrap();
        assert_eq!(latest.iteration, 3);
    }

    #[test]
    fn test_list() {
        let (store, _dir) = create_test_store();

        let mut task = Task::new("Test task");

        // Save multiple checkpoints
        for i in 1..=3 {
            task.iterations = i;
            store.save(&task, None).unwrap();
        }

        let checkpoints = store.list(task.id).unwrap();
        assert_eq!(checkpoints.len(), 3);
    }

    #[test]
    fn test_prune() {
        let (store, _dir) = create_test_store();

        let mut task = Task::new("Test task");

        // Save 5 checkpoints
        for i in 1..=5 {
            task.iterations = i;
            store.save(&task, None).unwrap();
        }

        // Prune to keep only 2
        let pruned = store.prune(task.id, 2).unwrap();
        assert_eq!(pruned, 3);

        let remaining = store.list(task.id).unwrap();
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn test_delete_all() {
        let (store, _dir) = create_test_store();

        let mut task = Task::new("Test task");

        // Save 3 checkpoints
        for i in 1..=3 {
            task.iterations = i;
            store.save(&task, None).unwrap();
        }

        let deleted = store.delete_all(task.id).unwrap();
        assert_eq!(deleted, 3);

        let remaining = store.list(task.id).unwrap();
        assert!(remaining.is_empty());
    }
}
