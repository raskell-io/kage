//! Agent supervisor - manages agent lifecycles with PTY

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::sync::broadcast;

use crate::agent::AgentId;
use crate::config::Config;
use crate::memory::{MemoryContent, MemoryEntry, MemoryScope};
use crate::subscription::pool::{RequestContext, SubscriptionPool};
use crate::subscription::{SubscriptionId, SubscriptionRegistry};
use crate::task::{ApprovalAction, ApprovalId, ApprovalLevel, ApprovalRequest, TaskId};

use super::protocol::{AgentInfo, ApprovalInfo, OutputLine};

/// Detected workspace configuration for AI agents
#[derive(Debug, Clone, Default)]
pub struct WorkspaceConfig {
    /// Claude Code specific config (.claude/)
    pub claude_dir: Option<PathBuf>,
    /// CLAUDE.md instructions file
    pub claude_md: Option<PathBuf>,
    /// settings.json in .claude/
    pub claude_settings: Option<PathBuf>,
    /// OpenAI Codex config (.codex/)
    pub codex_dir: Option<PathBuf>,
    /// Cursor config (.cursor/)
    pub cursor_dir: Option<PathBuf>,
    /// Generic .ai/ or .agent/ config
    pub generic_ai_dir: Option<PathBuf>,
}

impl WorkspaceConfig {
    /// Detect workspace configuration from a directory
    pub fn detect(working_dir: &Path) -> Self {
        let mut config = Self::default();

        // Claude Code: .claude/ folder
        let claude_dir = working_dir.join(".claude");
        if claude_dir.is_dir() {
            config.claude_dir = Some(claude_dir.clone());

            // Check for CLAUDE.md in .claude/
            let claude_md = claude_dir.join("CLAUDE.md");
            if claude_md.is_file() {
                config.claude_md = Some(claude_md);
            }

            // Check for settings.json
            let settings = claude_dir.join("settings.json");
            if settings.is_file() {
                config.claude_settings = Some(settings);
            }
        }

        // Also check for CLAUDE.md in root (some projects use this)
        if config.claude_md.is_none() {
            let root_claude_md = working_dir.join("CLAUDE.md");
            if root_claude_md.is_file() {
                config.claude_md = Some(root_claude_md);
            }
        }

        // OpenAI Codex: .codex/ folder
        let codex_dir = working_dir.join(".codex");
        if codex_dir.is_dir() {
            config.codex_dir = Some(codex_dir);
        }

        // Cursor: .cursor/ folder
        let cursor_dir = working_dir.join(".cursor");
        if cursor_dir.is_dir() {
            config.cursor_dir = Some(cursor_dir);
        }

        // Generic: .ai/ or .agent/
        let ai_dir = working_dir.join(".ai");
        let agent_dir = working_dir.join(".agent");
        if ai_dir.is_dir() {
            config.generic_ai_dir = Some(ai_dir);
        } else if agent_dir.is_dir() {
            config.generic_ai_dir = Some(agent_dir);
        }

        config
    }

    /// Check if this is a Claude Code configured workspace
    pub fn has_claude_config(&self) -> bool {
        self.claude_dir.is_some() || self.claude_md.is_some()
    }

    /// Get a description of detected configs
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        if self.claude_dir.is_some() {
            parts.push(".claude/");
        }
        if self.claude_md.is_some() {
            parts.push("CLAUDE.md");
        }
        if self.codex_dir.is_some() {
            parts.push(".codex/");
        }
        if self.cursor_dir.is_some() {
            parts.push(".cursor/");
        }
        if self.generic_ai_dir.is_some() {
            parts.push(".ai/");
        }
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join(", ")
        }
    }
}

/// Screen update notification (just signals that screen changed)
#[derive(Clone, Debug)]
pub struct ScreenUpdate {
    pub agent_id: AgentId,
    pub timestamp: i64,
}

/// Supervisor manages all active agents
pub struct Supervisor {
    config: Config,
    agents: HashMap<AgentId, ManagedAgent>,
    agent_counter: u32,
    /// Subscription pool for API key management
    subscription_pool: Option<Arc<SubscriptionPool>>,
    /// Cached API keys (loaded once at init, never touches keychain again)
    api_key_cache: HashMap<SubscriptionId, String>,
    /// Pending approval requests
    pending_approvals: HashMap<ApprovalId, ApprovalRequest>,
    /// Agent to approval level mapping (from tasks)
    agent_approval_levels: HashMap<AgentId, ApprovalLevel>,
    /// Agent to task mapping
    agent_tasks: HashMap<AgentId, TaskId>,
}

