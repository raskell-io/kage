//! Agent supervisor - manages agent lifecycles with PTY

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::sync::broadcast;

use crate::agent::AgentId;
use crate::config::Config;
use crate::subscription::pool::{RequestContext, SubscriptionPool};
use crate::subscription::{SubscriptionId, SubscriptionRegistry};

use super::protocol::{AgentInfo, OutputLine};

/// Supervisor manages all active agents
pub struct Supervisor {
    config: Config,
    agents: HashMap<AgentId, ManagedAgent>,
    agent_counter: u32,
    /// Subscription pool for API key management
    subscription_pool: Option<Arc<SubscriptionPool>>,
}

/// A managed agent with PTY and output tracking
struct ManagedAgent {
    /// Agent info
    info: AgentInfo,
    /// PTY master handle (for writing) - wrapped in Mutex for Sync
    pty_master: Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>,
    /// Writer for PTY input - wrapped in Mutex for Sync
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    /// Output history
    output_history: Vec<OutputLine>,
    /// Broadcast sender for live output
    output_tx: broadcast::Sender<OutputLine>,
    /// Child process handle - wrapped in Mutex for Sync
    child: Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
    /// When the agent started
    started_at: Instant,
    /// Maximum output history lines
    max_history: usize,
    /// Subscription being used by this agent
    subscription_id: Option<SubscriptionId>,
    /// Token count for usage tracking (estimated from output)
    tokens_used: u64,
}

impl Supervisor {
    /// Create a new supervisor
    pub fn new(config: Config) -> Self {
        Self {
            config,
            agents: HashMap::new(),
            agent_counter: 0,
            subscription_pool: None,
        }
    }

    /// Create a supervisor with a subscription pool
    pub fn with_subscription_pool(config: Config, pool: Arc<SubscriptionPool>) -> Self {
        Self {
            config,
            agents: HashMap::new(),
            agent_counter: 0,
            subscription_pool: Some(pool),
        }
    }

    /// Set the subscription pool
    pub fn set_subscription_pool(&mut self, pool: Arc<SubscriptionPool>) {
        self.subscription_pool = Some(pool);
    }

    /// Initialize subscription pool from config
    pub fn init_subscription_pool(&mut self) -> Result<()> {
        let db_path = self.config.daemon.state_dir.join("subscriptions.redb");
        let registry = Arc::new(SubscriptionRegistry::open(db_path)?);
        let pool = Arc::new(SubscriptionPool::new(registry));
        self.subscription_pool = Some(pool);
        tracing::info!("Subscription pool initialized");
        Ok(())
    }

    /// Get the subscription pool
    pub fn subscription_pool(&self) -> Option<&Arc<SubscriptionPool>> {
        self.subscription_pool.as_ref()
    }

