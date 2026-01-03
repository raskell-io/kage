# Kage (影) - Agentic Work Orchestrator

## Vision

Kage is a local-first agentic work orchestrator written in Rust that enables Claude Code instances and other AI agents to work autonomously while the user is away. Unlike agent-deck (which is a session manager with a TUI), Kage is a **supervisor system** that:

1. Manages autonomous agent lifecycles
2. Enables agents to iterate on their own with guardrails
3. Shares context between agents
4. Organizes work across multiple repositories via namespaces
5. Runs locally or remotely (cloud, on-premise, next to LLM backends)

## Problems with agent-deck's Approach

| Issue | agent-deck | Kage Solution |
|-------|-----------|---------------|
| Passive management | Just wraps tmux sessions | Active supervisor with task queues |
| No autonomy | Agents wait for user input | Goal-oriented agents with iteration loops |
| No context sharing | Sessions are isolated | Event-sourced context bus |
| Single machine | tmux-bound to localhost | gRPC-based distributed architecture |
| No persistence | tmux sessions are ephemeral | Embedded file-based state persistence |
| Limited config | TOML for MCPs only | Layered config with runtime overrides |

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                           Kage Daemon                               │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────────┐  │
│  │  Supervisor  │  │ Context Bus  │  │     Namespace Registry   │  │
│  │  (Agent Mgr) │  │ (Event Store)│  │   (Multi-repo Catalog)   │  │
│  └──────┬───────┘  └──────┬───────┘  └────────────┬─────────────┘  │
│         │                 │                       │                 │
│  ┌──────┴─────────────────┴───────────────────────┴──────────────┐ │
│  │                     Task Scheduler                             │ │
│  │  (Work Queues, Priority, Dependencies, Checkpoints)            │ │
│  └───────────────────────────────────────────────────────────────┘ │
│         │                 │                       │                 │
│  ┌──────┴───────┐  ┌──────┴───────┐  ┌───────────┴───────────┐    │
│  │ Agent Pool   │  │ MCP Gateway  │  │   Plugin Runtime      │    │
│  │ (Executors)  │  │ (Shared MCPs)│  │   (WASM/Native)       │    │
│  └──────────────┘  └──────────────┘  └───────────────────────┘    │
└─────────────────────────────────────────────────────────────────────┘
                              │
                              │ gRPC / Unix Socket
                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│                         Kage CLI / TUI                              │
│  kage task add "fix auth bug in api-server"                         │
│  kage watch --namespace backend                                     │
│  kage context inject <agent-id> --from <other-agent-id>             │
└─────────────────────────────────────────────────────────────────────┘
```

## Core Components

### 1. Supervisor (Agent Manager)

The heart of Kage. Unlike agent-deck's passive session wrapper:

```rust
pub struct Supervisor {
    agents: HashMap<AgentId, AgentHandle>,
    policies: SupervisorPolicy,
    health_monitor: HealthMonitor,
}

pub struct SupervisorPolicy {
    max_concurrent_agents: usize,
    auto_restart: bool,
    iteration_limits: IterationLimits,
    approval_required: ApprovalLevel,  // None, OnWrite, OnCommit, Always
    resource_limits: ResourceLimits,
}
```

**Key capabilities:**
- Spawn agents with specific goals/tasks
- Monitor agent health and progress
- Automatically restart failed agents
- Enforce iteration limits and guardrails
- Pause/resume agents for user review

### 2. Memory System (Two-Tier)

Enables agents to share context, learn from each other, and build persistent knowledge:

```rust
/// Two-tier memory architecture
pub struct MemorySystem {
    working: WorkingMemory,      // Fast, in-memory, session-scoped
    longterm: LongTermMemory,    // Persistent, append-only logs
    bus: ContextBus,             // Real-time pub/sub
}

/// Memory scopes control visibility
pub enum MemoryScope {
    Agent(AgentId),              // Private to one agent
    Namespace(String),           // Shared within namespace
    Global,                      // Available to all (opt-in)
}