/// A managed agent with PTY and virtual terminal
struct ManagedAgent {
    /// Agent info
    info: AgentInfo,
    /// PTY master handle (for writing) - wrapped in Mutex for Sync
    pty_master: Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>,
    /// Writer for PTY input - wrapped in Mutex for Sync
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    /// Virtual terminal parser - maintains screen state
    vt_parser: Arc<Mutex<vt100::Parser>>,
    /// Broadcast sender for screen update notifications
    screen_tx: broadcast::Sender<ScreenUpdate>,
    /// Legacy output history for compatibility - shared with reader thread
    output_history: Arc<std::sync::Mutex<Vec<OutputLine>>>,
    /// Legacy broadcast sender for live output (used by attach)
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
    /// Index of last output line processed for memory events
    memory_processed_idx: usize,
}

impl Supervisor {
    /// Create a new supervisor
    pub fn new(config: Config) -> Self {
        Self {
            config,
            agents: HashMap::new(),
            agent_counter: 0,
            subscription_pool: None,
            api_key_cache: HashMap::new(),
            pending_approvals: HashMap::new(),
            agent_approval_levels: HashMap::new(),
            agent_tasks: HashMap::new(),
        }
    }

    /// Create a supervisor with a subscription pool
    pub fn with_subscription_pool(config: Config, pool: Arc<SubscriptionPool>) -> Self {
        Self {
            config,
            agents: HashMap::new(),
            agent_counter: 0,
            subscription_pool: Some(pool),
            api_key_cache: HashMap::new(),
            pending_approvals: HashMap::new(),
            agent_approval_levels: HashMap::new(),
            agent_tasks: HashMap::new(),
        }
    }

    /// Set the subscription pool
    pub fn set_subscription_pool(&mut self, pool: Arc<SubscriptionPool>) {
        self.subscription_pool = Some(pool);
    }

    /// Initialize subscription pool from config
    ///
    /// API keys are loaded from the database (subscription.api_key field).
    /// Keychain is only accessed as fallback for legacy subscriptions.
    pub fn init_subscription_pool(&mut self) -> Result<()> {
        use crate::secrets::{SecretScope, get as get_secret};

        let db_path = self.config.daemon.state_dir.join("subscriptions.redb");
        let registry = Arc::new(SubscriptionRegistry::open(db_path)?);
        let pool = Arc::new(SubscriptionPool::new(registry.clone()));

        // Load API keys into memory cache
        let subscriptions = pool.list_subscriptions();
        let mut needs_migration = Vec::new();

        for sub in &subscriptions {
            // First: try to get from subscription.api_key (stored in database)
            if let Some(ref key) = sub.api_key {
                tracing::debug!("Loaded API key from database for: {}", sub.name);
                self.api_key_cache.insert(sub.id, key.clone());
            } else {
                // Fallback: try keychain (legacy subscriptions)
                match get_secret(&sub.api_key_ref, &SecretScope::Global) {
                    Ok(Some(key)) => {
                        tracing::debug!("Loaded API key from keychain for: {} (will migrate to database)", sub.name);
                        self.api_key_cache.insert(sub.id, key.clone());
                        // Mark for migration to database
                        needs_migration.push((sub.id, sub.name.clone(), key));
                    }
                    Ok(None) => {
                        tracing::warn!("API key not found for subscription: {}", sub.name);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load API key for {}: {}", sub.name, e);
                    }
                }
            }
        }

        // Migrate legacy subscriptions to store key in database
        for (id, name, key) in needs_migration {
            if let Err(e) = registry.update_api_key(id, &key) {
                tracing::warn!("Failed to migrate API key for {} to database: {}", name, e);
            } else {
                tracing::info!("Migrated API key for {} to database (no more keychain prompts)", name);
            }
        }

        self.subscription_pool = Some(pool.clone());

        let cached = self.api_key_cache.len();
        let total = subscriptions.len();
        if total == 0 {
            tracing::info!("No subscriptions in pool. Add credentials via 'kage subscription add' or the setup wizard.");
        } else {
            tracing::info!("Subscription pool initialized: {}/{} subscriptions ready", cached, total);
        }
        Ok(())
    }