    /// Spawn a new agent
    pub async fn spawn(
        &mut self,
        working_dir: PathBuf,
        namespace: Option<String>,
        prompt: Option<String>,
        model: Option<String>,
        max_iterations: Option<u32>,
    ) -> Result<AgentId> {
        let id = AgentId::new();
        self.agent_counter += 1;
        let name = format!("shadow-{}", self.agent_counter);

        tracing::info!(
            "Spawning agent {} in {:?} (namespace: {:?})",
            name,
            working_dir,
            namespace
        );

        // Try to acquire a subscription from the pool
        let (subscription_id, api_key) = if let Some(ref pool) = self.subscription_pool {
            let context = RequestContext::new()
                .with_agent(id)
                .with_namespace(namespace.as_deref().unwrap_or("default"));

            match pool.acquire(&context) {
                Ok(lease) => {
                    let sub_id = lease.id();
                    let key_ref = lease.api_key_ref().to_string();

                    // Get the actual API key from keychain
                    let api_key = crate::secrets::get(&key_ref, &crate::secrets::SecretScope::Global)?
                        .ok_or_else(|| anyhow::anyhow!("API key not found in keychain: {}", key_ref))?;

                    tracing::info!(
                        "Agent {} acquired subscription {} ({})",
                        name,
                        lease.name(),
                        sub_id
                    );

                    // Don't call success/failure yet - we'll do that when agent completes
                    // Just store the subscription info
                    (Some(sub_id), Some(api_key))
                }
                Err(e) => {
                    tracing::warn!("No subscription available, spawning without API key: {}", e);
                    (None, None)
                }
            }
        } else {
            tracing::debug!("No subscription pool configured, spawning without managed API key");
            (None, None)
        };

        // Create PTY
        let pty_system = native_pty_system();
        let pty_pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to create PTY")?;

        // Build command
        let mut cmd = CommandBuilder::new("claude");

        // Set API key if we have one from subscription pool
        if let Some(ref key) = api_key {
            cmd.env("ANTHROPIC_API_KEY", key);
        }

        // Add model if specified
        if let Some(ref m) = model {
            cmd.arg("--model");
            cmd.arg(m);
        }

        // Add prompt if specified
        if let Some(ref p) = prompt {
            cmd.arg("--print");
            cmd.arg(p);
        }

        // Set working directory
        cmd.cwd(&working_dir);

        // Spawn the process
        let child = pty_pair
            .slave
            .spawn_command(cmd)
            .context("Failed to spawn claude process")?;

        let pid = child.process_id();

        // Get writer for input
        let writer = pty_pair
            .master
            .take_writer()
            .context("Failed to get PTY writer")?;

        // Create output broadcast channel
        let (output_tx, _) = broadcast::channel::<OutputLine>(1000);

        // Create agent info
        let info = AgentInfo {
            id,
            name: name.clone(),
            status: "running".to_string(),
            working_dir,
            namespace,
            iteration: 0,
            max_iterations: max_iterations.unwrap_or(self.config.claude.max_iterations),
            started_at: chrono::Utc::now().timestamp(),
            pid,
        };

        // Create managed agent
        let agent = ManagedAgent {
            info,
            pty_master: Mutex::new(Some(pty_pair.master)),
            writer: Mutex::new(Some(writer)),
            output_history: Vec::new(),
            output_tx: output_tx.clone(),
            child: Mutex::new(Some(child)),
            started_at: Instant::now(),
            max_history: 10000,
            subscription_id,
            tokens_used: 0,
        };

        self.agents.insert(id, agent);

        // Start output reader task
        self.spawn_output_reader(id);

        tracing::info!("Agent {} ({}) spawned with PID {:?}", name, id, pid);
        Ok(id)
    }

