//! Configuration loading and management
//!
//! Kage uses a layered configuration system:
//! 1. Global: ~/.config/kage/config.toml
//! 2. Namespace: ~/.config/kage/namespaces/<name>/config.toml
//! 3. Repository: <repo>/.kage/config.toml
//! 4. Environment variables
//! 5. CLI flags
//!
//! Later layers override earlier ones.

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Global Kage configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Daemon configuration
    #[serde(default)]
    pub daemon: DaemonConfig,

    /// Default Claude settings
    #[serde(default)]
    pub claude: ClaudeConfig,

    /// Memory/context settings
    #[serde(default)]
    pub memory: MemoryConfig,

    /// Namespace definitions
    #[serde(default)]
    pub namespaces: std::collections::HashMap<String, NamespaceConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            daemon: DaemonConfig::default(),
            claude: ClaudeConfig::default(),
            memory: MemoryConfig::default(),
            namespaces: std::collections::HashMap::new(),
        }
    }
}

/// Daemon-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Unix socket path
    #[serde(default = "default_socket_path")]
    pub socket_path: PathBuf,

    /// State directory
    #[serde(default = "default_state_dir")]
    pub state_dir: PathBuf,

    /// Log directory
    #[serde(default = "default_log_dir")]
    pub log_dir: PathBuf,

    /// Maximum concurrent agents
    #[serde(default = "default_max_agents")]
    pub max_agents: usize,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            socket_path: default_socket_path(),
            state_dir: default_state_dir(),
            log_dir: default_log_dir(),
            max_agents: default_max_agents(),
        }
    }
}

/// Claude Code specific settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeConfig {
    /// Default model (opus, sonnet, haiku)
    #[serde(default = "default_model")]
    pub model: String,

    /// Default CLI flags
    #[serde(default)]
    pub flags: Vec<String>,

    /// Default iteration limit
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,

    /// Default approval level
    #[serde(default = "default_approval")]
    pub approval: String,

    /// Path to custom CLAUDE.md
    pub claude_md: Option<PathBuf>,
}

impl Default for ClaudeConfig {
    fn default() -> Self {
        Self {
            model: default_model(),
            flags: Vec::new(),
            max_iterations: default_max_iterations(),
            approval: default_approval(),
            claude_md: None,
        }
    }
}

/// Memory/context configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Working memory TTL (e.g., "24h")
    #[serde(default = "default_working_memory_ttl")]
    pub working_memory_ttl: String,

    /// Long-term memory retention (e.g., "90d")
    #[serde(default = "default_long_term_retention")]
    pub long_term_retention: String,

    /// Max entries per namespace
    #[serde(default = "default_max_entries")]
    pub max_entries_per_namespace: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            working_memory_ttl: default_working_memory_ttl(),
            long_term_retention: default_long_term_retention(),
            max_entries_per_namespace: default_max_entries(),
        }
    }
}

/// Namespace configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceConfig {
    /// Repositories in this namespace
    #[serde(default)]
    pub repositories: Vec<RepositoryConfig>,

    /// Memory sharing mode (full, explicit, none)
    #[serde(default = "default_memory_sharing")]
    pub memory_sharing: String,

    /// Namespace-specific Claude settings
    pub claude: Option<ClaudeConfig>,
}

/// Repository configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryConfig {
    /// Repository name
    pub name: String,

    /// Local path
    pub path: PathBuf,

    /// Git remote URL (optional)
    pub remote: Option<String>,
}

// Default value functions
fn default_socket_path() -> PathBuf {
    if let Some(runtime_dir) = dirs::runtime_dir() {
        runtime_dir.join("kage.sock")
    } else {
        PathBuf::from("/tmp/kage.sock")
    }
}

fn default_state_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kage")
        .join("state")
}

fn default_log_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kage")
        .join("logs")
}

fn default_max_agents() -> usize {
    10
}

fn default_model() -> String {
    "sonnet".to_string()
}

fn default_max_iterations() -> u32 {
    10
}

fn default_approval() -> String {
    "on-commit".to_string()
}

fn default_working_memory_ttl() -> String {
    "24h".to_string()
}

fn default_long_term_retention() -> String {
    "90d".to_string()
}

fn default_max_entries() -> usize {
    10000
}

fn default_memory_sharing() -> String {
    "full".to_string()
}

/// Check if this is the first run (no initialization marker exists)
pub fn is_first_run() -> bool {
    let initialized_marker = dirs::config_dir()
        .map(|d| d.join(".initialized"))
        .unwrap_or_else(|| PathBuf::from("~/.config/kage/.initialized"));

    !initialized_marker.exists()
}

/// Mark the first run as complete (creates initialization marker)
pub fn mark_initialized() -> Result<()> {
    if let Some(config_dir) = dirs::config_dir() {
        std::fs::create_dir_all(&config_dir)?;
        let marker = config_dir.join(".initialized");
        std::fs::write(marker, "")?;
    }
    Ok(())
}

/// Load configuration from all sources
pub fn load() -> Result<Config> {
    // TODO: Implement full config loading with layering
    Ok(Config::default())
}

/// Get the config directory path
pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kage")
}

// Use dirs crate for XDG paths
mod dirs {
    use std::path::PathBuf;

    pub fn config_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("io", "raskell", "kage")
            .map(|d| d.config_dir().to_path_buf())
    }

    pub fn data_local_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("io", "raskell", "kage")
            .map(|d| d.data_local_dir().to_path_buf())
    }

    pub fn runtime_dir() -> Option<PathBuf> {
        directories::ProjectDirs::from("io", "raskell", "kage")
            .and_then(|d| d.runtime_dir().map(|p| p.to_path_buf()))
    }
}
