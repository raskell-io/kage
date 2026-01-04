//! MCP (Model Context Protocol) server management
//!
//! Manages MCP server configurations and lifecycle for Claude Code sessions.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// MCP server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    /// Server name/identifier
    pub name: String,
    /// Command to run the server
    pub command: String,
    /// Command arguments
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Whether the server is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Server description
    #[serde(default)]
    pub description: String,
    /// Server category for grouping
    #[serde(default)]
    pub category: McpCategory,
}

fn default_true() -> bool {
    true
}

/// MCP server category for grouping in UI
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum McpCategory {
    /// Memory and context servers
    Memory,
    /// File system access
    Filesystem,
    /// Search and retrieval
    Search,
    /// Development tools
    #[default]
    Tools,
    /// Custom/other
    Custom,
}

impl McpCategory {
    pub fn name(&self) -> &'static str {
        match self {
            McpCategory::Memory => "Memory",
            McpCategory::Filesystem => "Filesystem",
            McpCategory::Search => "Search",
            McpCategory::Tools => "Tools",
            McpCategory::Custom => "Custom",
        }
    }

    pub fn all() -> &'static [McpCategory] {
        &[
            McpCategory::Memory,
            McpCategory::Filesystem,
            McpCategory::Search,
            McpCategory::Tools,
            McpCategory::Custom,
        ]
    }
}

/// Scope for MCP configuration changes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum McpScope {
    /// Apply to current session only
    #[default]
    Local,
    /// Apply globally to all sessions
    Global,
}

impl McpScope {
    pub fn name(&self) -> &'static str {
        match self {
            McpScope::Local => "LOCAL",
            McpScope::Global => "GLOBAL",
        }
    }

    pub fn toggle(&self) -> Self {
        match self {
            McpScope::Local => McpScope::Global,
            McpScope::Global => McpScope::Local,
        }
    }
}

/// MCP configuration manager
#[derive(Debug, Clone, Default)]
pub struct McpConfig {
    /// Available MCP servers
    pub servers: Vec<McpServer>,
    /// Global overrides (server name -> enabled)
    pub global_overrides: HashMap<String, bool>,
    /// Per-session overrides (agent_id -> (server name -> enabled))
    pub session_overrides: HashMap<String, HashMap<String, bool>>,
}

impl McpConfig {
    /// Create new MCP config with default servers
    pub fn new() -> Self {
        Self {
            servers: Self::default_servers(),
            global_overrides: HashMap::new(),
            session_overrides: HashMap::new(),
        }
    }