/// What agents can remember and share
pub enum MemoryEntry {
    // Code understanding
    FileDiscovered { path: PathBuf, summary: String, structure: Option<AST> },
    PatternLearned { pattern: String, examples: Vec<String>, confidence: f32 },
    DependencyMapped { from: String, to: String, relationship: DepType },

    // Task context
    ErrorEncountered { error: String, resolution: Option<String>, worked: bool },
    DecisionMade { decision: String, reasoning: String, alternatives: Vec<String> },
    TaskCompleted { task_id: TaskId, summary: String, artifacts: Vec<PathBuf> },

    // Collaboration
    InsightShared { topic: String, content: String, from_agent: AgentId },
    QuestionAsked { question: String, answer: Option<String> },
}

/// Real-time context sharing
pub struct ContextBus {
    subscriptions: HashMap<AgentId, Vec<Subscription>>,
    tx: broadcast::Sender<MemoryEntry>,
}
```

**Key capabilities:**
- **Working Memory**: Fast session context, auto-expires
- **Long-Term Memory**: Append-only logs, indexed, queryable
- **Scoped Sharing**: agent → namespace → global
- **Real-Time Bus**: Agents see discoveries as they happen
- **Replay**: New agents catch up on namespace history
- **Injection**: Push context from one agent to another

### 3. Namespace Registry

Multi-repository organization:

```rust
pub struct NamespaceRegistry {
    namespaces: HashMap<String, Namespace>,
}

pub struct Namespace {
    name: String,                       // e.g., "backend", "frontend", "infra"
    repositories: Vec<Repository>,
    shared_context: ContextScope,
    default_agent_config: AgentConfig,
}

pub struct Repository {
    name: String,
    path: PathBuf,                      // Local path
    remote: Option<String>,             // Git remote URL
    branch_policy: BranchPolicy,
}
```

**Example configuration:**
```toml
[namespaces.backend]
repositories = [
    { name = "api-server", path = "~/code/api-server" },
    { name = "auth-service", path = "~/code/auth-service" },
    { name = "shared-lib", path = "~/code/shared-lib" },
]
context_sharing = "full"  # Agents in this namespace share all context

[namespaces.frontend]
repositories = [
    { name = "web-app", path = "~/code/web-app" },
    { name = "mobile-app", path = "~/code/mobile-app" },
]
context_sharing = "selective"  # Only share explicitly marked context
```

### 4. Task Scheduler

Advanced work queue management:

```rust
pub struct TaskScheduler {
    queues: HashMap<Priority, VecDeque<Task>>,
    dependencies: DependencyGraph,
    checkpoints: HashMap<TaskId, Checkpoint>,
}

pub struct Task {
    id: TaskId,
    goal: String,                       // Natural language goal
    namespace: Option<String>,
    repository: Option<String>,
    priority: Priority,
    dependencies: Vec<TaskId>,
    constraints: TaskConstraints,
    iteration_config: IterationConfig,
}

pub struct IterationConfig {
    max_iterations: u32,
    checkpoint_interval: u32,           // Save state every N iterations
    success_criteria: Vec<Criterion>,   // How to know when done
    abort_criteria: Vec<Criterion>,     // When to stop and ask for help
}
```

### 5. Agent Providers (Extensible)

Trait-based architecture with Claude Code as primary provider:

```rust
/// Core trait for agent providers - implement this to add new AI agents
pub trait AgentProvider: Send + Sync {
    fn name(&self) -> &str;
    fn spawn(&self, config: AgentConfig) -> Result<AgentHandle>;
    fn detect_status(&self, output: &str) -> AgentStatus;
    fn supports_mcp(&self) -> bool;
}

/// Claude Code - the primary, first-class provider
pub struct ClaudeCodeProvider {
    default_model: String,              // opus, sonnet, haiku
    default_flags: Vec<String>,
}