    /// Spawn a task to read agent output
    fn spawn_output_reader(&mut self, id: AgentId) {
        if let Some(agent) = self.agents.get(&id) {
            let master = agent.pty_master.lock().unwrap().take();
            if let Some(master) = master {
                let output_tx = agent.output_tx.clone();

                // Spawn blocking task to read PTY output
                std::thread::spawn(move || {
                    let mut reader = master.try_clone_reader().ok();
                    if reader.is_none() {
                        tracing::warn!("Failed to get PTY reader for agent {}", id);
                        return;
                    }
                    let reader = reader.as_mut().unwrap();

                    let mut buf = [0u8; 4096];
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) => {
                                // EOF - process exited
                                tracing::debug!("Agent {} output EOF", id);
                                break;
                            }
                            Ok(n) => {
                                let text = String::from_utf8_lossy(&buf[..n]).to_string();
                                let line = OutputLine {
                                    text,
                                    is_error: false,
                                    timestamp: chrono::Utc::now().timestamp(),
                                };
                                // Ignore send errors (no receivers)
                                let _ = output_tx.send(line);
                            }
                            Err(e) => {
                                tracing::debug!("Agent {} read error: {}", id, e);
                                break;
                            }
                        }
                    }
                });
            }
        }
    }

    /// Kill an agent
    pub async fn kill(&mut self, id: AgentId, force: bool) -> Result<()> {
        let agent = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("Agent {} not found", id))?;

        tracing::info!("Killing agent {} (force: {})", agent.info.name, force);

        // Check if we have a child process
        let has_child = agent.child.lock().unwrap().is_some();

        if has_child {
            if force {
                // Force kill immediately
                let mut child_guard = agent.child.lock().unwrap();
                if let Some(ref mut child) = *child_guard {
                    child.kill()?;
                    let _ = child.wait();
                }
            } else {
                // Try graceful shutdown first - send Ctrl+C
                {
                    let mut writer_guard = agent.writer.lock().unwrap();
                    if let Some(ref mut writer) = *writer_guard {
                        let _ = writer.write_all(&[0x03]); // Ctrl+C
                        let _ = writer.flush();
                    }
                } // writer_guard dropped here

                // Wait a bit for graceful shutdown (no mutex held across await)
                tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                // Check if still running and force kill if needed
                let mut child_guard = agent.child.lock().unwrap();
                if let Some(ref mut child) = *child_guard {
                    match child.try_wait() {
                        Ok(Some(_)) => {
                            // Already exited
                        }
                        _ => {
                            // Still running, force kill
                            child.kill()?;
                        }
                    }
                    let _ = child.wait();
                }
            }
        }

        agent.info.status = "stopped".to_string();
        *agent.child.lock().unwrap() = None;
        *agent.writer.lock().unwrap() = None;

        // Release subscription if one was used
        if let Some(sub_id) = agent.subscription_id {
            if let Some(ref pool) = self.subscription_pool {
                // Agent was stopped/killed - mark as failure for usage tracking
                if let Err(e) = pool.release(sub_id, false, 0) {
                    tracing::warn!("Failed to release subscription {}: {}", sub_id, e);
                }
                // Clear sticky session for this agent
                pool.clear_sticky_session(id);
            }
        }

        Ok(())
    }

    /// List agents
    pub async fn list(
        &self,
        namespace: Option<String>,
        include_stopped: bool,
    ) -> Vec<AgentInfo> {
        self.agents
            .values()
            .filter(|a| {
                let ns_match = namespace
                    .as_ref()
                    .map(|ns| a.info.namespace.as_ref() == Some(ns))
                    .unwrap_or(true);
                let status_match = include_stopped || a.info.status != "stopped";
                ns_match && status_match
            })
            .map(|a| a.info.clone())
            .collect()
    }

    /// List all agents (for internal use)
    pub async fn list_all(&self) -> Vec<AgentInfo> {
        self.agents.values().map(|a| a.info.clone()).collect()
    }

    /// Get agent info
    pub async fn get_info(&self, id: AgentId) -> Option<AgentInfo> {
        self.agents.get(&id).map(|a| a.info.clone())
    }

    /// Send input to an agent
    pub async fn send_input(&self, id: AgentId, input: &str) -> Result<()> {
        let agent = self
            .agents
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("Agent {} not found", id))?;

        if agent.info.status != "running" {
            anyhow::bail!("Agent {} is not running", id);
        }

        // We need mutable access to the writer, but we're in an immutable context
        // This is a limitation - we'd need interior mutability for proper input handling
        // For now, bail out
        anyhow::bail!("Input requires mutable access - use attach instead")
    }

    /// Get agent output
    pub async fn get_output(
        &self,
        id: AgentId,
        lines: usize,
    ) -> Result<(Vec<OutputLine>, bool)> {
        let agent = self
            .agents
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("Agent {} not found", id))?;

        let history = &agent.output_history;
        let total = history.len();

        if lines == 0 || lines >= total {
            Ok((history.clone(), false))
        } else {
            let start = total - lines;
            Ok((history[start..].to_vec(), start > 0))
        }
    }

    /// Attach to agent output stream
    pub async fn attach(
        &self,
        id: AgentId,
    ) -> Result<broadcast::Receiver<OutputLine>> {
        let agent = self
            .agents
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("Agent {} not found", id))?;

        Ok(agent.output_tx.subscribe())
    }

    /// Pause an agent (send SIGSTOP)
    pub async fn pause(&mut self, id: AgentId) -> Result<()> {
        let agent = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("Agent {} not found", id))?;

        if agent.info.status != "running" {
            anyhow::bail!("Agent {} is not running", id);
        }

        // Send Ctrl+Z to pause
        let mut writer_guard = agent.writer.lock().unwrap();
        if let Some(ref mut writer) = *writer_guard {
            writer.write_all(&[0x1a])?; // Ctrl+Z
            writer.flush()?;
        }
        drop(writer_guard);

        agent.info.status = "paused".to_string();
        tracing::info!("Agent {} paused", agent.info.name);
        Ok(())
    }

    /// Resume an agent
    pub async fn resume(&mut self, id: AgentId) -> Result<()> {
        let agent = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("Agent {} not found", id))?;

        if agent.info.status != "paused" {
            anyhow::bail!("Agent {} is not paused", id);
        }

        // Send "fg" to resume (this is a simplification - real implementation would use signals)
        let mut writer_guard = agent.writer.lock().unwrap();
        if let Some(ref mut writer) = *writer_guard {
            writer.write_all(b"fg\n")?;
            writer.flush()?;
        }
        drop(writer_guard);

        agent.info.status = "running".to_string();
        tracing::info!("Agent {} resumed", agent.info.name);
        Ok(())
    }

    /// Shutdown all agents
    pub async fn shutdown_all(&mut self) {
        let ids: Vec<AgentId> = self.agents.keys().copied().collect();
        for id in ids {
            if let Err(e) = self.kill(id, false).await {
                tracing::warn!("Failed to kill agent {}: {}", id, e);
                // Try force kill
                let _ = self.kill(id, true).await;
            }
        }
    }

    /// Check and update agent statuses
    pub async fn health_check(&mut self) {
        // Collect agents that completed to release their subscriptions after the loop
        let mut completed_agents: Vec<(AgentId, Option<SubscriptionId>, bool, u64)> = Vec::new();

        for (agent_id, agent) in self.agents.iter_mut() {
            let mut child_guard = agent.child.lock().unwrap();
            if let Some(ref mut child) = *child_guard {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        // Process exited
                        let success = status.success();
                        if success {
                            agent.info.status = "completed".to_string();
                        } else {
                            agent.info.status = "failed".to_string();
                        }
                        tracing::info!(
                            "Agent {} exited with status: {:?}",
                            agent.info.name,
                            status
                        );

                        // Mark for subscription release
                        if agent.subscription_id.is_some() {
                            completed_agents.push((
                                *agent_id,
                                agent.subscription_id,
                                success,
                                agent.tokens_used,
                            ));
                        }
                    }
                    Ok(None) => {
                        // Still running
                    }
                    Err(e) => {
                        tracing::warn!("Failed to check agent {} status: {}", agent.info.name, e);
                    }
                }
            }
        }

        // Release subscriptions for completed agents
        if let Some(ref pool) = self.subscription_pool {
            for (agent_id, sub_id, success, tokens) in completed_agents {
                if let Some(sub_id) = sub_id {
                    if let Err(e) = pool.release(sub_id, success, tokens) {
                        tracing::warn!("Failed to release subscription {}: {}", sub_id, e);
                    }
                    pool.clear_sticky_session(agent_id);
                    tracing::debug!(
                        "Released subscription {} for agent {} (success: {}, tokens: {})",
                        sub_id,
                        agent_id,
                        success,
                        tokens
                    );
                }
            }
        }
    }

    /// Update token usage for an agent (called when parsing output)
    pub fn update_token_usage(&mut self, id: AgentId, tokens: u64) {
        if let Some(agent) = self.agents.get_mut(&id) {
            agent.tokens_used += tokens;
        }
    }
}

// Note: Clone is needed for the daemon to share the supervisor across connections
// We use Arc<RwLock<Supervisor>> in the daemon, so this Clone is not actually used
// but is required by the original trait bounds
impl Clone for Supervisor {
    fn clone(&self) -> Self {
        // This is a shallow clone - agents are not cloned
        Self {
            config: self.config.clone(),
            agents: HashMap::new(),
            agent_counter: self.agent_counter,
            subscription_pool: self.subscription_pool.clone(),
        }
    }
}
