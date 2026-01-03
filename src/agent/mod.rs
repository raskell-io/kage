//! Agent execution and provider traits
//!
//! Kage uses a trait-based architecture for agent providers.
//! Claude Code is the primary provider with first-class support.

mod handle;
mod provider;
pub mod providers;

pub use handle::AgentHandle;
pub use provider::{AgentConfig, AgentProvider, AgentStatus};

use serde::{Deserialize, Serialize};
use ulid::Ulid;

/// Unique identifier for an agent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(Ulid);

impl AgentId {
    /// Generate a new agent ID
    pub fn new() -> Self {
        Self(Ulid::new())
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for AgentId {
    type Err = ulid::DecodeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Ulid::from_string(s)?))
    }
}
