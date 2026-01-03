//! Claude Code provider - primary agent implementation

use anyhow::Result;
use async_trait::async_trait;

use crate::agent::{AgentConfig, AgentHandle, AgentId, AgentProvider, AgentStatus};

/// Claude Code agent provider
///
/// This is the primary, first-class agent provider for Kage.
pub struct ClaudeCodeProvider {
    /// Default model to use
    pub default_model: String,

    /// Default CLI flags
    pub default_flags: Vec<String>,
}

impl Default for ClaudeCodeProvider {
    fn default() -> Self {
        Self {
            default_model: "sonnet".to_string(),
            default_flags: vec![],
        }
    }
}

impl ClaudeCodeProvider {
    /// Create a new Claude Code provider
    pub fn new() -> Self {
        Self::default()
    }

    /// Create with custom defaults
    pub fn with_model(model: &str) -> Self {
        Self {
            default_model: model.to_string(),
            ..Self::default()
        }
    }
}

#[async_trait]
impl AgentProvider for ClaudeCodeProvider {
    fn name(&self) -> &str {
        "claude-code"
    }

    async fn spawn(&self, config: AgentConfig) -> Result<AgentHandle> {
        let id = AgentId::new();
        tracing::info!(
            "Spawning Claude Code agent {} in {:?}",
            id,
            config.working_dir
        );

        // TODO: Implement actual spawning with PTY
        // 1. Allocate PTY
        // 2. Spawn `claude` process
        // 3. Set up input/output channels
        // 4. Start output reader task

        let handle = AgentHandle::new(id);
        Ok(handle)
    }

    fn detect_status(&self, output: &str) -> AgentStatus {
        // Detect Claude Code status from output patterns
        if output.contains("Waiting for input") || output.contains("What would you like") {
            AgentStatus::Waiting
        } else if output.contains("Error") || output.contains("error:") {
            AgentStatus::Error
        } else if output.contains("Working") || output.contains("Reading") || output.contains("Writing") {
            AgentStatus::Running
        } else {
            AgentStatus::Idle
        }
    }

    fn supports_mcp(&self) -> bool {
        true
    }

    fn command(&self) -> &str {
        "claude"
    }

    fn default_args(&self) -> Vec<String> {
        let mut args = vec![];

        // Add model flag if not default
        if self.default_model != "sonnet" {
            args.push("--model".to_string());
            args.push(self.default_model.clone());
        }

        args.extend(self.default_flags.clone());
        args
    }
}
