# Kage Roadmap

## Overview

This roadmap outlines the implementation phases for Kage, from MVP to full ecosystem. Each phase builds on the previous, delivering usable functionality incrementally.

---

## Phase 1: Foundation (MVP)

**Goal:** Basic daemon that can spawn and manage Claude Code agents via CLI.

### 1.1 Project Scaffolding
- [ ] Initialize Cargo workspace
- [ ] Set up directory structure (`src/`, `proto/`, `tests/`)
- [ ] Configure dependencies in `Cargo.toml`
- [ ] Set up `tracing` for structured logging
- [ ] Create basic error types with `thiserror`

### 1.2 Configuration System
- [ ] Define `Config` struct with serde
- [ ] Implement TOML loader with defaults
- [ ] Support XDG directory paths (`directories` crate)
- [ ] Layer: global config → namespace config → CLI overrides
- [ ] Validate configuration on load

### 1.3 Daemon Core
- [ ] Create daemon binary entry point
- [ ] Implement Unix socket listener (Tokio)
- [ ] Define simple JSON-RPC or custom protocol for IPC
- [ ] Implement graceful shutdown handling
- [ ] PID file management

### 1.4 Agent Provider Trait & Claude Code Integration
- [ ] Define `AgentProvider` trait for extensibility:
  ```rust
  pub trait AgentProvider: Send + Sync {
      fn name(&self) -> &str;
      fn spawn(&self, config: AgentConfig) -> Result<AgentHandle>;
      fn detect_status(&self, output: &str) -> AgentStatus;
      fn supports_mcp(&self) -> bool;
  }
  ```
- [ ] Implement `ClaudeCodeProvider` as primary/default provider
- [ ] PTY allocation with `portable-pty`
- [ ] Capture stdout/stderr streams
- [ ] Send input to agent process
- [ ] Track agent state (Starting, Running, Waiting, Stopped, Failed)
- [ ] Parse Claude Code output for status detection

### 1.5 Basic Supervisor
- [ ] `Supervisor` struct managing `HashMap<AgentId, AgentHandle>`
- [ ] Commands: `spawn`, `kill`, `list`, `attach`, `detach`
- [ ] Basic health monitoring (process alive check)
- [ ] Auto-cleanup of dead agents

### 1.6 Embedded State Persistence
- [ ] Set up `redb` for key-value storage (pure Rust, no dependencies)
- [ ] Define tables: agents, tasks, config snapshots
- [ ] Persist agent metadata on spawn
- [ ] Restore agent registry on daemon restart
- [ ] Simple versioning for storage format changes

### 1.7 Secrets & Credentials Management
- [ ] Native integration with OS keychain (keyring crate)
  - [ ] macOS Keychain
  - [ ] Linux Secret Service (GNOME Keyring, KWallet)
  - [ ] Windows Credential Manager
- [ ] `kage secret set <name> <value>` - Store secret
- [ ] `kage secret list` - List secret names (not values)
- [ ] `kage secret delete <name>` - Remove secret
- [ ] Secret scoping: global, namespace, or repository
- [ ] Environment variable injection for agent processes
- [ ] Never persist secrets to disk (always use OS keychain)

### 1.8 Layered Claude Settings
- [ ] Global Claude settings: `~/.config/kage/claude.toml`
- [ ] Namespace Claude settings: `~/.config/kage/namespaces/<name>/claude.toml`
- [ ] Repository Claude settings: `<repo>/.kage/claude.toml`
- [ ] Merge order: global → namespace → repo (later overrides earlier)
- [ ] Settings include:
  - [ ] Model selection (opus, sonnet, haiku)
  - [ ] Default flags (--dangerously-skip-permissions, etc.)
  - [ ] MCP server toggles
  - [ ] Iteration limits and approval levels
  - [ ] Custom system prompts / CLAUDE.md paths

### 1.12 Multi-Subscription Management (Claude Code 20x Max)
**Goal:** Enable users to register and manage multiple Claude Code subscriptions for parallel productivity scaling.

- [ ] Define `Subscription` struct:
  ```rust
  pub struct Subscription {
      id: SubscriptionId,
      name: String,                    // User-friendly name ("work-account", "personal")
      api_key_ref: String,             // Reference to keychain entry
      provider: ProviderType,          // ClaudeCode, API, etc.
      status: SubscriptionStatus,      // Active, RateLimited, Exhausted, Disabled
      rate_limit_state: RateLimitState,
      usage: UsageMetrics,
      priority: u8,                    // Higher priority used first
      tags: Vec<String>,               // For routing rules
  }
  ```