    /// Get default MCP servers
    pub fn default_servers() -> Vec<McpServer> {
        vec![
            McpServer {
                name: "memory".to_string(),
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "@anthropics/mcp-memory".to_string()],
                env: HashMap::new(),
                enabled: true,
                description: "Persistent memory across conversations".to_string(),
                category: McpCategory::Memory,
            },
            McpServer {
                name: "filesystem".to_string(),
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "@anthropics/mcp-filesystem".to_string()],
                env: HashMap::new(),
                enabled: true,
                description: "File system access and operations".to_string(),
                category: McpCategory::Filesystem,
            },
            McpServer {
                name: "exa".to_string(),
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "exa-mcp-server".to_string()],
                env: HashMap::new(),
                enabled: false,
                description: "Web search via Exa API".to_string(),
                category: McpCategory::Search,
            },
            McpServer {
                name: "github".to_string(),
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "@anthropics/mcp-github".to_string()],
                env: HashMap::new(),
                enabled: false,
                description: "GitHub repository access".to_string(),
                category: McpCategory::Tools,
            },
            McpServer {
                name: "postgres".to_string(),
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "@anthropics/mcp-postgres".to_string()],
                env: HashMap::new(),
                enabled: false,
                description: "PostgreSQL database access".to_string(),
                category: McpCategory::Tools,
            },
            McpServer {
                name: "brave-search".to_string(),
                command: "npx".to_string(),
                args: vec!["-y".to_string(), "@anthropics/mcp-brave-search".to_string()],
                env: HashMap::new(),
                enabled: false,
                description: "Web search via Brave Search API".to_string(),
                category: McpCategory::Search,
            },
        ]
    }

    /// Check if a server is enabled for a session
    pub fn is_enabled(&self, server_name: &str, session_id: Option<&str>) -> bool {
        // Check session override first
        if let Some(sid) = session_id {
            if let Some(session_overrides) = self.session_overrides.get(sid) {
                if let Some(&enabled) = session_overrides.get(server_name) {
                    return enabled;
                }
            }
        }

        // Check global override
        if let Some(&enabled) = self.global_overrides.get(server_name) {
            return enabled;
        }

        // Fall back to default
        self.servers
            .iter()
            .find(|s| s.name == server_name)
            .map(|s| s.enabled)
            .unwrap_or(false)
    }

    /// Toggle a server's enabled state
    pub fn toggle(&mut self, server_name: &str, scope: McpScope, session_id: Option<&str>) {
        let current = self.is_enabled(server_name, session_id);

        match scope {
            McpScope::Global => {
                self.global_overrides.insert(server_name.to_string(), !current);
            }
            McpScope::Local => {
                if let Some(sid) = session_id {
                    let session_overrides = self.session_overrides
                        .entry(sid.to_string())
                        .or_insert_with(HashMap::new);
                    session_overrides.insert(server_name.to_string(), !current);
                }
            }
        }
    }

    /// Get servers grouped by category
    pub fn by_category(&self) -> HashMap<McpCategory, Vec<&McpServer>> {
        let mut grouped: HashMap<McpCategory, Vec<&McpServer>> = HashMap::new();
        for server in &self.servers {
            grouped.entry(server.category).or_default().push(server);
        }
        grouped
    }

    /// Load from Claude Code settings file
    pub fn load_from_claude_settings() -> Option<Self> {
        let config_path = dirs_config_path()?;
        if config_path.exists() {
            // Parse Claude Code's settings.json
            let content = std::fs::read_to_string(&config_path).ok()?;
            let json: serde_json::Value = serde_json::from_str(&content).ok()?;

            let mut servers = Vec::new();
            if let Some(mcp_servers) = json.get("mcpServers").and_then(|v| v.as_object()) {
                for (name, config) in mcp_servers {
                    let command = config.get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or("npx")
                        .to_string();

                    let args: Vec<String> = config.get("args")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                        .unwrap_or_default();

                    let env: HashMap<String, String> = config.get("env")
                        .and_then(|v| v.as_object())
                        .map(|obj| obj.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect())
                        .unwrap_or_default();

                    servers.push(McpServer {
                        name: name.clone(),
                        command,
                        args,
                        env,
                        enabled: true,
                        description: String::new(),
                        category: McpCategory::Custom,
                    });
                }
            }

            if !servers.is_empty() {
                return Some(Self {
                    servers,
                    global_overrides: HashMap::new(),
                    session_overrides: HashMap::new(),
                });
            }
        }
        None
    }

    /// Save to Claude Code settings file
    pub fn save_to_claude_settings(&self, scope: McpScope) -> anyhow::Result<()> {
        let config_path = match scope {
            McpScope::Global => dirs_config_path(),
            McpScope::Local => None, // Local saves to session-specific path
        };

        if let Some(path) = config_path {
            let mut json: serde_json::Value = if path.exists() {
                let content = std::fs::read_to_string(&path)?;
                serde_json::from_str(&content)?
            } else {
                serde_json::json!({})
            };

            let mut mcp_servers = serde_json::Map::new();
            for server in &self.servers {
                if self.is_enabled(&server.name, None) {
                    let mut server_config = serde_json::Map::new();
                    server_config.insert("command".to_string(), serde_json::json!(server.command));
                    server_config.insert("args".to_string(), serde_json::json!(server.args));
                    if !server.env.is_empty() {
                        server_config.insert("env".to_string(), serde_json::json!(server.env));
                    }
                    mcp_servers.insert(server.name.clone(), serde_json::Value::Object(server_config));
                }
            }

            json["mcpServers"] = serde_json::Value::Object(mcp_servers);

            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            std::fs::write(&path, serde_json::to_string_pretty(&json)?)?;
        }

        Ok(())
    }
}

/// Get Claude Code settings path
fn dirs_config_path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().join(".claude").join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_servers() {
        let config = McpConfig::new();
        assert!(!config.servers.is_empty());
        assert!(config.servers.iter().any(|s| s.name == "memory"));
    }

    #[test]
    fn test_toggle() {
        let mut config = McpConfig::new();
        let initial = config.is_enabled("memory", None);
        config.toggle("memory", McpScope::Global, None);
        assert_eq!(config.is_enabled("memory", None), !initial);
    }

    #[test]
    fn test_session_override() {
        let mut config = McpConfig::new();
        config.toggle("memory", McpScope::Local, Some("session-1"));

        // Session 1 should be toggled
        let session1_enabled = config.is_enabled("memory", Some("session-1"));

        // Session 2 should use default
        let session2_enabled = config.is_enabled("memory", Some("session-2"));
        let default_enabled = config.is_enabled("memory", None);

        assert_ne!(session1_enabled, default_enabled);
        assert_eq!(session2_enabled, default_enabled);
    }
}
