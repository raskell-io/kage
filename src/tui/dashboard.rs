//! Interactive dashboard for monitoring agents
//!
//! Shows:
//! - Active agents with status filtering (working/idle/waiting)
//! - Real-time stream output from selected agent
//! - Task queue and logs (collapsible)
//! - Themeable styling (Catppuccin, Dracula, etc.)

use std::io::{self, Stdout};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame, Terminal,
};

use ansi_to_tui::IntoText;

use crate::daemon::protocol::{AgentInfo as DaemonAgentInfo, ApprovalInfo as DaemonApprovalInfo, TaskInfo as DaemonTaskInfo};
use crate::secrets::{self, ClaudeCodeAuth};
use super::theme::{Theme, ColorPalette, StatusSymbols};

/// Message from background data fetcher
enum DataUpdate {
    /// Updated agent list
    Agents(Vec<DaemonAgentInfo>),
    /// Updated task list
    Tasks(Vec<DaemonTaskInfo>),
    /// Daemon status
    Status {
        connected: bool,
        version: Option<String>,
        uptime_secs: Option<u64>,
    },
    /// Agent output lines
    AgentOutput {
        agent_id: String,
        lines: Vec<OutputLineDisplay>,
        has_more: bool,
    },
    /// Pending approvals
    Approvals(Vec<DaemonApprovalInfo>),
    /// Log message
    Log(LogEntry),
    /// Real-time daemon event
    Event(crate::daemon::protocol::DaemonEvent),
    /// Subscription count (for setup wizard)
    SubscriptionCount(usize),
    /// Subscription added result
    SubscriptionAdded { success: bool, error: Option<String> },
}

/// Output line for display
#[derive(Debug, Clone)]
struct OutputLineDisplay {
    text: String,
    is_error: bool,
    timestamp: i64,
}

/// Action request from TUI to daemon
#[derive(Debug, Clone)]
enum Action {
    /// Spawn a new agent (runs interactive claude session like tmux)
    SpawnAgent {
        repo: String,
        namespace: Option<String>,
        pty_rows: u16,
        pty_cols: u16,
    },
    /// Kill an agent
    KillAgent { id: String },
    /// Pause an agent
    PauseAgent { id: String },
    /// Resume an agent
    ResumeAgent { id: String },
    /// Cancel a task
    CancelTask { id: String },
    /// Request output for a specific agent
    RequestOutput { id: String },
    /// Approve an action
    Approve { id: String },
    /// Reject an action
    Reject { id: String },
    /// Add a subscription
    AddSubscription {
        name: String,
        api_key: String,
    },
    /// Resize agent PTY
    ResizeAgent {
        id: String,
        rows: u16,
        cols: u16,
    },
    /// Send input to agent
    SendInput {
        id: String,
        input: String,
    },
}

/// Setup wizard step
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupStep {
    Welcome,
    AddApiKey,
    Complete,
}

/// Setup wizard state for first-time configuration
struct SetupWizardState {
    /// Current step
    step: SetupStep,
    /// Whether setup is needed
    needs_setup: bool,
    /// Whether user dismissed the wizard
    dismissed: bool,
    /// Subscription name input
    sub_name: String,
    /// API key input (masked)
    api_key: String,
    /// Cursor position
    cursor: usize,
    /// Which field is focused (0 = name, 1 = api_key)
    focus: usize,
    /// Error message to display
    error: Option<String>,
    /// Whether setup completed successfully
    setup_complete: bool,
    /// Detected Claude Code authentication (if any)
    detected_auth: Option<ClaudeCodeAuth>,
    /// Whether to use detected auth
    use_detected_auth: bool,
}

impl SetupWizardState {
    fn new() -> Self {
        // Try to detect existing Claude Code authentication
        let detected_auth = secrets::detect_claude_code_auth();
        let use_detected = detected_auth.is_some();

        Self {
            step: SetupStep::Welcome,
            needs_setup: false,
            dismissed: false,
            sub_name: "claude".to_string(),
            api_key: String::new(),
            cursor: 0,
            focus: 1, // Start on API key field
            error: None,
            setup_complete: false,
            detected_auth,
            use_detected_auth: use_detected,
        }
    }

    fn reset(&mut self) {
        self.step = SetupStep::Welcome;
        self.sub_name = "claude".to_string();
        self.api_key.clear();
        self.cursor = 0;
        self.focus = 1;
        self.error = None;
    }

    fn next_step(&mut self) {
        self.step = match self.step {
            SetupStep::Welcome => SetupStep::AddApiKey,
            SetupStep::AddApiKey => SetupStep::Complete,
            SetupStep::Complete => SetupStep::Complete,
        };
        self.cursor = 0;
    }

    fn current_field(&self) -> &str {
        match self.focus {
            0 => &self.sub_name,
            _ => &self.api_key,
        }
    }

    fn current_field_mut(&mut self) -> &mut String {
        match self.focus {
            0 => &mut self.sub_name,
            _ => &mut self.api_key,
        }
    }

    fn insert_char(&mut self, c: char) {
        let cursor = self.cursor;
        let field = self.current_field_mut();
        if cursor <= field.len() {
            field.insert(cursor, c);
            self.cursor += 1;
        }
    }

    fn delete_char(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            let cursor = self.cursor;
            let field = self.current_field_mut();
            if !field.is_empty() && cursor < field.len() {
                field.remove(cursor);
            }
        }
    }

    fn is_valid(&self) -> bool {
        if self.use_detected_auth && self.detected_auth.is_some() {
            !self.sub_name.trim().is_empty()
        } else {
            !self.sub_name.trim().is_empty() && !self.api_key.trim().is_empty()
        }
    }

    fn get_api_key(&self) -> Option<String> {
        if self.use_detected_auth {
            self.detected_auth.as_ref().map(|a| a.access_token.clone())
        } else if !self.api_key.trim().is_empty() {
            Some(self.api_key.trim().to_string())
        } else {
            None
        }
    }
}

/// Agent status filter
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentFilter {
    /// Show all agents
    #[default]
    All,
    /// Only working/running agents
    Working,
    /// Only idle/completed agents
    Idle,
    /// Only agents waiting for user input
    Waiting,
}

/// Dashboard application state
pub struct Dashboard {
    /// Theme configuration
    theme: Theme,
    /// Currently focused panel
    focus: Panel,
    /// Agent list state
    agents: AgentListState,
    /// Agent filter (all/working/idle/waiting)
    agent_filter: AgentFilter,
    /// Agent stream output state (renamed from output)
    stream: OutputState,
    /// Stream scroll position
    stream_scroll: usize,
    /// Task list state
    tasks: TaskListState,
    /// Log entries
    logs: LogState,
    /// Pending approvals
    approvals: ApprovalListState,
    /// Daemon connection status
    daemon_status: DaemonStatus,
    /// Daemon version
    daemon_version: Option<String>,
    /// Daemon uptime
    daemon_uptime: Option<u64>,
    /// Show help overlay
    show_help: bool,
    /// Show agent details popup
    show_agent_details: bool,
    /// Show task details popup
    show_task_details: bool,
    /// Show approvals popup
    show_approvals: bool,
    /// Show spawn agent dialog
    show_spawn_dialog: bool,
    /// Fullscreen stream mode (no panels, just agent output)
    fullscreen_stream: bool,
    /// Fullscreen logs mode
    fullscreen_logs: bool,
    /// Tasks panel collapsed
    tasks_collapsed: bool,
    /// Logs panel collapsed
    logs_collapsed: bool,
    /// Spawn dialog state
    spawn_dialog: SpawnDialogState,
    /// Setup wizard state
    setup_wizard: SetupWizardState,
    /// Last tick time (for animations)
    last_tick: Instant,
    /// Last data refresh
    last_refresh: Instant,
    /// Spinner animation frame
    spinner_frame: usize,
    /// Should quit
    should_quit: bool,
    /// Data update receiver
    data_rx: Option<mpsc::Receiver<DataUpdate>>,
    /// Channel to send actions to daemon
    action_tx: Option<mpsc::Sender<Action>>,
    /// Last stream panel size for PTY resize tracking (rows, cols)
    last_stream_size: (u16, u16),
    /// Last agent ID for which we sent a resize
    last_resized_agent: Option<String>,
    /// Current terminal size (for spawning agents with correct initial size)
    terminal_size: (u16, u16),
    /// Prefix key active (Ctrl+B was pressed, waiting for command)
    prefix_active: bool,
}

/// Which panel is focused
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel {
    Agents,
    Stream,
    Tasks,
    Logs,
}

impl Panel {
    fn next(&self) -> Self {
        match self {
            Self::Agents => Self::Stream,
            Self::Stream => Self::Tasks,
            Self::Tasks => Self::Logs,
            Self::Logs => Self::Agents,
        }
    }

    fn prev(&self) -> Self {
        match self {
            Self::Agents => Self::Logs,
            Self::Stream => Self::Agents,
            Self::Tasks => Self::Stream,
            Self::Logs => Self::Tasks,
        }
    }
}

/// Daemon connection status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DaemonStatus {
    Connected,
    Disconnected,
    Connecting,
}

/// Mock agent data for display
#[derive(Debug, Clone)]
struct AgentInfo {
    id: String,
    name: String,
    status: AgentDisplayStatus,
    namespace: String,
    repository: String,
    iterations: u32,
    max_iterations: u32,
    current_action: String,
    started_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentDisplayStatus {
    /// Agent is actively working
    Working,
    /// Agent is idle/completed
    Idle,
    /// Agent is waiting for user input
    Waiting,
    /// Agent is paused
    Paused,
    /// Agent encountered an error
    Error,
}

/// Agent list state
struct AgentListState {
    items: Vec<AgentInfo>,
    state: ListState,
}

impl AgentListState {
    fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            items: Vec::new(),
            state,
        }
    }

    fn update_from_daemon(&mut self, agents: Vec<DaemonAgentInfo>) {
        let selected_id = self.selected().map(|a| a.id.clone());

        self.items = agents.into_iter().map(|a| {
            let status = match a.status.as_str() {
                "running" => AgentDisplayStatus::Working,
                "waiting" | "awaiting_input" => AgentDisplayStatus::Waiting,
                "paused" => AgentDisplayStatus::Paused,
                "completed" | "stopped" | "idle" => AgentDisplayStatus::Idle,
                _ => AgentDisplayStatus::Error,
            };

            let started_ago = format_duration_ago(a.started_at);
            let repo_name = a.working_dir.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            AgentInfo {
                id: a.id.to_string(),
                name: a.name,
                status,
                namespace: a.namespace.unwrap_or_else(|| "default".to_string()),
                repository: repo_name,
                iterations: a.iteration,
                max_iterations: a.max_iterations,
                current_action: format!("Iteration {}/{}", a.iteration, a.max_iterations),
                started_at: started_ago,
            }
        }).collect();

        // Preserve selection if possible
        if let Some(id) = selected_id {
            if let Some(idx) = self.items.iter().position(|a| a.id == id) {
                self.state.select(Some(idx));
            } else if !self.items.is_empty() {
                self.state.select(Some(0));
            }
        } else if !self.items.is_empty() && self.state.selected().is_none() {
            self.state.select(Some(0));
        }
    }

    fn next(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => (i + 1) % self.items.len(),
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn previous(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn selected(&self) -> Option<&AgentInfo> {
        self.state.selected().and_then(|i| self.items.get(i))
    }
}

/// Mock task data for display
#[derive(Debug, Clone)]
struct TaskInfo {
    id: String,
    goal: String,
    status: TaskDisplayStatus,
    namespace: Option<String>,
    agent: Option<String>,
    iterations: u32,
    max_iterations: u32,
    created: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskDisplayStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
}

/// Task list state
struct TaskListState {
    items: Vec<TaskInfo>,
    state: ListState,
}

impl TaskListState {
    fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            items: Vec::new(),
            state,
        }
    }

    fn update_from_daemon(&mut self, tasks: Vec<DaemonTaskInfo>) {
        let selected_id = self.selected().map(|t| t.id.clone());

        self.items = tasks.into_iter().map(|t| {
            let status = match t.status.as_str() {
                "pending" => TaskDisplayStatus::Pending,
                "running" => TaskDisplayStatus::Running,
                "paused" => TaskDisplayStatus::Paused,
                "completed" => TaskDisplayStatus::Completed,
                "failed" => TaskDisplayStatus::Failed,
                _ => TaskDisplayStatus::Pending,
            };

            let created_ago = format_duration_ago(t.created_at);

            TaskInfo {
                id: t.id.to_string(),
                goal: t.goal,
                status,
                namespace: t.namespace,
                agent: t.agent.map(|a| a.to_string()),
                iterations: t.iterations,
                max_iterations: t.max_iterations,
                created: created_ago,
            }
        }).collect();

        // Preserve selection if possible
        if let Some(id) = selected_id {
            if let Some(idx) = self.items.iter().position(|t| t.id == id) {
                self.state.select(Some(idx));
            } else if !self.items.is_empty() {
                self.state.select(Some(0));
            }
        } else if !self.items.is_empty() && self.state.selected().is_none() {
            self.state.select(Some(0));
        }
    }

    fn next(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => (i + 1) % self.items.len(),
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn previous(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn selected(&self) -> Option<&TaskInfo> {
        self.state.selected().and_then(|i| self.items.get(i))
    }
}

/// Log entry
#[derive(Debug, Clone)]
struct LogEntry {
    timestamp: String,
    level: LogLevel,
    source: String,
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogLevel {
    Info,
    Success,
    Warning,
    Error,
    Debug,
}

/// Approval display info
#[derive(Debug, Clone)]
struct ApprovalDisplayInfo {
    id: String,
    agent_id: String,
    summary: String,
    context: Vec<String>,
    created: String,
}

/// Approval list state
struct ApprovalListState {
    items: Vec<ApprovalDisplayInfo>,
    state: ListState,
}

impl ApprovalListState {
    fn new() -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            items: Vec::new(),
            state,
        }
    }

    fn update_from_daemon(&mut self, approvals: Vec<DaemonApprovalInfo>) {
        let selected_id = self.selected().map(|a| a.id.clone());

        self.items = approvals.into_iter().map(|a| {
            let created_ago = format_duration_ago(a.created_at);
            ApprovalDisplayInfo {
                id: a.id.to_string(),
                agent_id: a.agent_id.to_string(),
                summary: a.summary,
                context: a.context,
                created: created_ago,
            }
        }).collect();

        // Preserve selection if possible
        if let Some(id) = selected_id {
            if let Some(idx) = self.items.iter().position(|a| a.id == id) {
                self.state.select(Some(idx));
            } else if !self.items.is_empty() {
                self.state.select(Some(0));
            }
        } else if !self.items.is_empty() && self.state.selected().is_none() {
            self.state.select(Some(0));
        }
    }

    fn next(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => (i + 1) % self.items.len(),
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn previous(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn selected(&self) -> Option<&ApprovalDisplayInfo> {
        self.state.selected().and_then(|i| self.items.get(i))
    }
}

/// Log state
struct LogState {
    entries: Vec<LogEntry>,
    scroll: usize,
    max_entries: usize,
}

/// Available agent providers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum AgentProvider {
    #[default]
    ClaudeCode,
    // Future providers can be added here
}

impl AgentProvider {
    fn name(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
        }
    }

    fn command(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
        }
    }

    fn all() -> &'static [AgentProvider] {
        &[AgentProvider::ClaudeCode]
    }

    fn next(&self) -> Self {
        match self {
            Self::ClaudeCode => Self::ClaudeCode, // Only one for now
        }
    }

    fn prev(&self) -> Self {
        match self {
            Self::ClaudeCode => Self::ClaudeCode, // Only one for now
        }
    }
}

