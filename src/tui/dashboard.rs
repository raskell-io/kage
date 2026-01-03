//! Interactive dashboard for monitoring agents
//!
//! Shows:
//! - Active agents and their status
//! - Recent memory/context events
//! - Task queue and progress
//! - Real-time logs

use std::io::{self, Stdout};
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
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Gauge, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame, Terminal,
};

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
    /// Task list state
    tasks: TaskListState,
    /// Log entries
    logs: LogState,
    /// Daemon connection status
    daemon_status: DaemonStatus,
    /// Show help overlay
    show_help: bool,
    /// Show agent details popup
    show_agent_details: bool,
    /// Show task details popup
    show_task_details: bool,
    /// Last tick time (for animations)
    last_tick: Instant,
    /// Should quit
    should_quit: bool,
}

/// Which panel is focused
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Panel {
    Agents,
    Tasks,
    Logs,
}

impl Panel {
    fn next(&self) -> Self {
        match self {
            Self::Agents => Self::Tasks,
            Self::Tasks => Self::Logs,
            Self::Logs => Self::Agents,
        }
    }

    fn prev(&self) -> Self {
        match self {
            Self::Agents => Self::Logs,
            Self::Tasks => Self::Agents,
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
            items: Self::mock_agents(),
            state,
        }
    }

