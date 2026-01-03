//! Interactive dashboard for monitoring agents
//!
//! Shows:
//! - Active agents and their status
//! - Recent memory/context events
//! - Task queue and progress
//! - Real-time logs

use std::io::{self, Stdout};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame, Terminal,
};

use crate::daemon::protocol::{AgentInfo as DaemonAgentInfo, ApprovalInfo as DaemonApprovalInfo, TaskInfo as DaemonTaskInfo};

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
}

/// Purple theme colors matching the mascot
mod theme {
    use ratatui::style::Color;

    pub const PURPLE: Color = Color::Rgb(167, 139, 250);
    pub const DARK_PURPLE: Color = Color::Rgb(139, 92, 246);
    pub const LIGHT_PURPLE: Color = Color::Rgb(196, 181, 253);
    pub const BG: Color = Color::Rgb(24, 24, 27);
    pub const BG_HIGHLIGHT: Color = Color::Rgb(39, 39, 42);
    pub const BORDER: Color = Color::Rgb(63, 63, 70);
    pub const DIM: Color = Color::Rgb(113, 113, 122);
    pub const TEXT: Color = Color::Rgb(244, 244, 245);
    pub const SUCCESS: Color = Color::Rgb(134, 239, 172);
    pub const WARNING: Color = Color::Rgb(253, 224, 71);
    pub const ERROR: Color = Color::Rgb(252, 165, 165);
    pub const INFO: Color = Color::Rgb(147, 197, 253);
}

/// Dashboard application state
pub struct Dashboard {
    /// Currently focused panel
    focus: Panel,
    /// Agent list state
    agents: AgentListState,
    /// Agent output state
    output: OutputState,
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
    /// Last tick time (for animations)
    last_tick: Instant,
    /// Last data refresh
    last_refresh: Instant,
    /// Should quit
    should_quit: bool,
    /// Data update receiver
    data_rx: Option<mpsc::Receiver<DataUpdate>>,
    /// Channel to send actions to daemon
    action_tx: Option<mpsc::Sender<Action>>,
}

/// Which panel is focused
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel {
    Agents,
    Output,
    Tasks,
    Logs,
}

impl Panel {
    fn next(&self) -> Self {
        match self {
            Self::Agents => Self::Output,
            Self::Output => Self::Tasks,
            Self::Tasks => Self::Logs,
            Self::Logs => Self::Agents,
        }
    }