/// Spawn dialog state for creating new agents
struct SpawnDialogState {
    /// Which field is focused (0 = repo, 1 = provider, 2 = namespace)
    focus: usize,
    /// Repository path
    repo: String,
    /// Selected provider
    provider: AgentProvider,
    /// Namespace (optional)
    namespace: String,
    /// Cursor position in current text field
    cursor: usize,
}

impl SpawnDialogState {
    fn new() -> Self {
        // Default to current directory
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());

        Self {
            focus: 0, // Start on repo field
            repo: cwd,
            provider: AgentProvider::default(),
            namespace: String::new(),
            cursor: 0,
        }
    }

    fn reset(&mut self) {
        let cwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string());

        self.focus = 0;
        self.repo = cwd;
        self.provider = AgentProvider::default();
        self.namespace.clear();
        self.cursor = 0;
    }

    /// Get the current text field (repo or namespace, not provider which is a selector)
    fn current_text_field(&self) -> Option<&str> {
        match self.focus {
            0 => Some(&self.repo),
            2 => Some(&self.namespace),
            _ => None, // Provider is a selector, not a text field
        }
    }

    fn current_text_field_mut(&mut self) -> Option<&mut String> {
        match self.focus {
            0 => Some(&mut self.repo),
            2 => Some(&mut self.namespace),
            _ => None, // Provider is a selector, not a text field
        }
    }

    fn next_field(&mut self) {
        self.focus = (self.focus + 1) % 3;
        if let Some(field) = self.current_text_field() {
            self.cursor = field.len();
        }
    }

    fn prev_field(&mut self) {
        self.focus = if self.focus == 0 { 2 } else { self.focus - 1 };
        if let Some(field) = self.current_text_field() {
            self.cursor = field.len();
        }
    }

    fn insert_char(&mut self, c: char) {
        let cursor = self.cursor;
        // Edit the appropriate field based on focus
        match self.focus {
            0 => {
                if cursor <= self.repo.len() {
                    self.repo.insert(cursor, c);
                    self.cursor += 1;
                }
            }
            2 => {
                if cursor <= self.namespace.len() {
                    self.namespace.insert(cursor, c);
                    self.cursor += 1;
                }
            }
            _ => {} // Provider is a selector, not a text field
        }
    }

    fn delete_char(&mut self) {
        if self.cursor > 0 && self.focus != 1 {
            self.cursor -= 1;
            let cursor = self.cursor;
            match self.focus {
                0 => {
                    if !self.repo.is_empty() && cursor < self.repo.len() {
                        self.repo.remove(cursor);
                    }
                }
                2 => {
                    if !self.namespace.is_empty() && cursor < self.namespace.len() {
                        self.namespace.remove(cursor);
                    }
                }
                _ => {}
            }
        }
    }

    fn move_cursor_left(&mut self) {
        if self.current_text_field().is_some() {
            self.cursor = self.cursor.saturating_sub(1);
        } else if self.focus == 1 {
            // Provider selector - cycle through options
            self.provider = self.provider.prev();
        }
    }

    fn move_cursor_right(&mut self) {
        if let Some(field) = self.current_text_field() {
            if self.cursor < field.len() {
                self.cursor += 1;
            }
        } else if self.focus == 1 {
            // Provider selector - cycle through options
            self.provider = self.provider.next();
        }
    }

    fn move_cursor_home(&mut self) {
        self.cursor = 0;
    }

    fn move_cursor_end(&mut self) {
        if let Some(field) = self.current_text_field() {
            self.cursor = field.len();
        }
    }

    fn is_valid(&self) -> bool {
        // Only repo is required - we just run claude in that directory
        !self.repo.trim().is_empty()
    }
}

/// Agent output state
struct OutputState {
    /// Current agent ID being viewed
    agent_id: Option<String>,
    /// Output lines
    lines: Vec<OutputLineDisplay>,
    /// Scroll position (from bottom, 0 = at bottom)
    scroll: usize,
    /// Whether there's more output available
    has_more: bool,
    /// Auto-scroll to bottom
    auto_scroll: bool,
}

impl OutputState {
    fn new() -> Self {
        Self {
            agent_id: None,
            lines: Vec::new(),
            scroll: 0,
            has_more: false,
            auto_scroll: true,
        }
    }

    fn set_agent(&mut self, agent_id: Option<String>) {
        if self.agent_id != agent_id {
            self.agent_id = agent_id;
            self.lines.clear();
            self.scroll = 0;
            self.auto_scroll = true;
        }
    }

    fn update(&mut self, agent_id: String, lines: Vec<OutputLineDisplay>, has_more: bool) {
        if Some(&agent_id) == self.agent_id.as_ref() {
            self.lines = lines;
            self.has_more = has_more;
            if self.auto_scroll {
                self.scroll = 0;
            }
        }
    }

    fn scroll_up(&mut self, amount: usize) {
        self.scroll = self.scroll.saturating_add(amount).min(self.lines.len().saturating_sub(1));
        self.auto_scroll = false;
    }

    fn scroll_down(&mut self, amount: usize) {
        self.scroll = self.scroll.saturating_sub(amount);
        if self.scroll == 0 {
            self.auto_scroll = true;
        }
    }

    fn scroll_to_bottom(&mut self) {
        self.scroll = 0;
        self.auto_scroll = true;
    }

    fn scroll_to_top(&mut self) {
        self.scroll = self.lines.len().saturating_sub(1);
        self.auto_scroll = false;
    }
}

impl LogState {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            scroll: 0,
            max_entries: 100,
        }
    }

    fn add(&mut self, entry: LogEntry) {
        self.entries.push(entry);
        if self.entries.len() > self.max_entries {
            self.entries.remove(0);
        }
        // Auto-scroll to bottom
        if self.entries.len() > 1 {
            self.scroll = self.entries.len().saturating_sub(1);
        }
    }

    fn add_info(&mut self, source: &str, message: &str) {
        self.add(LogEntry {
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            level: LogLevel::Info,
            source: source.to_string(),
            message: message.to_string(),
        });
    }

    fn add_success(&mut self, source: &str, message: &str) {
        self.add(LogEntry {
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            level: LogLevel::Success,
            source: source.to_string(),
            message: message.to_string(),
        });
    }

    fn scroll_down(&mut self) {
        if self.scroll < self.entries.len().saturating_sub(1) {
            self.scroll += 1;
        }
    }

    fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }
}

impl Default for Dashboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Dashboard {
    /// Create a new dashboard with default theme
    pub fn new() -> Self {
        Self::with_theme(Theme::default())
    }

    /// Create a dashboard with a specific theme
    pub fn with_theme(theme: Theme) -> Self {
        Self {
            theme,
            focus: Panel::Agents,
            agents: AgentListState::new(),
            agent_filter: AgentFilter::default(),
            stream: OutputState::new(),
            stream_scroll: 0,
            tasks: TaskListState::new(),
            logs: LogState::new(),
            approvals: ApprovalListState::new(),
            daemon_status: DaemonStatus::Disconnected,
            daemon_version: None,
            daemon_uptime: None,
            show_help: false,
            show_agent_details: false,
            show_task_details: false,
            show_approvals: false,
            show_spawn_dialog: false,
            fullscreen_stream: false,
            fullscreen_logs: false,
            tasks_collapsed: true,  // Collapsed by default
            logs_collapsed: true,   // Collapsed by default
            spawn_dialog: SpawnDialogState::new(),
            setup_wizard: SetupWizardState::new(),
            last_tick: Instant::now(),
            last_refresh: Instant::now(),
            spinner_frame: 0,
            should_quit: false,
            data_rx: None,
            action_tx: None,
            last_stream_size: (0, 0),
            last_resized_agent: None,
            terminal_size: (24, 80),  // Default, updated on first draw
            prefix_active: false,
        }
    }