    fn mock_agents() -> Vec<AgentInfo> {
        vec![
            AgentInfo {
                id: "01HQXK...".into(),
                name: "shadow-1".into(),
                status: AgentDisplayStatus::Running,
                namespace: "backend".into(),
                repository: "api-service".into(),
                iterations: 3,
                max_iterations: 10,
                current_action: "Implementing user auth endpoint".into(),
                started_at: "2m ago".into(),
            },
            AgentInfo {
                id: "01HQXJ...".into(),
                name: "shadow-2".into(),
                status: AgentDisplayStatus::Idle,
                namespace: "frontend".into(),
                repository: "web-app".into(),
                iterations: 0,
                max_iterations: 10,
                current_action: "Waiting for task".into(),
                started_at: "15m ago".into(),
            },
            AgentInfo {
                id: "01HQXI...".into(),
                name: "shadow-3".into(),
                status: AgentDisplayStatus::Paused,
                namespace: "backend".into(),
                repository: "payment-service".into(),
                iterations: 7,
                max_iterations: 10,
                current_action: "Awaiting approval: git commit".into(),
                started_at: "1h ago".into(),
            },
        ]
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
            items: Self::mock_tasks(),
            state,
        }
    }

    fn mock_tasks() -> Vec<TaskInfo> {
        vec![
            TaskInfo {
                id: "01HQX1...".into(),
                goal: "Implement user authentication with JWT".into(),
                status: TaskDisplayStatus::Running,
                namespace: Some("backend".into()),
                agent: Some("shadow-1".into()),
                iterations: 3,
                max_iterations: 10,
                created: "5m ago".into(),
            },
            TaskInfo {
                id: "01HQX2...".into(),
                goal: "Fix payment webhook handling".into(),
                status: TaskDisplayStatus::Paused,
                namespace: Some("backend".into()),
                agent: Some("shadow-3".into()),
                iterations: 7,
                max_iterations: 10,
                created: "1h ago".into(),
            },
            TaskInfo {
                id: "01HQX3...".into(),
                goal: "Add dark mode toggle to settings".into(),
                status: TaskDisplayStatus::Pending,
                namespace: Some("frontend".into()),
                agent: None,
                iterations: 0,
                max_iterations: 10,
                created: "2h ago".into(),
            },
            TaskInfo {
                id: "01HQX4...".into(),
                goal: "Write unit tests for auth module".into(),
                status: TaskDisplayStatus::Completed,
                namespace: Some("backend".into()),
                agent: None,
                iterations: 5,
                max_iterations: 10,
                created: "3h ago".into(),
            },
            TaskInfo {
                id: "01HQX5...".into(),
                goal: "Update dependencies to latest".into(),
                status: TaskDisplayStatus::Failed,
                namespace: None,
                agent: None,
                iterations: 2,
                max_iterations: 10,
                created: "4h ago".into(),
            },
        ]
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

/// Log state
struct LogState {
    entries: Vec<LogEntry>,
    scroll: usize,
}

impl LogState {
    fn new() -> Self {
        Self {
            entries: Self::mock_logs(),
            scroll: 0,
        }
    }

    fn mock_logs() -> Vec<LogEntry> {
        vec![
            LogEntry {
                timestamp: "10:45:32".into(),
                level: LogLevel::Info,
                source: "shadow-1".into(),
                message: "Starting task: Implement user authentication".into(),
            },
            LogEntry {
                timestamp: "10:45:35".into(),
                level: LogLevel::Debug,
                source: "shadow-1".into(),
                message: "Reading file: src/auth/mod.rs".into(),
            },
            LogEntry {
                timestamp: "10:45:38".into(),
                level: LogLevel::Success,
                source: "memory".into(),
                message: "Pattern learned: JWT token validation".into(),
            },
            LogEntry {
                timestamp: "10:46:12".into(),
                level: LogLevel::Info,
                source: "shadow-1".into(),
                message: "Created file: src/auth/jwt.rs".into(),
            },
            LogEntry {
                timestamp: "10:46:45".into(),
                level: LogLevel::Warning,
                source: "shadow-3".into(),
                message: "Iteration limit approaching (7/10)".into(),
            },
            LogEntry {
                timestamp: "10:47:01".into(),
                level: LogLevel::Info,
                source: "shadow-3".into(),
                message: "Checkpoint saved at iteration 7".into(),
            },
            LogEntry {
                timestamp: "10:47:15".into(),
                level: LogLevel::Warning,
                source: "shadow-3".into(),
                message: "Awaiting approval for: git commit".into(),
            },
            LogEntry {
                timestamp: "10:48:22".into(),
                level: LogLevel::Error,
                source: "shadow-2".into(),
                message: "Task failed: dependency not found".into(),
            },
            LogEntry {
                timestamp: "10:49:01".into(),
                level: LogLevel::Success,
                source: "context".into(),
                message: "Memory shared: Error resolution for shadow-2".into(),
            },
            LogEntry {
                timestamp: "10:49:33".into(),
                level: LogLevel::Info,
                source: "daemon".into(),
                message: "Agent shadow-2 reassigned to new task".into(),
            },
        ]
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
            tasks: TaskListState::new(),
            logs: LogState::new(),
            daemon_status: DaemonStatus::Connected,
            show_help: false,
            show_agent_details: false,
            show_task_details: false,
            last_tick: Instant::now(),
            should_quit: false,
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

        // Global keys
        match key {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Tab => self.focus = self.focus.next(),
            KeyCode::BackTab => self.focus = self.focus.prev(),
            KeyCode::Char('1') => self.focus = Panel::Agents,
            KeyCode::Char('2') => self.focus = Panel::Tasks,
            KeyCode::Char('3') => self.focus = Panel::Logs,

            // Panel-specific
            KeyCode::Up | KeyCode::Char('k') => self.handle_up(),
            KeyCode::Down | KeyCode::Char('j') => self.handle_down(),
            KeyCode::Enter => self.handle_enter(),

            // Actions
            KeyCode::Char('n') if modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+N: New agent
            }
            KeyCode::Char('t') if modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+T: New task
            }
            KeyCode::Char('k') if modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+K: Kill agent
            }
            KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+P: Pause/resume
            }
            KeyCode::Char('a') if modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+A: Approve action
            }
            KeyCode::Char('r') => {
                // Refresh
                self.refresh();
            }

            _ => {}
        }
    }

    fn handle_up(&mut self) {
        match self.focus {
            Panel::Agents => self.agents.previous(),
            Panel::Tasks => self.tasks.previous(),
            Panel::Logs => self.logs.scroll_up(),
        }
    }

    fn handle_down(&mut self) {
        match self.focus {
            Panel::Agents => self.agents.next(),
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
            Panel::Tasks => {
                if self.tasks.selected().is_some() {
                    self.show_task_details = true;
                }
            }
            Panel::Logs => {}
        }
    }

    fn refresh(&mut self) {
        // TODO: Refresh data from daemon
        self.agents.items = AgentListState::mock_agents();
        self.tasks.items = TaskListState::mock_tasks();
        self.logs.entries = LogState::mock_logs();
    }

    fn on_tick(&mut self) {
        // TODO: Update data, animations, etc.
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

        let stats = Paragraph::new(Line::from(vec![
            Span::styled(format!("{}", running_agents), Style::default().fg(theme::SUCCESS)),
            Span::styled(" running  ", Style::default().fg(theme::DIM)),
            Span::styled(format!("{}", pending_tasks), Style::default().fg(theme::WARNING)),
            Span::styled(" pending", Style::default().fg(theme::DIM)),
        ]))
        .alignment(Alignment::Right)
        .block(Block::default().borders(Borders::BOTTOM).border_style(Style::default().fg(theme::BORDER)));
        f.render_widget(stats, chunks[2]);
    }

    /// Render the main content area
    fn render_main(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(35), // Agents
                Constraint::Percentage(35), // Tasks
                Constraint::Percentage(30), // Logs
            ])
            .split(area);

        self.render_agents_panel(f, chunks[0]);
        self.render_tasks_panel(f, chunks[1]);
        self.render_logs_panel(f, chunks[2]);
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
        let hints = vec![
            ("Tab", "Switch panel"),
            ("↑↓", "Navigate"),
            ("Enter", "Details"),
            ("r", "Refresh"),
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
                Span::styled("  1 / 2 / 3        ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Jump to panel", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  ↑/k  ↓/j         ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Navigate list items", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  Enter            ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("View details", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(""),
            Line::from(Span::styled("Actions", Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Ctrl+N           ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Spawn new agent", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  Ctrl+T           ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Add new task", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  Ctrl+K           ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Kill selected agent", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  Ctrl+P           ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Pause/resume agent", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  Ctrl+A           ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Approve pending action", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(""),
            Line::from(Span::styled("General", Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  r                ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Refresh data", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  ?                ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Toggle this help", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  q                ", Style::default().fg(theme::LIGHT_PURPLE)),
                Span::styled("Quit dashboard", Style::default().fg(theme::TEXT)),
            ]),
            Line::from(""),
            Line::from(Span::styled("Press any key to close", Style::default().fg(theme::DIM))),
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

/// Run the interactive dashboard (public entry point)
pub async fn run() -> Result<()> {
    let mut dashboard = Dashboard::new();
    // Run synchronously since TUI is blocking
    tokio::task::spawn_blocking(move || dashboard.run()).await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_panel_navigation() {
        assert_eq!(Panel::Agents.next(), Panel::Tasks);
        assert_eq!(Panel::Tasks.next(), Panel::Logs);
        assert_eq!(Panel::Logs.next(), Panel::Agents);

        assert_eq!(Panel::Agents.prev(), Panel::Logs);
        assert_eq!(Panel::Tasks.prev(), Panel::Agents);
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
