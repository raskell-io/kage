//! Kage (影) - Shadow agents for autonomous code work
//!
//! A local-first agentic work orchestrator written in Rust.
//!
//! # Architecture
//!
//! - **Daemon**: Supervisor managing agent lifecycles
//! - **Agent**: Claude Code provider and execution
//! - **Memory**: Two-tier context sharing system
//! - **Task**: Goal-oriented work with checkpoints
//! - **Namespace**: Multi-repository organization
//!
//! # Example
//!
//! ```ignore
//! use kage::daemon::Supervisor;
//! use kage::agent::ClaudeCodeProvider;
//!
//! let supervisor = Supervisor::new(config).await?;
//! let agent_id = supervisor.spawn(task).await?;
//! ```

pub mod agent;
pub mod cli;
pub mod config;
pub mod daemon;
pub mod memory;
pub mod namespace;
pub mod secrets;
pub mod storage;
pub mod subscription;
pub mod task;

#[cfg(feature = "tui")]
pub mod tui;

#[cfg(feature = "server")]
pub mod rpc;

// Re-export commonly used types
pub use agent::{AgentHandle, AgentId, AgentProvider, AgentStatus};
pub use config::Config;
pub use daemon::Supervisor;
pub use memory::{MemoryEntry, MemoryScope, MemorySystem};
pub use namespace::{Namespace, NamespaceId};
pub use subscription::{Subscription, SubscriptionId, SubscriptionPool};
pub use task::{CheckpointStore, Task, TaskId, TaskRegistry, TaskScheduler, TaskStatus};

/// Kage version
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
