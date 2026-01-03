//! Agent handle - represents a running agent instance

use std::process::Child;

use tokio::sync::mpsc;

use super::{AgentId, AgentStatus};

/// Handle to a running agent
#[derive(Debug)]
pub struct AgentHandle {
    /// Unique agent ID
    pub id: AgentId,

    /// Current status
    pub status: AgentStatus,

    /// Child process (if running locally)
    process: Option<Child>,

    /// Channel to send input to agent
    input_tx: Option<mpsc::Sender<String>>,

    /// Channel to receive output from agent
    output_rx: Option<mpsc::Receiver<AgentOutput>>,
}

/// Output from an agent
#[derive(Debug, Clone)]
pub struct AgentOutput {
    /// Output text
    pub text: String,

    /// Whether this is stderr
    pub is_error: bool,

    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl AgentHandle {
    /// Create a new agent handle
    pub fn new(id: AgentId) -> Self {
        Self {
            id,
            status: AgentStatus::Starting,
            process: None,
            input_tx: None,
            output_rx: None,
        }
    }

    /// Send input to the agent
    pub async fn send(&self, input: &str) -> anyhow::Result<()> {
        if let Some(tx) = &self.input_tx {
            tx.send(input.to_string()).await?;
            Ok(())
        } else {
            anyhow::bail!("Agent input channel not available")
        }
    }

    /// Receive output from the agent
    pub async fn recv(&mut self) -> Option<AgentOutput> {
        if let Some(rx) = &mut self.output_rx {
            rx.recv().await
        } else {
            None
        }
    }

    /// Check if agent process is still running
    pub fn is_alive(&mut self) -> bool {
        if let Some(ref mut process) = self.process {
            match process.try_wait() {
                Ok(None) => true,
                _ => false,
            }
        } else {
            false
        }
    }

    /// Kill the agent process
    pub fn kill(&mut self) -> anyhow::Result<()> {
        if let Some(ref mut process) = self.process {
            process.kill()?;
        }
        self.status = AgentStatus::Stopped;
        Ok(())
    }
}

impl Clone for AgentHandle {
    fn clone(&self) -> Self {
        // Note: This is a shallow clone for supervisor bookkeeping
        // The actual process and channels are not cloned
        Self {
            id: self.id,
            status: self.status,
            process: None,
            input_tx: self.input_tx.clone(),
            output_rx: None,
        }
    }
}
