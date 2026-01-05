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
  <a href="#enterprise-ready">
    <img alt="Enterprise Ready" src="https://img.shields.io/badge/Enterprise-Ready-10b981?style=for-the-badge">
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

> **Not your typical agent platform.** Kage is a CLI/TUI-first tool built for developers and power users who live in the terminal. No web dashboards, no drag-and-drop workflows, no managed cloud services. Just a single binary that gives you complete control over your AI agents through the command line.

## Quick Start

```bash
# Install via script
curl -fsSL https://kage.raskell.io/install.sh | sh

# Or via Cargo
cargo install raskell-kage

# Or via Docker
docker pull ghcr.io/raskell-io/kage:latest

# Start the daemon and launch the dashboard
kage daemon start
kage dashboard
```

## Features

| Feature | Description |
|---------|-------------|
| **Supervisor Pattern** | Spawn agents with goals, monitor progress, enforce iteration limits |
| **Task Management** | Define success/abort criteria, checkpoints, dependencies, and approval workflows |
| **Approval Workflows** | Control agent actions with configurable gates (none, on-write, on-commit, always) |
| **Two-Tier Memory** | Working memory + persistent long-term storage with cross-agent sharing |
| **Multi-Subscription** | Pool multiple Claude subscriptions for parallel scaling with intelligent routing |
| **Namespace Organization** | Group repositories, share context, scope configuration |
| **Secrets Management** | Secure credential storage via OS keychain (never plaintext) |
| **Interactive Dashboard** | Real-time TUI for monitoring agents, tasks, and approvals |
| **Single Binary** | Zero dependencies, just download and run |

## Why Kage?

Running a single Claude Code session is straightforward. But what happens when you need agents working across multiple repositories? When you want to step away and let them work autonomously? When they need to share what they've learned?

Kage solves these problems with a supervisor architecture designed for developers who prefer the terminal over browser-based interfaces. Unlike platforms like Copilot Workspace, Devin, or other agentic solutions that focus on web UIs and managed experiences, Kage is:

- **CLI/TUI-first** — Every feature accessible from the command line, scriptable and composable
- **Local and self-hosted** — Your machine, your data, your control. No cloud lock-in
- **Transparent** — See exactly what agents are doing, no black-box abstractions
- **Spawn agents with specific goals** — Give your agents clear objectives and let them work
- **Monitor health and progress** — Track what your agents are doing in real-time
- **Enforce iteration limits** — Set guardrails to prevent runaway execution
- **Checkpoint and resume** — Save state, review progress, provide guidance, and continue
- **Approval gates** — Control what agents can do autonomously vs. what needs your sign-off

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                       Kage Daemon                            │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐          │
│  │   Agent 1   │  │   Agent 2   │  │   Agent 3   │   ...    │
│  │  (Claude)   │  │  (Claude)   │  │  (Claude)   │          │
│  └─────────────┘  └─────────────┘  └─────────────┘          │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐ │
│  │                   Task Scheduler                        │ │
│  │  Queue • Checkpoints • Success/Abort Criteria          │ │
│  └────────────────────────────────────────────────────────┘ │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐ │
│  │                    Memory Store                         │ │
│  │  ┌──────────────────┐  ┌──────────────────┐            │ │
│  │  │  Working Memory  │  │  Long-term Memory │            │ │
│  │  └──────────────────┘  └──────────────────┘            │ │
│  └────────────────────────────────────────────────────────┘ │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐ │
│  │                 Subscription Pool                       │ │
│  │  ┌────┐ ┌────┐ ┌────┐ ┌────┐                           │ │
│  │  │ S1 │ │ S2 │ │ S3 │ │ S4 │  (health + rate limiting) │ │
│  │  └────┘ └────┘ └────┘ └────┘                           │ │
│  └────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

## CLI Reference

### Dashboard

```bash
kage                    # Launch interactive dashboard
kage dashboard          # Same as above
```

### Daemon

```bash
kage daemon start       # Start the daemon
kage daemon stop        # Stop the daemon
kage daemon status      # Check daemon status
```

### Agents

```bash
kage agent spawn                      # Spawn a new agent
kage agent spawn --prompt "Fix bug"   # Spawn with initial goal
kage agent list                       # List active agents
kage agent attach <id>                # Attach to an agent
kage agent detach                     # Detach from agent
kage agent kill <id>                  # Kill an agent
kage agent status <id>                # Show agent status
```

### Tasks

```bash
# Create a task with guardrails
kage task add "Refactor auth module" \
    --max-iterations 20 \
    --checkpoint-every 5 \
    --approval on-commit \
    --success-on "tests-pass" \
    --abort-on "error-count:5:compilation failed"

kage task list                        # List tasks
kage task show <id>                   # Show task details
kage task pause <id>                  # Pause a task
kage task resume <id> --guidance "Focus on edge cases"
kage task cancel <id>                 # Cancel a task

# Checkpoint management
kage task checkpoint list <id>        # List checkpoints
kage task checkpoint show <id>        # Show checkpoint details
```

