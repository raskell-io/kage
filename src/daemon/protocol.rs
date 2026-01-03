//! IPC Protocol for daemon communication
//!
//! Uses a simple length-prefixed MessagePack protocol over Unix sockets.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::agent::AgentId;
use crate::memory::{MemoryEntry, MemoryScope};
use crate::task::{ApprovalAction, ApprovalId, TaskId};

/// Request from client to daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Ping the daemon
    Ping,

    /// Get daemon status
    Status,

    /// Spawn a new agent
    SpawnAgent {
        /// Working directory for the agent
        working_dir: PathBuf,
        /// Namespace (optional)
        namespace: Option<String>,
        /// Initial prompt/goal (optional)
        prompt: Option<String>,
        /// Model to use (optional, defaults to config)
        model: Option<String>,
        /// Maximum iterations (optional, defaults to config)
        max_iterations: Option<u32>,
    },

    /// Kill an agent
    KillAgent {
        /// Agent ID to kill
        id: AgentId,
        /// Force kill without graceful shutdown
        force: bool,
    },

    /// List all agents
    ListAgents {
        /// Filter by namespace
        namespace: Option<String>,
        /// Include stopped agents
        include_stopped: bool,
    },

    /// Get agent details
    GetAgent {
        /// Agent ID
        id: AgentId,
    },

    /// Send input to an agent
    SendInput {
        /// Agent ID
        id: AgentId,
        /// Input text
        input: String,
    },

    /// Get agent output
    GetOutput {
        /// Agent ID
        id: AgentId,
        /// Number of lines (0 = all available)
        lines: usize,
    },

    /// Attach to agent (stream output)
    Attach {
        /// Agent ID
        id: AgentId,
    },

    /// Detach from agent
    Detach {
        /// Agent ID
        id: AgentId,
    },

    /// Pause an agent
    PauseAgent {
        /// Agent ID
        id: AgentId,
    },

    /// Resume an agent
    ResumeAgent {
        /// Agent ID
        id: AgentId,
    },

    /// Add a task
    AddTask {
        /// Task goal
        goal: String,
        /// Namespace
        namespace: Option<String>,
        /// Repository path
        repository: Option<PathBuf>,
        /// Max iterations
        max_iterations: Option<u32>,
    },

    /// List tasks
    ListTasks {
        /// Filter by status
        status: Option<String>,
    },

    /// Cancel a task
    CancelTask {
        /// Task ID
        id: TaskId,
    },

    /// List pending approvals
    ListApprovals,

    /// Approve an action
    Approve {
        /// Approval ID
        id: ApprovalId,
    },

    /// Reject an action
    Reject {
        /// Approval ID
        id: ApprovalId,
        /// Reason for rejection (optional)
        reason: Option<String>,
    },

    /// Query memory entries
    QueryMemory {
        /// Text search (optional)
        text: Option<String>,
        /// Scope filter (optional)
        scope: Option<String>,
        /// Memory type filter (optional)
        memory_type: Option<String>,
        /// Tag filters
        tags: Vec<String>,
        /// Since timestamp (optional)
        since: Option<i64>,
        /// Result limit (optional)
        limit: Option<usize>,
    },

    /// Get a specific memory entry
    GetMemory {
        /// Memory ID
        id: String,
    },

    /// Store a memory entry
    StoreMemory {
        /// Memory content
        entry: MemoryEntry,
        /// Scope
        scope: MemoryScope,
    },

    /// Prune old memory entries
    PruneMemory {
        /// Delete entries older than this many days
        older_than_days: u32,
        /// Dry run (don't actually delete)
        dry_run: bool,
    },

    /// Shutdown the daemon
    Shutdown,

    /// Subscribe to daemon events (real-time streaming)
    Subscribe {
        /// Event types to subscribe to (empty = all)
        event_types: Vec<String>,
    },

    /// Unsubscribe from daemon events
    Unsubscribe,
}

/// Response from daemon to client
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Success with no data
    Ok,

    /// Error response
    Error {
        /// Error message
        message: String,
    },

    /// Pong response
    Pong {
        /// Daemon version
        version: String,
        /// Uptime in seconds
        uptime_secs: u64,
    },

    /// Daemon status
    Status {
        /// Daemon version
        version: String,
        /// Uptime in seconds
        uptime_secs: u64,
        /// Number of active agents
        active_agents: usize,
        /// Number of pending tasks
        pending_tasks: usize,
        /// Number of running tasks
        running_tasks: usize,
    },

    /// Agent spawned
    AgentSpawned {
        /// New agent ID
        id: AgentId,
    },

    /// Agent list
    AgentList {
        /// List of agents
        agents: Vec<AgentInfo>,
    },

    /// Agent details
    AgentDetails {
        /// Agent info
        agent: AgentInfo,
    },

    /// Agent output
    AgentOutput {
        /// Output lines
        lines: Vec<OutputLine>,
        /// Whether there's more output
        has_more: bool,
    },

    /// Task added
    TaskAdded {
        /// New task ID
        id: TaskId,
    },

    /// Task list
    TaskList {
        /// List of tasks
        tasks: Vec<TaskInfo>,
    },

    /// Streaming output line (for attach)
    StreamLine {
        /// Output text
        text: String,
        /// Whether this is stderr
        is_error: bool,
        /// Timestamp
        timestamp: i64,
    },

    /// Stream ended
    StreamEnd,

    /// Approval list
    ApprovalList {
        /// List of pending approvals
        approvals: Vec<ApprovalInfo>,
    },

    /// Memory query results
    MemoryList {
        /// List of memory entries
        entries: Vec<MemoryInfo>,
        /// Total count (may be more than returned)
        total: usize,
    },

    /// Memory entry details
    MemoryDetails {
        /// Memory entry
        entry: MemoryInfo,
    },

    /// Memory entry stored
    MemoryStored {
        /// Memory ID
        id: String,
    },

    /// Memory pruned
    MemoryPruned {
        /// Number of entries deleted
        count: usize,
        /// Bytes freed
        bytes_freed: u64,
    },

    /// Subscription confirmed
    Subscribed,

    /// Daemon event (streamed to subscribers)
    Event(DaemonEvent),
}

