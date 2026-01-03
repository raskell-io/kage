# Kage (影)

A local-first agentic work orchestrator that enables Claude Code agents to work autonomously, share context, and scale across multiple repositories.

> Shadow agents working invisibly, executing with precision.

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](LICENSE)

## Overview

Kage acts as a **supervisor** for AI agents, not just a session manager. It enables you to:

- **Spawn agents with specific goals** — Give your agents clear objectives and let them work
- **Monitor health and progress** — Track what your agents are doing in real-time
- **Enforce iteration limits** — Set guardrails to prevent runaway execution
- **Checkpoint and resume** — Save state, review progress, provide guidance, and continue

## Features

### Supervisor Pattern
Kage manages the complete lifecycle of AI agents. Spawn agents, monitor their health, enforce limits, and enable checkpoint/resume workflows for human-in-the-loop collaboration.

### Event Sourcing
All agent context is stored as immutable events, enabling full audit trails, cross-agent context sharing, replay for debugging, and time-travel queries.

### Namespace Organization
Group repositories into namespaces (e.g., "backend", "frontend"). Agents within a namespace automatically share context and coordinate work.

### Two-Tier Memory
Working memory for fast, session-scoped context. Long-term memory persisted to disk for discoveries, patterns, and decisions that survive restarts.

### Multi-Subscription Pooling
Register multiple Claude Code subscriptions to scale your development capacity. Kage handles intelligent routing, rate limit management, and automatic failover.

### Single Binary
Distributed as a single static binary with zero runtime dependencies. No database server, no external services. Just download and run.

## Installation

### Quick Install

```bash
curl -fsSL https://kage.raskell.io/install.sh | sh
```

### Cargo

```bash
cargo install kage
```

### From Source

```bash
git clone https://github.com/raskell-io/kage.git
cd kage
cargo build --release
```

## Quick Start

```bash
# Start the daemon
kage daemon start

# Spawn an agent in your project
kage agent spawn --repo ~/code/myproject

# List running agents
kage agent list

# Attach to an agent
kage agent attach <agent-id>

# Detach: Ctrl+A, then D

# Assign a task
kage task add "Fix all TypeScript errors in the project"
```

## Configuration

Kage uses TOML for configuration with layered precedence:

```
CLI flags > Environment > Task-specific > Namespace > Global
```

### Global Config

```toml
# ~/.config/kage/config.toml

[daemon]
socket = "/tmp/kage.sock"
log_level = "info"

[defaults]
max_iterations = 50
checkpoint_interval = 10

[namespaces.backend]
repos = [
    "~/code/api-server",
    "~/code/auth-service",
]
memory_sharing = "full"
```

### Per-Project Config

```toml
# .kage/config.toml

max_iterations = 100
```

## File Locations

| Purpose | Path |
|---------|------|
| Global config | `~/.config/kage/config.toml` |
| State directory | `~/.local/share/kage/state/` |
| Daemon logs | `~/.local/share/kage/logs/` |
| Unix socket | `/tmp/kage.sock` |
| Per-project config | `.kage/config.toml` |

Secrets (API keys) are stored in your OS keychain, never in plaintext files.

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                   Kage Daemon                       │
├─────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐ │
│  │   Agent 1   │  │   Agent 2   │  │   Agent 3   │ │
│  │  (Claude)   │  │  (Claude)   │  │  (Claude)   │ │
│  └─────────────┘  └─────────────┘  └─────────────┘ │
│                                                     │
│  ┌─────────────────────────────────────────────┐   │
│  │              Memory Store                    │   │
│  │  ┌──────────────┐  ┌──────────────┐         │   │
│  │  │   Working    │  │  Long-term   │         │   │
│  │  │   Memory     │  │   Memory     │         │   │
│  │  └──────────────┘  └──────────────┘         │   │
│  └─────────────────────────────────────────────┘   │
│                                                     │
│  ┌─────────────────────────────────────────────┐   │
│  │           Subscription Pool                  │   │
│  │  ┌────┐ ┌────┐ ┌────┐ ┌────┐               │   │
│  │  │ S1 │ │ S2 │ │ S3 │ │ S4 │  ...          │   │
│  │  └────┘ └────┘ └────┘ └────┘               │   │
│  └─────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────┘
```

## CLI Reference

### Daemon

```bash
kage daemon start       # Start the daemon
kage daemon stop        # Stop the daemon
kage daemon status      # Check daemon status
```

### Agents

```bash
kage agent spawn        # Spawn a new agent
kage agent list         # List active agents
kage agent attach <id>  # Attach to an agent
kage agent kill <id>    # Kill an agent
```

### Tasks

```bash
kage task add "<goal>"  # Add a new task
kage task list          # List tasks
kage task status <id>   # Check task status
kage task resume <id>   # Resume a paused task
```

### Memory

```bash
kage memory search "<query>"  # Search memories
kage memory list              # List recent memories
kage memory inject <target> --from <source>  # Share context
```

### Subscriptions

```bash
kage subscription add     # Add a subscription
kage subscription list    # List subscriptions
kage subscription usage   # View usage stats
```

## Built With

- **Rust** — Memory-safe, fast, and reliable
- **Tokio** — Async runtime
- **redb** — Embedded key-value store (pure Rust)
- **tonic** — gRPC framework
- **ratatui** — Terminal UI

## Documentation

Full documentation is available at [kage.raskell.io/docs](https://kage.raskell.io/docs).

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

Apache 2.0 — see [LICENSE](LICENSE) for details.

## Related Projects

- [Claude Code](https://www.anthropic.com/claude-code) — The primary AI agent supported by Kage
- [Sentinel](https://github.com/raskell-io/sentinel) — Security-first reverse proxy from raskell.io
