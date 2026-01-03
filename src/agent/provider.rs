//! Agent provider trait definition

use std::path::PathBuf;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::AgentHandle;

/// Agent status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentStatus {
    /// Agent is starting up
    Starting,
    /// Agent is running and processing
    Running,
    /// Agent is waiting for input
    Waiting,
    /// Agent is idle
    Idle,
    /// Agent has stopped
    Stopped,
    /// Agent encountered an error
    Error,
}

/// Configuration for spawning an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Working directory (repository path)
    pub working_dir: PathBuf,

    /// Namespace this agent belongs to
    pub namespace: Option<String>,

    /// Initial prompt/goal
    pub initial_prompt: Option<String>,

    /// Model to use (opus, sonnet, haiku)
    pub model: Option<String>,

    /// Additional CLI flags
    pub flags: Vec<String>,

    /// Maximum iterations before checkpoint
    pub max_iterations: u32,

    /// Approval level (none, on-write, on-commit, always)
    pub approval: String,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            working_dir: PathBuf::from("."),
            namespace: None,
            initial_prompt: None,
            model: None,
            flags: Vec::new(),
            max_iterations: 10,
            approval: "on-commit".to_string(),
        }
    }
}

/// Trait for agent providers
///
/// Implement this trait to add support for new AI agents.
/// Claude Code is the primary implementation.
#[async_trait]
pub trait AgentProvider: Send + Sync {
    /// Provider name (e.g., "claude-code")
    fn name(&self) -> &str;

    /// Spawn a new agent instance
    async fn spawn(&self, config: AgentConfig) -> Result<AgentHandle>;

    /// Detect agent status from output
    fn detect_status(&self, output: &str) -> AgentStatus;

    /// Check if this provider supports MCP
    fn supports_mcp(&self) -> bool;

    /// Get the command to run the agent
    fn command(&self) -> &str;

    /// Get default arguments
    fn default_args(&self) -> Vec<String>;
}