### Approvals

```bash
kage approval list                    # List pending approvals
kage approval show <id>               # View approval details
kage approval approve <id>            # Approve an action
kage approval approve all             # Approve all pending
kage approval reject <id> --reason "Use existing utils"
```

### Memory

```bash
kage memory list                      # List memory entries
kage memory search "auth patterns"    # Search memories
kage memory show <id>                 # Show entry details
kage memory share <id> --scope global # Share to wider scope
kage memory watch                     # Stream updates
kage memory export -o backup.json     # Export memory
```

### Subscriptions

```bash
# Add subscriptions for parallel scaling
kage subscription add --name "work" --priority 150
kage subscription add --name "personal" --priority 100

kage subscription list                # List subscriptions
kage subscription status              # Pool overview
kage subscription usage --period week # Usage stats

# Configure routing
kage subscription route --namespace backend --subscriptions "work,personal"
kage subscription route --priority high --subscriptions "work"
```

### Namespaces

```bash
kage namespace create backend         # Create namespace
kage namespace add-repo backend ~/code/api
kage namespace add-repo backend ~/code/auth
kage namespace list                   # List namespaces
kage namespace show backend           # Show details
```

### Secrets

```bash
kage secret set API_KEY               # Set global secret
kage secret set DB_PASS --scope namespace:backend
kage secret list                      # List secret names
kage secret delete API_KEY            # Delete secret
```

## Task Features

### Success Criteria

Automatically complete tasks when conditions are met:

```bash
--success-on "tests-pass"                    # Test suite passes
--success-on "pattern:Implementation complete"  # Output matches
--success-on "file-exists:src/auth/jwt.ts"   # File created
--success-on "command:npm run build"         # Command succeeds
```

### Abort Criteria

Automatically stop tasks when problems occur:

```bash
--abort-on "error-count:5:TypeError"         # Too many errors
--abort-on "repeated-output:3:10"            # Stuck in loop
--abort-on "no-progress:5"                   # No file changes
```

### Approval Levels

| Level | Description |
|-------|-------------|
| `none` | Full autonomy |
| `on-write` | Approve file modifications |
| `on-commit` | Approve git commits (default) |
| `always` | Approve all actions |

## Enterprise Ready

Kage is built with enterprise requirements in mind while staying true to local-first principles:

| Capability | Description |
|------------|-------------|
| **Multi-User Tenancy** | gRPC server mode enables team-wide deployments with isolated namespaces |
| **Credential Management** | Secure storage via OS keychain (macOS Keychain, Linux Secret Service, Windows Credential Manager) |
| **Cloud Storage** | Optional cloud backends for memory and context history (S3, GCS, Azure Blob) |
| **Audit Trails** | Immutable event logs for compliance and debugging |
| **Access Control** | Namespace-level permissions and API key scoping |

Local-first by default, cloud-enabled when you need it.

## Configuration

Kage uses TOML for configuration with layered precedence:

```
CLI flags > Environment > Task-specific > Namespace > Global
```

```toml
# ~/.config/kage/config.toml

[daemon]
socket_path = "/tmp/kage.sock"
max_agents = 10

[claude]
model = "sonnet"
max_iterations = 30
approval = "on-commit"

[tasks]
default_approval = "on-commit"
default_checkpoint_every = 3

[namespaces.backend]
repos = ["~/code/api", "~/code/auth"]
memory_sharing = "full"

[subscriptions.routing]
backend = ["work-main", "work-backup"]
high = ["premium"]
```

## File Locations

| Purpose | Path |
|---------|------|
| Global config | `~/.config/kage/config.toml` |
| State directory | `~/.local/share/kage/state/` |
| Daemon logs | `~/.local/share/kage/logs/` |
| Checkpoints | `~/.local/share/kage/state/checkpoints/` |
| Unix socket | `/tmp/kage.sock` |
| Per-project config | `.kage/config.toml` |

Secrets (API keys) are stored in your OS keychain, never in plaintext files.

## Built With

- **Rust** — Memory-safe, fast, and reliable
- **Tokio** — Async runtime
- **redb** — Embedded key-value store (pure Rust, zero dependencies)
- **tonic** — gRPC framework
- **ratatui** — Terminal UI
- **portable-pty** — Cross-platform PTY support

## Documentation

Full documentation is available at [kage.raskell.io/docs](https://kage.raskell.io/docs).

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

Apache 2.0 — see [LICENSE](LICENSE) for details.

## Related Projects

- [Claude Code](https://www.anthropic.com/claude-code) — The primary AI agent supported by Kage
- [Sentinel](https://github.com/raskell-io/sentinel) — Security-first reverse proxy from raskell.io