    /// Get the subscription pool
    pub fn subscription_pool(&self) -> Option<&Arc<SubscriptionPool>> {
        self.subscription_pool.as_ref()
    }

    /// List all subscriptions
    pub fn list_subscriptions(&self) -> Vec<crate::subscription::Subscription> {
        if let Some(ref pool) = self.subscription_pool {
            pool.list_subscriptions()
        } else {
            Vec::new()
        }
    }

    /// Add a new subscription
    ///
    /// Stores the API key directly in the subscription database (no keychain needed).
    /// The key is also cached in memory for spawning agents.
    pub fn add_subscription(&mut self, name: String, api_key: String) -> Result<()> {
        use crate::subscription::{ProviderType, Subscription};

        // Create subscription with API key stored directly in database
        // No keychain storage needed - the database persists across restarts
        let key_ref = format!("subscription:{}", name);
        let subscription = Subscription::new(&name, &key_ref)
            .with_provider(ProviderType::ClaudeCode)
            .with_api_key(&api_key);

        // Add to pool/registry and cache the API key in memory
        if let Some(ref pool) = self.subscription_pool {
            let sub_id = pool.add_subscription(subscription)?;
            // Cache the API key so spawn() uses it directly
            self.api_key_cache.insert(sub_id, api_key);
            tracing::info!("Added subscription: {} (stored in database)", name);
            Ok(())
        } else {
            anyhow::bail!("Subscription pool not initialized")
        }
    }

    /// Spawn a new agent
    pub async fn spawn(
        &mut self,
        working_dir: PathBuf,
        namespace: Option<String>,
        prompt: Option<String>,
        model: Option<String>,
        max_iterations: Option<u32>,
        pty_rows: Option<u16>,
        pty_cols: Option<u16>,
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

        // Detect workspace configuration
        let workspace_config = WorkspaceConfig::detect(&working_dir);
        if workspace_config.has_claude_config() {
            tracing::info!(
                "Detected workspace config for {}: {}",
                name,
                workspace_config.describe()
            );
        } else {
            tracing::debug!(
                "No workspace config found in {:?} (will use defaults)",
                working_dir
            );
        }

        // Try to acquire a subscription from the pool
        let (subscription_id, api_key) = if let Some(ref pool) = self.subscription_pool {
            let context = RequestContext::new()
                .with_agent(id)
                .with_namespace(namespace.as_deref().unwrap_or("default"));

            match pool.acquire(&context) {
                Ok(lease) => {
                    let sub_id = lease.id();

                    // Get API key from in-memory cache (never touches keychain here)
                    let api_key = self.api_key_cache.get(&sub_id)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!(
                            "API key not cached for subscription: {} ({}). Restart daemon to reload keys.",
                            lease.name(),
                            sub_id
                        ))?;

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

        // Create PTY with specified size or defaults
        let actual_rows = pty_rows.unwrap_or(24);
        let actual_cols = pty_cols.unwrap_or(80);
        tracing::info!(
            "Creating PTY with size {}x{} (requested: {:?}x{:?})",
            actual_cols, actual_rows, pty_cols, pty_rows
        );

        let pty_system = native_pty_system();
        let pty_pair = pty_system
            .openpty(PtySize {
                rows: actual_rows,
                cols: actual_cols,
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
            tokens_used: None,
        };

        // Create managed agent with vt100 parser and shared output history
        let output_history = Arc::new(std::sync::Mutex::new(Vec::new()));
        let vt_parser = Arc::new(Mutex::new(vt100::Parser::new(actual_rows, actual_cols, 1000)));
        let (screen_tx, _) = broadcast::channel(16);

        let agent = ManagedAgent {
            info,
            pty_master: Mutex::new(Some(pty_pair.master)),
            writer: Mutex::new(Some(writer)),
            vt_parser,
            screen_tx,
            output_history,
            output_tx: output_tx.clone(),
            child: Mutex::new(Some(child)),
            started_at: Instant::now(),
            max_history: 10000,
            subscription_id,
            tokens_used: 0,
            memory_processed_idx: 0,
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
            // Get the reader from the master without taking ownership
            // The master stays in the agent for resizing
            let reader = {
                let guard = agent.pty_master.lock().unwrap();
                if let Some(ref master) = *guard {
                    master.try_clone_reader().ok()
                } else {
                    None
                }
            };

            if let Some(mut reader) = reader {
                let output_tx = agent.output_tx.clone();
                let output_history = agent.output_history.clone();
                let vt_parser = agent.vt_parser.clone();
                let screen_tx = agent.screen_tx.clone();
                let max_history = agent.max_history;

                // Spawn blocking task to read PTY output
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) => {
                                // EOF - process exited
                                tracing::debug!("Agent {} output EOF", id);
                                break;
                            }
                            Ok(n) => {
                                // Feed raw bytes to vt100 parser for proper terminal emulation
                                if let Ok(mut parser) = vt_parser.lock() {
                                    parser.process(&buf[..n]);
                                }

                                // Notify about screen update
                                let _ = screen_tx.send(ScreenUpdate {
                                    agent_id: id,
                                    timestamp: chrono::Utc::now().timestamp(),
                                });

                                // Legacy: also store as text for compatibility
                                let text = String::from_utf8_lossy(&buf[..n]).to_string();
                                let line = OutputLine {
                                    text,
                                    is_error: false,
                                    timestamp: chrono::Utc::now().timestamp(),
                                };

                                // Store in history (legacy)
                                if let Ok(mut history) = output_history.lock() {
                                    history.push(line.clone());
                                    // Trim to max history size
                                    let len = history.len();
                                    if len > max_history {
                                        history.drain(0..len - max_history);
                                    }
                                }

                                // Send to live subscribers (legacy - for attach)
                                let _ = output_tx.send(line);
                            }
                            Err(e) => {
                                tracing::debug!("Agent {} read error: {}", id, e);
                                break;
                            }
                        }
                    }
                });
            } else {
                tracing::warn!("Failed to get PTY reader for agent {}", id);
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

        // Get the writer and write input
        let mut writer_guard = agent.writer.lock().unwrap();
        if let Some(ref mut writer) = *writer_guard {
            writer.write_all(input.as_bytes())
                .context("Failed to write to agent PTY")?;
            writer.flush()
                .context("Failed to flush agent PTY")?;
            Ok(())
        } else {
            anyhow::bail!("Agent {} has no writer available", id)
        }
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

        let history = agent.output_history.lock().unwrap();
        let total = history.len();

        if lines == 0 || lines >= total {
            Ok((history.clone(), false))
        } else {
            let start = total - lines;
            Ok((history[start..].to_vec(), start > 0))
        }
    }