- [ ] Subscription Registry:
  - [ ] `kage subscription add --name <name>` - Add subscription (prompts for API key)
  - [ ] `kage subscription list` - List all subscriptions with status
  - [ ] `kage subscription remove <name>` - Remove subscription
  - [ ] `kage subscription status` - Show pool health overview
  - [ ] `kage subscription enable/disable <name>` - Toggle subscription
- [ ] Subscription Pool Manager:
  - [ ] Track rate limits per subscription (requests/min, tokens/day)
  - [ ] Automatic cooldown when rate limited (back-off timer)
  - [ ] Round-robin with health-aware routing
  - [ ] Priority-based selection (use preferred subscription first)
  - [ ] Sticky sessions (keep agent on same subscription when possible)
- [ ] Routing Rules:
  ```toml
  [subscriptions.routing]
  # Route by namespace
  backend = ["work-primary", "work-secondary"]
  experiments = ["personal"]

  # Route by task priority
  high_priority = ["work-primary"]
  batch_jobs = ["personal", "work-secondary"]
  ```
- [ ] Usage Tracking:
  - [ ] Per-subscription request count, token usage, error rate
  - [ ] Daily/weekly/monthly aggregates
  - [ ] `kage subscription usage [--period 7d]` - View usage stats
  - [ ] Export usage data for billing reconciliation
- [ ] Health Monitoring:
  - [ ] Periodic health checks (can subscription authenticate?)
  - [ ] Alert on subscription quota exhaustion
  - [ ] Alert on elevated error rates
  - [ ] Auto-disable failing subscriptions
- [ ] Credential Storage:
  - [ ] All API keys stored in OS keychain (never on disk)
  - [ ] Support for environment variable overrides
  - [ ] Support for credential rotation without agent restart

**Deliverable:** `kage subscription add --name team-1 && kage subscription add --name team-2` then agents automatically use both subscriptions in parallel

### 1.9 First-Run Onboarding TUI
- [ ] Detect first run (no config exists)
- [ ] Launch TUI splash screen with Kage branding
- [ ] Interactive setup wizard:
  - [ ] Claude Code API key setup (store in keychain)
  - [ ] Default namespace creation
  - [ ] First repository registration
  - [ ] Quick explanation of core concepts
- [ ] Feature tour with animated demos
- [ ] "What's New" screen on version upgrades
- [ ] Skip option for power users (`kage --skip-onboarding`)

### 1.10 CLI (Basic)
- [ ] `kage daemon start` - Start daemon in background
- [ ] `kage daemon stop` - Stop daemon
- [ ] `kage agent spawn` - Spawn new agent
- [ ] `kage agent list` - List active agents
- [ ] `kage agent attach <id>` - Attach to agent (interactive)
- [ ] `kage agent kill <id>` - Kill agent

### 1.11 Single Namespace Support
- [ ] Define `Namespace` struct
- [ ] Load namespace from config
- [ ] Associate agents with namespace
- [ ] Filter agents by namespace in CLI

**Deliverable:** `kage daemon start && kage agent spawn --repo ~/code/myproject && kage agent attach <id>`

---

## Phase 2: Autonomy

**Goal:** Agents can work toward goals autonomously with iteration loops and checkpoints.

### 2.1 Task System
- [ ] Define `Task` struct with goal, constraints, iteration config
- [ ] Implement `TaskId` generation (ULID)
- [ ] `kage task add "<goal>"` command
- [ ] Task states: Pending, Running, Paused, Completed, Failed
- [ ] Associate tasks with agents

### 2.2 Task Scheduler
- [ ] Priority queues for tasks
- [ ] Dependency graph (task A depends on task B)
- [ ] Assign tasks to available agents
- [ ] Fair scheduling across namespaces

### 2.3 Iteration Loop
- [ ] Define `IterationConfig` (max iterations, checkpoint interval)
- [ ] Implement iteration counter per task
- [ ] Detect "iteration complete" from agent output
- [ ] Pause on iteration limit, notify user

### 2.4 Checkpoint System
- [ ] Define `Checkpoint` struct (task state, conversation context)
- [ ] Save checkpoint as MessagePack file on interval
- [ ] Save checkpoint on iteration limit/pause
- [ ] `kage checkpoint list <task-id>`
- [ ] `kage checkpoint show <checkpoint-id>`