/// Daemon event for real-time streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum DaemonEvent {
    /// Agent spawned
    AgentSpawned {
        agent: AgentInfo,
    },

    /// Agent status changed
    AgentStatusChanged {
        id: AgentId,
        old_status: String,
        new_status: String,
    },

    /// Agent killed/stopped
    AgentStopped {
        id: AgentId,
        reason: String,
    },

    /// Agent output line
    AgentOutput {
        id: AgentId,
        line: OutputLine,
    },

    /// Task added
    TaskAdded {
        task: TaskInfo,
    },

    /// Task status changed
    TaskStatusChanged {
        id: TaskId,
        old_status: String,
        new_status: String,
    },

    /// Task completed
    TaskCompleted {
        id: TaskId,
        success: bool,
        message: Option<String>,
    },

    /// Approval created
    ApprovalCreated {
        approval: ApprovalInfo,
    },

    /// Approval resolved
    ApprovalResolved {
        id: ApprovalId,
        approved: bool,
    },

    /// Connection heartbeat
    Heartbeat {
        timestamp: i64,
    },
}

/// Agent information for responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    /// Agent ID
    pub id: AgentId,
    /// Display name
    pub name: String,
    /// Current status
    pub status: String,
    /// Working directory
    pub working_dir: PathBuf,
    /// Namespace
    pub namespace: Option<String>,
    /// Current iteration
    pub iteration: u32,
    /// Max iterations
    pub max_iterations: u32,
    /// Started at (unix timestamp)
    pub started_at: i64,
    /// PID of the process
    pub pid: Option<u32>,
}

/// Output line
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputLine {
    /// Line text
    pub text: String,
    /// Whether this is stderr
    pub is_error: bool,
    /// Timestamp (unix timestamp)
    pub timestamp: i64,
}

/// Approval information for responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalInfo {
    /// Approval ID
    pub id: ApprovalId,
    /// Agent ID
    pub agent_id: AgentId,
    /// Task ID (if associated)
    pub task_id: Option<TaskId>,
    /// Action type
    pub action: ApprovalAction,
    /// Action summary
    pub summary: String,
    /// Created at (unix timestamp)
    pub created_at: i64,
    /// Context lines
    pub context: Vec<String>,
}

/// Task information for responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    /// Task ID
    pub id: TaskId,
    /// Goal
    pub goal: String,
    /// Status
    pub status: String,
    /// Assigned agent
    pub agent: Option<AgentId>,
    /// Namespace
    pub namespace: Option<String>,
    /// Iterations completed
    pub iterations: u32,
    /// Max iterations
    pub max_iterations: u32,
    /// Created at (unix timestamp)
    pub created_at: i64,
}

/// Memory entry information for responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryInfo {
    /// Memory ID
    pub id: String,
    /// Created at (unix timestamp)
    pub created_at: i64,
    /// Created by agent ID
    pub created_by: String,
    /// Content type (e.g., "file_discovered", "error_encountered")
    pub content_type: String,
    /// Content summary
    pub content_summary: String,
    /// Full content (serialized)
    pub content: String,
    /// Tags
    pub tags: Vec<String>,
    /// Scope (e.g., "agent:xxx", "namespace:backend", "global")
    pub scope: String,
}

/// Encode a message with length prefix
pub fn encode_message<T: Serialize>(msg: &T) -> anyhow::Result<Vec<u8>> {
    let data = rmp_serde::to_vec(msg)?;
    let len = (data.len() as u32).to_be_bytes();
    let mut buf = Vec::with_capacity(4 + data.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&data);
    Ok(buf)
}

/// Decode a message from bytes (without length prefix)
pub fn decode_message<T: for<'de> Deserialize<'de>>(data: &[u8]) -> anyhow::Result<T> {
    Ok(rmp_serde::from_slice(data)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_request() {
        let req = Request::Ping;
        let encoded = encode_message(&req).unwrap();

        // Skip length prefix
        let decoded: Request = decode_message(&encoded[4..]).unwrap();

        match decoded {
            Request::Ping => {}
            _ => panic!("Wrong request type"),
        }
    }

    #[test]
    fn test_encode_decode_response() {
        let resp = Response::Pong {
            version: "0.1.0".into(),
            uptime_secs: 100,
        };
        let encoded = encode_message(&resp).unwrap();

        // Skip length prefix
        let decoded: Response = decode_message(&encoded[4..]).unwrap();

        match decoded {
            Response::Pong { version, uptime_secs } => {
                assert_eq!(version, "0.1.0");
                assert_eq!(uptime_secs, 100);
            }
            _ => panic!("Wrong response type"),
        }
    }
}
