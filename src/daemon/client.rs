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
        pty_rows: Option<u16>,
        pty_cols: Option<u16>,
    ) -> Result<Response> {
        self.request(&Request::SpawnAgent {
            working_dir,
            namespace,
            prompt,
            model,
            max_iterations,
            pty_rows,
            pty_cols,
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

    /// Get agent output (legacy - raw chunks)
    pub async fn get_output(&mut self, id: crate::agent::AgentId, lines: usize) -> Result<Response> {
        self.request(&Request::GetOutput { id, lines }).await
    }

    /// Get agent screen content (parsed terminal output)
    pub async fn get_screen_content(&mut self, id: crate::agent::AgentId) -> Result<Response> {
        self.request(&Request::GetScreenContent { id }).await
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

    /// Resize agent PTY
    pub async fn resize_agent(&mut self, id: crate::agent::AgentId, rows: u16, cols: u16) -> Result<Response> {
        self.request(&Request::ResizeAgent { id, rows, cols }).await
    }

    /// Cancel a task
    pub async fn cancel_task(&mut self, id: crate::task::TaskId) -> Result<Response> {
        self.request(&Request::CancelTask { id }).await
    }

    /// List pending approvals
    pub async fn list_approvals(&mut self) -> Result<Response> {
        self.request(&Request::ListApprovals).await
    }

    /// Approve an action
    pub async fn approve(&mut self, id: crate::task::ApprovalId) -> Result<Response> {
        self.request(&Request::Approve { id }).await
    }

    /// Reject an action
    pub async fn reject(&mut self, id: crate::task::ApprovalId, reason: Option<String>) -> Result<Response> {
        self.request(&Request::Reject { id, reason }).await
    }

    /// Shutdown the daemon
    pub async fn shutdown(&mut self) -> Result<Response> {
        self.request(&Request::Shutdown).await
    }

    /// List registered subscriptions
    pub async fn list_subscriptions(&mut self) -> Result<Response> {
        self.request(&Request::ListSubscriptions).await
    }

    /// Add a new subscription
    pub async fn add_subscription(&mut self, name: String, api_key: String) -> Result<Response> {
        self.request(&Request::AddSubscription { name, api_key }).await
    }

    /// Query memory entries
    pub async fn query_memory(
        &mut self,
        text: Option<String>,
        scope: Option<String>,
        memory_type: Option<String>,
        tags: Vec<String>,
        since: Option<i64>,
        limit: Option<usize>,
    ) -> Result<Response> {
        self.request(&Request::QueryMemory {
            text,
            scope,
            memory_type,
            tags,
            since,
            limit,
        })
        .await
    }

    /// Get a specific memory entry
    pub async fn get_memory(&mut self, id: String) -> Result<Response> {
        self.request(&Request::GetMemory { id }).await
    }

    /// Store a memory entry
    pub async fn store_memory(
        &mut self,
        entry: crate::memory::MemoryEntry,
        scope: crate::memory::MemoryScope,
    ) -> Result<Response> {
        self.request(&Request::StoreMemory { entry, scope }).await
    }

    /// Prune old memory entries
    pub async fn prune_memory(&mut self, older_than_days: u32, dry_run: bool) -> Result<Response> {
        self.request(&Request::PruneMemory {
            older_than_days,
            dry_run,
        })
        .await
    }

    /// Attach context to an agent
    pub async fn attach_context(
        &mut self,
        agent_id: crate::agent::AgentId,
        memory_id: crate::memory::MemoryId,
        ref_type: crate::memory::ContextRefType,
    ) -> Result<Response> {
        self.request(&Request::AttachContext {
            agent_id,
            memory_id,
            ref_type,
        })
        .await
    }

    /// Detach context from an agent
    pub async fn detach_context(
        &mut self,
        agent_id: crate::agent::AgentId,
        memory_id: crate::memory::MemoryId,
    ) -> Result<Response> {
        self.request(&Request::DetachContext { agent_id, memory_id })
            .await
    }

    /// List context refs for an agent
    pub async fn list_context_refs(
        &mut self,
        agent_id: crate::agent::AgentId,
        ref_type: Option<String>,
    ) -> Result<Response> {
        self.request(&Request::ListContextRefs { agent_id, ref_type })
            .await
    }

    /// Inherit context from parent agent to child
    pub async fn inherit_context(
        &mut self,
        from_agent: crate::agent::AgentId,
        to_agent: crate::agent::AgentId,
        entries: Option<Vec<crate::memory::MemoryId>>,
    ) -> Result<Response> {
        self.request(&Request::InheritContext {
            from_agent,
            to_agent,
            entries,
        })
        .await
    }

    /// Subscribe to daemon events (real-time streaming)
    ///
    /// After calling this, use `read_event()` to receive events.
    pub async fn subscribe(&mut self, event_types: Vec<String>) -> Result<()> {
        let data = protocol::encode_message(&Request::Subscribe { event_types })?;
        self.stream.write_all(&data).await?;

        // Wait for Subscribed confirmation
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut data = vec![0u8; len];
        self.stream.read_exact(&mut data).await?;

        let response: Response = protocol::decode_message(&data)?;
        match response {
            Response::Subscribed => Ok(()),
            Response::Error { message } => anyhow::bail!("{}", message),
            _ => anyhow::bail!("Unexpected response to subscribe"),
        }
    }

    /// Read next event (for subscribe mode)
    ///
    /// Returns None if the connection is closed.
    pub async fn read_event(&mut self) -> Result<Option<protocol::DaemonEvent>> {
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
        match response {
            Response::Event(event) => Ok(Some(event)),
            Response::Error { message } => anyhow::bail!("{}", message),
            _ => anyhow::bail!("Unexpected response in event stream"),
        }
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

/// Ensure the daemon is running, starting it in the background if needed
pub async fn ensure_running() -> Result<()> {
    let socket_path = default_socket_path();

    if is_daemon_running(&socket_path).await {
        tracing::debug!("Daemon already running");
        return Ok(());
    }

    tracing::info!("Starting daemon in background...");

    // Spawn daemon as a separate background process (not a tokio task)
    // This ensures the daemon survives when the TUI/CLI exits
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        use std::process::Command;

        // Get the current executable path
        let exe = std::env::current_exe()?;

        // Spawn the daemon process
        let mut cmd = Command::new(&exe);
        cmd.args(["daemon", "start", "--foreground"]);

        // Detach from terminal
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        // Create new process group so daemon survives parent exit
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }

        cmd.spawn()?;
    }

    #[cfg(not(unix))]
    {
        // On non-Unix platforms, fall back to in-process daemon
        let cfg = crate::config::load()?;
        let daemon_cfg = cfg.clone();
        tokio::spawn(async move {
            match super::Daemon::new(daemon_cfg) {
                Ok(mut d) => {
                    if let Err(e) = d.run().await {
                        tracing::error!("Daemon error: {}", e);
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to create daemon: {}", e);
                }
            }
        });
    }

    // Wait briefly for daemon to start
    for _ in 0..20 {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        if is_daemon_running(&socket_path).await {
            tracing::info!("Daemon started successfully");
            return Ok(());
        }
    }

    // Continue anyway - dashboard will show disconnected status
    tracing::warn!("Daemon may not have started, continuing anyway");
    Ok(())
}

// Helper module for XDG paths
mod dirs {
    use std::path::PathBuf;

    pub fn runtime_dir() -> Option<PathBuf> {
        std::env::var("XDG_RUNTIME_DIR").ok().map(PathBuf::from)
    }
}