### 2.5 Resume with Guidance
- [ ] `kage task resume <task-id>` command
- [ ] `--extend-iterations N` flag
- [ ] `--guidance "<text>"` flag to inject context
- [ ] Restore from checkpoint, continue execution

### 2.6 Success/Abort Criteria
- [ ] Define `Criterion` enum (pattern match, file exists, tests pass)
- [ ] Evaluate criteria after each iteration
- [ ] Auto-complete task on success criteria met
- [ ] Auto-pause on abort criteria met
- [ ] Notify user of completion/abort

### 2.7 Approval Workflow
- [ ] Define `ApprovalLevel` (None, OnWrite, OnCommit, Always)
- [ ] Hook into agent actions (file write, git commit)
- [ ] Pause agent and queue approval request
- [ ] `kage approval list` / `kage approval approve <id>`
- [ ] Auto-timeout with configurable behavior

**Deliverable:** `kage task add "Fix all TypeScript errors" --max-iterations 10 --approval on-commit`

---

## Phase 3: Context Sharing & Memory

**Goal:** Agents can share discoveries, learn from each other, and build persistent memory.

### 3.1 Memory Store Architecture
- [ ] Two-tier memory system:
  - **Working Memory**: Current session context (in-memory, fast)
  - **Long-Term Memory**: Persisted discoveries (append-only logs)
- [ ] Memory scopes:
  - `agent` - Private to one agent
  - `namespace` - Shared within namespace
  - `global` - Available to all agents (opt-in)
- [ ] Memory types:
  ```rust
  pub enum MemoryEntry {
      // Code understanding
      FileDiscovered { path: PathBuf, summary: String, structure: Option<AST> },
      PatternLearned { pattern: String, examples: Vec<String>, confidence: f32 },
      DependencyMapped { from: String, to: String, relationship: DepType },

      // Task context
      ErrorEncountered { error: String, resolution: Option<String>, worked: bool },
      DecisionMade { decision: String, reasoning: String, alternatives: Vec<String> },
      TaskCompleted { task_id: TaskId, summary: String, artifacts: Vec<Path> },

      // Collaboration
      InsightShared { topic: String, content: String, from_agent: AgentId },
      QuestionAsked { question: String, answer: Option<String>, from_agent: AgentId },
  }
  ```

### 3.2 Event Log Persistence
- [ ] Append-only log files (daily rotation)
- [ ] MessagePack serialization (compact, fast)
- [ ] Write-ahead logging for crash safety
- [ ] Automatic compaction of old logs
- [ ] Event IDs (ULID) for ordering and deduplication
- [ ] Index files for efficient queries:
  - By agent, namespace, memory type
  - By time range
  - By topic/keyword

### 3.3 Context Bus (Real-Time)
- [ ] In-memory pub/sub for live events
- [ ] Agents auto-subscribe to their namespace
- [ ] Subscription filters by memory type
- [ ] Replay historical events on agent spawn
- [ ] Configurable replay depth (last N events, last N hours)

### 3.4 Memory Sharing Controls
- [ ] Per-namespace settings:
  ```toml
  [namespaces.backend]
  memory_sharing = "full"       # All memories shared automatically

  [namespaces.experiments]
  memory_sharing = "explicit"   # Only share what's marked
  ```
- [ ] Per-memory visibility:
  - `kage memory share <id> --scope namespace`
  - `kage memory share <id> --scope global`
- [ ] Cross-namespace sharing with explicit opt-in
- [ ] Memory access audit log

### 3.5 Context Injection
- [ ] `kage context inject <target-agent> --from <source-agent>`
- [ ] Filters:
  - `--type patterns` - Only learned patterns
  - `--type errors` - Error resolutions
  - `--topic "auth"` - Keyword filter
  - `--since 1h` - Time-based
- [ ] Automatic summarization for large context
- [ ] Inject methods:
  - As Claude CLAUDE.md additions
  - As conversation context
  - As files in working directory

### 3.6 Memory Queries
- [ ] `kage memory search "<query>"` - Full-text search
- [ ] `kage memory list --namespace backend --type patterns`
- [ ] `kage memory show <id>` - View specific entry
- [ ] `kage memory export --format json|markdown`
- [ ] Future: semantic/vector search

### 3.7 Memory Lifecycle
- [ ] Configurable retention policies:
  ```toml
  [memory]
  working_memory_ttl = "24h"
  long_term_retention = "90d"
  max_entries_per_namespace = 10000
  ```