impl AgentProvider for ClaudeCodeProvider {
    fn name(&self) -> &str { "claude-code" }
    fn supports_mcp(&self) -> bool { true }
    // ... full implementation
}

/// Agent handle returned by providers
pub struct AgentHandle {
    process: Child,
    pty: PtyMaster,                     // For interactive control
    input_tx: Sender<String>,
    output_rx: Receiver<AgentOutput>,
    state: AgentState,
}
```

**Built-in providers:**
- `ClaudeCodeProvider` - Primary, full MCP support
- Future: `OpenAIProvider`, `GeminiProvider`, `AiderProvider`

### 6. Configuration System

Layered, everything configurable:

```
~/.config/kage/
├── config.toml           # Global defaults
├── namespaces/
│   ├── backend.toml      # Namespace-specific config
│   └── frontend.toml
├── agents/
│   ├── claude-code.toml  # Agent type defaults
│   └── custom-agent.toml
├── mcps/
│   ├── global.toml       # Shared MCP definitions
│   └── per-project/
└── policies/
    ├── approval.toml     # Approval workflows
    └── resources.toml    # Resource limits
```

**Config precedence:** CLI flags > Environment > Task-specific > Namespace > Global

## Deployment Modes

**Single Binary, Zero Dependencies** - Download and run. First launch shows onboarding TUI.

### Local Mode (Default, Single-User)
```bash
kage
# First run: TUI onboarding wizard
# After setup: Unix socket at /tmp/kage.sock
# State stored in ~/.local/share/kage/state/
```

### Server Mode (Multi-User Optional)
```bash
kage server start --listen 0.0.0.0:50051 --tls
# gRPC server with TLS
# Single-user by default (no auth needed for localhost)

# Opt-in to multi-user:
kage server enable-multiuser
kage user invite alice@example.com
```

### Team/VPS Deployment
```bash
# On VPS:
kage server start --listen 0.0.0.0:50051 --tls --multiuser

# Users connect:
kage connect vps.example.com:50051

# Everyone can see shared agents, private agents stay private
kage watch --all  # Admin sees everything
```

## Key Differentiators from agent-deck

### 1. Autonomous Iteration
```bash
# agent-deck: Start session, manually interact
agent-deck add . -c claude

# Kage: Define goal, let agent iterate
kage task add "Implement user authentication with JWT" \
    --namespace backend \
    --repo api-server \
    --max-iterations 10 \
    --checkpoint-every 2 \
    --approval on-commit
```

### 2. Context Injection
```bash
# Inject context from one agent to another
kage context inject agent-123 --from agent-456 --filter "auth patterns"

# Agent-456's discoveries about auth patterns are now available to agent-123
```

### 3. Cross-Repo Awareness
```bash
# Task spans multiple repos
kage task add "Update API client in web-app to match new endpoints in api-server" \
    --namespace backend,frontend \
    --repos api-server,web-app
```

### 4. Checkpoint & Resume
```bash
# Agent hit iteration limit, saved checkpoint
kage task resume task-789 --extend-iterations 5

# Or review checkpoint and provide guidance
kage checkpoint review task-789
kage task resume task-789 --guidance "Try using the existing auth middleware"
```

### 5. Memory & Context Sharing
```bash
# See what agents have learned
kage memory list --namespace backend
kage memory search "authentication patterns"

# Share specific knowledge globally
kage memory share mem-123 --scope global

# Inject context from one agent to another
kage context inject agent-456 --from agent-123 --type patterns