    /// Set the theme
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    /// Get available theme names
    pub fn available_themes() -> Vec<&'static str> {
        Theme::available()
    }

    /// Create dashboard with data channel
    pub fn with_data_channel(mut self, rx: mpsc::Receiver<DataUpdate>, action_tx: mpsc::Sender<Action>) -> Self {
        self.data_rx = Some(rx);
        self.action_tx = Some(action_tx);
        self
    }

    /// Send an action to the daemon
    fn send_action(&self, action: Action) {
        if let Some(ref tx) = self.action_tx {
            let _ = tx.send(action);
        }
    }

    /// Convert a key event to a string to send to the PTY
    fn key_to_string(&self, key: KeyCode, modifiers: KeyModifiers) -> String {
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);

        match key {
            KeyCode::Char(c) => {
                if ctrl {
                    // Ctrl+letter = ASCII control code
                    let code = (c.to_ascii_lowercase() as u8).wrapping_sub(b'a').wrapping_add(1);
                    String::from(code as char)
                } else {
                    c.to_string()
                }
            }
            KeyCode::Enter => "\r".to_string(),
            KeyCode::Backspace => "\x7f".to_string(),
            KeyCode::Tab => "\t".to_string(),
            KeyCode::Esc => "\x1b".to_string(),
            KeyCode::Up => "\x1b[A".to_string(),
            KeyCode::Down => "\x1b[B".to_string(),
            KeyCode::Right => "\x1b[C".to_string(),
            KeyCode::Left => "\x1b[D".to_string(),
            KeyCode::Home => "\x1b[H".to_string(),
            KeyCode::End => "\x1b[F".to_string(),
            KeyCode::PageUp => "\x1b[5~".to_string(),
            KeyCode::PageDown => "\x1b[6~".to_string(),
            KeyCode::Delete => "\x1b[3~".to_string(),
            KeyCode::Insert => "\x1b[2~".to_string(),
            KeyCode::F(n) => match n {
                1 => "\x1bOP".to_string(),
                2 => "\x1bOQ".to_string(),
                3 => "\x1bOR".to_string(),
                4 => "\x1bOS".to_string(),
                5 => "\x1b[15~".to_string(),
                6 => "\x1b[17~".to_string(),
                7 => "\x1b[18~".to_string(),
                8 => "\x1b[19~".to_string(),
                9 => "\x1b[20~".to_string(),
                10 => "\x1b[21~".to_string(),
                11 => "\x1b[23~".to_string(),
                12 => "\x1b[24~".to_string(),
                _ => String::new(),
            },
            _ => String::new(),
        }
    }

    /// Get color palette shorthand
    #[inline]
    fn c(&self) -> &ColorPalette {
        &self.theme.colors
    }

    /// Get symbols shorthand
    #[inline]
    fn s(&self) -> &StatusSymbols {
        &self.theme.symbols
    }

    /// Get status color for an agent
    fn agent_status_color(&self, status: AgentDisplayStatus) -> ratatui::style::Color {
        match status {
            AgentDisplayStatus::Working => self.c().status_working,
            AgentDisplayStatus::Idle => self.c().status_idle,
            AgentDisplayStatus::Waiting => self.c().status_waiting,
            AgentDisplayStatus::Paused => self.c().status_paused,
            AgentDisplayStatus::Error => self.c().status_error,
        }
    }

    /// Get status symbol for an agent
    fn agent_status_symbol(&self, status: AgentDisplayStatus) -> &'static str {
        match status {
            AgentDisplayStatus::Working => self.s().working,
            AgentDisplayStatus::Idle => self.s().idle,
            AgentDisplayStatus::Waiting => self.s().waiting,
            AgentDisplayStatus::Paused => self.s().paused,
            AgentDisplayStatus::Error => self.s().error,
        }
    }

    /// Check if agent matches current filter
    fn agent_matches_filter(&self, agent: &AgentInfo) -> bool {
        match self.agent_filter {
            AgentFilter::All => true,
            AgentFilter::Working => agent.status == AgentDisplayStatus::Working,
            AgentFilter::Idle => agent.status == AgentDisplayStatus::Idle,
            AgentFilter::Waiting => agent.status == AgentDisplayStatus::Waiting,
        }
    }

    /// Get filtered agents
    fn filtered_agents(&self) -> Vec<&AgentInfo> {
        self.agents.items.iter()
            .filter(|a| self.agent_matches_filter(a))
            .collect()
    }

    /// Count agents by status
    fn agent_counts(&self) -> (usize, usize, usize) {
        let working = self.agents.items.iter().filter(|a| a.status == AgentDisplayStatus::Working).count();
        let idle = self.agents.items.iter().filter(|a| a.status == AgentDisplayStatus::Idle).count();
        let waiting = self.agents.items.iter().filter(|a| a.status == AgentDisplayStatus::Waiting).count();
        (working, idle, waiting)
    }

    /// Cycle to next filter
    fn next_filter(&mut self) {
        self.agent_filter = match self.agent_filter {
            AgentFilter::All => AgentFilter::Working,
            AgentFilter::Working => AgentFilter::Idle,
            AgentFilter::Idle => AgentFilter::Waiting,
            AgentFilter::Waiting => AgentFilter::All,
        };
    }

    /// Run the dashboard
    pub fn run(&mut self) -> Result<()> {
        // Setup terminal
        // NOTE: Mouse capture is disabled to allow normal terminal copy/paste
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Initialize terminal size immediately
        if let Ok(size) = terminal.size() {
            self.terminal_size = (size.height, size.width);
        }

        // Run the event loop
        let result = self.run_loop(&mut terminal);

        // Restore terminal
        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        result
    }

    /// Main event loop
    fn run_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        let tick_rate = Duration::from_millis(100);

        loop {
            terminal.draw(|f| self.render(f))?;

            // Check if we need to resize the agent PTY
            self.check_and_resize_pty(terminal);

            let timeout = tick_rate
                .checked_sub(self.last_tick.elapsed())
                .unwrap_or(Duration::ZERO);

            if event::poll(timeout)? {
                match event::read()? {
                    Event::Key(key) => {
                        if key.kind == KeyEventKind::Press {
                            self.handle_input(key.code, key.modifiers);
                        }
                    }
                    Event::Resize(_cols, _rows) => {
                        // Terminal resized - PTY resize will be handled on next draw cycle
                    }
                    _ => {}
                }
            }

            if self.last_tick.elapsed() >= tick_rate {
                self.on_tick();
                self.last_tick = Instant::now();
            }

            if self.should_quit {
                return Ok(());
            }
        }
    }

    /// Check if the stream panel size changed and resize the agent PTY if needed
    fn check_and_resize_pty(&mut self, terminal: &Terminal<CrosstermBackend<Stdout>>) {
        let term_size = terminal.size().unwrap_or_default();

        // Store terminal size for spawning new agents
        self.terminal_size = (term_size.height, term_size.width);

        // Calculate stream panel size based on actual layout from render_main():
        // - Header: 3 lines, Footer: 2 lines
        // - Main area = term_size.height - 5
        // - When collapsed: top row is 98% of main, otherwise 55%
        // - Stream panel is 70% of top row width (minus separator)
        // - Stream panel has 1 line for title, so content area is height - 1

        let main_height = term_size.height.saturating_sub(5); // header(3) + footer(2)

        let (stream_height, stream_width) = if self.fullscreen_stream {
            // Fullscreen uses entire terminal
            (term_size.height, term_size.width)
        } else {
            // Calculate based on layout
            let both_collapsed = self.tasks_collapsed && self.logs_collapsed;
            let top_row_percent = if both_collapsed { 98 } else { 55 };

            // Top row height (minus 1 for horizontal separator)
            let top_row_height = (main_height as u32 * top_row_percent / 100) as u16;

            // Stream panel content height (minus 1 for title line)
            let panel_height = top_row_height.saturating_sub(1);

            // Stream panel is 70% of width (minus 1 for vertical separator)
            let available_width = term_size.width.saturating_sub(1);
            let panel_width = (available_width as u32 * 70 / 100) as u16;

            (panel_height, panel_width)
        };

        let new_size = (stream_height, stream_width);

        // Check if we need to send a resize
        let current_agent = self.stream.agent_id.clone();
        let size_changed = new_size != self.last_stream_size;
        let agent_changed = current_agent != self.last_resized_agent;

        if let Some(ref agent_id) = current_agent {
            if size_changed || agent_changed {
                self.send_action(Action::ResizeAgent {
                    id: agent_id.clone(),
                    rows: new_size.0,
                    cols: new_size.1,
                });
                self.last_stream_size = new_size;
                self.last_resized_agent = current_agent;
            }
        }
    }

    /// Handle keyboard input
    fn handle_input(&mut self, key: KeyCode, modifiers: KeyModifiers) {
        // Check if we're in passthrough mode (Stream panel focused with agent, or fullscreen stream)
        let in_passthrough = (self.focus == Panel::Stream || self.fullscreen_stream)
            && self.stream.agent_id.is_some()
            && !self.show_help
            && !self.show_agent_details
            && !self.show_task_details
            && !self.show_approvals
            && !self.show_spawn_dialog
            && !self.setup_wizard.needs_setup;

        // Handle prefix mode (Ctrl+B was pressed)
        if self.prefix_active {
            self.prefix_active = false;
            match key {
                // Ctrl+B, Ctrl+B = send literal Ctrl+B to agent
                KeyCode::Char('b') if modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(ref agent_id) = self.stream.agent_id {
                        self.send_action(Action::SendInput {
                            id: agent_id.clone(),
                            input: "\x02".to_string(),
                        });
                    }
                }
                // Ctrl+B, d = detach (exit fullscreen or unfocus stream)
                KeyCode::Char('d') => {
                    if self.fullscreen_stream {
                        self.fullscreen_stream = false;
                    } else {
                        self.focus = Panel::Agents;
                    }
                }
                // Ctrl+B, f = fullscreen toggle
                KeyCode::Char('f') => {
                    self.fullscreen_stream = !self.fullscreen_stream;
                    // Force resize on next frame by resetting last size
                    self.last_stream_size = (0, 0);
                }
                // Ctrl+B, 1-4 = switch panels
                KeyCode::Char('1') => { self.focus = Panel::Agents; self.fullscreen_stream = false; }
                KeyCode::Char('2') => { self.focus = Panel::Stream; }
                KeyCode::Char('3') => { self.focus = Panel::Tasks; self.fullscreen_stream = false; }
                KeyCode::Char('4') => { self.focus = Panel::Logs; self.fullscreen_stream = false; }
                // Ctrl+B, n = new agent
                KeyCode::Char('n') => { self.show_spawn_dialog = true; self.fullscreen_stream = false; }
                // Ctrl+B, k = kill agent
                KeyCode::Char('k') => self.handle_kill_agent(),
                // Ctrl+B, ? = help
                KeyCode::Char('?') => self.show_help = true,
                // Ctrl+B, q = quit
                KeyCode::Char('q') => self.should_quit = true,
                // Ctrl+B, [ = scroll mode (PageUp)
                KeyCode::Char('[') => self.stream.scroll_up(10),
                // Ctrl+B, ] = scroll down
                KeyCode::Char(']') => self.stream.scroll_down(10),
                _ => {}
            }
            return;
        }

        // In passthrough mode, send keystrokes directly to agent
        if in_passthrough {
            // Ctrl+B activates prefix mode
            if modifiers.contains(KeyModifiers::CONTROL) && key == KeyCode::Char('b') {
                self.prefix_active = true;
                return;
            }

            // Send keystroke to agent
            if let Some(ref agent_id) = self.stream.agent_id {
                let input = self.key_to_string(key, modifiers);
                if !input.is_empty() {
                    self.send_action(Action::SendInput {
                        id: agent_id.clone(),
                        input,
                    });
                }
            }
            return;
        }

        // Fullscreen logs mode - Ctrl+B or Esc to exit
        if self.fullscreen_logs {
            if modifiers.contains(KeyModifiers::CONTROL) && key == KeyCode::Char('b') {
                self.prefix_active = true;
                return;
            }
            match key {
                KeyCode::Esc => self.fullscreen_logs = false,
                KeyCode::Up | KeyCode::Char('k') => self.logs.scroll_up(),
                KeyCode::Down | KeyCode::Char('j') => self.logs.scroll_down(),
                KeyCode::PageUp => { for _ in 0..20 { self.logs.scroll_up(); } }
                KeyCode::PageDown => { for _ in 0..20 { self.logs.scroll_down(); } }
                _ => {}
            }
            return;
        }

        // Close popups first
        if self.show_help {
            match key {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                    self.show_help = false;
                }
                _ => {}
            }
            return;
        }

        // Setup wizard takes priority
        if self.setup_wizard.needs_setup && !self.setup_wizard.dismissed {
            match self.setup_wizard.step {
                SetupStep::Welcome => {
                    match key {
                        KeyCode::Enter => {
                            // If Claude Code is detected, add subscription directly
                            if self.setup_wizard.detected_auth.is_some() && self.setup_wizard.use_detected_auth {
                                self.handle_add_subscription();
                            } else {
                                self.setup_wizard.next_step();
                            }
                        }
                        KeyCode::Esc | KeyCode::Char('s') => {
                            // Skip setup
                            self.setup_wizard.dismissed = true;
                        }
                        _ => {}
                    }
                }
                SetupStep::AddApiKey => {
                    match key {
                        KeyCode::Esc => {
                            // Skip setup
                            self.setup_wizard.dismissed = true;
                        }
                        KeyCode::Enter => {
                            if self.setup_wizard.is_valid() {
                                // Submit the subscription
                                self.handle_add_subscription();
                            }
                        }
                        KeyCode::Tab => {
                            self.setup_wizard.focus = if self.setup_wizard.focus == 0 { 1 } else { 0 };
                            self.setup_wizard.cursor = self.setup_wizard.current_field().len();
                        }
                        KeyCode::BackTab => {
                            self.setup_wizard.focus = if self.setup_wizard.focus == 0 { 1 } else { 0 };
                            self.setup_wizard.cursor = self.setup_wizard.current_field().len();
                        }
                        KeyCode::Backspace => {
                            if self.setup_wizard.use_detected_auth {
                                // Switch to manual mode when deleting
                                self.setup_wizard.use_detected_auth = false;
                            }
                            self.setup_wizard.delete_char();
                        }
                        KeyCode::Left => {
                            self.setup_wizard.cursor = self.setup_wizard.cursor.saturating_sub(1);
                        }
                        KeyCode::Right => {
                            let len = self.setup_wizard.current_field().len();
                            if self.setup_wizard.cursor < len {
                                self.setup_wizard.cursor += 1;
                            }
                        }
                        KeyCode::Char('d') if self.setup_wizard.detected_auth.is_some() && self.setup_wizard.focus == 1 => {
                            // Toggle between detected and manual API key
                            self.setup_wizard.use_detected_auth = !self.setup_wizard.use_detected_auth;
                            if self.setup_wizard.use_detected_auth {
                                self.setup_wizard.api_key.clear();
                            }
                            self.setup_wizard.cursor = 0;
                        }
                        KeyCode::Char(c) => {
                            // Typing in API key field disables detected auth
                            if self.setup_wizard.use_detected_auth && self.setup_wizard.focus == 1 {
                                self.setup_wizard.use_detected_auth = false;
                            }
                            self.setup_wizard.insert_char(c);
                        }
                        _ => {}
                    }
                }
                SetupStep::Complete => {
                    match key {
                        KeyCode::Enter | KeyCode::Esc => {
                            self.setup_wizard.dismissed = true;
                            self.setup_wizard.needs_setup = false;
                        }
                        _ => {}
                    }
                }
            }
            return;
        }

        if self.show_agent_details {
            match key {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                    self.show_agent_details = false;
                }
                _ => {}
            }
            return;
        }

        if self.show_task_details {
            match key {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                    self.show_task_details = false;
                }
                _ => {}
            }
            return;
        }

        if self.show_approvals {
            match key {
                KeyCode::Esc | KeyCode::Char('a') | KeyCode::Char('q') => {
                    self.show_approvals = false;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.approvals.previous();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.approvals.next();
                }
                KeyCode::Char('y') | KeyCode::Enter => {
                    // Approve selected
                    self.handle_approve();
                }
                KeyCode::Char('n') | KeyCode::Char('r') => {
                    // Reject selected
                    self.handle_reject();
                }
                KeyCode::Char('Y') => {
                    // Approve all
                    self.handle_approve_all();
                }
                _ => {}
            }
            return;
        }

        if self.show_spawn_dialog {
            match key {
                KeyCode::Esc => {
                    self.show_spawn_dialog = false;
                    self.spawn_dialog.reset();
                }
                KeyCode::Enter => {
                    if self.spawn_dialog.is_valid() {
                        self.handle_spawn_agent();
                        self.show_spawn_dialog = false;
                        self.spawn_dialog.reset();
                    }
                }
                KeyCode::Tab => {
                    self.spawn_dialog.next_field();
                }
                KeyCode::BackTab => {
                    self.spawn_dialog.prev_field();
                }
                KeyCode::Backspace => {
                    self.spawn_dialog.delete_char();
                }
                KeyCode::Left => {
                    self.spawn_dialog.move_cursor_left();
                }
                KeyCode::Right => {
                    self.spawn_dialog.move_cursor_right();
                }
                KeyCode::Home => {
                    self.spawn_dialog.move_cursor_home();
                }
                KeyCode::End => {
                    self.spawn_dialog.move_cursor_end();
                }
                KeyCode::Char(c) => {
                    self.spawn_dialog.insert_char(c);
                }
                _ => {}
            }
            return;
        }

        // Global keys - Ctrl combinations first (they need priority)
        match (key, modifiers.contains(KeyModifiers::CONTROL)) {
            // Ctrl+C or Ctrl+Q to quit
            (KeyCode::Char('c'), true) | (KeyCode::Char('q'), true) => {
                self.should_quit = true;
                return;
            }
            // Ctrl+K: Kill selected agent
            (KeyCode::Char('k'), true) => {
                self.handle_kill_agent();
                return;
            }
            // Ctrl+P: Pause/resume agent
            (KeyCode::Char('p'), true) => {
                self.handle_pause_resume_agent();
                return;
            }
            _ => {}
        }

        // Regular keys
        match key {
            // Quit
            KeyCode::Char('q') => self.should_quit = true,

            // Help
            KeyCode::Char('?') => self.show_help = true,

            // Panel navigation
            KeyCode::Tab => self.focus = self.focus.next(),
            KeyCode::BackTab => self.focus = self.focus.prev(),
            KeyCode::Char('1') => self.focus = Panel::Agents,
            KeyCode::Char('2') => {
                if self.focus == Panel::Stream {
                    self.fullscreen_stream = !self.fullscreen_stream;
                    // Force resize on next frame by resetting last size
                    self.last_stream_size = (0, 0);
                } else {
                    self.focus = Panel::Stream;
                }
            }
            KeyCode::Char('3') => self.focus = Panel::Tasks,
            KeyCode::Char('4') => {
                if self.focus == Panel::Logs {
                    self.fullscreen_logs = !self.fullscreen_logs;
                } else {
                    self.focus = Panel::Logs;
                    self.logs_collapsed = false; // Expand when focusing
                }
            }

            // Horizontal panel navigation (vim-style)
            KeyCode::Char('h') | KeyCode::Left => self.handle_left(),
            KeyCode::Char('l') | KeyCode::Right => self.handle_right(),

            // Vertical navigation
            KeyCode::Up | KeyCode::Char('k') => self.handle_up(),
            KeyCode::Down | KeyCode::Char('j') => self.handle_down(),

            // Selection/details
            KeyCode::Enter => self.handle_enter(),

            // Output panel scrolling
            KeyCode::Char('g') => self.handle_scroll_top(),
            KeyCode::Char('G') => self.handle_scroll_bottom(),
            KeyCode::PageUp => self.handle_page_up(),
            KeyCode::PageDown => self.handle_page_down(),

            // Actions
            KeyCode::Char(' ') => self.handle_space(),         // Pause/resume or toggle
            KeyCode::Char('x') | KeyCode::Delete => self.handle_delete(), // Kill/cancel
            KeyCode::Backspace => self.handle_delete(),        // Kill/cancel
            KeyCode::Char('c') => self.handle_cancel(),        // Cancel task
            KeyCode::Char('a') => self.show_approvals = true,  // Open approvals popup
            KeyCode::Char('n') => self.show_spawn_dialog = true, // New agent
            KeyCode::Char('r') => self.refresh(),              // Refresh
            KeyCode::Esc => self.handle_escape(),              // Clear selection / close

            // Fullscreen toggle (when on Stream panel - only if no agent, otherwise passthrough handles it)
            KeyCode::Char('f') if self.focus == Panel::Stream && self.stream.agent_id.is_none() => {
                self.fullscreen_stream = true;
            }

            // Agent filter cycling
            KeyCode::Char('F') => self.next_filter(),

            // Panel collapse toggles
            KeyCode::Char('t') => self.tasks_collapsed = !self.tasks_collapsed,
            KeyCode::Char('L') => self.logs_collapsed = !self.logs_collapsed,

            _ => {}
        }
    }

    /// Handle left arrow / h key
    fn handle_left(&mut self) {
        self.focus = match self.focus {
            Panel::Stream => Panel::Agents,
            Panel::Logs => Panel::Tasks,
            _ => self.focus,
        };
    }

    /// Handle right arrow / l key
    fn handle_right(&mut self) {
        self.focus = match self.focus {
            Panel::Agents => Panel::Stream,
            Panel::Tasks => Panel::Logs,
            _ => self.focus,
        };
    }

    fn handle_up(&mut self) {
        match self.focus {
            Panel::Agents => self.agents.previous(),
            Panel::Stream => self.stream.scroll_up(1),
            Panel::Tasks => self.tasks.previous(),
            Panel::Logs => self.logs.scroll_up(),
        }
    }

    fn handle_down(&mut self) {
        match self.focus {
            Panel::Agents => self.agents.next(),
            Panel::Stream => self.stream.scroll_down(1),
            Panel::Tasks => self.tasks.next(),
            Panel::Logs => self.logs.scroll_down(),
        }
    }

    fn handle_enter(&mut self) {
        match self.focus {
            Panel::Agents => {
                if self.agents.selected().is_some() {
                    self.show_agent_details = true;
                }
            }
            Panel::Stream => {
                // Toggle auto-scroll
                self.stream.auto_scroll = !self.stream.auto_scroll;
                if self.stream.auto_scroll {
                    self.stream.scroll_to_bottom();
                }
            }
            Panel::Tasks => {
                if self.tasks.selected().is_some() {
                    self.show_task_details = true;
                }
            }
            Panel::Logs => {}
        }
    }

    fn handle_scroll_top(&mut self) {
        if self.focus == Panel::Stream {
            self.stream.scroll_to_top();
        }
    }

    fn handle_scroll_bottom(&mut self) {
        if self.focus == Panel::Stream {
            self.stream.scroll_to_bottom();
        }
    }

    fn handle_page_up(&mut self) {
        if self.focus == Panel::Stream {
            self.stream.scroll_up(20);
        }
    }

    fn handle_page_down(&mut self) {
        if self.focus == Panel::Stream {
            self.stream.scroll_down(20);
        }
    }

    /// Handle add subscription from setup wizard
    fn handle_add_subscription(&mut self) {
        let name = self.setup_wizard.sub_name.trim().to_string();
        let api_key = match self.setup_wizard.get_api_key() {
            Some(key) => key,
            None => {
                self.setup_wizard.error = Some("No API key provided".to_string());
                return;
            }
        };

        self.logs.add_info("setup", &format!("Adding subscription '{}'...", name));
        self.send_action(Action::AddSubscription { name, api_key });
    }

    /// Handle spawn agent from dialog
    fn handle_spawn_agent(&mut self) {
        let repo = self.spawn_dialog.repo.trim().to_string();
        let namespace = if self.spawn_dialog.namespace.trim().is_empty() {
            None
        } else {
            Some(self.spawn_dialog.namespace.trim().to_string())
        };

        // Calculate initial PTY size based on current terminal size
        let (pty_rows, pty_cols) = self.calculate_stream_panel_size();

        self.logs.add_info("dashboard", &format!(
            "Spawning {} in {} (PTY: {}x{}, term: {}x{})",
            self.spawn_dialog.provider.name(),
            truncate(&repo, 20),
            pty_cols, pty_rows,
            self.terminal_size.1, self.terminal_size.0
        ));
        self.send_action(Action::SpawnAgent { repo, namespace, pty_rows, pty_cols });
    }

    /// Calculate the stream panel size based on current terminal dimensions
    fn calculate_stream_panel_size(&self) -> (u16, u16) {
        let (term_height, term_width) = self.terminal_size;

        if self.fullscreen_stream {
            return (term_height, term_width);
        }

        // Based on layout from render_main():
        // - Header: 3 lines, Footer: 2 lines
        // - Main area = term_height - 5
        // - When collapsed: top row is 98% of main, otherwise 55%
        // - Stream panel is 70% of width

        let main_height = term_height.saturating_sub(5);
        let both_collapsed = self.tasks_collapsed && self.logs_collapsed;
        let top_row_percent = if both_collapsed { 98 } else { 55 };

        let top_row_height = (main_height as u32 * top_row_percent / 100) as u16;
        let panel_height = top_row_height.saturating_sub(1); // minus title line

        let available_width = term_width.saturating_sub(1);
        let panel_width = (available_width as u32 * 70 / 100) as u16;

        (panel_height.max(10), panel_width.max(40))
    }

    /// Handle Ctrl+K - kill selected agent
    fn handle_kill_agent(&mut self) {
        if let Some(agent) = self.agents.selected() {
            let id = agent.id.clone();
            self.logs.add_info("dashboard", &format!("Killing agent {}...", &id[..8.min(id.len())]));
            self.send_action(Action::KillAgent { id });
        }
    }

    /// Handle Ctrl+P - pause/resume selected agent
    fn handle_pause_resume_agent(&mut self) {
        if let Some(agent) = self.agents.selected() {
            let id = agent.id.clone();
            match agent.status {
                AgentDisplayStatus::Working => {
                    self.logs.add_info("dashboard", &format!("Pausing agent {}...", &id[..8.min(id.len())]));
                    self.send_action(Action::PauseAgent { id });
                }
                AgentDisplayStatus::Paused => {
                    self.logs.add_info("dashboard", &format!("Resuming agent {}...", &id[..8.min(id.len())]));
                    self.send_action(Action::ResumeAgent { id });
                }
                _ => {}
            }
        }
    }

    /// Handle Space - context-sensitive action
    fn handle_space(&mut self) {
        match self.focus {
            Panel::Agents => self.handle_pause_resume_agent(),
            Panel::Stream => {
                // Toggle auto-scroll
                self.stream.auto_scroll = !self.stream.auto_scroll;
                if self.stream.auto_scroll {
                    self.stream.scroll_to_bottom();
                }
            }
            Panel::Tasks => {
                // Could be used to toggle task priority or start task
            }
            Panel::Logs => {}
        }
    }

    /// Handle Delete/Backspace/x - kill agent or cancel task
    fn handle_delete(&mut self) {
        match self.focus {
            Panel::Agents => self.handle_kill_agent(),
            Panel::Tasks => self.handle_cancel(),
            _ => {}
        }
    }

    /// Handle c - cancel selected task
    fn handle_cancel(&mut self) {
        if self.focus == Panel::Tasks || self.focus == Panel::Agents {
            if let Some(task) = self.tasks.selected() {
                let id = task.id.clone();
                self.logs.add_info("dashboard", &format!("Cancelling task {}...", &id[..8.min(id.len())]));
                self.send_action(Action::CancelTask { id });
            }
        }
    }

    /// Handle Escape - close popups or clear selection
    fn handle_escape(&mut self) {
        if self.show_help {
            self.show_help = false;
        } else if self.show_agent_details {
            self.show_agent_details = false;
        } else if self.show_task_details {
            self.show_task_details = false;
        } else if self.show_approvals {
            self.show_approvals = false;
        }
        // Could also deselect items if needed
    }

    /// Handle approve - approve the selected action
    fn handle_approve(&mut self) {
        if let Some(approval) = self.approvals.selected() {
            let id = approval.id.clone();
            self.logs.add_info("dashboard", &format!("Approving action {}...", &id[..8.min(id.len())]));
            self.send_action(Action::Approve { id });
        }
    }

    /// Handle reject - reject the selected action
    fn handle_reject(&mut self) {
        if let Some(approval) = self.approvals.selected() {
            let id = approval.id.clone();
            self.logs.add_info("dashboard", &format!("Rejecting action {}...", &id[..8.min(id.len())]));
            self.send_action(Action::Reject { id });
        }
    }

    /// Handle approve all - approve all pending actions
    fn handle_approve_all(&mut self) {
        let count = self.approvals.items.len();
        if count > 0 {
            self.logs.add_info("dashboard", &format!("Approving {} action(s)...", count));
            for approval in &self.approvals.items {
                self.send_action(Action::Approve { id: approval.id.clone() });
            }
        }
    }

    fn refresh(&mut self) {
        // Data is refreshed via the data channel in process_data_updates
        self.logs.add_info("dashboard", "Refreshing...");
    }

    fn on_tick(&mut self) {
        // Process any pending data updates
        self.process_data_updates();
    }

    fn process_data_updates(&mut self) {
        // Track if we need to request output for a different agent
        let mut request_output_for: Option<String> = None;

        // Collect all updates first to avoid borrow conflicts
        let updates: Vec<DataUpdate> = if let Some(ref rx) = self.data_rx {
            let mut updates = Vec::new();
            while let Ok(update) = rx.try_recv() {
                updates.push(update);
            }
            updates
        } else {
            Vec::new()
        };

        // Process collected updates
        for update in updates {
            match update {
                DataUpdate::Agents(agents) => {
                    let prev_selected = self.agents.selected().map(|a| a.id.clone());
                    self.agents.update_from_daemon(agents);
                    let new_selected = self.agents.selected().map(|a| a.id.clone());

                    // If selection changed, update output panel
                    if prev_selected != new_selected {
                        self.stream.set_agent(new_selected.clone());
                        request_output_for = new_selected;
                    }
                }
                DataUpdate::Tasks(tasks) => {
                    self.tasks.update_from_daemon(tasks);
                }
                DataUpdate::Status { connected, version, uptime_secs } => {
                    self.daemon_status = if connected {
                        DaemonStatus::Connected
                    } else {
                        DaemonStatus::Disconnected
                    };
                    self.daemon_version = version;
                    self.daemon_uptime = uptime_secs;
                }
                DataUpdate::AgentOutput { agent_id, lines, has_more } => {
                    self.stream.update(agent_id, lines, has_more);
                }
                DataUpdate::Approvals(approvals) => {
                    self.approvals.update_from_daemon(approvals);
                }
                DataUpdate::Log(entry) => {
                    self.logs.add(entry);
                }
                DataUpdate::Event(event) => {
                    self.handle_daemon_event(event);
                }
                DataUpdate::SubscriptionCount(count) => {
                    // Log if no subscriptions, but don't show wizard automatically
                    // The daemon auto-detects Claude Code credentials on startup
                    // Users can also add subscriptions via CLI: kage subscription add
                    if count == 0 && !self.setup_wizard.setup_complete {
                        self.logs.add_info("subscriptions", "No subscriptions found. Checking for Claude Code credentials...");
                        self.setup_wizard.setup_complete = true; // Don't repeat this message
                    }
                }
                DataUpdate::SubscriptionAdded { success, error } => {
                    if success {
                        self.setup_wizard.error = None;
                        self.setup_wizard.next_step(); // Move to Complete step
                        self.setup_wizard.setup_complete = true;
                        self.logs.add_success("setup", "Subscription added successfully!");
                    } else {
                        self.setup_wizard.error = error;
                    }
                }
            }
        }

        // Request output for newly selected agent
        if let Some(agent_id) = request_output_for {
            self.send_action(Action::RequestOutput { id: agent_id });
        }
    }

    /// Handle a real-time daemon event
    fn handle_daemon_event(&mut self, event: crate::daemon::protocol::DaemonEvent) {
        use crate::daemon::protocol::DaemonEvent;

        match event {
            DaemonEvent::AgentSpawned { agent } => {
                self.logs.add_info("event", &format!("Agent {} spawned", &agent.id.to_string()[..8]));
                // Agent list will be updated on next poll
            }
            DaemonEvent::AgentStatusChanged { id, old_status, new_status } => {
                self.logs.add_info("event", &format!(
                    "Agent {} status: {} → {}",
                    &id.to_string()[..8], old_status, new_status
                ));
                // Update agent status in list if present
                let id_str = id.to_string();
                if let Some(agent) = self.agents.items.iter_mut().find(|a| a.id == id_str) {
                    agent.status = match new_status.as_str() {
                        "running" => AgentDisplayStatus::Working,
                        "paused" => AgentDisplayStatus::Paused,
                        "completed" | "stopped" => AgentDisplayStatus::Idle,
                        _ => AgentDisplayStatus::Error,
                    };
                }
            }
            DaemonEvent::AgentStopped { id, reason } => {
                self.logs.add_info("event", &format!(
                    "Agent {} stopped: {}",
                    &id.to_string()[..8], reason
                ));
            }
            DaemonEvent::AgentOutput { id, line } => {
                // Add output line if this is the current agent
                let id_str = id.to_string();
                if self.stream.agent_id.as_ref() == Some(&id_str) {
                    self.stream.lines.push(OutputLineDisplay {
                        text: line.text,
                        is_error: line.is_error,
                        timestamp: line.timestamp,
                    });
                    // Auto-scroll to bottom if enabled
                    if self.stream.auto_scroll {
                        self.stream.scroll = 0;
                    }
                }
            }
            DaemonEvent::TaskAdded { task } => {
                self.logs.add_info("event", &format!(
                    "Task added: {}",
                    truncate(&task.goal, 40)
                ));
            }
            DaemonEvent::TaskStatusChanged { id, old_status, new_status } => {
                self.logs.add_info("event", &format!(
                    "Task {} status: {} → {}",
                    &id.to_string()[..8], old_status, new_status
                ));
            }
            DaemonEvent::TaskCompleted { id, success, message } => {
                let status = if success { "completed" } else { "failed" };
                let msg = message.map(|m| format!(": {}", truncate(&m, 40))).unwrap_or_default();
                self.logs.add_info("event", &format!(
                    "Task {} {}{}",
                    &id.to_string()[..8], status, msg
                ));
            }
            DaemonEvent::ApprovalCreated { approval } => {
                // Log as warning (using add directly since add_warning doesn't exist)
                self.logs.add(LogEntry {
                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    level: LogLevel::Warning,
                    source: "event".to_string(),
                    message: format!("Approval needed: {}", truncate(&approval.summary, 40)),
                });
                // Add to approvals list for popup
                self.approvals.items.push(ApprovalDisplayInfo {
                    id: approval.id.to_string(),
                    agent_id: approval.agent_id.to_string(),
                    summary: approval.summary,
                    context: approval.context,
                    created: "just now".to_string(),
                });
            }
            DaemonEvent::ApprovalResolved { id, approved } => {
                let status = if approved { "approved" } else { "rejected" };
                self.logs.add_info("event", &format!(
                    "Approval {} {}",
                    &id.to_string()[..8], status
                ));
                // Remove from approvals list
                let id_str = id.to_string();
                self.approvals.items.retain(|a| a.id != id_str);
            }
            DaemonEvent::Heartbeat { timestamp: _ } => {
                // Keep-alive, no action needed
            }
        }
    }

    /// Render the dashboard
    fn render(&self, f: &mut Frame) {
        let area = f.size();

        // Main background
        let bg = Block::default().style(Style::default().bg(self.c().bg));
        f.render_widget(bg, area);

        // Fullscreen stream mode - no decorations
        if self.fullscreen_stream {
            self.render_fullscreen_stream(f, area);
            return;
        }

        // Fullscreen logs mode
        if self.fullscreen_logs {
            self.render_fullscreen_logs(f, area);
            return;
        }

        // Main layout
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // Header
                Constraint::Min(10),    // Main content
                Constraint::Length(2),  // Footer
            ])
            .split(area);

        self.render_header(f, chunks[0]);
        self.render_main(f, chunks[1]);
        self.render_footer(f, chunks[2]);

        // Render popups
        if self.show_help {
            self.render_help_popup(f, area);
        }
        if self.show_agent_details {
            self.render_agent_details_popup(f, area);
        }
        if self.show_task_details {
            self.render_task_details_popup(f, area);
        }
        if self.show_approvals {
            self.render_approvals_popup(f, area);
        }
        if self.show_spawn_dialog {
            self.render_spawn_dialog(f, area);
        }

        // Setup wizard renders on top of everything
        if self.setup_wizard.needs_setup && !self.setup_wizard.dismissed {
            self.render_setup_wizard(f, area);
        }
    }

    /// Render the header
    fn render_header(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(20), // Logo
                Constraint::Min(20),    // Status
                Constraint::Length(25), // Stats
            ])
            .split(area);

        // Logo
        let logo = Paragraph::new(Line::from(vec![
            Span::styled("  影 ", Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD)),
            Span::styled("KAGE", Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD)),
        ]))
        .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(self.c().border)));
        f.render_widget(logo, chunks[0]);

        // Daemon status
        let status_color = match self.daemon_status {
            DaemonStatus::Connected => self.c().success,
            DaemonStatus::Disconnected => self.c().error,
            DaemonStatus::Connecting => self.c().warning,
        };
        let status_text = match self.daemon_status {
            DaemonStatus::Connected => "● Connected",
            DaemonStatus::Disconnected => "○ Disconnected",
            DaemonStatus::Connecting => "◐ Connecting...",
        };

        let status = Paragraph::new(Line::from(vec![
            Span::styled("Daemon: ", Style::default().fg(self.c().text_muted)),
            Span::styled(status_text, Style::default().fg(status_color)),
        ]))
        .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(self.c().border)));
        f.render_widget(status, chunks[1]);

        // Stats - Show agent counts by status
        let (working, idle, waiting) = self.agent_counts();
        let pending_tasks = self.tasks.items.iter().filter(|t| t.status == TaskDisplayStatus::Pending).count();
        let pending_approvals = self.approvals.items.len();

        let mut stats_spans = vec![
            // Working agents
            Span::styled(self.s().working, Style::default().fg(self.c().status_working)),
            Span::styled(format!("{}", working), Style::default().fg(self.c().status_working)),
            Span::styled(" ", Style::default()),
            // Idle agents
            Span::styled(self.s().idle, Style::default().fg(self.c().status_idle)),
            Span::styled(format!("{}", idle), Style::default().fg(self.c().status_idle)),
            Span::styled(" ", Style::default()),
            // Waiting agents (highlight if any)
            Span::styled(self.s().waiting, Style::default().fg(if waiting > 0 { self.c().status_waiting } else { self.c().text_muted })),
            Span::styled(format!("{}", waiting), Style::default().fg(if waiting > 0 { self.c().status_waiting } else { self.c().text_muted })),
        ];

        // Add pending tasks
        if pending_tasks > 0 {
            stats_spans.push(Span::styled("  ", Style::default()));
            stats_spans.push(Span::styled(
                format!("󰄬 {}", pending_tasks),
                Style::default().fg(self.c().info),
            ));
        }

        // Add approvals indicator if there are pending approvals
        if pending_approvals > 0 {
            stats_spans.push(Span::styled("  ", Style::default()));
            stats_spans.push(Span::styled(
                format!("⚠ {}", pending_approvals),
                Style::default().fg(self.c().error).add_modifier(Modifier::BOLD),
            ));
        }

        let stats = Paragraph::new(Line::from(stats_spans))
            .alignment(Alignment::Right)
            .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(self.c().border)));
        f.render_widget(stats, chunks[2]);
    }

    /// Render the main content area
    fn render_main(&self, f: &mut Frame, area: Rect) {
        // Determine bottom row height based on collapse state
        let both_collapsed = self.tasks_collapsed && self.logs_collapsed;
        let bottom_height = if both_collapsed { 2 } else { 45 };  // 2 lines when collapsed (just header)

        // Split into top and bottom rows with horizontal separator
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(100 - bottom_height), // Top: Agents + Stream
                Constraint::Length(1),                        // Horizontal separator
                Constraint::Percentage(bottom_height),        // Bottom: Tasks + Logs
            ])
            .split(area);

        // Top row: Agents (left) + separator + Stream (right)
        let top_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(30), // Agents
                Constraint::Length(1),      // Vertical separator
                Constraint::Percentage(70), // Stream
            ])
            .split(rows[0]);

        // Bottom row: Tasks (left) + separator + Logs (right)
        let bottom_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50), // Tasks
                Constraint::Length(1),      // Vertical separator
                Constraint::Percentage(50), // Logs
            ])
            .split(rows[2]);

        // Render panels
        self.render_agents_panel(f, top_cols[0]);
        self.render_stream_panel(f, top_cols[2]);
        self.render_tasks_panel(f, bottom_cols[0]);
        self.render_logs_panel(f, bottom_cols[2]);

        // Draw separators
        self.render_vertical_separator(f, top_cols[1]);
        self.render_horizontal_separator(f, rows[1]);
        self.render_vertical_separator(f, bottom_cols[1]);
    }

    /// Render a vertical separator line
    fn render_vertical_separator(&self, f: &mut Frame, area: Rect) {
        let sep: String = (0..area.height).map(|_| "│\n").collect();
        let widget = Paragraph::new(sep.trim_end())
            .style(Style::default().fg(self.c().border));
        f.render_widget(widget, area);
    }

    /// Render a horizontal separator line
    fn render_horizontal_separator(&self, f: &mut Frame, area: Rect) {
        let sep: String = (0..area.width).map(|_| "─").collect();
        let widget = Paragraph::new(sep)
            .style(Style::default().fg(self.c().border));
        f.render_widget(widget, area);
    }

    /// Render the agents panel
    fn render_agents_panel(&self, f: &mut Frame, area: Rect) {
        let is_focused = self.focus == Panel::Agents;

        // Build filter info for title
        let filter_text = match self.agent_filter {
            AgentFilter::All => "all",
            AgentFilter::Working => "working",
            AgentFilter::Idle => "idle",
            AgentFilter::Waiting => "waiting",
        };

        // Filter agents
        let filtered: Vec<&AgentInfo> = self.filtered_agents();

        let items: Vec<ListItem> = filtered.iter().map(|agent| {
            let status_color = self.agent_status_color(agent.status);
            let status_symbol = self.agent_status_symbol(agent.status);

            let line = Line::from(vec![
                Span::styled(format!("{} ", status_symbol), Style::default().fg(status_color)),
                Span::styled(&agent.name, Style::default().fg(self.c().text)),
                Span::styled(format!(" ({})", agent.namespace), Style::default().fg(self.c().text_muted)),
            ]);

            ListItem::new(vec![
                line,
                Line::from(Span::styled(
                    format!("  {} • {}", truncate(&agent.repository, 20), agent.started_at),
                    Style::default().fg(self.c().text_muted),
                )),
            ])
        }).collect();

        // Title shows filter and counts
        let (working, idle, waiting) = self.agent_counts();
        let title = format!(
            "Agents [1] {} {}{} {}{} {}{}",
            filter_text,
            self.s().working, working,
            self.s().idle, idle,
            self.s().waiting, waiting
        );

        // Render title line
        let title_area = Rect { height: 1, ..area };
        let title_widget = Paragraph::new(Line::from(Span::styled(
            title,
            Style::default().fg(if is_focused { self.c().accent } else { self.c().text_dim }).add_modifier(Modifier::BOLD),
        )));
        f.render_widget(title_widget, title_area);

        // Render list below title
        let list_area = Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area };
        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .bg(self.c().bg_highlight)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");

        f.render_stateful_widget(list, list_area, &mut self.agents.state.clone());
    }

    /// Render the stream panel (agent output)
    fn render_stream_panel(&self, f: &mut Frame, area: Rect) {
        let is_focused = self.focus == Panel::Stream;

        let title = if self.prefix_active {
            "Stream [2] ^B-".to_string()  // Waiting for command
        } else if let Some(ref agent_id) = self.stream.agent_id {
            let short_id = if agent_id.len() > 8 {
                &agent_id[..8]
            } else {
                agent_id
            };
            if is_focused {
                format!("Stream [2] {} │ ^B: menu", short_id)
            } else {
                format!("Stream [2] {}", short_id)
            }
        } else {
            "Stream [2]".to_string()
        };

        // Render title line
        let title_area = Rect { height: 1, ..area };
        let scroll_info = if self.stream.scroll > 0 {
            format!(" ↑{}", self.stream.scroll)
        } else {
            String::new()
        };
        let title_color = if self.prefix_active {
            self.c().warning  // Highlight prefix mode
        } else if is_focused {
            self.c().accent
        } else {
            self.c().text_dim
        };
        let title_line = Line::from(vec![
            Span::styled(
                title,
                Style::default().fg(title_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(scroll_info, Style::default().fg(self.c().text_muted)),
        ]);
        f.render_widget(Paragraph::new(title_line), title_area);

        let content_area = Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area };
        let inner_height = content_area.height as usize;

        if self.stream.lines.is_empty() {
            let empty_msg = if self.stream.agent_id.is_some() {
                "No output yet..."
            } else {
                "Select an agent to view output"
            };
            let content = Paragraph::new(Line::from(Span::styled(
                empty_msg,
                Style::default().fg(self.c().text_muted),
            )));
            f.render_widget(content, content_area);
        } else {
            // Calculate visible range (scroll is from bottom, 0 = at bottom)
            let total_lines = self.stream.lines.len();
            let end_idx = total_lines.saturating_sub(self.stream.scroll);
            let start_idx = end_idx.saturating_sub(inner_height);

            let max_width = content_area.width as usize;

            let visible_lines: Vec<Line> = self.stream.lines[start_idx..end_idx]
                .iter()
                .filter_map(|line| {
                    // Convert ANSI codes to ratatui styles
                    line.text.as_bytes().into_text().ok().and_then(|text| text.lines.into_iter().next())
                })
                .collect();

            // Don't wrap - PTY already wrapped at correct width
            let content = Paragraph::new(visible_lines);
            f.render_widget(content, content_area);
        }
    }

    /// Render fullscreen stream (no decorations, just output)
    fn render_fullscreen_stream(&self, f: &mut Frame, area: Rect) {
        // Content area (leave 1 line for status bar)
        let content_area = Rect {
            height: area.height.saturating_sub(1),
            ..area
        };
        let inner_height = content_area.height as usize;

        if self.stream.lines.is_empty() {
            let empty_msg = "Passthrough mode - type to interact";
            let content = Paragraph::new(Line::from(Span::styled(
                empty_msg,
                Style::default().fg(self.c().text_muted),
            )))
            .alignment(Alignment::Center);
            f.render_widget(content, content_area);
        } else {
            // Calculate visible range
            let total_lines = self.stream.lines.len();
            let end_idx = total_lines.saturating_sub(self.stream.scroll);
            let start_idx = end_idx.saturating_sub(inner_height);

            let visible_lines: Vec<Line> = self.stream.lines[start_idx..end_idx]
                .iter()
                .filter_map(|line| {
                    // Convert ANSI codes to ratatui styles
                    line.text.as_bytes().into_text().ok().and_then(|text| text.lines.into_iter().next())
                })
                .collect();

            // Don't wrap - PTY already wrapped at correct width
            let content = Paragraph::new(visible_lines)
                .style(Style::default().bg(self.c().bg));

            f.render_widget(content, content_area);

        }

        // Always show minimal status bar with mode indicator at bottom
        let status_area = Rect {
            x: area.x,
            y: area.y + area.height - 1,
            width: area.width,
            height: 1,
        };

        // Get mode info
        let (mode_name, mode_color) = self.current_mode();
        let mode_indicator = format!(" {} ", mode_name);

        // Build status line: left side hints, right side mode
        let left_status = if self.stream.scroll > 0 {
            format!(" ↑{} │ ^B: menu", self.stream.scroll)
        } else {
            " ^B: menu".to_string()
        };

        // Calculate spacing
        let mode_width = mode_indicator.len() as u16;
        let left_width = left_status.len() as u16;
        let spacing = (area.width.saturating_sub(left_width + mode_width)) as usize;

        let status_line = Line::from(vec![
            Span::styled(left_status, Style::default().fg(self.c().text_muted).bg(self.c().bg_surface)),
            Span::styled(" ".repeat(spacing), Style::default().bg(self.c().bg_surface)),
            Span::styled(mode_indicator, Style::default().fg(Color::Black).bg(mode_color).add_modifier(Modifier::BOLD)),
        ]);

        let status_bar = Paragraph::new(status_line);
        f.render_widget(status_bar, status_area);
    }

    /// Render fullscreen logs (no decorations, just log entries)
    fn render_fullscreen_logs(&self, f: &mut Frame, area: Rect) {
        let inner_height = area.height.saturating_sub(2) as usize; // Leave room for header and footer

        // Header
        let header_area = Rect { height: 1, ..area };
        let errors = self.logs.entries.iter().filter(|e| matches!(e.level, LogLevel::Error)).count();
        let header_text = format!(" Logs ({} entries, {} errors) | Press 4 or Esc to exit fullscreen ",
            self.logs.entries.len(), errors);
        let header = Paragraph::new(Line::from(Span::styled(
            header_text,
            Style::default().fg(self.c().text).bg(self.c().bg_surface).add_modifier(Modifier::BOLD),
        )));
        f.render_widget(header, header_area);

        // Content area
        let content_area = Rect {
            y: area.y + 1,
            height: area.height.saturating_sub(2),
            ..area
        };

        if self.logs.entries.is_empty() {
            let empty_msg = "No log entries";
            let content = Paragraph::new(Line::from(Span::styled(
                empty_msg,
                Style::default().fg(self.c().text_muted),
            )))
            .alignment(Alignment::Center);
            f.render_widget(content, content_area);
            return;
        }

        // Calculate visible range (scroll from bottom)
        let total_entries = self.logs.entries.len();
        let end_idx = total_entries.saturating_sub(self.logs.scroll);
        let start_idx = end_idx.saturating_sub(inner_height);

        let visible_logs: Vec<Line> = self.logs.entries[start_idx..end_idx]
            .iter()
            .map(|entry| {
                let level_style = match entry.level {
                    LogLevel::Error => Style::default().fg(self.c().error),
                    LogLevel::Warning => Style::default().fg(self.c().warning),
                    LogLevel::Info => Style::default().fg(self.c().accent),
                    LogLevel::Success => Style::default().fg(self.c().success),
                    LogLevel::Debug => Style::default().fg(self.c().text_muted),
                };
                let level_icon = match entry.level {
                    LogLevel::Error => "✗",
                    LogLevel::Warning => "⚠",
                    LogLevel::Info => "ℹ",
                    LogLevel::Success => "✓",
                    LogLevel::Debug => "·",
                };
                Line::from(vec![
                    Span::styled(format!("{} ", entry.timestamp), Style::default().fg(self.c().text_dim)),
                    Span::styled(format!("{} ", level_icon), level_style),
                    Span::styled(format!("[{}] ", entry.source), Style::default().fg(self.c().accent)),
                    Span::styled(&entry.message, Style::default().fg(self.c().text)),
                ])
            })
            .collect();

        let content = Paragraph::new(visible_logs)
            .style(Style::default().bg(self.c().bg))
            .wrap(ratatui::widgets::Wrap { trim: false });

        f.render_widget(content, content_area);

        // Footer with scroll info
        let footer_area = Rect {
            x: area.x,
            y: area.y + area.height - 1,
            width: area.width,
            height: 1,
        };
        let scroll_info = if self.logs.scroll > 0 {
            format!(" ↑{} entries ", self.logs.scroll)
        } else {
            " (latest) ".to_string()
        };
        let footer = Paragraph::new(Line::from(Span::styled(
            scroll_info,
            Style::default().fg(self.c().text_muted).bg(self.c().bg_surface),
        )))
        .alignment(Alignment::Right);
        f.render_widget(footer, footer_area);
    }

    /// Render the tasks panel
    fn render_tasks_panel(&self, f: &mut Frame, area: Rect) {
        let is_focused = self.focus == Panel::Tasks;

        let pending = self.tasks.items.iter().filter(|t| t.status == TaskDisplayStatus::Pending).count();
        let running = self.tasks.items.iter().filter(|t| t.status == TaskDisplayStatus::Running).count();

        // Collapsed title
        let collapse_symbol = if self.tasks_collapsed { self.s().collapsed } else { self.s().expanded };
        let title = format!(
            "{} Tasks [3] ({} pending, {} running)",
            collapse_symbol,
            pending,
            running
        );

        // Render title line
        let title_area = Rect { height: 1, ..area };
        let title_widget = Paragraph::new(Line::from(Span::styled(
            title,
            Style::default().fg(if is_focused { self.c().accent } else { self.c().text_dim }).add_modifier(Modifier::BOLD),
        )));
        f.render_widget(title_widget, title_area);

        // If collapsed, just render the header
        if self.tasks_collapsed {
            return;
        }

        let items: Vec<ListItem> = self.tasks.items.iter().map(|task| {
            let status_style = match task.status {
                TaskDisplayStatus::Pending => Style::default().fg(self.c().text_muted),
                TaskDisplayStatus::Running => Style::default().fg(self.c().success),
                TaskDisplayStatus::Paused => Style::default().fg(self.c().warning),
                TaskDisplayStatus::Completed => Style::default().fg(self.c().info),
                TaskDisplayStatus::Failed => Style::default().fg(self.c().error),
            };
            let status_icon = match task.status {
                TaskDisplayStatus::Pending => "○",
                TaskDisplayStatus::Running => "▶",
                TaskDisplayStatus::Paused => "⏸",
                TaskDisplayStatus::Completed => "✓",
                TaskDisplayStatus::Failed => "✗",
            };

            let agent_info = task.agent.as_ref().map(|a| format!(" → {}", a)).unwrap_or_default();

            let line = Line::from(vec![
                Span::styled(format!("{} ", status_icon), status_style),
                Span::styled(truncate(&task.goal, 30), Style::default().fg(self.c().text)),
            ]);

            ListItem::new(vec![
                line,
                Line::from(vec![
                    Span::styled(format!("  [{}/{}]", task.iterations, task.max_iterations), Style::default().fg(self.c().text_muted)),
                    Span::styled(agent_info, Style::default().fg(self.c().accent_bright)),
                ]),
            ])
        }).collect();

        // Render list below title
        let list_area = Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area };
        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .bg(self.c().bg_highlight)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");

        f.render_stateful_widget(list, list_area, &mut self.tasks.state.clone());
    }

    /// Render the logs panel
    fn render_logs_panel(&self, f: &mut Frame, area: Rect) {
        let is_focused = self.focus == Panel::Logs;

        let errors = self.logs.entries.iter().filter(|e| matches!(e.level, LogLevel::Error)).count();

        // Collapsed title
        let collapse_symbol = if self.logs_collapsed { self.s().collapsed } else { self.s().expanded };
        let title = if errors > 0 {
            format!("{} Logs [4] ({} entries, {} errors)", collapse_symbol, self.logs.entries.len(), errors)
        } else {
            format!("{} Logs [4] ({} entries)", collapse_symbol, self.logs.entries.len())
        };

        // Render title line
        let title_area = Rect { height: 1, ..area };
        let title_widget = Paragraph::new(Line::from(Span::styled(
            title,
            Style::default().fg(if is_focused { self.c().accent } else { self.c().text_dim }).add_modifier(Modifier::BOLD),
        )));
        f.render_widget(title_widget, title_area);

        // If collapsed, just render the header
        if self.logs_collapsed {
            return;
        }

        // Content area below title
        let content_area = Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area };

        // Calculate available width for message
        // Prefix: "HH:MM:SS ℹ [source] " = timestamp(8) + space(1) + level(2) + source(~10) = ~21 chars
        let max_msg_width = content_area.width.saturating_sub(21) as usize;

        let visible_logs: Vec<ListItem> = self.logs.entries
            .iter()
            .skip(self.logs.scroll)
            .take(content_area.height as usize)
            .map(|entry| {
                let level_style = match entry.level {
                    LogLevel::Info => Style::default().fg(self.c().info),
                    LogLevel::Success => Style::default().fg(self.c().success),
                    LogLevel::Warning => Style::default().fg(self.c().warning),
                    LogLevel::Error => Style::default().fg(self.c().error),
                    LogLevel::Debug => Style::default().fg(self.c().text_muted),
                };
                let level_char = match entry.level {
                    LogLevel::Info => "ℹ",
                    LogLevel::Success => "✓",
                    LogLevel::Warning => "⚠",
                    LogLevel::Error => "✗",
                    LogLevel::Debug => "·",
                };

                ListItem::new(Line::from(vec![
                    Span::styled(format!("{} ", entry.timestamp), Style::default().fg(self.c().text_muted)),
                    Span::styled(format!("{} ", level_char), level_style),
                    Span::styled(format!("[{}] ", entry.source), Style::default().fg(self.c().accent_bright)),
                    Span::styled(truncate(&entry.message, max_msg_width.max(10)), Style::default().fg(self.c().text)),
                ]))
            })
            .collect();

        let list = List::new(visible_logs);
        f.render_widget(list, content_area);
    }

    /// Get current mode for status bar
    fn current_mode(&self) -> (&'static str, Color) {
        // Check if we're in passthrough mode (keystrokes go to agent)
        let in_passthrough = (self.focus == Panel::Stream || self.fullscreen_stream)
            && self.stream.agent_id.is_some()
            && !self.show_help
            && !self.show_agent_details
            && !self.show_task_details
            && !self.show_approvals
            && !self.show_spawn_dialog
            && !self.setup_wizard.needs_setup;

        if self.prefix_active {
            ("PENDING", Color::Rgb(250, 180, 100))  // Orange - waiting for command
        } else if in_passthrough {
            ("INSERT", Color::Rgb(130, 200, 130))   // Green - input goes to agent
        } else {
            ("NORMAL", Color::Rgb(130, 170, 230))   // Blue - navigating UI
        }
    }

    /// Render the footer with mode indicator
    fn render_footer(&self, f: &mut Frame, area: Rect) {
        // Left side: key hints
        let panel_hint = match self.focus {
            Panel::Agents => "1:Agents",
            Panel::Stream => "2:Stream",
            Panel::Tasks => "3:Tasks",
            Panel::Logs => "4:Logs",
        };

        let hints = vec![
            ("n", "New"),
            ("F", "Filter"),
            ("t/L", "Toggle"),
            (panel_hint, ""),
            ("?", "Help"),
            ("q", "Quit"),
        ];

        let hint_spans: Vec<Span> = hints
            .iter()
            .flat_map(|(key, desc)| {
                vec![
                    Span::styled(format!(" {} ", key), Style::default().fg(self.c().bg).bg(self.c().accent)),
                    Span::styled(format!(" {}  ", desc), Style::default().fg(self.c().text_muted)),
                ]
            })
            .collect();

        // Right side: mode indicator (vim/helix style)
        let (mode_text, mode_color) = self.current_mode();
        let mode_indicator = format!(" {} ", mode_text);
        let mode_width = mode_indicator.len() as u16;

        // Split area for hints (left) and mode (right)
        let hints_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width.saturating_sub(mode_width + 1),
            height: area.height,
        };
        let mode_area = Rect {
            x: area.x + area.width.saturating_sub(mode_width),
            y: area.y,
            width: mode_width,
            height: area.height,
        };

        // Render hints on left
        let footer = Paragraph::new(Line::from(hint_spans));
        f.render_widget(footer, hints_area);

        // Render mode indicator on right
        let mode_widget = Paragraph::new(Line::from(Span::styled(
            mode_indicator,
            Style::default()
                .fg(Color::Black)
                .bg(mode_color)
                .add_modifier(Modifier::BOLD),
        )));
        f.render_widget(mode_widget, mode_area);
    }

    /// Render help popup
    fn render_help_popup(&self, f: &mut Frame, area: Rect) {
        let popup_area = centered_rect(60, 70, area);

        let help_text = vec![
            Line::from(""),
            Line::from(Span::styled("Navigation", Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Tab / Shift+Tab  ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Switch between panels", Style::default().fg(self.c().text)),
            ]),
            Line::from(vec![
                Span::styled("  1 / 2 / 3 / 4    ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Jump to panel", Style::default().fg(self.c().text)),
            ]),
            Line::from(vec![
                Span::styled("  ↑/k  ↓/j         ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Navigate up/down", Style::default().fg(self.c().text)),
            ]),
            Line::from(vec![
                Span::styled("  ←/h  →/l         ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Navigate left/right", Style::default().fg(self.c().text)),
            ]),
            Line::from(vec![
                Span::styled("  Enter            ", Style::default().fg(self.c().accent_bright)),
                Span::styled("View details", Style::default().fg(self.c().text)),
            ]),
            Line::from(vec![
                Span::styled("  Esc              ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Close popup", Style::default().fg(self.c().text)),
            ]),
            Line::from(""),
            Line::from(Span::styled("Agent Actions", Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  n                ", Style::default().fg(self.c().accent_bright)),
                Span::styled("New agent (spawn dialog)", Style::default().fg(self.c().text)),
            ]),
            Line::from(vec![
                Span::styled("  Ctrl+K / x / Del ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Kill agent", Style::default().fg(self.c().text)),
            ]),
            Line::from(vec![
                Span::styled("  Ctrl+P / Space   ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Pause/resume agent", Style::default().fg(self.c().text)),
            ]),
            Line::from(""),
            Line::from(Span::styled("Task Actions", Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  c / x / Del      ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Cancel task", Style::default().fg(self.c().text)),
            ]),
            Line::from(""),
            Line::from(Span::styled("Output Panel", Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  g / G            ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Scroll to top/bottom", Style::default().fg(self.c().text)),
            ]),
            Line::from(vec![
                Span::styled("  PgUp / PgDn      ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Page up/down", Style::default().fg(self.c().text)),
            ]),
            Line::from(vec![
                Span::styled("  Space            ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Toggle auto-scroll", Style::default().fg(self.c().text)),
            ]),
            Line::from(""),
            Line::from(Span::styled("Approvals", Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  a                ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Open approvals popup", Style::default().fg(self.c().text)),
            ]),
            Line::from(vec![
                Span::styled("  y / Enter        ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Approve selected action", Style::default().fg(self.c().text)),
            ]),
            Line::from(vec![
                Span::styled("  n / r            ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Reject selected action", Style::default().fg(self.c().text)),
            ]),
            Line::from(vec![
                Span::styled("  Y                ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Approve all pending", Style::default().fg(self.c().text)),
            ]),
            Line::from(""),
            Line::from(Span::styled("General", Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  r                ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Refresh", Style::default().fg(self.c().text)),
            ]),
            Line::from(vec![
                Span::styled("  ?                ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Help", Style::default().fg(self.c().text)),
            ]),
            Line::from(vec![
                Span::styled("  q / Ctrl+C       ", Style::default().fg(self.c().accent_bright)),
                Span::styled("Quit", Style::default().fg(self.c().text)),
            ]),
            Line::from(""),
            Line::from(Span::styled("Press Esc or ? to close", Style::default().fg(self.c().text_muted))),
        ];

        let help = Paragraph::new(help_text)
            .block(
                Block::default()
                    .title(Span::styled(" Help ", Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD)))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.c().accent))
                    .style(Style::default().bg(self.c().bg)),
            );

        f.render_widget(ratatui::widgets::Clear, popup_area);
        f.render_widget(help, popup_area);
    }

    /// Render agent details popup
    fn render_agent_details_popup(&self, f: &mut Frame, area: Rect) {
        let popup_area = centered_rect(70, 60, area);

        let content = if let Some(agent) = self.agents.selected() {
            let status_color = match agent.status {
                AgentDisplayStatus::Working => self.c().success,
                AgentDisplayStatus::Idle => self.c().text_muted,
                AgentDisplayStatus::Waiting => self.c().warning,
                AgentDisplayStatus::Paused => self.c().info,
                AgentDisplayStatus::Error => self.c().error,
            };
            let status_text = match agent.status {
                AgentDisplayStatus::Working => "Working",
                AgentDisplayStatus::Idle => "Idle",
                AgentDisplayStatus::Waiting => "Waiting",
                AgentDisplayStatus::Paused => "Paused",
                AgentDisplayStatus::Error => "Error",
            };

            vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Name:       ", Style::default().fg(self.c().text_muted)),
                    Span::styled(&agent.name, Style::default().fg(self.c().text).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled("  ID:         ", Style::default().fg(self.c().text_muted)),
                    Span::styled(&agent.id, Style::default().fg(self.c().accent_bright)),
                ]),
                Line::from(vec![
                    Span::styled("  Status:     ", Style::default().fg(self.c().text_muted)),
                    Span::styled(status_text, Style::default().fg(status_color)),
                ]),
                Line::from(vec![
                    Span::styled("  Namespace:  ", Style::default().fg(self.c().text_muted)),
                    Span::styled(&agent.namespace, Style::default().fg(self.c().text)),
                ]),
                Line::from(vec![
                    Span::styled("  Repository: ", Style::default().fg(self.c().text_muted)),
                    Span::styled(&agent.repository, Style::default().fg(self.c().text)),
                ]),
                Line::from(vec![
                    Span::styled("  Iterations: ", Style::default().fg(self.c().text_muted)),
                    Span::styled(format!("{} / {}", agent.iterations, agent.max_iterations), Style::default().fg(self.c().text)),
                ]),
                Line::from(vec![
                    Span::styled("  Started:    ", Style::default().fg(self.c().text_muted)),
                    Span::styled(&agent.started_at, Style::default().fg(self.c().text)),
                ]),
                Line::from(""),
                Line::from(Span::styled("  Current Action:", Style::default().fg(self.c().text_muted))),
                Line::from(Span::styled(format!("  {}", agent.current_action), Style::default().fg(self.c().accent_bright))),
                Line::from(""),
                Line::from(""),
                Line::from(Span::styled("  [Enter/Esc] Close   [k] Kill   [p] Pause   [a] Attach", Style::default().fg(self.c().text_muted))),
            ]
        } else {
            vec![Line::from(Span::styled("No agent selected", Style::default().fg(self.c().text_muted)))]
        };

        let details = Paragraph::new(content)
            .block(
                Block::default()
                    .title(Span::styled(" Agent Details ", Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD)))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.c().accent))
                    .style(Style::default().bg(self.c().bg)),
            );

        f.render_widget(ratatui::widgets::Clear, popup_area);
        f.render_widget(details, popup_area);
    }

    /// Render task details popup
    fn render_task_details_popup(&self, f: &mut Frame, area: Rect) {
        let popup_area = centered_rect(70, 60, area);

        let content = if let Some(task) = self.tasks.selected() {
            let status_color = match task.status {
                TaskDisplayStatus::Pending => self.c().text_muted,
                TaskDisplayStatus::Running => self.c().success,
                TaskDisplayStatus::Paused => self.c().warning,
                TaskDisplayStatus::Completed => self.c().info,
                TaskDisplayStatus::Failed => self.c().error,
            };
            let status_text = match task.status {
                TaskDisplayStatus::Pending => "Pending",
                TaskDisplayStatus::Running => "Running",
                TaskDisplayStatus::Paused => "Paused",
                TaskDisplayStatus::Completed => "Completed",
                TaskDisplayStatus::Failed => "Failed",
            };

            vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("  ID:         ", Style::default().fg(self.c().text_muted)),
                    Span::styled(&task.id, Style::default().fg(self.c().accent_bright)),
                ]),
                Line::from(vec![
                    Span::styled("  Status:     ", Style::default().fg(self.c().text_muted)),
                    Span::styled(status_text, Style::default().fg(status_color)),
                ]),
                Line::from(vec![
                    Span::styled("  Namespace:  ", Style::default().fg(self.c().text_muted)),
                    Span::styled(task.namespace.as_deref().unwrap_or("-"), Style::default().fg(self.c().text)),
                ]),
                Line::from(vec![
                    Span::styled("  Agent:      ", Style::default().fg(self.c().text_muted)),
                    Span::styled(task.agent.as_deref().unwrap_or("-"), Style::default().fg(self.c().text)),
                ]),
                Line::from(vec![
                    Span::styled("  Iterations: ", Style::default().fg(self.c().text_muted)),
                    Span::styled(format!("{} / {}", task.iterations, task.max_iterations), Style::default().fg(self.c().text)),
                ]),
                Line::from(vec![
                    Span::styled("  Created:    ", Style::default().fg(self.c().text_muted)),
                    Span::styled(&task.created, Style::default().fg(self.c().text)),
                ]),
                Line::from(""),
                Line::from(Span::styled("  Goal:", Style::default().fg(self.c().text_muted))),
                Line::from(Span::styled(format!("  {}", task.goal), Style::default().fg(self.c().accent_bright))),
                Line::from(""),
                Line::from(""),
                Line::from(Span::styled("  [Enter/Esc] Close   [r] Resume   [c] Cancel   [v] View Checkpoint", Style::default().fg(self.c().text_muted))),
            ]
        } else {
            vec![Line::from(Span::styled("No task selected", Style::default().fg(self.c().text_muted)))]
        };

        let details = Paragraph::new(content)
            .block(
                Block::default()
                    .title(Span::styled(" Task Details ", Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD)))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.c().accent))
                    .style(Style::default().bg(self.c().bg)),
            );

        f.render_widget(ratatui::widgets::Clear, popup_area);
        f.render_widget(details, popup_area);
    }

    /// Render setup wizard
    fn render_setup_wizard(&self, f: &mut Frame, area: Rect) {
        let popup_area = centered_rect(60, 60, area);

        let content = match self.setup_wizard.step {
            SetupStep::Welcome => {
                // If Claude Code is detected, show a simpler message
                if let Some(ref auth) = self.setup_wizard.detected_auth {
                    let email = auth.email.as_deref().unwrap_or("your account");
                    let masked = secrets::mask_token(&auth.access_token);
                    vec![
                        Line::from(""),
                        Line::from(Span::styled(
                            "  ✓ Claude Code Detected!",
                            Style::default().fg(self.c().success).add_modifier(Modifier::BOLD),
                        )),
                        Line::from(""),
                        Line::from(Span::styled(
                            "  Found existing Claude Code authentication:",
                            Style::default().fg(self.c().text),
                        )),
                        Line::from(""),
                        Line::from(vec![
                            Span::styled("    Account:  ", Style::default().fg(self.c().text_muted)),
                            Span::styled(email, Style::default().fg(self.c().accent)),
                        ]),
                        Line::from(vec![
                            Span::styled("    Token:    ", Style::default().fg(self.c().text_muted)),
                            Span::styled(masked, Style::default().fg(self.c().text)),
                        ]),
                        Line::from(""),
                        Line::from(""),
                        Line::from(Span::styled(
                            "  Press Enter to add this credential to Kage's subscription pool.",
                            Style::default().fg(self.c().text),
                        )),
                        Line::from(""),
                        Line::from(""),
                        Line::from(vec![
                            Span::styled(" Enter ", Style::default().fg(self.c().bg).bg(self.c().success)),
                            Span::styled(" Add credential  ", Style::default().fg(self.c().text_muted)),
                            Span::styled(" s/Esc ", Style::default().fg(self.c().bg).bg(self.c().warning)),
                            Span::styled(" Skip", Style::default().fg(self.c().text_muted)),
                        ]),
                    ]
                } else {
                    vec![
                        Line::from(""),
                        Line::from(Span::styled(
                            "  Welcome to Kage!",
                            Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD),
                        )),
                        Line::from(""),
                        Line::from(Span::styled(
                            "  影 Shadow agents for autonomous code work",
                            Style::default().fg(self.c().text_muted).add_modifier(Modifier::ITALIC),
                        )),
                        Line::from(""),
                        Line::from(""),
                        Line::from(Span::styled(
                            "  Before you can spawn agents, you need to configure",
                            Style::default().fg(self.c().text),
                        )),
                        Line::from(Span::styled(
                            "  your Claude Code API credentials.",
                            Style::default().fg(self.c().text),
                        )),
                        Line::from(""),
                        Line::from(Span::styled(
                            "  This wizard will help you set up:",
                            Style::default().fg(self.c().text),
                        )),
                        Line::from(""),
                        Line::from(Span::styled(
                            "    • Claude API subscription",
                            Style::default().fg(self.c().accent_bright),
                        )),
                        Line::from(""),
                        Line::from(""),
                        Line::from(""),
                        Line::from(vec![
                            Span::styled(" Enter ", Style::default().fg(self.c().bg).bg(self.c().success)),
                            Span::styled(" Continue  ", Style::default().fg(self.c().text_muted)),
                            Span::styled(" s/Esc ", Style::default().fg(self.c().bg).bg(self.c().warning)),
                            Span::styled(" Skip setup", Style::default().fg(self.c().text_muted)),
                        ]),
                    ]
                }
            }
            SetupStep::AddApiKey => {
                let name_focused = self.setup_wizard.focus == 0;
                let key_focused = self.setup_wizard.focus == 1;

                let name_style = if name_focused {
                    Style::default().fg(self.c().accent)
                } else {
                    Style::default().fg(self.c().text_muted)
                };
                let name_value_style = if name_focused {
                    Style::default().fg(self.c().text).bg(self.c().bg_highlight)
                } else {
                    Style::default().fg(self.c().text)
                };

                let key_style = if key_focused {
                    Style::default().fg(self.c().accent)
                } else {
                    Style::default().fg(self.c().text_muted)
                };
                let key_value_style = if key_focused {
                    Style::default().fg(self.c().text).bg(self.c().bg_highlight)
                } else {
                    Style::default().fg(self.c().text)
                };

                // Show name with cursor if focused
                let name_display = if name_focused {
                    let cursor = self.setup_wizard.cursor;
                    let name = &self.setup_wizard.sub_name;
                    if cursor < name.len() {
                        format!("  {}│{}", &name[..cursor], &name[cursor..])
                    } else {
                        format!("  {}│", name)
                    }
                } else {
                    format!("  {}", self.setup_wizard.sub_name)
                };

                // Show API key - either detected or manual entry
                let (key_display, key_display_style) = if self.setup_wizard.use_detected_auth {
                    if let Some(ref auth) = self.setup_wizard.detected_auth {
                        let masked = secrets::mask_token(&auth.access_token);
                        let account_info = auth.email.as_deref().unwrap_or("Claude Code");
                        (
                            format!("  ✓ {} ({})", masked, account_info),
                            Style::default().fg(self.c().success),
                        )
                    } else {
                        ("  (no detected credentials)".to_string(), Style::default().fg(self.c().text_muted))
                    }
                } else {
                    // Manual entry mode
                    let masked_key: String = "*".repeat(self.setup_wizard.api_key.len());
                    let display = if key_focused {
                        let cursor = self.setup_wizard.cursor;
                        if cursor < masked_key.len() {
                            format!("  {}│{}", &masked_key[..cursor], &masked_key[cursor..])
                        } else {
                            format!("  {}│", masked_key)
                        }
                    } else if masked_key.is_empty() {
                        "  (paste your API key)".to_string()
                    } else {
                        format!("  {}", masked_key)
                    };
                    let style = if self.setup_wizard.api_key.is_empty() && !key_focused {
                        Style::default().fg(self.c().text_muted).add_modifier(Modifier::ITALIC)
                    } else {
                        key_value_style
                    };
                    (display, style)
                };

                let can_submit = self.setup_wizard.is_valid();
                let submit_style = if can_submit {
                    Style::default().fg(self.c().bg).bg(self.c().success)
                } else {
                    Style::default().fg(self.c().text_muted).bg(self.c().bg_highlight)
                };

                // Build title based on whether we detected auth
                let title = if self.setup_wizard.detected_auth.is_some() {
                    "  ✓ Claude Code Detected"
                } else {
                    "  Add Claude Subscription"
                };
                let title_style = if self.setup_wizard.detected_auth.is_some() {
                    Style::default().fg(self.c().success).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD)
                };

                let mut lines = vec![
                    Line::from(""),
                    Line::from(Span::styled(title, title_style)),
                    Line::from(""),
                    Line::from(Span::styled(
                        "  Your API key will be stored securely in your OS keychain.",
                        Style::default().fg(self.c().text_muted),
                    )),
                    Line::from(""),
                    Line::from(Span::styled("  Name:", name_style)),
                    Line::from(Span::styled(name_display, name_value_style)),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("  API Key: ", key_style),
                        if !self.setup_wizard.use_detected_auth {
                            Span::styled("*", Style::default().fg(self.c().error))
                        } else {
                            Span::raw("")
                        },
                    ]),
                    Line::from(Span::styled(key_display, key_display_style)),
                    Line::from(""),
                ];

                if let Some(ref error) = self.setup_wizard.error {
                    lines.push(Line::from(Span::styled(
                        format!("  Error: {}", error),
                        Style::default().fg(self.c().error),
                    )));
                    lines.push(Line::from(""));
                }

                lines.push(Line::from(""));

                // Build footer with toggle option if detected auth available
                let mut footer_spans = vec![
                    Span::styled(" Tab ", Style::default().fg(self.c().bg).bg(self.c().accent)),
                    Span::styled(" Switch field  ", Style::default().fg(self.c().text_muted)),
                ];

                if self.setup_wizard.detected_auth.is_some() {
                    footer_spans.push(Span::styled(" d ", Style::default().fg(self.c().bg).bg(self.c().accent)));
                    if self.setup_wizard.use_detected_auth {
                        footer_spans.push(Span::styled(" Enter manually  ", Style::default().fg(self.c().text_muted)));
                    } else {
                        footer_spans.push(Span::styled(" Use detected  ", Style::default().fg(self.c().text_muted)));
                    }
                }

                footer_spans.extend([
                    Span::styled(" Enter ", submit_style),
                    Span::styled(" Save  ", Style::default().fg(self.c().text_muted)),
                    Span::styled(" Esc ", Style::default().fg(self.c().bg).bg(self.c().warning)),
                    Span::styled(" Skip", Style::default().fg(self.c().text_muted)),
                ]);

                lines.push(Line::from(footer_spans));

                lines
            }
            SetupStep::Complete => {
                vec![
                    Line::from(""),
                    Line::from(Span::styled(
                        "  ✓ Setup Complete!",
                        Style::default().fg(self.c().success).add_modifier(Modifier::BOLD),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "  Your subscription has been added successfully.",
                        Style::default().fg(self.c().text),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "  You can now spawn agents to work on your code.",
                        Style::default().fg(self.c().text),
                    )),
                    Line::from(""),
                    Line::from(Span::styled(
                        "  Press 'n' to spawn your first agent!",
                        Style::default().fg(self.c().accent_bright),
                    )),
                    Line::from(""),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled(" Enter ", Style::default().fg(self.c().bg).bg(self.c().success)),
                        Span::styled(" Continue to Dashboard", Style::default().fg(self.c().text_muted)),
                    ]),
                ]
            }
        };

        let title = match self.setup_wizard.step {
            SetupStep::Welcome => {
                if self.setup_wizard.detected_auth.is_some() {
                    " Claude Code Detected "
                } else {
                    " First-Time Setup "
                }
            }
            SetupStep::AddApiKey => " Add API Key ",
            SetupStep::Complete => " Setup Complete ",
        };

        let dialog = Paragraph::new(content)
            .block(
                Block::default()
                    .title(Span::styled(
                        title,
                        Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.c().accent))
                    .style(Style::default().bg(self.c().bg)),
            );

        f.render_widget(ratatui::widgets::Clear, popup_area);
        f.render_widget(dialog, popup_area);
    }

    /// Render spawn agent dialog
    fn render_spawn_dialog(&self, f: &mut Frame, area: Rect) {
        let popup_area = centered_rect(70, 40, area);

        // Build the dialog content
        let mut lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  Start an interactive agent session (like tmux)",
                Style::default().fg(self.c().text_muted),
            )),
            Line::from(""),
        ];

        // Repository field
        let repo_focused = self.spawn_dialog.focus == 0;
        let repo_style = if repo_focused {
            Style::default().fg(self.c().accent)
        } else {
            Style::default().fg(self.c().text_muted)
        };
        let repo_value_style = if repo_focused {
            Style::default().fg(self.c().text).bg(self.c().bg_highlight)
        } else {
            Style::default().fg(self.c().text)
        };

        lines.push(Line::from(vec![
            Span::styled("  Directory: ", repo_style),
        ]));

        // Show repo value with cursor if focused
        let repo_display = if repo_focused {
            let cursor_pos = self.spawn_dialog.cursor;
            let repo = &self.spawn_dialog.repo;
            if cursor_pos < repo.len() {
                format!("  {}│{}", &repo[..cursor_pos], &repo[cursor_pos..])
            } else {
                format!("  {}│", repo)
            }
        } else {
            format!("  {}", self.spawn_dialog.repo)
        };
        lines.push(Line::from(Span::styled(repo_display, repo_value_style)));
        lines.push(Line::from(""));

        // Provider selector
        let provider_focused = self.spawn_dialog.focus == 1;
        let provider_style = if provider_focused {
            Style::default().fg(self.c().accent)
        } else {
            Style::default().fg(self.c().text_muted)
        };

        lines.push(Line::from(vec![
            Span::styled("  Provider: ", provider_style),
        ]));

        // Show provider options
        let provider_display = if provider_focused {
            format!("  ◀ {} ▶", self.spawn_dialog.provider.name())
        } else {
            format!("  {}", self.spawn_dialog.provider.name())
        };
        let provider_value_style = if provider_focused {
            Style::default().fg(self.c().text).bg(self.c().bg_highlight)
        } else {
            Style::default().fg(self.c().text)
        };
        lines.push(Line::from(Span::styled(provider_display, provider_value_style)));
        lines.push(Line::from(""));

        // Namespace field (optional)
        let ns_focused = self.spawn_dialog.focus == 2;
        let ns_style = if ns_focused {
            Style::default().fg(self.c().accent)
        } else {
            Style::default().fg(self.c().text_muted)
        };
        let ns_value_style = if ns_focused {
            Style::default().fg(self.c().text).bg(self.c().bg_highlight)
        } else {
            Style::default().fg(self.c().text)
        };

        lines.push(Line::from(vec![
            Span::styled("  Namespace: ", ns_style),
            Span::styled("(optional)", Style::default().fg(self.c().text_muted)),
        ]));

        // Show namespace value with cursor if focused
        let ns_display = if ns_focused {
            let cursor_pos = self.spawn_dialog.cursor;
            let ns = &self.spawn_dialog.namespace;
            if cursor_pos < ns.len() {
                format!("  {}│{}", &ns[..cursor_pos], &ns[cursor_pos..])
            } else {
                format!("  {}│", ns)
            }
        } else if self.spawn_dialog.namespace.is_empty() {
            "  (default)".to_string()
        } else {
            format!("  {}", self.spawn_dialog.namespace)
        };
        let ns_display_style = if self.spawn_dialog.namespace.is_empty() && !ns_focused {
            Style::default().fg(self.c().text_muted).add_modifier(Modifier::ITALIC)
        } else {
            ns_value_style
        };
        lines.push(Line::from(Span::styled(ns_display, ns_display_style)));
        lines.push(Line::from(""));
        lines.push(Line::from(""));

        // Footer hints
        let can_submit = self.spawn_dialog.is_valid();
        let submit_style = if can_submit {
            Style::default().fg(self.c().bg).bg(self.c().success)
        } else {
            Style::default().fg(self.c().text_muted).bg(self.c().bg_highlight)
        };

        lines.push(Line::from(vec![
            Span::styled(" Tab ", Style::default().fg(self.c().bg).bg(self.c().accent)),
            Span::styled(" Next field  ", Style::default().fg(self.c().text_muted)),
            Span::styled(" Enter ", submit_style),
            Span::styled(" Spawn  ", Style::default().fg(self.c().text_muted)),
            Span::styled(" Esc ", Style::default().fg(self.c().bg).bg(self.c().error)),
            Span::styled(" Cancel", Style::default().fg(self.c().text_muted)),
        ]));

        let dialog = Paragraph::new(lines)
            .block(
                Block::default()
                    .title(Span::styled(
                        " New Agent ",
                        Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.c().accent))
                    .style(Style::default().bg(self.c().bg)),
            );

        f.render_widget(ratatui::widgets::Clear, popup_area);
        f.render_widget(dialog, popup_area);
    }

    /// Render approvals popup
    fn render_approvals_popup(&self, f: &mut Frame, area: Rect) {
        let popup_area = centered_rect(75, 70, area);

        let title = format!(
            " Pending Approvals ({}) ",
            self.approvals.items.len()
        );

        // Build list items
        let items: Vec<ListItem> = self.approvals.items.iter().map(|approval| {
            let short_id = if approval.id.len() > 8 {
                &approval.id[..8]
            } else {
                &approval.id
            };
            let short_agent = if approval.agent_id.len() > 8 {
                &approval.agent_id[..8]
            } else {
                &approval.agent_id
            };

            ListItem::new(vec![
                Line::from(vec![
                    Span::styled("⚠ ", Style::default().fg(self.c().warning)),
                    Span::styled(truncate(&approval.summary, 50), Style::default().fg(self.c().text)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  Agent: {} ", short_agent), Style::default().fg(self.c().accent_bright)),
                    Span::styled(format!("ID: {} ", short_id), Style::default().fg(self.c().text_muted)),
                    Span::styled(&approval.created, Style::default().fg(self.c().text_muted)),
                ]),
            ])
        }).collect();

        if items.is_empty() {
            let content = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled("  No pending approvals", Style::default().fg(self.c().text_muted))),
                Line::from(""),
                Line::from(Span::styled("  Agents will request approval for file writes, git commits,", Style::default().fg(self.c().text_muted))),
                Line::from(Span::styled("  and other sensitive actions based on the approval level.", Style::default().fg(self.c().text_muted))),
                Line::from(""),
                Line::from(""),
                Line::from(Span::styled("  [Esc/a] Close", Style::default().fg(self.c().text_muted))),
            ])
            .block(
                Block::default()
                    .title(Span::styled(&title, Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD)))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.c().accent))
                    .style(Style::default().bg(self.c().bg)),
            );

            f.render_widget(ratatui::widgets::Clear, popup_area);
            f.render_widget(content, popup_area);
        } else {
            // Split popup into list and footer
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(5),
                    Constraint::Length(3),
                ])
                .split(popup_area);

            let list = List::new(items)
                .block(
                    Block::default()
                        .title(Span::styled(&title, Style::default().fg(self.c().accent).add_modifier(Modifier::BOLD)))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(self.c().accent))
                        .style(Style::default().bg(self.c().bg)),
                )
                .highlight_style(
                    Style::default()
                        .bg(self.c().bg_highlight)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▸ ");

            let footer = Paragraph::new(Line::from(vec![
                Span::styled(" y/Enter ", Style::default().fg(self.c().bg).bg(self.c().success)),
                Span::styled(" Approve  ", Style::default().fg(self.c().text_muted)),
                Span::styled(" n/r ", Style::default().fg(self.c().bg).bg(self.c().error)),
                Span::styled(" Reject  ", Style::default().fg(self.c().text_muted)),
                Span::styled(" Y ", Style::default().fg(self.c().bg).bg(self.c().warning)),
                Span::styled(" Approve All  ", Style::default().fg(self.c().text_muted)),
                Span::styled(" Esc ", Style::default().fg(self.c().bg).bg(self.c().accent)),
                Span::styled(" Close", Style::default().fg(self.c().text_muted)),
            ]))
            .alignment(Alignment::Center)
            .block(Block::default().style(Style::default().bg(self.c().bg)));

            f.render_widget(ratatui::widgets::Clear, popup_area);
            f.render_stateful_widget(list, chunks[0], &mut self.approvals.state.clone());
            f.render_widget(footer, chunks[1]);
        }
    }
}