- [ ] Automatic summarization of old context
- [ ] Manual cleanup: `kage memory prune --older-than 30d`
- [ ] Archive to external storage (S3, etc.) for compliance

**Deliverable:** Agent working on auth automatically sees patterns, errors, and decisions from other agents in same namespace.

---

## Phase 4: Distribution

**Goal:** Kage can run remotely and coordinate across machines (same single binary).

### 4.1 gRPC Protocol
- [ ] Define `kage.proto` with all RPC methods
- [ ] Generate Rust code with `tonic-build`
- [ ] Implement gRPC server in daemon
- [ ] Implement gRPC client in CLI
- [ ] Support both Unix socket and TCP

### 4.2 TLS & Authentication
- [ ] Generate self-signed certs for dev
- [ ] Load certs from config
- [ ] Implement token-based auth
- [ ] mTLS for production deployments

### 4.3 Multi-User Support (Optional, Zero-Config Default)
**Design principle:** Single-user is default. No multi-user config needed unless you want it.

- [ ] Implicit "default user" when running locally (no auth needed)
- [ ] `kage server enable-multiuser` to opt-in
- [ ] User management:
  - [ ] `kage user invite <email>` - Generate invite token
  - [ ] `kage user list` / `kage user remove`
  - [ ] Roles: `admin`, `member`, `viewer`
- [ ] Namespace visibility controls:
  - [ ] `private` (default) - Only owner sees
  - [ ] `team` - Members can view, owner controls
  - [ ] `shared` - Full collaboration
- [ ] Global activity feed: `kage watch --all` (admin only)
- [ ] Per-user secrets isolation (separate keychain entries)
- [ ] Graceful single→multi-user migration (existing data stays yours)

### 4.4 State Directory Sync
- [ ] State directory can be on network filesystem (NFS, etc.)
- [ ] Optional: rsync/rclone sync to remote storage
- [ ] File locking for concurrent access
- [ ] Conflict resolution for multi-writer scenarios

### 4.4 Remote Agent Execution
- [ ] Define remote agent config (SSH, container, etc.)
- [ ] Implement SSH executor for remote machines
- [ ] Stream agent output over gRPC
- [ ] Handle network interruptions gracefully

### 4.5 Multi-Node Coordination
- [ ] Simple leader election via file lock
- [ ] In-memory event bus with gRPC streaming between nodes
- [ ] Agent affinity to nodes
- [ ] Cross-node context replication via shared state directory

### 4.6 Hybrid Mode
- [ ] CLI connects to remote primary
- [ ] Fallback to local daemon if remote unavailable
- [ ] Sync state between local and remote on reconnect
- [ ] CRDT-based conflict resolution for offline edits

**Deliverable:** `kage --remote cloud.example.com task add "Deploy to staging"`

---

## Phase 5: Ecosystem

**Goal:** Extensibility, integrations, and polish.

### 5.1 Plugin System
- [ ] Define plugin trait for custom agent drivers
- [ ] WASM runtime with `wasmtime` for sandboxed plugins
- [ ] Plugin discovery from config
- [ ] Plugin lifecycle management

### 5.2 MCP Gateway
- [ ] Shared MCP server pool (like agent-deck)
- [ ] Unix socket proxying
- [ ] Per-agent MCP configuration
- [ ] MCP health monitoring
- [ ] Dynamic MCP enable/disable

### 5.3 TUI Dashboard
- [ ] Implement with `ratatui`
- [ ] Views: agent list, task queue, context stream
- [ ] Real-time updates via daemon subscription
- [ ] Keyboard shortcuts for common actions
- [ ] Mouse support

### 5.4 Web Dashboard (Optional)
- [ ] REST API alongside gRPC
- [ ] Simple SPA for browser access
- [ ] WebSocket for real-time updates
- [ ] OAuth integration for team access

### 5.5 IDE Extensions
- [ ] VS Code extension for Kage commands
- [ ] Status bar with agent activity
- [ ] Inline context from Kage
- [ ] JetBrains plugin (stretch)

### 5.6 Observability
- [ ] Prometheus metrics endpoint
- [ ] OpenTelemetry tracing integration
- [ ] Structured log export
- [ ] Alerting hooks (webhook, Slack, etc.)

### 5.7 Additional Agent Providers
- [ ] OpenAI Codex / ChatGPT CLI (when available)
- [ ] Google Gemini CLI
- [ ] Aider integration
- [ ] Cursor integration
- [ ] Custom agent templates via config

