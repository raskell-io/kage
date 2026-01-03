//! CLI command handling for Kage

use anyhow::Result;
use clap::Subcommand;

pub mod commands;

/// Top-level CLI commands
#[derive(Subcommand)]
pub enum Commands {
    /// Launch interactive dashboard (TUI)
    #[cfg(feature = "tui")]
    Dashboard,

    /// Daemon management
    Daemon {
        #[command(subcommand)]
        command: DaemonCommands,
    },

    /// Agent management
    Agent {
        #[command(subcommand)]
        command: AgentCommands,
    },

    /// Task management
    Task {
        #[command(subcommand)]
        command: TaskCommands,
    },

    /// Memory and context operations
    Memory {
        #[command(subcommand)]
        command: MemoryCommands,
    },

    /// Namespace management
    Namespace {
        #[command(subcommand)]
        command: NamespaceCommands,
    },

    /// Secret management (OS keychain)
    Secret {
        #[command(subcommand)]
        command: SecretCommands,
    },

    /// Subscription management (multi-subscription pooling)
    Subscription {
        #[command(subcommand)]
        command: SubscriptionCommands,
    },

    /// User management (multi-user mode)
    #[cfg(feature = "server")]
    User {
        #[command(subcommand)]
        command: UserCommands,
    },

    /// Server mode (remote/multi-user)
    #[cfg(feature = "server")]
    Server {
        #[command(subcommand)]
        command: ServerCommands,
    },

    /// Connect to remote Kage server
    #[cfg(feature = "server")]
    Connect {
        /// Remote server address (e.g., vps.example.com:50051)
        address: String,
    },
}

#[derive(Subcommand)]
pub enum DaemonCommands {
    /// Start the daemon
    Start {
        /// Run in foreground (don't daemonize)
        #[arg(short, long)]
        foreground: bool,
    },
    /// Stop the daemon
    Stop,
    /// Show daemon status
    Status,
}

