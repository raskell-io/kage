# Kage (影) - Claude Code Project Guide

## Project Overview

Kage is a local-first agentic work orchestrator written in Rust. It enables **Claude Code** agents to work autonomously while the user is away, share context with each other, and scale across multiple repositories.

**Primary Agent:** Claude Code (first-class support)
**Extensible:** Trait-based architecture supports other providers (OpenAI, Gemini, etc.)

**Core Philosophy:** Shadow agents working invisibly, executing with precision.

## Architecture Principles

### 1. Supervisor Pattern
Kage acts as a supervisor for AI agents, not just a session manager. The supervisor:
- Spawns agents with specific goals
- Monitors health and progress
- Enforces iteration limits and guardrails
- Enables checkpoint/resume workflows

### 2. Event Sourcing for Context
All agent context is stored as immutable events. This enables:
- Full audit trails
- Cross-agent context sharing
- Replay for debugging
- Time-travel queries

### 3. Namespace-Based Organization
Repositories are grouped into namespaces (e.g., "backend", "frontend"). Agents within a namespace can share context and coordinate work.

### 4. Configuration Layers
Config precedence: CLI flags > Environment > Task-specific > Namespace > Global

### 5. Multi-Subscription Pooling
Kage supports registering multiple Claude Code subscriptions (e.g., "Claude Code 20x Max") to enable true parallel productivity scaling:
- **Subscription Pool**: Register multiple API keys/accounts in the subscription registry
- **Intelligent Routing**: Automatically route agent requests to available subscriptions
- **Rate Limit Management**: Track rate limits per subscription, rotate to avoid throttling
- **Load Balancing**: Distribute workload evenly across subscriptions
- **Fallback & Retry**: Automatic failover when a subscription hits limits
- **Usage Tracking**: Per-subscription usage metrics for cost management
- **Subscription Health**: Monitor subscription status, alert on quota exhaustion

This enables users to scale their development capacity linearly by adding more subscriptions.

## Code Conventions

### Rust Style
- Follow Rust 2021 edition idioms
- Use `thiserror` for error types, `anyhow` for application errors
- Prefer `async/await` with Tokio runtime
- Use `tracing` for structured logging, not `log` or `println!`
- All public APIs must have doc comments
- Use `#[must_use]` on functions returning values that shouldn't be ignored

### Error Handling
```rust
// Define domain-specific errors with thiserror
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Agent {id} not found")]
    NotFound { id: AgentId },

    #[error("Agent {id} failed to start: {reason}")]
    StartFailed { id: AgentId, reason: String },

    #[error("Iteration limit exceeded for agent {id}")]
    IterationLimitExceeded { id: AgentId },
}

// Use anyhow::Result in application code
pub async fn run_task(task: Task) -> anyhow::Result<TaskResult> {
    // ...
}
```

### Async Patterns
```rust
// Use tokio::select! for concurrent operations
tokio::select! {
    result = agent.wait() => handle_completion(result),
    _ = shutdown_rx.recv() => handle_shutdown(),
    _ = tokio::time::sleep(timeout) => handle_timeout(),
}

// Use channels for inter-component communication
let (tx, rx) = tokio::sync::mpsc::channel(100);
```

### Module Structure
```
src/
├── main.rs           # CLI entry point (single binary)
├── lib.rs            # Library root, re-exports
├── daemon/           # Daemon process logic
│   ├── supervisor.rs # Agent lifecycle management
│   └── scheduler.rs  # Task scheduling
├── agent/            # Agent execution
│   ├── provider.rs   # AgentProvider trait
│   ├── claude.rs     # ClaudeCodeProvider (primary)
│   └── handle.rs     # AgentHandle, PTY management
├── subscription/     # Multi-subscription management
│   ├── pool.rs       # Subscription pool & routing
│   ├── registry.rs   # Subscription CRUD operations
│   ├── health.rs     # Health monitoring & alerts
│   └── usage.rs      # Usage tracking & metrics
├── memory/           # Two-tier memory system
│   ├── working.rs    # In-memory session context
│   ├── longterm.rs   # Append-only persistent storage
│   ├── bus.rs        # Real-time pub/sub
│   ├── entry.rs      # MemoryEntry types
│   └── query.rs      # Search and filtering
├── namespace/        # Namespace registry
├── task/             # Task queue and checkpoints
├── secrets/          # OS keychain integration
├── config/           # Layered configuration
├── cli/              # CLI commands
├── tui/              # Terminal UI + onboarding
├── storage/          # redb wrapper, event logs
└── rpc/              # gRPC server/client
```

### Naming Conventions
- Types: `PascalCase` (e.g., `AgentExecutor`, `TaskScheduler`)
- Functions: `snake_case` (e.g., `spawn_agent`, `get_context`)
- Constants: `SCREAMING_SNAKE_CASE` (e.g., `MAX_ITERATIONS`)
- Modules: `snake_case` (e.g., `context_bus`, `agent_pool`)