**Deliverable:** Full-featured orchestrator with plugins, TUI, and integrations.

---

## Stretch Goals

### S1. Semantic Context Search
- [ ] Embed context events with local model (llama.cpp integration)
- [ ] Vector storage with pure Rust HNSW
- [ ] Semantic similarity search over context events

### S2. Agent Collaboration Protocols
- [ ] Agents can request help from other agents
- [ ] Handoff protocol for task delegation
- [ ] Parallel agent coordination

### S3. Cost Tracking
- [ ] Track API usage per agent/task
- [ ] Budget limits per namespace
- [ ] Cost estimation before task start

### S4. Replay & Debug
- [ ] Record full agent session (inputs, outputs, state)
- [ ] Replay session for debugging
- [ ] Diff between runs

### S5. Advanced Subscription Management
- [ ] **Subscription Marketplace**: Share unused capacity with team members
- [ ] **Auto-scaling**: Automatically add/remove subscriptions based on demand
- [ ] **Cost Alerts**: Notify when approaching budget limits
- [ ] **Subscription Borrowing**: Temporarily use another user's quota (with permission)
- [ ] **Predictive Routing**: ML-based routing to optimize for cost/speed
- [ ] **Multi-provider Pooling**: Mix Claude, OpenAI, Gemini subscriptions in same pool

---

## Milestones

| Milestone | Phases | Description |
|-----------|--------|-------------|
| **v0.1** | Phase 1 | Usable CLI for spawning/managing agents |
| **v0.2** | Phase 2 | Autonomous tasks with checkpoints |
| **v0.3** | Phase 3 | Context sharing between agents |
| **v0.4** | Phase 4 | Remote/distributed operation |
| **v1.0** | Phase 5 | Full ecosystem, production-ready |

---

## Design Decisions Log

### DD-001: Why Rust over Go?
- Memory safety without GC pauses
- Excellent async ecosystem (Tokio)
- Better type system for complex state machines
- Lower resource usage for long-running daemon
- Growing ecosystem for systems programming

### DD-002: Why gRPC over REST?
- Bi-directional streaming for real-time updates
- Strong typing with protobuf
- Efficient binary protocol
- Well-supported in Rust (tonic)
- Easy to add streaming later

### DD-003: Why redb over SQLite?
- Pure Rust, no C dependencies (truly single binary)
- Zero-configuration setup
- ACID transactions with memory-mapped I/O
- Compiles into the binary, no external files needed
- Designed for embedded use cases

### DD-004: Why event sourcing for context?
- Full audit trail of agent discoveries
- Replay capability for debugging
- Natural fit for distributed systems
- Enables time-travel queries
- Easier cross-agent synchronization

### DD-005: Why ULID over UUID?
- Lexicographically sortable (chronological order)
- URL-safe, no hyphens
- Same randomness as UUID
- Smaller string representation

### DD-006: Why single binary, zero runtime dependencies?
- **Linux**: Fully static with musl (runs anywhere)
- **macOS/Windows**: Just works, no installers
- Simple deployment: download and run
- Works in restricted environments (no sudo, no package manager)
- Cloud-friendly: `FROM scratch` container images
- No version mismatches with external services

### DD-007: Why OS keychain for secrets?
- Battle-tested security (Apple/Microsoft/Linux maintainers)
- Never touches disk in plaintext
- Per-user isolation on multi-user systems
- Works with hardware security modules where available
- No custom crypto to audit

### DD-008: Why single-user default with multi-user opt-in?
- Zero friction for personal use (no auth config)
- "Implicit default user" pattern - abstractions work same either way
- `kage server enable-multiuser` when ready
- Existing data migrates seamlessly (becomes yours)
- No wasted complexity for solo developers

### DD-009: Why multi-subscription pooling?
- **Linear scaling**: Add subscriptions = add parallel capacity
- **Rate limit immunity**: Rotate subscriptions to avoid throttling
- **Cost optimization**: Use different tiers for different workloads
- **Team flexibility**: Separate billing/quotas per project or team
- **Resilience**: No single point of failure if one subscription has issues
- **Usage visibility**: Track which projects consume what resources
- Users shouldn't have to manually manage which API key to use

---

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for guidelines on contributing to Kage.

Priority areas for contribution:
- Phase 1 implementation
- Documentation
- Testing infrastructure
- Cross-platform support (Windows)