    fn prev(&self) -> Self {
        match self {
            Self::Agents => Self::Logs,
            Self::Output => Self::Agents,
            Self::Tasks => Self::Output,
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
    Running,
    Idle,
    Paused,
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
                "running" => AgentDisplayStatus::Running,
                "paused" => AgentDisplayStatus::Paused,
                "completed" | "stopped" => AgentDisplayStatus::Idle,
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
    /// Create a new dashboard
    pub fn new() -> Self {
        Self {
            focus: Panel::Agents,
            agents: AgentListState::new(),
            output: OutputState::new(),
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
            last_tick: Instant::now(),
            last_refresh: Instant::now(),
            should_quit: false,
            data_rx: None,
            action_tx: None,
        }
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

    /// Run the dashboard
    pub fn run(&mut self) -> Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Run the event loop
        let result = self.run_loop(&mut terminal);

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result
    }

    /// Main event loop
    fn run_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        let tick_rate = Duration::from_millis(100);

        loop {
            terminal.draw(|f| self.render(f))?;

            let timeout = tick_rate
                .checked_sub(self.last_tick.elapsed())
                .unwrap_or(Duration::ZERO);

            if event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.handle_input(key.code, key.modifiers);
                    }
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

    /// Handle keyboard input
    fn handle_input(&mut self, key: KeyCode, modifiers: KeyModifiers) {
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
            KeyCode::Char('2') => self.focus = Panel::Output,
            KeyCode::Char('3') => self.focus = Panel::Tasks,
            KeyCode::Char('4') => self.focus = Panel::Logs,

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
            KeyCode::Char('r') => self.refresh(),              // Refresh
            KeyCode::Esc => self.handle_escape(),              // Clear selection / close

            _ => {}
        }
    }

    /// Handle left arrow / h key
    fn handle_left(&mut self) {
        self.focus = match self.focus {
            Panel::Output => Panel::Agents,
            Panel::Logs => Panel::Tasks,
            _ => self.focus,
        };
    }

    /// Handle right arrow / l key
    fn handle_right(&mut self) {
        self.focus = match self.focus {
            Panel::Agents => Panel::Output,
            Panel::Tasks => Panel::Logs,
            _ => self.focus,
        };
    }

    fn handle_up(&mut self) {
        match self.focus {
            Panel::Agents => self.agents.previous(),
            Panel::Output => self.output.scroll_up(1),
            Panel::Tasks => self.tasks.previous(),
            Panel::Logs => self.logs.scroll_up(),
        }
    }

    fn handle_down(&mut self) {
        match self.focus {
            Panel::Agents => self.agents.next(),
            Panel::Output => self.output.scroll_down(1),
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
            Panel::Output => {
                // Toggle auto-scroll
                self.output.auto_scroll = !self.output.auto_scroll;
                if self.output.auto_scroll {
                    self.output.scroll_to_bottom();
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
        if self.focus == Panel::Output {
            self.output.scroll_to_top();
        }
    }

    fn handle_scroll_bottom(&mut self) {
        if self.focus == Panel::Output {
            self.output.scroll_to_bottom();
        }
    }

    fn handle_page_up(&mut self) {
        if self.focus == Panel::Output {
            self.output.scroll_up(20);
        }
    }

    fn handle_page_down(&mut self) {
        if self.focus == Panel::Output {
            self.output.scroll_down(20);
        }
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
                AgentDisplayStatus::Running => {
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
            Panel::Output => {
                // Toggle auto-scroll
                self.output.auto_scroll = !self.output.auto_scroll;
                if self.output.auto_scroll {
                    self.output.scroll_to_bottom();
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

        if let Some(ref rx) = self.data_rx {
            // Process all available updates (non-blocking)
            while let Ok(update) = rx.try_recv() {
                match update {
                    DataUpdate::Agents(agents) => {
                        let prev_selected = self.agents.selected().map(|a| a.id.clone());
                        self.agents.update_from_daemon(agents);
                        let new_selected = self.agents.selected().map(|a| a.id.clone());

                        // If selection changed, update output panel
                        if prev_selected != new_selected {
                            self.output.set_agent(new_selected.clone());
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
                        self.output.update(agent_id, lines, has_more);
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
                // Update agent in list if present
                if let Some(agent) = self.agents.agents.iter_mut().find(|a| a.id == id.to_string()) {
                    agent.status = new_status;
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
                if let Some(current_agent) = &self.output.agent_id {
                    if current_agent == &id.to_string() {
                        self.output.lines.push(OutputLineDisplay {
                            text: line.text,
                            is_error: line.is_error,
                            timestamp: line.timestamp,
                        });
                        // Auto-scroll to bottom
                        if !self.output.lines.is_empty() {
                            self.output.scroll = self.output.lines.len().saturating_sub(1);
                        }
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
                self.logs.add_warning("event", &format!(
                    "Approval needed: {}",
                    truncate(&approval.summary, 40)
                ));
                // Add to approvals list for popup
                self.approvals.items.push(ApprovalItem {
                    id: approval.id.to_string(),
                    agent_id: approval.agent_id.to_string(),
                    summary: approval.summary,
                    action_type: format!("{:?}", approval.action),
                    created_at: approval.created_at,
                    context: approval.context,
                });
            }
            DaemonEvent::ApprovalResolved { id, approved } => {
                let status = if approved { "approved" } else { "rejected" };
                self.logs.add_info("event", &format!(
                    "Approval {} {}",
                    &id.to_string()[..8], status
                ));
                // Remove from approvals list
                self.approvals.items.retain(|a| a.id != id.to_string());
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
        let bg = Block::default().style(Style::default().bg(theme::BG));
        f.render_widget(bg, area);

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
            Span::styled("  影 ", Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD)),
            Span::styled("KAGE", Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD)),
        ]))
        .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(theme::BORDER)));
        f.render_widget(logo, chunks[0]);

        // Daemon status
        let status_color = match self.daemon_status {
            DaemonStatus::Connected => theme::SUCCESS,
            DaemonStatus::Disconnected => theme::ERROR,
            DaemonStatus::Connecting => theme::WARNING,
        };
        let status_text = match self.daemon_status {
            DaemonStatus::Connected => "● Connected",
            DaemonStatus::Disconnected => "○ Disconnected",
            DaemonStatus::Connecting => "◐ Connecting...",
        };

        let status = Paragraph::new(Line::from(vec![
            Span::styled("Daemon: ", Style::default().fg(theme::DIM)),
            Span::styled(status_text, Style::default().fg(status_color)),
        ]))
        .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(theme::BORDER)));
        f.render_widget(status, chunks[1]);

        // Stats
        let running_agents = self.agents.items.iter().filter(|a| a.status == AgentDisplayStatus::Running).count();
        let pending_tasks = self.tasks.items.iter().filter(|t| t.status == TaskDisplayStatus::Pending).count();
        let pending_approvals = self.approvals.items.len();

        let mut stats_spans = vec![
            Span::styled(format!("{}", running_agents), Style::default().fg(theme::SUCCESS)),
            Span::styled(" running  ", Style::default().fg(theme::DIM)),
            Span::styled(format!("{}", pending_tasks), Style::default().fg(theme::WARNING)),
            Span::styled(" pending", Style::default().fg(theme::DIM)),
        ];

        // Add approvals indicator if there are pending approvals
        if pending_approvals > 0 {
            stats_spans.push(Span::styled("  ", Style::default()));
            stats_spans.push(Span::styled(
                format!("⚠ {} approval{}", pending_approvals, if pending_approvals == 1 { "" } else { "s" }),
                Style::default().fg(theme::ERROR).add_modifier(Modifier::BOLD),
            ));
        }

        let stats = Paragraph::new(Line::from(stats_spans))
            .alignment(Alignment::Right)
            .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(theme::BORDER)));
        f.render_widget(stats, chunks[2]);
    }

    /// Render the main content area
    fn render_main(&self, f: &mut Frame, area: Rect) {
        // Split into top and bottom rows
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(55), // Top: Agents + Output
                Constraint::Percentage(45), // Bottom: Tasks + Logs
            ])
            .split(area);

        // Top row: Agents (left) + Output (right)
        let top_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(35), // Agents
                Constraint::Percentage(65), // Output
            ])
            .split(rows[0]);

        // Bottom row: Tasks (left) + Logs (right)
        let bottom_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50), // Tasks
                Constraint::Percentage(50), // Logs
            ])
            .split(rows[1]);

        self.render_agents_panel(f, top_cols[0]);
        self.render_output_panel(f, top_cols[1]);
        self.render_tasks_panel(f, bottom_cols[0]);
        self.render_logs_panel(f, bottom_cols[1]);
    }

    /// Render the agents panel
    fn render_agents_panel(&self, f: &mut Frame, area: Rect) {
        let is_focused = self.focus == Panel::Agents;
        let border_color = if is_focused { theme::PURPLE } else { theme::BORDER };

        let items: Vec<ListItem> = self.agents.items.iter().map(|agent| {
            let status_style = match agent.status {
                AgentDisplayStatus::Running => Style::default().fg(theme::SUCCESS),
                AgentDisplayStatus::Idle => Style::default().fg(theme::DIM),
                AgentDisplayStatus::Paused => Style::default().fg(theme::WARNING),
                AgentDisplayStatus::Error => Style::default().fg(theme::ERROR),
            };
            let status_icon = match agent.status {
                AgentDisplayStatus::Running => "▶",
                AgentDisplayStatus::Idle => "◯",
                AgentDisplayStatus::Paused => "⏸",
                AgentDisplayStatus::Error => "✗",
            };

            let line = Line::from(vec![
                Span::styled(format!("{} ", status_icon), status_style),
                Span::styled(&agent.name, Style::default().fg(theme::TEXT)),
                Span::styled(format!(" ({})", agent.namespace), Style::default().fg(theme::DIM)),
            ]);

            ListItem::new(vec![
                line,
                Line::from(Span::styled(
                    format!("  {} [{}/{}]", truncate(&agent.current_action, 25), agent.iterations, agent.max_iterations),
                    Style::default().fg(theme::DIM),
                )),
            ])
        }).collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .title(Span::styled(
                        " Agents [1] ",
                        Style::default().fg(if is_focused { theme::PURPLE } else { theme::DIM }).add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color)),
            )
            .highlight_style(
                Style::default()
                    .bg(theme::BG_HIGHLIGHT)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");

        f.render_stateful_widget(list, area, &mut self.agents.state.clone());
    }

    /// Render the output panel
    fn render_output_panel(&self, f: &mut Frame, area: Rect) {
        let is_focused = self.focus == Panel::Output;
        let border_color = if is_focused { theme::PURPLE } else { theme::BORDER };

        let title = if let Some(ref agent_id) = self.output.agent_id {
            let short_id = if agent_id.len() > 8 {
                &agent_id[..8]
            } else {
                agent_id
            };
            format!(" Output [2] - {} ", short_id)
        } else {
            " Output [2] ".to_string()
        };

        let inner_height = area.height.saturating_sub(2) as usize;

        if self.output.lines.is_empty() {
            // Empty state
            let empty_msg = if self.output.agent_id.is_some() {
                "No output yet..."
            } else {
                "Select an agent to view output"
            };

            let content = Paragraph::new(Line::from(Span::styled(
                empty_msg,
                Style::default().fg(theme::DIM),
            )))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .title(Span::styled(
                        &title,
                        Style::default()
                            .fg(if is_focused { theme::PURPLE } else { theme::DIM })
                            .add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color)),
            );

            f.render_widget(content, area);
        } else {
            // Calculate visible range (scroll is from bottom, 0 = at bottom)
            let total_lines = self.output.lines.len();
            let end_idx = total_lines.saturating_sub(self.output.scroll);
            let start_idx = end_idx.saturating_sub(inner_height);

            let visible_lines: Vec<Line> = self.output.lines[start_idx..end_idx]
                .iter()
                .map(|line| {
                    let text_style = if line.is_error {
                        Style::default().fg(theme::ERROR)
                    } else {
                        Style::default().fg(theme::TEXT)
                    };

                    Line::from(Span::styled(&line.text, text_style))
                })
                .collect();

            // Scroll indicator
            let scroll_info = if self.output.scroll > 0 {
                format!(" ↑{} ", self.output.scroll)
            } else if self.output.has_more {
                " ... ".to_string()
            } else {
                String::new()
            };

            let content = Paragraph::new(visible_lines)
                .block(
                    Block::default()
                        .title(Span::styled(
                            &title,
                            Style::default()
                                .fg(if is_focused { theme::PURPLE } else { theme::DIM })
                                .add_modifier(Modifier::BOLD),
                        ))
                        .title_bottom(Line::from(Span::styled(
                            &scroll_info,
                            Style::default().fg(theme::DIM),
                        )).alignment(Alignment::Right))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(border_color)),
                )
                .wrap(ratatui::widgets::Wrap { trim: false });

            f.render_widget(content, area);
        }
    }

    /// Render the tasks panel
    fn render_tasks_panel(&self, f: &mut Frame, area: Rect) {
        let is_focused = self.focus == Panel::Tasks;
        let border_color = if is_focused { theme::PURPLE } else { theme::BORDER };

        let items: Vec<ListItem> = self.tasks.items.iter().map(|task| {
            let status_style = match task.status {
                TaskDisplayStatus::Pending => Style::default().fg(theme::DIM),
                TaskDisplayStatus::Running => Style::default().fg(theme::SUCCESS),
                TaskDisplayStatus::Paused => Style::default().fg(theme::WARNING),
                TaskDisplayStatus::Completed => Style::default().fg(theme::INFO),
                TaskDisplayStatus::Failed => Style::default().fg(theme::ERROR),
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
                Span::styled(truncate(&task.goal, 30), Style::default().fg(theme::TEXT)),
            ]);

            ListItem::new(vec![
                line,
                Line::from(vec![
                    Span::styled(format!("  [{}/{}]", task.iterations, task.max_iterations), Style::default().fg(theme::DIM)),
                    Span::styled(agent_info, Style::default().fg(theme::LIGHT_PURPLE)),
                ]),
            ])
        }).collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .title(Span::styled(
                        " Tasks [2] ",
                        Style::default().fg(if is_focused { theme::PURPLE } else { theme::DIM }).add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color)),
            )
            .highlight_style(
                Style::default()
                    .bg(theme::BG_HIGHLIGHT)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");

        f.render_stateful_widget(list, area, &mut self.tasks.state.clone());
    }

    /// Render the logs panel
    fn render_logs_panel(&self, f: &mut Frame, area: Rect) {
        let is_focused = self.focus == Panel::Logs;
        let border_color = if is_focused { theme::PURPLE } else { theme::BORDER };

        let visible_logs: Vec<ListItem> = self.logs.entries
            .iter()
            .skip(self.logs.scroll)
            .take(area.height.saturating_sub(2) as usize)
            .map(|entry| {
                let level_style = match entry.level {
                    LogLevel::Info => Style::default().fg(theme::INFO),
                    LogLevel::Success => Style::default().fg(theme::SUCCESS),
                    LogLevel::Warning => Style::default().fg(theme::WARNING),
                    LogLevel::Error => Style::default().fg(theme::ERROR),
                    LogLevel::Debug => Style::default().fg(theme::DIM),
                };
                let level_char = match entry.level {
                    LogLevel::Info => "ℹ",
                    LogLevel::Success => "✓",
                    LogLevel::Warning => "⚠",
                    LogLevel::Error => "✗",
                    LogLevel::Debug => "·",
                };

                ListItem::new(Line::from(vec![
                    Span::styled(format!("{} ", entry.timestamp), Style::default().fg(theme::DIM)),
                    Span::styled(format!("{} ", level_char), level_style),
                    Span::styled(format!("[{}] ", entry.source), Style::default().fg(theme::LIGHT_PURPLE)),
                    Span::styled(truncate(&entry.message, 30), Style::default().fg(theme::TEXT)),
                ]))
            })
            .collect();

        let list = List::new(visible_logs)
            .block(
                Block::default()
                    .title(Span::styled(
                        " Logs [3] ",
                        Style::default().fg(if is_focused { theme::PURPLE } else { theme::DIM }).add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border_color)),
            );

        f.render_widget(list, area);
    }

    /// Render the footer
    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let panel_hint = match self.focus {
            Panel::Agents => "1:Agents",
            Panel::Output => "2:Output",
            Panel::Tasks => "3:Tasks",
            Panel::Logs => "4:Logs",
        };
        let hints = vec![
            ("Tab", "Panel"),
            (panel_hint, ""),
            ("↑↓/jk", "Scroll"),
            ("Enter", "Select"),
            ("?", "Help"),
            ("q", "Quit"),
        ];

        let hint_spans: Vec<Span> = hints
            .iter()
            .flat_map(|(key, desc)| {
                vec![
                    Span::styled(format!(" {} ", key), Style::default().fg(theme::BG).bg(theme::PURPLE)),
                    Span::styled(format!(" {}  ", desc), Style::default().fg(theme::DIM)),
                ]
            })
            .collect();

        let footer = Paragraph::new(Line::from(hint_spans));
        f.render_widget(footer, area);
    }

    /// Render help popup
    fn render_help_popup(&self, f: &mut Frame, area: Rect) {
        let popup_area = centered_rect(60, 70, area);

        let help_text = vec![
            Line::from(""),
            Line::from(Span::styled("Navigation", Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Tab / Shift+Tab  ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Switch between panels", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  1 / 2 / 3 / 4    ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Jump to panel", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  ↑/k  ↓/j         ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Navigate up/down", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  ←/h  →/l         ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Navigate left/right", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  Enter            ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("View details", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  Esc              ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Close popup", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(""),
            Line::from(Span::styled("Agent Actions", Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Ctrl+K / x / Del ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Kill agent", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  Ctrl+P / Space   ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Pause/resume agent", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(""),
            Line::from(Span::styled("Task Actions", Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  c / x / Del      ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Cancel task", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(""),
            Line::from(Span::styled("Output Panel", Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  g / G            ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Scroll to top/bottom", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  PgUp / PgDn      ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Page up/down", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  Space            ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Toggle auto-scroll", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(""),
            Line::from(Span::styled("Approvals", Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  a                ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Open approvals popup", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  y / Enter        ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Approve selected action", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  n / r            ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Reject selected action", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  Y                ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Approve all pending", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(""),
            Line::from(Span::styled("General", Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  r                ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Refresh", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  ?                ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Help", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  q / Ctrl+C       ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Quit", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(""),
            Line::from(Span::styled("Press Esc or ? to close", Style::default().fg(theme::DIM))),
        ];

        let help = Paragraph::new(help_text)
            .block(
                Block::default()
                    .title(Span::styled(" Help ", Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD)))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme::PURPLE))
                    .style(Style::default().bg(theme::BG)),
            );

        f.render_widget(ratatui::widgets::Clear, popup_area);
        f.render_widget(help, popup_area);
    }

    /// Render agent details popup
    fn render_agent_details_popup(&self, f: &mut Frame, area: Rect) {
        let popup_area = centered_rect(70, 60, area);

        let content = if let Some(agent) = self.agents.selected() {
            let status_color = match agent.status {
                AgentDisplayStatus::Running => theme::SUCCESS,
                AgentDisplayStatus::Idle => theme::DIM,
                AgentDisplayStatus::Paused => theme::WARNING,
                AgentDisplayStatus::Error => theme::ERROR,
            };
            let status_text = match agent.status {
                AgentDisplayStatus::Running => "Running",
                AgentDisplayStatus::Idle => "Idle",
                AgentDisplayStatus::Paused => "Paused",
                AgentDisplayStatus::Error => "Error",
            };

            vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Name:       ", Style::default().fg(theme::DIM)),
                    Span::styled(&agent.name, Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled("  ID:         ", Style::default().fg(theme::DIM)),
                    Span::styled(&agent.id, Style::default().fg(theme::LIGHT_PURPLE)),
                ]),
                Line::from(vec![
                    Span::styled("  Status:     ", Style::default().fg(theme::DIM)),
                    Span::styled(status_text, Style::default().fg(status_color)),
                ]),
                Line::from(vec![
                    Span::styled("  Namespace:  ", Style::default().fg(theme::DIM)),
                    Span::styled(&agent.namespace, Style::default().fg(theme::TEXT)),
                ]),
                Line::from(vec![
                    Span::styled("  Repository: ", Style::default().fg(theme::DIM)),
                    Span::styled(&agent.repository, Style::default().fg(theme::TEXT)),
                ]),
                Line::from(vec![
                    Span::styled("  Iterations: ", Style::default().fg(theme::DIM)),
                    Span::styled(format!("{} / {}", agent.iterations, agent.max_iterations), Style::default().fg(theme::TEXT)),
                ]),
                Line::from(vec![
                    Span::styled("  Started:    ", Style::default().fg(theme::DIM)),
                    Span::styled(&agent.started_at, Style::default().fg(theme::TEXT)),
                ]),
                Line::from(""),
                Line::from(Span::styled("  Current Action:", Style::default().fg(theme::DIM))),
                Line::from(Span::styled(format!("  {}", agent.current_action), Style::default().fg(theme::LIGHT_PURPLE))),
                Line::from(""),
                Line::from(""),
                Line::from(Span::styled("  [Enter/Esc] Close   [k] Kill   [p] Pause   [a] Attach", Style::default().fg(theme::DIM))),
            ]
        } else {
            vec![Line::from(Span::styled("No agent selected", Style::default().fg(theme::DIM)))]
        };

        let details = Paragraph::new(content)
            .block(
                Block::default()
                    .title(Span::styled(" Agent Details ", Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD)))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme::PURPLE))
                    .style(Style::default().bg(theme::BG)),
            );

        f.render_widget(ratatui::widgets::Clear, popup_area);
        f.render_widget(details, popup_area);
    }

    /// Render task details popup
    fn render_task_details_popup(&self, f: &mut Frame, area: Rect) {
        let popup_area = centered_rect(70, 60, area);

        let content = if let Some(task) = self.tasks.selected() {
            let status_color = match task.status {
                TaskDisplayStatus::Pending => theme::DIM,
                TaskDisplayStatus::Running => theme::SUCCESS,
                TaskDisplayStatus::Paused => theme::WARNING,
                TaskDisplayStatus::Completed => theme::INFO,
                TaskDisplayStatus::Failed => theme::ERROR,
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
                    Span::styled("  ID:         ", Style::default().fg(theme::DIM)),
                    Span::styled(&task.id, Style::default().fg(theme::LIGHT_PURPLE)),
                ]),
                Line::from(vec![
                    Span::styled("  Status:     ", Style::default().fg(theme::DIM)),
                    Span::styled(status_text, Style::default().fg(status_color)),
                ]),
                Line::from(vec![
                    Span::styled("  Namespace:  ", Style::default().fg(theme::DIM)),
                    Span::styled(task.namespace.as_deref().unwrap_or("-"), Style::default().fg(theme::TEXT)),
                ]),
                Line::from(vec![
                    Span::styled("  Agent:      ", Style::default().fg(theme::DIM)),
                    Span::styled(task.agent.as_deref().unwrap_or("-"), Style::default().fg(theme::TEXT)),
                ]),
                Line::from(vec![
                    Span::styled("  Iterations: ", Style::default().fg(theme::DIM)),
                    Span::styled(format!("{} / {}", task.iterations, task.max_iterations), Style::default().fg(theme::TEXT)),
                ]),
                Line::from(vec![
                    Span::styled("  Created:    ", Style::default().fg(theme::DIM)),
                    Span::styled(&task.created, Style::default().fg(theme::TEXT)),
                ]),
                Line::from(""),
                Line::from(Span::styled("  Goal:", Style::default().fg(theme::DIM))),
                Line::from(Span::styled(format!("  {}", task.goal), Style::default().fg(theme::LIGHT_PURPLE))),
                Line::from(""),
                Line::from(""),
                Line::from(Span::styled("  [Enter/Esc] Close   [r] Resume   [c] Cancel   [v] View Checkpoint", Style::default().fg(theme::DIM))),
            ]
        } else {
            vec![Line::from(Span::styled("No task selected", Style::default().fg(theme::DIM)))]
        };

        let details = Paragraph::new(content)
            .block(
                Block::default()
                    .title(Span::styled(" Task Details ", Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD)))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme::PURPLE))
                    .style(Style::default().bg(theme::BG)),
            );

        f.render_widget(ratatui::widgets::Clear, popup_area);
        f.render_widget(details, popup_area);
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
                    Span::styled("⚠ ", Style::default().fg(theme::WARNING)),
                    Span::styled(truncate(&approval.summary, 50), Style::default().fg(theme::TEXT)),
                ]),
                Line::from(vec![
                    Span::styled(format!("  Agent: {} ", short_agent), Style::default().fg(theme::LIGHT_PURPLE)),
                    Span::styled(format!("ID: {} ", short_id), Style::default().fg(theme::DIM)),
                    Span::styled(&approval.created, Style::default().fg(theme::DIM)),
                ]),
            ])
        }).collect();

        if items.is_empty() {
            let content = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled("  No pending approvals", Style::default().fg(theme::DIM))),
                Line::from(""),
                Line::from(Span::styled("  Agents will request approval for file writes, git commits,", Style::default().fg(theme::DIM))),
                Line::from(Span::styled("  and other sensitive actions based on the approval level.", Style::default().fg(theme::DIM))),
                Line::from(""),
                Line::from(""),
                Line::from(Span::styled("  [Esc/a] Close", Style::default().fg(theme::DIM))),
            ])
            .block(
                Block::default()
                    .title(Span::styled(&title, Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD)))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme::PURPLE))
                    .style(Style::default().bg(theme::BG)),
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
                        .title(Span::styled(&title, Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD)))
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(theme::PURPLE))
                        .style(Style::default().bg(theme::BG)),
                )
                .highlight_style(
                    Style::default()
                        .bg(theme::BG_HIGHLIGHT)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▸ ");

            let footer = Paragraph::new(Line::from(vec![
                Span::styled(" y/Enter ", Style::default().fg(theme::BG).bg(theme::SUCCESS)),
                Span::styled(" Approve  ", Style::default().fg(theme::DIM)),
                Span::styled(" n/r ", Style::default().fg(theme::BG).bg(theme::ERROR)),
                Span::styled(" Reject  ", Style::default().fg(theme::DIM)),
                Span::styled(" Y ", Style::default().fg(theme::BG).bg(theme::WARNING)),
                Span::styled(" Approve All  ", Style::default().fg(theme::DIM)),
                Span::styled(" Esc ", Style::default().fg(theme::BG).bg(theme::PURPLE)),
                Span::styled(" Close", Style::default().fg(theme::DIM)),
            ]))
            .alignment(Alignment::Center)
            .block(Block::default().style(Style::default().bg(theme::BG)));

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

/// Truncate string with ellipsis
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len.saturating_sub(1)])
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

                // Process pending actions
                for action in pending_actions.drain(..) {
                    match action {
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
                    }
                }

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

                // Fetch output for current agent
                if let Some(ref agent_id) = current_agent_id {
                    if let Ok(id) = agent_id.parse::<crate::agent::AgentId>() {
                        match client.get_output(id, 500).await {
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
            Err(_) => {
                // Send disconnected status
                let _ = tx.send(DataUpdate::Status {
                    connected: false,
                    version: None,
                    uptime_secs: None,
                });
            }
        }

        // Wait before next fetch (can be longer now since events are real-time)
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}

/// Background task that listens for real-time daemon events
async fn event_stream_listener(
    tx: mpsc::Sender<DataUpdate>,
    socket_path: std::path::PathBuf,
) {
    use crate::daemon::client::DaemonClient;

    loop {
        // Try to connect and subscribe
        match DaemonClient::connect(&socket_path).await {
            Ok(mut client) => {
                // Subscribe to all events
                if client.subscribe(vec![]).await.is_ok() {
                    let _ = tx.send(DataUpdate::Log(LogEntry {
                        timestamp: Instant::now(),
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
                                    timestamp: Instant::now(),
                                    level: LogLevel::Warning,
                                    source: "events".to_string(),
                                    message: "Event stream disconnected".to_string(),
                                }));
                                break;
                            }
                            Err(e) => {
                                let _ = tx.send(DataUpdate::Log(LogEntry {
                                    timestamp: Instant::now(),
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
        assert_eq!(Panel::Agents.next(), Panel::Output);
        assert_eq!(Panel::Output.next(), Panel::Tasks);
        assert_eq!(Panel::Tasks.next(), Panel::Logs);
        assert_eq!(Panel::Logs.next(), Panel::Agents);

        assert_eq!(Panel::Agents.prev(), Panel::Logs);
        assert_eq!(Panel::Output.prev(), Panel::Agents);
        assert_eq!(Panel::Tasks.prev(), Panel::Output);
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