### Testing
- Unit tests in the same file as the code (`#[cfg(test)]`)
- Integration tests in `tests/` directory
- Use `tokio::test` for async tests
- Mock external dependencies with traits

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_agent_spawn() {
        let supervisor = Supervisor::new(test_config());
        let agent_id = supervisor.spawn(AgentConfig::default()).await.unwrap();
        assert!(supervisor.is_running(agent_id).await);
    }
}
```

## Key Types

### IDs
All IDs are type-safe wrappers:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AgentId(Ulid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(Ulid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NamespaceId(Ulid);
```

### Configuration
Use serde with TOML:
```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub daemon: DaemonConfig,

    #[serde(default)]
    pub namespaces: HashMap<String, NamespaceConfig>,
}
```

## File Locations

| Purpose | Path |
|---------|------|
| Global config | `~/.config/kage/config.toml` |
| State directory | `~/.local/share/kage/state/` |
| Agent registry | `~/.local/share/kage/state/agents.redb` |
| Task state | `~/.local/share/kage/state/tasks.redb` |
| Subscription state | `~/.local/share/kage/state/subscriptions.redb` |
| Subscription usage | `~/.local/share/kage/state/usage/` |
| Event logs | `~/.local/share/kage/state/events/` |
| Checkpoints | `~/.local/share/kage/state/checkpoints/` |
| Daemon logs | `~/.local/share/kage/logs/` |
| Unix socket | `/tmp/kage.sock` or `$XDG_RUNTIME_DIR/kage.sock` |
| Per-project config | `.kage/config.toml` |

Note: Subscription API keys are stored in OS keychain, never in state files.

## Single Binary Architecture

Kage is distributed as a **single static binary** with zero runtime dependencies:
- No database server, no external services
- **Linux**: Fully static with musl target
- **macOS/Windows**: Just works
- All storage embedded (redb + append-only logs)
- First run: TUI onboarding wizard
- Single-user default, multi-user opt-in

## Dependencies (Cargo.toml)

Core dependencies to use:
- `tokio` - Async runtime (features: full)
- `tonic` - gRPC framework
- `prost` - Protocol buffers
- `serde` - Serialization
- `toml` - Config parsing
- `rmp-serde` - MessagePack for state serialization
- `redb` - Embedded key-value store (pure Rust, no dependencies)
- `keyring` - OS keychain integration (macOS Keychain, Linux Secret Service, Windows Credential Manager)
- `clap` - CLI parsing (derive feature)
- `ratatui` - TUI framework
- `crossterm` - Terminal control
- `portable-pty` - Pseudo-terminal
- `tracing` + `tracing-subscriber` - Logging
- `thiserror` + `anyhow` - Error handling
- `ulid` - Unique IDs
- `notify` - File watching
- `directories` - XDG paths

### Why These Choices?
- **redb over SQLite**: Pure Rust, compiles into binary, no C dependencies
- **MessagePack over JSON**: Compact binary format, faster serialization
- **Append-only logs**: Simple, crash-safe event storage

## Common Tasks

### Adding a New CLI Command
1. Create command module in `src/cli/commands/`
2. Add to command enum in `src/cli/mod.rs`
3. Implement handler that calls daemon via RPC

### Adding a New Agent Provider
1. Implement `AgentProvider` trait in `src/agent/providers/`
   ```rust
   pub trait AgentProvider: Send + Sync {
       fn name(&self) -> &str;
       fn spawn(&self, config: AgentConfig) -> Result<AgentHandle>;
       fn detect_status(&self, output: &str) -> AgentStatus;
       fn supports_mcp(&self) -> bool;
   }
   ```
2. Register in provider registry (`src/agent/registry.rs`)
3. Add configuration schema to `AgentConfig`
4. See `ClaudeCodeProvider` as reference implementation

### Adding a Context Event Type
1. Add variant to `ContextEvent` enum
2. Update event serialization
3. Add any relevant subscriptions

## Commit Message Format
```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`
Scopes: `daemon`, `agent`, `context`, `task`, `cli`, `tui`, `config`

Example:
```
feat(agent): add checkpoint save on iteration limit

Agents now automatically save state when hitting iteration limits,
allowing users to review progress and resume with guidance.

Closes #42
```

## Performance Considerations

- Use `Arc<T>` for shared ownership, avoid cloning large structures
- Prefer `&str` over `String` in function signatures where possible
- Use `tokio::sync::RwLock` for read-heavy shared state
- Batch storage writes where possible (redb transactions)
- Use streaming for large context transfers
- Memory-map large files rather than loading entirely

## Security

- Never store API keys in state files
- Use OS keychain for sensitive credentials
- Validate all external input (task descriptions, config files)
- Run agents in isolated working directories
- Implement rate limiting for RPC endpoints
