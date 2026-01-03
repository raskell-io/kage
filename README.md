<div align="center">

<h1 align="center">
  <img src=".github/static/kage-mascot.png" alt="kage mascot" width="96" />
  <br>
  Kage (影)
</h1>

<p align="center">
  <em>A local-first agentic work orchestrator.</em><br>
  <em>Shadow agents working invisibly, executing with precision.</em>
</p>

<p align="center">
  <a href="https://www.rust-lang.org/">
    <img alt="Rust" src="https://img.shields.io/badge/Rust-stable-000000?logo=rust&logoColor=white&style=for-the-badge">
  </a>
  <a href="https://www.anthropic.com/claude-code">
    <img alt="Claude Code" src="https://img.shields.io/badge/Claude%20Code-Native-e07b53?style=for-the-badge">
  </a>
  <a href="LICENSE">
    <img alt="License" src="https://img.shields.io/badge/License-Apache--2.0-a78bfa?style=for-the-badge">
  </a>
</p>

<p align="center">
  <a href="https://kage.raskell.io/docs">Documentation</a> •
  <a href="https://github.com/raskell-io/kage/discussions">Discussions</a> •
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

</div>

---

Kage enables **Claude Code** agents to work autonomously while you're away, share context with each other, and scale across multiple repositories.

## Quick Start

```bash
# Install
curl -fsSL https://kage.raskell.io/install.sh | sh

# Or via Cargo
cargo install kage

# Run
kage daemon start
kage agent spawn --repo ~/code/myproject
```

## Features

| Feature | Description |
|---------|-------------|
| **Supervisor Pattern** | Spawn agents with goals, monitor progress, enforce limits |
| **Event Sourcing** | Immutable audit trails, replay, time-travel queries |
| **Namespace Organization** | Group repos, share context across agents |
| **Two-Tier Memory** | Working memory + persistent long-term storage |
| **Multi-Subscription** | Pool multiple Claude subscriptions for parallel scaling |
| **Single Binary** | Zero dependencies, just download and run |

## Why Kage

Running a single Claude Code session is straightforward. But what happens when you need agents working across multiple repositories? When you want to step away and let them work autonomously? When they need to share what they've learned?

Kage solves these problems with a supervisor architecture:

- **Spawn agents with specific goals** — Give your agents clear objectives and let them work
- **Monitor health and progress** — Track what your agents are doing in real-time
- **Enforce iteration limits** — Set guardrails to prevent runaway execution
- **Checkpoint and resume** — Save state, review progress, provide guidance, and continue

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

## Configuration

Kage uses TOML for configuration with layered precedence:

```
CLI flags > Environment > Task-specific > Namespace > Global
```

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

## File Locations

| Purpose | Path |
|---------|------|
| Global config | `~/.config/kage/config.toml` |
| State directory | `~/.local/share/kage/state/` |
| Daemon logs | `~/.local/share/kage/logs/` |
| Unix socket | `/tmp/kage.sock` |
| Per-project config | `.kage/config.toml` |

Secrets (API keys) are stored in your OS keychain, never in plaintext files.

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