# Watch memory stream in real-time
kage memory watch --namespace backend
```

## Technology Choices

| Component | Choice | Rationale |
|-----------|--------|-----------|
| Language | Rust | Performance, safety, async ecosystem |
| Async Runtime | Tokio | Industry standard, excellent performance |
| RPC | tonic (gRPC) | Type-safe, streaming, bi-directional |
| Storage | Embedded append-only log + redb | Zero dependencies, single binary |
| Serialization | serde + TOML/MessagePack | TOML for config, MessagePack for state |
| TUI | ratatui | Mature Rust TUI framework |
| PTY | portable-pty | Cross-platform pseudo-terminal |
| CLI | clap | Best-in-class CLI parsing |
| Plugin System | wasmtime (WASM) | Sandboxed extensibility |

### Storage Design (No External DB)

```
~/.local/share/kage/
├── state/
│   ├── agents.redb          # Agent registry (embedded key-value)
│   ├── tasks.redb           # Task queue and state
│   ├── events/              # Append-only event logs
│   │   ├── 2024-01-15.log   # Daily rotation
│   │   └── 2024-01-16.log
│   └── checkpoints/         # Task checkpoints (MessagePack)
│       └── <task-id>.ckpt
├── logs/
│   └── daemon.log
└── run/
    └── kage.sock            # Unix socket (or /tmp/kage.sock)
```

**Why redb?**
- Pure Rust, compiles into single binary
- ACID transactions
- Zero external dependencies
- Memory-mapped for performance
- Designed for embedded use

## Implementation Phases

### Phase 1: Foundation (MVP)
- Single binary daemon with Unix socket IPC
- Basic supervisor (spawn/kill/list agents)
- Embedded redb state persistence
- CLI with basic commands
- Single namespace support

### Phase 2: Autonomy
- Task scheduler with iteration loops
- Checkpoint system (MessagePack files)
- Success/abort criteria evaluation
- Approval workflow

### Phase 3: Context Sharing
- Append-only event log implementation
- Context bus with in-memory subscriptions
- Cross-agent context injection
- Namespace-scoped context

### Phase 4: Distribution
- gRPC server mode (same binary, different flags)
- State directory sync for multi-node
- Remote agent execution via SSH
- File-based state replication

### Phase 5: Ecosystem
- Plugin system (WASM)
- MCP gateway with pooling
- TUI dashboard
- IDE extensions

## Files to Create

```
kage/
├── Cargo.toml                 # Single binary, no workspace needed
├── build.rs                   # Proto compilation
├── .claude/
│   ├── CLAUDE.md              # Project conventions for Claude
│   └── ROADMAP.md             # Detailed implementation roadmap
├── src/
│   ├── main.rs                # CLI + daemon entry point (single binary)
│   ├── lib.rs
│   ├── daemon/
│   │   ├── mod.rs
│   │   ├── supervisor.rs
│   │   ├── scheduler.rs
│   │   └── health.rs
│   ├── agent/
│   │   ├── mod.rs
│   │   ├── executor.rs
│   │   ├── types.rs
│   │   └── claude_code.rs
│   ├── context/
│   │   ├── mod.rs
│   │   ├── bus.rs
│   │   └── events.rs
│   ├── storage/               # Embedded storage layer
│   │   ├── mod.rs
│   │   ├── kv.rs              # redb wrapper
│   │   ├── eventlog.rs        # Append-only log
│   │   └── checkpoint.rs      # MessagePack checkpoints
│   ├── namespace/
│   │   ├── mod.rs
│   │   └── registry.rs
│   ├── task/
│   │   ├── mod.rs
│   │   └── queue.rs
│   ├── config/
│   │   ├── mod.rs
│   │   └── loader.rs
│   ├── cli/
│   │   ├── mod.rs
│   │   └── commands/
│   ├── tui/
│   │   ├── mod.rs
│   │   └── views/
│   └── rpc/
│       ├── mod.rs
│       └── service.rs
├── proto/
│   └── kage.proto
└── tests/
    └── integration/
```

## Summary

Kage is not a rewrite of agent-deck—it's a fundamentally different approach:

- **agent-deck** = Session manager (passive, TUI-focused, tmux wrapper)
- **Kage** = Work orchestrator (active, goal-oriented, supervisor pattern)

The key insight is that managing sessions is not enough. Users want agents that can **work autonomously** toward goals, **share what they learn**, **checkpoint progress**, and **scale across repositories and machines**.
