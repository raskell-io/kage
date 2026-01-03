//! Daemon client for CLI communication

use std::path::Path;

use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use super::protocol::{self, Request, Response};

/// Client for communicating with the daemon
pub struct DaemonClient {
    stream: UnixStream,
}

impl DaemonClient {
    /// Connect to the daemon
    pub async fn connect(socket_path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket_path)
            .await
            .with_context(|| format!("Failed to connect to daemon at {:?}", socket_path))?;

        Ok(Self { stream })
    }

    /// Send a request and receive a response
    pub async fn request(&mut self, req: &Request) -> Result<Response> {
        // Encode and send request
        let data = protocol::encode_message(req)?;
        self.stream.write_all(&data).await?;

        // Read response length
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        // Sanity check
        if len > 10 * 1024 * 1024 {
            anyhow::bail!("Response too large: {} bytes", len);
        }

        // Read response data
        let mut data = vec![0u8; len];
        self.stream.read_exact(&mut data).await?;

        // Decode response
        let response: Response = protocol::decode_message(&data)?;
        Ok(response)
    }

    /// Ping the daemon
    pub async fn ping(&mut self) -> Result<(String, u64)> {
        match self.request(&Request::Ping).await? {
            Response::Pong { version, uptime_secs } => Ok((version, uptime_secs)),
            Response::Error { message } => anyhow::bail!("{}", message),
            _ => anyhow::bail!("Unexpected response"),
        }
    }

    /// Get daemon status
    pub async fn status(&mut self) -> Result<Response> {
        self.request(&Request::Status).await
    }

    /// Spawn a new agent
    pub async fn spawn_agent(
        &mut self,
        working_dir: std::path::PathBuf,
        namespace: Option<String>,
        prompt: Option<String>,
        model: Option<String>,
        max_iterations: Option<u32>,
    ) -> Result<Response> {
        self.request(&Request::SpawnAgent {
            working_dir,
            namespace,
            prompt,
            model,
            max_iterations,
        })
        .await
    }

    /// Kill an agent
    pub async fn kill_agent(&mut self, id: crate::agent::AgentId, force: bool) -> Result<Response> {
        self.request(&Request::KillAgent { id, force }).await
    }

    /// List agents
    pub async fn list_agents(
        &mut self,
        namespace: Option<String>,
        include_stopped: bool,
    ) -> Result<Response> {
        self.request(&Request::ListAgents {
            namespace,
            include_stopped,
        })
        .await
    }

    /// Get agent details
    pub async fn get_agent(&mut self, id: crate::agent::AgentId) -> Result<Response> {
        self.request(&Request::GetAgent { id }).await
    }

    /// Send input to agent
    pub async fn send_input(&mut self, id: crate::agent::AgentId, input: String) -> Result<Response> {
        self.request(&Request::SendInput { id, input }).await
    }

    /// Get agent output
    pub async fn get_output(&mut self, id: crate::agent::AgentId, lines: usize) -> Result<Response> {
        self.request(&Request::GetOutput { id, lines }).await
    }

    /// Attach to agent (returns stream for reading)
    pub async fn attach(&mut self, id: crate::agent::AgentId) -> Result<()> {
        let data = protocol::encode_message(&Request::Attach { id })?;
        self.stream.write_all(&data).await?;
        Ok(())
    }

    /// Read next stream line (for attach mode)
    pub async fn read_stream_line(&mut self) -> Result<Option<Response>> {
        // Read response length
        let mut len_buf = [0u8; 4];
        match self.stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e.into()),
        }

        let len = u32::from_be_bytes(len_buf) as usize;
        if len > 10 * 1024 * 1024 {
            anyhow::bail!("Response too large: {} bytes", len);
        }

        let mut data = vec![0u8; len];
        self.stream.read_exact(&mut data).await?;

        let response: Response = protocol::decode_message(&data)?;
        Ok(Some(response))
    }

    /// List tasks
    pub async fn list_tasks(&mut self, status: Option<String>) -> Result<Response> {
        self.request(&Request::ListTasks { status }).await
    }

    /// Pause an agent
    pub async fn pause_agent(&mut self, id: crate::agent::AgentId) -> Result<Response> {
        self.request(&Request::PauseAgent { id }).await
    }

    /// Resume an agent
    pub async fn resume_agent(&mut self, id: crate::agent::AgentId) -> Result<Response> {
        self.request(&Request::ResumeAgent { id }).await
    }

    /// Cancel a task
    pub async fn cancel_task(&mut self, id: crate::task::TaskId) -> Result<Response> {
        self.request(&Request::CancelTask { id }).await
    }

    /// Shutdown the daemon
    pub async fn shutdown(&mut self) -> Result<Response> {
        self.request(&Request::Shutdown).await
    }
}

/// Check if daemon is running
pub async fn is_daemon_running(socket_path: &Path) -> bool {
    if !socket_path.exists() {
        return false;
    }

    match DaemonClient::connect(socket_path).await {
        Ok(mut client) => client.ping().await.is_ok(),
        Err(_) => false,
    }
}

/// Get default socket path
pub fn default_socket_path() -> std::path::PathBuf {
    if let Some(runtime_dir) = dirs::runtime_dir() {
        runtime_dir.join("kage.sock")
    } else if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        std::path::PathBuf::from(xdg).join("kage.sock")
    } else {
        std::path::PathBuf::from("/tmp/kage.sock")
    }
}

// Helper module for XDG paths
mod dirs {
    use std::path::PathBuf;

    pub fn runtime_dir() -> Option<PathBuf> {
        std::env::var("XDG_RUNTIME_DIR").ok().map(PathBuf::from)
    }
}