    /// Get agent screen content from vt100 parser
    /// Returns Vec of screen lines with ANSI color codes preserved
    pub fn get_screen_content(&self, id: AgentId) -> Result<Vec<OutputLine>> {
        let agent = self
            .agents
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("Agent {} not found", id))?;

        let parser = agent.vt_parser.lock().unwrap();
        let screen = parser.screen();
        let timestamp = chrono::Utc::now().timestamp();
        let (_, cols) = screen.size();

        // Get visible screen rows with ANSI formatting preserved
        let lines: Vec<OutputLine> = screen
            .rows_formatted(0, cols)
            .map(|bytes| {
                // Convert bytes to string, preserving ANSI codes
                let text = String::from_utf8_lossy(&bytes).to_string();
                OutputLine {
                    text,
                    is_error: false,
                    timestamp,
                }
            })
            .collect();

        Ok(lines)
    }

    /// Subscribe to screen update notifications
    pub fn subscribe_screen_updates(&self, id: AgentId) -> Result<broadcast::Receiver<ScreenUpdate>> {
        let agent = self
            .agents
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("Agent {} not found", id))?;

        Ok(agent.screen_tx.subscribe())
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

    /// Resize agent PTY and vt100 parser
    pub fn resize(&self, id: AgentId, rows: u16, cols: u16) -> Result<()> {
        let agent = self
            .agents
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("Agent {} not found", id))?;

        // Get the PTY master and resize it
        if let Some(ref master) = *agent.pty_master.lock().unwrap() {
            master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            }).context("Failed to resize PTY")?;
            tracing::debug!("Resized agent {} PTY to {}x{}", agent.info.name, cols, rows);
        }

        // Also resize the vt100 parser to match
        if let Ok(mut parser) = agent.vt_parser.lock() {
            parser.set_size(rows, cols);
        }

        Ok(())
    }

    /// Get agent output as text lines (for criteria checking)
    pub fn get_output_lines(&self, id: AgentId) -> Vec<String> {
        self.agents
            .get(&id)
            .and_then(|a| a.output_history.lock().ok())
            .map(|history| history.iter().map(|l| l.text.clone()).collect())
            .unwrap_or_default()
    }

    /// Get agent working directory
    pub fn get_working_dir(&self, id: AgentId) -> Option<std::path::PathBuf> {
        self.agents
            .get(&id)
            .map(|a| a.info.working_dir.clone())
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

    // ========== Approval Methods ==========

    /// Set approval level for an agent (called when spawning for a task)
    pub fn set_agent_approval(&mut self, agent_id: AgentId, level: ApprovalLevel, task_id: Option<TaskId>) {
        self.agent_approval_levels.insert(agent_id, level);
        if let Some(tid) = task_id {
            self.agent_tasks.insert(agent_id, tid);
        }
    }

    /// Get approval level for an agent
    pub fn get_agent_approval_level(&self, agent_id: AgentId) -> ApprovalLevel {
        self.agent_approval_levels.get(&agent_id).copied().unwrap_or(ApprovalLevel::None)
    }

    /// Check if an agent is waiting for approval
    pub fn is_agent_awaiting_approval(&self, agent_id: AgentId) -> bool {
        self.pending_approvals.values().any(|r| r.agent_id == agent_id)
    }

    /// Process agent output to detect actions requiring approval
    /// Returns Some(ApprovalRequest) if agent should be paused for approval
    pub fn check_output_for_approval(&mut self, agent_id: AgentId, line: &str) -> Option<ApprovalRequest> {
        let level = self.get_agent_approval_level(agent_id);
        if level == ApprovalLevel::None {
            return None;
        }

        // Detect file write patterns (Claude Code output format)
        if let Some(action) = self.detect_file_write(line) {
            if level.requires_approval(&action) {
                let context = self.get_output_context(agent_id, 5);
                let task_id = self.agent_tasks.get(&agent_id).copied();
                let request = ApprovalRequest::new(agent_id, task_id, action, context);
                self.pending_approvals.insert(request.id, request.clone());
                return Some(request);
            }
        }

        // Detect git commit patterns
        if let Some(action) = self.detect_git_commit(line) {
            if level.requires_approval(&action) {
                let context = self.get_output_context(agent_id, 5);
                let task_id = self.agent_tasks.get(&agent_id).copied();
                let request = ApprovalRequest::new(agent_id, task_id, action, context);
                self.pending_approvals.insert(request.id, request.clone());
                return Some(request);
            }
        }

        // Detect tool use (for Always level)
        if level == ApprovalLevel::Always {
            if let Some(action) = self.detect_tool_use(line) {
                let context = self.get_output_context(agent_id, 5);
                let task_id = self.agent_tasks.get(&agent_id).copied();
                let request = ApprovalRequest::new(agent_id, task_id, action, context);
                self.pending_approvals.insert(request.id, request.clone());
                return Some(request);
            }
        }

        None
    }

    /// Get recent output context for approval request
    fn get_output_context(&self, agent_id: AgentId, lines: usize) -> Vec<String> {
        self.agents
            .get(&agent_id)
            .and_then(|a| a.output_history.lock().ok())
            .map(|history| {
                history
                    .iter()
                    .rev()
                    .take(lines)
                    .rev()
                    .map(|l| l.text.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Detect file write from Claude Code output
    fn detect_file_write(&self, line: &str) -> Option<ApprovalAction> {
        // Claude Code outputs patterns like:
        // "✏️  Wrote to /path/to/file.rs"
        // "Write(/path/to/file.rs)"
        // "edit_file: /path/to/file.rs"
        let patterns = [
            ("Wrote to ", true),
            ("Write(", false),
            ("edit_file: ", true),
            ("Created ", true),
        ];

        for (pattern, has_path_after) in patterns {
            if line.contains(pattern) {
                let path_str = if has_path_after {
                    line.split(pattern).nth(1).unwrap_or("").trim()
                } else {
                    line.split(pattern)
                        .nth(1)
                        .and_then(|s| s.split(')').next())
                        .unwrap_or("")
                        .trim()
                };

                if !path_str.is_empty() {
                    return Some(ApprovalAction::FileWrite {
                        path: PathBuf::from(path_str),
                        lines_added: 0,  // Could parse from output if available
                        lines_removed: 0,
                    });
                }
            }
        }
        None
    }

    /// Detect git commit from Claude Code output
    fn detect_git_commit(&self, line: &str) -> Option<ApprovalAction> {
        // Claude Code outputs patterns like:
        // "git commit -m \"message\""
        // "[main abc1234] Commit message"
        if line.contains("git commit") || line.starts_with('[') && line.contains(']') {
            // Try to extract commit message
            let message = if let Some(start) = line.find("-m") {
                line[start + 2..]
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string()
            } else if line.starts_with('[') {
                line.split(']')
                    .nth(1)
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| "Commit".to_string())
            } else {
                "Commit".to_string()
            };

            return Some(ApprovalAction::GitCommit {
                message,
                files_changed: vec![],  // Could be parsed from git status
            });
        }
        None
    }

    /// Detect tool use for Always approval level
    fn detect_tool_use(&self, line: &str) -> Option<ApprovalAction> {
        // Detect any tool invocation patterns
        let tools = [
            ("Bash(", "Running command"),
            ("Read(", "Reading file"),
            ("Write(", "Writing file"),
            ("Edit(", "Editing file"),
            ("Glob(", "Finding files"),
            ("Grep(", "Searching files"),
        ];

        for (pattern, desc) in tools {
            if line.contains(pattern) {
                let tool_name = pattern.trim_end_matches('(');
                return Some(ApprovalAction::ToolUse {
                    tool: tool_name.to_string(),
                    description: desc.to_string(),
                });
            }
        }
        None
    }

    /// List all pending approvals
    pub fn list_approvals(&self) -> Vec<ApprovalInfo> {
        self.pending_approvals
            .values()
            .map(|req| ApprovalInfo {
                id: req.id,
                agent_id: req.agent_id,
                task_id: req.task_id,
                action: req.action.clone(),
                summary: req.action.summary(),
                created_at: req.created_at.timestamp(),
                context: req.context.clone(),
            })
            .collect()
    }

    /// Get pending approvals for a specific agent
    pub fn get_agent_pending_approval(&self, agent_id: AgentId) -> Option<&ApprovalRequest> {
        self.pending_approvals.values().find(|r| r.agent_id == agent_id)
    }

    /// Approve an action and resume the agent
    pub fn approve(&mut self, id: ApprovalId) -> Result<AgentId> {
        let request = self.pending_approvals.remove(&id)
            .ok_or_else(|| anyhow::anyhow!("Approval {} not found", id))?;

        let agent_id = request.agent_id;

        // Resume the agent by sending a confirmation
        // (In practice, Claude Code continues automatically after we un-pause)
        tracing::info!("Approved action for agent {}: {}", agent_id, request.action.summary());

        Ok(agent_id)
    }

    /// Reject an action and notify the agent
    pub fn reject(&mut self, id: ApprovalId, reason: Option<String>) -> Result<AgentId> {
        let request = self.pending_approvals.remove(&id)
            .ok_or_else(|| anyhow::anyhow!("Approval {} not found", id))?;

        let agent_id = request.agent_id;

        // Send rejection message to agent
        let msg = format!(
            "Action rejected: {}{}",
            request.action.summary(),
            reason.map(|r| format!(" ({})", r)).unwrap_or_default()
        );

        // Try to send the rejection message to the agent
        if let Err(e) = self.send_to_agent(agent_id, &msg) {
            tracing::warn!("Failed to send rejection to agent {}: {}", agent_id, e);
        }

        tracing::info!("Rejected action for agent {}: {}", agent_id, request.action.summary());

        Ok(agent_id)
    }

    /// Get count of pending approvals
    pub fn pending_approval_count(&self) -> usize {
        self.pending_approvals.len()
    }

    /// Clean up approval state when agent is killed
    fn cleanup_agent_approvals(&mut self, agent_id: AgentId) {
        self.agent_approval_levels.remove(&agent_id);
        self.agent_tasks.remove(&agent_id);
        self.pending_approvals.retain(|_, r| r.agent_id != agent_id);
    }

    /// Helper to send text to an agent's PTY
    fn send_to_agent(&self, id: AgentId, text: &str) -> Result<()> {
        if let Some(agent) = self.agents.get(&id) {
            if let Some(ref mut writer) = *agent.writer.lock().unwrap() {
                writeln!(writer, "{}", text)?;
            }
        }
        Ok(())
    }

    // --- Memory Event Detection ---

    /// Extract memory events from new output lines for all agents
    pub fn extract_memory_events(&mut self) -> Vec<(MemoryEntry, MemoryScope)> {
        let mut events = Vec::new();

        for (id, agent) in self.agents.iter_mut() {
            let start_idx = agent.memory_processed_idx;

            // Lock output history and process new lines
            let (new_lines, history_len) = if let Ok(history) = agent.output_history.lock() {
                let len = history.len();
                if start_idx >= len {
                    continue;
                }
                // Clone the lines we need to process
                let lines: Vec<String> = history[start_idx..].iter().map(|l| l.text.clone()).collect();
                (lines, len)
            } else {
                continue;
            };

            // Process new lines (outside of lock)
            for line in &new_lines {
                // Detect file discovery
                if let Some(content) = Self::detect_file_discovery(line) {
                    let entry = MemoryEntry::new(*id, content)
                        .with_tags(vec!["auto".to_string(), "file".to_string()]);
                    let scope = agent
                        .info
                        .namespace
                        .as_ref()
                        .map(|ns| MemoryScope::Namespace(ns.clone()))
                        .unwrap_or(MemoryScope::Agent(*id));
                    events.push((entry, scope));
                }

                // Detect errors
                if let Some(content) = Self::detect_error_encountered(line) {
                    let entry = MemoryEntry::new(*id, content)
                        .with_tags(vec!["auto".to_string(), "error".to_string()]);
                    let scope = agent
                        .info
                        .namespace
                        .as_ref()
                        .map(|ns| MemoryScope::Namespace(ns.clone()))
                        .unwrap_or(MemoryScope::Agent(*id));
                    events.push((entry, scope));
                }
            }

            // Update processed index
            agent.memory_processed_idx = history_len;
        }

        events
    }

    /// Detect file read/discovery from Claude Code output
    fn detect_file_discovery(line: &str) -> Option<MemoryContent> {
        // Claude Code outputs patterns like:
        // "Read file: /path/to/file.rs"
        // "Reading /path/to/file.rs"
        // "Viewing file: /path/to/file"
        // "📖 Read /path/to/file"
        let patterns = [
            ("Read file: ", ""),
            ("Read(", ")"),
            ("Reading ", ""),
            ("Viewing file: ", ""),
            ("📖 Read ", ""),
            ("Glob(", ")"),
            ("Search(", ")"),
        ];

        for (start, end) in patterns {
            if let Some(rest) = line.strip_prefix(start).or_else(|| {
                line.find(start).map(|i| &line[i + start.len()..])
            }) {
                let path_str = if end.is_empty() {
                    rest.split_whitespace().next().unwrap_or("")
                } else {
                    rest.split(end).next().unwrap_or("")
                };

                if !path_str.is_empty() && path_str.starts_with('/') {
                    return Some(MemoryContent::FileDiscovered {
                        path: PathBuf::from(path_str),
                        summary: format!("Agent read file: {}", path_str),
                        structure: None,
                    });
                }
            }
        }

        None
    }

    /// Detect error from Claude Code output
    fn detect_error_encountered(line: &str) -> Option<MemoryContent> {
        // Common error patterns
        let error_indicators = [
            "Error:",
            "error:",
            "ERROR:",
            "Failed:",
            "failed:",
            "FAILED:",
            "error[E",
            "Err(",
            "panic",
            "thread 'main' panicked",
            "Compilation failed",
            "Build failed",
            "Test failed",
        ];

        for indicator in error_indicators {
            if line.contains(indicator) {
                // Truncate long error messages
                let error_msg = if line.len() > 200 {
                    format!("{}...", &line[..200])
                } else {
                    line.to_string()
                };

                return Some(MemoryContent::ErrorEncountered {
                    error: error_msg,
                    resolution: None,
                    worked: false,
                });
            }
        }

        None
    }
}

// Note: Clone is needed for the daemon to share the supervisor across connections
// We use Arc<RwLock<Supervisor>> in the daemon, so this Clone is not actually used
// but is required by the original trait bounds
impl Clone for Supervisor {
    fn clone(&self) -> Self {
        // This is a shallow clone - agents and approvals are not cloned
        Self {
            config: self.config.clone(),
            agents: HashMap::new(),
            agent_counter: self.agent_counter,
            subscription_pool: self.subscription_pool.clone(),
            api_key_cache: self.api_key_cache.clone(),
            pending_approvals: HashMap::new(),
            agent_approval_levels: HashMap::new(),
            agent_tasks: HashMap::new(),
        }
    }
}