#[derive(Subcommand)]
pub enum AgentCommands {
    /// Spawn a new agent
    Spawn {
        /// Repository path
        #[arg(short, long, default_value = ".")]
        repo: std::path::PathBuf,

        /// Namespace to use
        #[arg(short, long)]
        namespace: Option<String>,

        /// Initial prompt/goal
        #[arg(short, long)]
        prompt: Option<String>,
    },
    /// List active agents
    List {
        /// Filter by namespace
        #[arg(short, long)]
        namespace: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Attach to an agent (interactive)
    Attach {
        /// Agent ID or name
        id: String,
    },
    /// Detach from current agent
    Detach,
    /// Kill an agent
    Kill {
        /// Agent ID or name
        id: String,

        /// Force kill without confirmation
        #[arg(short, long)]
        force: bool,
    },
    /// Show agent status and recent output
    Status {
        /// Agent ID or name
        id: String,
    },
}

#[derive(Subcommand)]
pub enum TaskCommands {
    /// Add a new task
    Add {
        /// Task goal (natural language)
        goal: String,

        /// Namespace
        #[arg(short, long)]
        namespace: Option<String>,

        /// Repository
        #[arg(short, long)]
        repo: Option<std::path::PathBuf>,

        /// Maximum iterations
        #[arg(long, default_value = "10")]
        max_iterations: u32,

        /// Checkpoint interval
        #[arg(long, default_value = "2")]
        checkpoint_every: u32,

        /// Approval level (none, on-write, on-commit, always)
        #[arg(long, default_value = "on-commit")]
        approval: String,

        /// Priority (0-255, higher = more urgent)
        #[arg(short, long, default_value = "100")]
        priority: u8,

        /// Depends on another task (can specify multiple times)
        #[arg(long)]
        depends_on: Vec<String>,
    },
    /// List tasks
    List {
        /// Filter by status (pending, running, paused, completed, failed, cancelled)
        #[arg(short, long)]
        status: Option<String>,

        /// Filter by namespace
        #[arg(short, long)]
        namespace: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Resume a paused task
    Resume {
        /// Task ID
        id: String,

        /// Extend iteration limit
        #[arg(long, default_value = "5")]
        extend_iterations: u32,

        /// Provide guidance
        #[arg(long)]
        guidance: Option<String>,
    },
    /// Show task details
    Show {
        /// Task ID
        id: String,
    },
    /// Pause a running task
    Pause {
        /// Task ID
        id: String,
    },
    /// Cancel a task
    Cancel {
        /// Task ID
        id: String,

        /// Force cancel without confirmation
        #[arg(short, long)]
        force: bool,
    },
    /// Manage task checkpoints
    Checkpoint {
        #[command(subcommand)]
        command: CheckpointCommands,
    },
}

#[derive(Subcommand)]
pub enum CheckpointCommands {
    /// List checkpoints for a task
    List {
        /// Task ID
        task_id: String,
    },
    /// Show checkpoint details
    Show {
        /// Task ID
        task_id: String,

        /// Checkpoint ID (defaults to latest)
        #[arg(short, long)]
        checkpoint_id: Option<String>,
    },
    /// Prune old checkpoints
    Prune {
        /// Task ID
        task_id: String,

        /// Number of checkpoints to keep
        #[arg(long, default_value = "3")]
        keep: usize,
    },
}

#[derive(Subcommand)]
pub enum MemoryCommands {
    /// List memory entries
    List {
        /// Filter by namespace
        #[arg(short, long)]
        namespace: Option<String>,

        /// Filter by type (patterns, errors, decisions, etc.)
        #[arg(short = 't', long)]
        r#type: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Search memory
    Search {
        /// Search query
        query: String,

        /// Filter by namespace
        #[arg(short, long)]
        namespace: Option<String>,
    },
    /// Show a specific memory entry
    Show {
        /// Memory entry ID
        id: String,
    },
    /// Share a memory entry to wider scope
    Share {
        /// Memory entry ID
        id: String,

        /// Target scope (namespace, global)
        #[arg(long)]
        scope: String,
    },
    /// Watch memory stream in real-time
    Watch {
        /// Filter by namespace
        #[arg(short, long)]
        namespace: Option<String>,
    },
    /// Export memory to file
    Export {
        /// Output format (json, markdown)
        #[arg(short, long, default_value = "json")]
        format: String,

        /// Output file
        #[arg(short, long)]
        output: std::path::PathBuf,
    },
    /// Prune old memory entries
    Prune {
        /// Delete entries older than (e.g., "30d", "1w")
        #[arg(long)]
        older_than: String,

        /// Dry run (don't actually delete)
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum NamespaceCommands {
    /// Create a new namespace
    Create {
        /// Namespace name
        name: String,
    },
    /// List namespaces
    List,
    /// Show namespace details
    Show {
        /// Namespace name
        name: String,
    },
    /// Add repository to namespace
    AddRepo {
        /// Namespace name
        namespace: String,

        /// Repository path
        path: std::path::PathBuf,

        /// Repository name (defaults to directory name)
        #[arg(short, long)]
        name: Option<String>,
    },
    /// Remove repository from namespace
    RemoveRepo {
        /// Namespace name
        namespace: String,

        /// Repository name
        repo: String,
    },
    /// Delete a namespace
    Delete {
        /// Namespace name
        name: String,

        /// Force delete without confirmation
        #[arg(short, long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub enum SecretCommands {
    /// Set a secret (stores in OS keychain)
    Set {
        /// Secret name
        name: String,

        /// Scope (global, namespace:<name>, repo:<path>)
        #[arg(short, long, default_value = "global")]
        scope: String,
    },
    /// List secret names (not values)
    List {
        /// Filter by scope
        #[arg(short, long)]
        scope: Option<String>,
    },
    /// Delete a secret
    Delete {
        /// Secret name
        name: String,

        /// Scope
        #[arg(short, long, default_value = "global")]
        scope: String,
    },
}

#[derive(Subcommand)]
pub enum SubscriptionCommands {
    /// Add a new subscription
    Add {
        /// Subscription name (user-friendly identifier)
        #[arg(short, long)]
        name: String,

        /// Provider type (claude-code, anthropic-api)
        #[arg(short, long, default_value = "claude-code")]
        provider: String,

        /// Priority (0-255, higher = used first)
        #[arg(long, default_value = "100")]
        priority: u8,

        /// Tags for routing (comma-separated)
        #[arg(short, long)]
        tags: Option<String>,

        /// Description
        #[arg(short, long)]
        description: Option<String>,
    },
    /// List all subscriptions
    List {
        /// Filter by status (active, disabled, rate_limited, unhealthy)
        #[arg(short, long)]
        status: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show subscription details
    Show {
        /// Subscription name or ID
        name: String,
    },
    /// Remove a subscription
    Remove {
        /// Subscription name or ID
        name: String,

        /// Force remove without confirmation
        #[arg(short, long)]
        force: bool,
    },
    /// Enable a subscription
    Enable {
        /// Subscription name or ID
        name: String,
    },
    /// Disable a subscription
    Disable {
        /// Subscription name or ID
        name: String,
    },
    /// Show pool status overview
    Status,
    /// Show usage statistics
    Usage {
        /// Time period (today, week, month)
        #[arg(short, long, default_value = "week")]
        period: String,

        /// Filter by subscription name
        #[arg(short, long)]
        subscription: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Set routing configuration
    Route {
        /// Namespace to route
        #[arg(short, long)]
        namespace: Option<String>,

        /// Priority level to route (high, normal, batch)
        #[arg(short, long)]
        priority: Option<String>,

        /// Subscriptions to route to (comma-separated names)
        #[arg(short, long)]
        subscriptions: String,
    },
}

#[cfg(feature = "server")]
#[derive(Subcommand)]
pub enum UserCommands {
    /// Invite a new user
    Invite {
        /// User email
        email: String,
    },
    /// List users
    List,
    /// Remove a user
    Remove {
        /// User ID or email
        id: String,
    },
}

#[cfg(feature = "server")]
#[derive(Subcommand)]
pub enum ServerCommands {
    /// Start server mode
    Start {
        /// Listen address
        #[arg(short, long, default_value = "0.0.0.0:50051")]
        listen: String,

        /// Enable TLS
        #[arg(long)]
        tls: bool,

        /// Enable multi-user mode
        #[arg(long)]
        multiuser: bool,
    },
    /// Stop server
    Stop,
    /// Enable multi-user mode
    EnableMultiuser,
    /// Show server status
    Status,
}

/// Execute a CLI command
pub async fn run(cmd: Commands) -> Result<()> {
    match cmd {
        #[cfg(feature = "tui")]
        Commands::Dashboard => crate::tui::dashboard::run().await,

        Commands::Daemon { command } => commands::daemon::run(command).await,
        Commands::Agent { command } => commands::agent::run(command).await,
        Commands::Task { command } => commands::task::run(command).await,
        Commands::Memory { command } => commands::memory::run(command).await,
        Commands::Namespace { command } => commands::namespace::run(command).await,
        Commands::Secret { command } => commands::secret::run(command).await,
        Commands::Subscription { command } => commands::subscription::run(command).await,

        #[cfg(feature = "server")]
        Commands::User { command } => commands::user::run(command).await,

        #[cfg(feature = "server")]
        Commands::Server { command } => commands::server::run(command).await,

        #[cfg(feature = "server")]
        Commands::Connect { address } => commands::connect::run(address).await,
    }
}