/// Helper to get a centered rect
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

/// Truncate string with ellipsis (handles multi-byte UTF-8 characters)
fn truncate(s: &str, max_len: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_len {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_len.saturating_sub(1)).collect();
        format!("{}…", truncated)
    }
}

/// Format a unix timestamp as "Xm ago" or "Xh ago"
fn format_duration_ago(timestamp: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let diff = now - timestamp;

    if diff < 60 {
        format!("{}s ago", diff)
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

/// Background task that fetches data from the daemon
async fn data_fetcher(
    tx: mpsc::Sender<DataUpdate>,
    action_rx: mpsc::Receiver<Action>,
    socket_path: std::path::PathBuf,
) {
    use crate::daemon::client::DaemonClient;
    use crate::daemon::protocol::Response;

    // Spawn event stream listener for real-time updates
    let event_tx = tx.clone();
    let event_socket_path = socket_path.clone();
    tokio::spawn(async move {
        event_stream_listener(event_tx, event_socket_path).await;
    });

    // Track which agent we should fetch output for
    let mut current_agent_id: Option<String> = None;

    // Pending actions to process
    let mut pending_actions: Vec<Action> = Vec::new();

    // Counter for metadata refresh (every N ticks)
    let mut metadata_tick: u32 = 0;

    // Track if we've checked subscriptions on startup
    let mut checked_subscriptions = false;

    loop {
        // Collect all pending actions (non-blocking)
        while let Ok(action) = action_rx.try_recv() {
            match &action {
                Action::RequestOutput { id } => {
                    current_agent_id = Some(id.clone());
                }
                _ => {
                    pending_actions.push(action);
                }
            }
        }

        // Try to connect and fetch data
        match DaemonClient::connect(&socket_path).await {
            Ok(mut client) => {
                // Send connected status
                let _ = tx.send(DataUpdate::Status {
                    connected: true,
                    version: None,
                    uptime_secs: None,
                });

                // Check subscriptions on first connection
                if !checked_subscriptions {
                    match client.list_subscriptions().await {
                        Ok(Response::SubscriptionList { subscriptions }) => {
                            let _ = tx.send(DataUpdate::SubscriptionCount(subscriptions.len()));
                        }
                        _ => {
                            // Assume no subscriptions if we can't fetch
                            let _ = tx.send(DataUpdate::SubscriptionCount(0));
                        }
                    }
                    checked_subscriptions = true;
                }

                // Process pending actions
                for action in pending_actions.drain(..) {
                    match action {
                        Action::SpawnAgent { repo, namespace, pty_rows, pty_cols } => {
                            let repo_path = std::path::PathBuf::from(&repo);
                            // No prompt - just run interactive claude session like tmux
                            // Pass PTY size so output is correctly sized from the start
                            match client.spawn_agent(repo_path, namespace, None, None, None, Some(pty_rows), Some(pty_cols)).await {
                                Ok(Response::AgentSpawned { id }) => {
                                    let _ = tx.send(DataUpdate::Log(LogEntry {
                                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                        level: LogLevel::Info,
                                        source: "spawn".to_string(),
                                        message: format!("Agent {} spawned successfully", &id.to_string()[..8]),
                                    }));
                                }
                                Ok(Response::Error { message }) => {
                                    let _ = tx.send(DataUpdate::Log(LogEntry {
                                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                        level: LogLevel::Error,
                                        source: "spawn".to_string(),
                                        message: format!("Spawn failed: {}", message),
                                    }));
                                }
                                Ok(_) => {
                                    // Unexpected response
                                }
                                Err(e) => {
                                    let _ = tx.send(DataUpdate::Log(LogEntry {
                                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                        level: LogLevel::Error,
                                        source: "spawn".to_string(),
                                        message: format!("Failed to spawn agent: {}", e),
                                    }));
                                }
                            }
                        }
                        Action::KillAgent { id } => {
                            if let Ok(agent_id) = id.parse::<crate::agent::AgentId>() {
                                let _ = client.kill_agent(agent_id, false).await;
                            }
                        }
                        Action::PauseAgent { id } => {
                            if let Ok(agent_id) = id.parse::<crate::agent::AgentId>() {
                                let _ = client.pause_agent(agent_id).await;
                            }
                        }
                        Action::ResumeAgent { id } => {
                            if let Ok(agent_id) = id.parse::<crate::agent::AgentId>() {
                                let _ = client.resume_agent(agent_id).await;
                            }
                        }
                        Action::CancelTask { id } => {
                            if let Ok(task_id) = id.parse::<crate::task::TaskId>() {
                                let _ = client.cancel_task(task_id).await;
                            }
                        }
                        Action::RequestOutput { .. } => {
                            // Already handled above
                        }
                        Action::Approve { id } => {
                            if let Ok(approval_id) = id.parse::<crate::task::ApprovalId>() {
                                let _ = client.approve(approval_id).await;
                            }
                        }
                        Action::Reject { id } => {
                            if let Ok(approval_id) = id.parse::<crate::task::ApprovalId>() {
                                let _ = client.reject(approval_id, None).await;
                            }
                        }
                        Action::AddSubscription { name, api_key } => {
                            match client.add_subscription(name.clone(), api_key).await {
                                Ok(_) => {
                                    let _ = tx.send(DataUpdate::SubscriptionAdded {
                                        success: true,
                                        error: None,
                                    });
                                }
                                Err(e) => {
                                    let _ = tx.send(DataUpdate::SubscriptionAdded {
                                        success: false,
                                        error: Some(e.to_string()),
                                    });
                                }
                            }
                        }
                        Action::ResizeAgent { id, rows, cols } => {
                            if let Ok(agent_id) = id.parse::<crate::agent::AgentId>() {
                                let _ = client.resize_agent(agent_id, rows, cols).await;
                            }
                        }
                        Action::SendInput { id, input } => {
                            if let Ok(agent_id) = id.parse::<crate::agent::AgentId>() {
                                let _ = client.send_input(agent_id, input).await;
                            }
                        }
                    }
                }

                // Fetch screen content for current agent - this is the fast path (every tick)
                if let Some(ref agent_id) = current_agent_id {
                    if let Ok(id) = agent_id.parse::<crate::agent::AgentId>() {
                        match client.get_screen_content(id).await {
                            Ok(Response::AgentOutput { lines, has_more }) => {
                                let display_lines: Vec<OutputLineDisplay> = lines
                                    .into_iter()
                                    .map(|l| OutputLineDisplay {
                                        text: l.text,
                                        is_error: l.is_error,
                                        timestamp: l.timestamp,
                                    })
                                    .collect();
                                let _ = tx.send(DataUpdate::AgentOutput {
                                    agent_id: agent_id.clone(),
                                    lines: display_lines,
                                    has_more,
                                });
                            }
                            _ => {}
                        }
                    }
                }

                // Fetch metadata less frequently (every 500ms = 25 ticks at 20ms)
                metadata_tick += 1;
                if metadata_tick >= 25 {
                    metadata_tick = 0;

                    // Fetch agents
                    match client.list_agents(None, true).await {
                        Ok(Response::AgentList { agents }) => {
                            // If no agent selected yet, select the first one
                            if current_agent_id.is_none() && !agents.is_empty() {
                                current_agent_id = Some(agents[0].id.to_string());
                            }
                            let _ = tx.send(DataUpdate::Agents(agents));
                        }
                        _ => {}
                    }

                    // Fetch tasks
                    match client.list_tasks(None).await {
                        Ok(Response::TaskList { tasks }) => {
                            let _ = tx.send(DataUpdate::Tasks(tasks));
                        }
                        _ => {}
                    }

                    // Fetch pending approvals
                    match client.list_approvals().await {
                        Ok(Response::ApprovalList { approvals }) => {
                            let _ = tx.send(DataUpdate::Approvals(approvals));
                        }
                        _ => {}
                    }

                    // Get status for version/uptime
                    match client.status().await {
                        Ok(Response::Status { version, uptime_secs, .. }) => {
                            let _ = tx.send(DataUpdate::Status {
                                connected: true,
                                version: Some(version),
                                uptime_secs: Some(uptime_secs),
                            });
                        }
                        _ => {}
                    }
                }
            }
            Err(_) => {
                // Send disconnected status
                let _ = tx.send(DataUpdate::Status {
                    connected: false,
                    version: None,
                    uptime_secs: None,
                });
            }
        }

        // Fast polling for responsive terminal (50fps)
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }
}

/// Background task that listens for real-time daemon events
async fn event_stream_listener(
    tx: mpsc::Sender<DataUpdate>,
    socket_path: std::path::PathBuf,
) {
    use crate::daemon::client::DaemonClient;

    fn now_str() -> String {
        chrono::Local::now().format("%H:%M:%S").to_string()
    }

    loop {
        // Try to connect and subscribe
        match DaemonClient::connect(&socket_path).await {
            Ok(mut client) => {
                // Subscribe to all events
                if client.subscribe(vec![]).await.is_ok() {
                    let _ = tx.send(DataUpdate::Log(LogEntry {
                        timestamp: now_str(),
                        level: LogLevel::Info,
                        source: "events".to_string(),
                        message: "Connected to real-time event stream".to_string(),
                    }));

                    // Read events in a loop
                    loop {
                        match client.read_event().await {
                            Ok(Some(event)) => {
                                // Forward event to dashboard
                                if tx.send(DataUpdate::Event(event)).is_err() {
                                    // Channel closed, exit
                                    return;
                                }
                            }
                            Ok(None) => {
                                // Connection closed
                                let _ = tx.send(DataUpdate::Log(LogEntry {
                                    timestamp: now_str(),
                                    level: LogLevel::Warning,
                                    source: "events".to_string(),
                                    message: "Event stream disconnected".to_string(),
                                }));
                                break;
                            }
                            Err(e) => {
                                let _ = tx.send(DataUpdate::Log(LogEntry {
                                    timestamp: now_str(),
                                    level: LogLevel::Error,
                                    source: "events".to_string(),
                                    message: format!("Event stream error: {}", e),
                                }));
                                break;
                            }
                        }
                    }
                }
            }
            Err(_) => {
                // Could not connect, will retry
            }
        }

        // Wait before reconnecting
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
    }
}

/// Run the interactive dashboard (public entry point)
pub async fn run() -> Result<()> {
    // Create channel for data updates
    let (tx, rx) = mpsc::channel();

    // Create channel for actions
    let (action_tx, action_rx) = mpsc::channel();

    // Get socket path
    let socket_path = crate::daemon::client::default_socket_path();

    // Spawn background data fetcher
    let fetch_tx = tx.clone();
    tokio::spawn(async move {
        data_fetcher(fetch_tx, action_rx, socket_path).await;
    });

    // Create dashboard with data channel
    let mut dashboard = Dashboard::new().with_data_channel(rx, action_tx);

    // Add initial log
    dashboard.logs.add_info("dashboard", "Starting dashboard...");

    // Run synchronously since TUI is blocking
    tokio::task::spawn_blocking(move || dashboard.run()).await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_panel_navigation() {
        assert_eq!(Panel::Agents.next(), Panel::Stream);
        assert_eq!(Panel::Stream.next(), Panel::Tasks);
        assert_eq!(Panel::Tasks.next(), Panel::Logs);
        assert_eq!(Panel::Logs.next(), Panel::Agents);

        assert_eq!(Panel::Agents.prev(), Panel::Logs);
        assert_eq!(Panel::Stream.prev(), Panel::Agents);
        assert_eq!(Panel::Tasks.prev(), Panel::Stream);
        assert_eq!(Panel::Logs.prev(), Panel::Tasks);
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hell…");
    }

    #[test]
    fn test_dashboard_default() {
        let dashboard = Dashboard::new();
        assert_eq!(dashboard.focus, Panel::Agents);
        assert!(!dashboard.show_help);
        assert!(!dashboard.should_quit);
    }
}
