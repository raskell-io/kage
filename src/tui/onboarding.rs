//! First-run onboarding wizard
//!
//! Guides new users through initial setup:
//! - Welcome screen with mascot
//! - API key configuration (stored in OS keychain)
//! - Default namespace setup
//! - Feature tour
//! - Completion

use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame, Terminal,
};

use crate::secrets::{self, ClaudeCodeAuth, SecretScope};

/// Purple theme colors matching the mascot
mod theme {
    use ratatui::style::Color;

    pub const PURPLE: Color = Color::Rgb(167, 139, 250);
    pub const DARK_PURPLE: Color = Color::Rgb(139, 92, 246);
    pub const LIGHT_PURPLE: Color = Color::Rgb(196, 181, 253);
    pub const VIOLET: Color = Color::Rgb(167, 139, 250);
    pub const MOON: Color = Color::Rgb(221, 214, 254);
    pub const SPARKLE: Color = Color::Rgb(192, 180, 252);
    pub const DIM: Color = Color::Rgb(113, 113, 122);
    pub const TEXT: Color = Color::Rgb(244, 244, 245);
    pub const SUCCESS: Color = Color::Rgb(134, 239, 172);
    pub const WARNING: Color = Color::Rgb(253, 224, 71);
    pub const ERROR: Color = Color::Rgb(252, 165, 165);
}

/// Compact mascot for the wizard
const MASCOT_SMALL: &str = r#"
           ·  ✦  )
                (    ☽
    ▄████████▄  )
  ▄████████████▄  ·
 ████  ◠◠  ██████ ✧
 █████    ████████
  ▀████████████▀
    ▀████████▀  ·
   ·    ✦
"#;

/// Onboarding wizard state
pub struct OnboardingWizard {
    /// Current step in the wizard
    current_step: WizardStep,
    /// API key input buffer
    api_key_input: String,
    /// Whether API key input is masked
    api_key_masked: bool,
    /// Namespace name input buffer
    namespace_input: String,
    /// Current feature tour page
    tour_page: usize,
    /// Whether to skip certain steps
    skip_api_key: bool,
    /// Error message to display
    error_message: Option<String>,
    /// Success message to display
    success_message: Option<String>,
    /// Whether wizard completed successfully
    completed: bool,
    /// Detected Claude Code authentication (if any)
    detected_auth: Option<ClaudeCodeAuth>,
    /// Whether user wants to use detected auth
    use_detected_auth: bool,
}

/// Wizard steps
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WizardStep {
    Welcome,
    ApiKey,
    Namespace,
    FeatureTour,
    Complete,
}

impl WizardStep {
    fn index(&self) -> usize {
        match self {
            Self::Welcome => 0,
            Self::ApiKey => 1,
            Self::Namespace => 2,
            Self::FeatureTour => 3,
            Self::Complete => 4,
        }
    }

    fn total() -> usize {
        5
    }

    fn next(&self) -> Option<Self> {
        match self {
            Self::Welcome => Some(Self::ApiKey),
            Self::ApiKey => Some(Self::Namespace),
            Self::Namespace => Some(Self::FeatureTour),
            Self::FeatureTour => Some(Self::Complete),
            Self::Complete => None,
        }
    }

    fn prev(&self) -> Option<Self> {
        match self {
            Self::Welcome => None,
            Self::ApiKey => Some(Self::Welcome),
            Self::Namespace => Some(Self::ApiKey),
            Self::FeatureTour => Some(Self::Namespace),
            Self::Complete => Some(Self::FeatureTour),
        }
    }
}

/// Feature tour content
struct TourPage {
    title: &'static str,
    icon: &'static str,
    description: &'static str,
    details: &'static [&'static str],
}

const TOUR_PAGES: &[TourPage] = &[
    TourPage {
        title: "Shadow Agents",
        icon: "  ",
        description: "Autonomous Claude Code agents that work while you're away",
        details: &[
            "Spawn agents with specific goals",
            "Agents iterate autonomously on tasks",
            "Configurable iteration limits and checkpoints",
            "Resume paused agents with guidance",
        ],
    },
    TourPage {
        title: "Namespaces",
        icon: "  ",
        description: "Organize repositories into logical groups",
        details: &[
            "Group related repos (backend, frontend, infra)",
            "Agents in a namespace share context",
            "Per-namespace configuration and secrets",
            "Easy multi-repo coordination",
        ],
    },
    TourPage {
        title: "Shared Memory",
        icon: "  ",
        description: "Agents learn from each other's discoveries",
        details: &[
            "Patterns learned by one agent help others",
            "Error resolutions are shared",
            "Codebase knowledge accumulates",
            "Configurable sharing scope",
        ],
    },
    TourPage {
        title: "Task Queue",
        icon: "  ",
        description: "Queue up work for your agents",
        details: &[
            "Add tasks with natural language goals",
            "Automatic checkpoints during execution",
            "Approval workflows (on-write, on-commit)",
            "Resume from checkpoints anytime",
        ],
    },
    TourPage {
        title: "Secure Secrets",
        icon: "  ",
        description: "API keys stored safely in your OS keychain",
        details: &[
            "macOS Keychain / Linux Secret Service",
            "Never stored on disk",
            "Scoped to global, namespace, or repo",
            "Automatic credential injection",
        ],
    },
];

impl Default for OnboardingWizard {
    fn default() -> Self {
        Self::new()
    }
}

impl OnboardingWizard {
    /// Create a new onboarding wizard
    pub fn new() -> Self {
        // Try to detect existing Claude Code authentication
        let detected_auth = secrets::detect_claude_code_auth();
        let use_detected = detected_auth.is_some();

        Self {
            current_step: WizardStep::Welcome,
            api_key_input: String::new(),
            api_key_masked: true,
            namespace_input: String::from("default"),
            tour_page: 0,
            skip_api_key: false,
            error_message: None,
            success_message: None,
            completed: false,
            detected_auth,
            use_detected_auth: use_detected,
        }
    }

    /// Run the onboarding wizard
    pub fn run(&mut self) -> Result<bool> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Run the wizard loop
        let result = self.run_loop(&mut terminal);

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result.map(|_| self.completed)
    }

    /// Main event loop
    fn run_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        loop {
            terminal.draw(|f| self.render(f))?;

            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        // Clear messages on any keypress
                        self.error_message = None;
                        self.success_message = None;

                        match self.handle_input(key.code) {
                            InputResult::Continue => {}
                            InputResult::Quit => return Ok(()),
                            InputResult::Complete => {
                                self.completed = true;
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
    }

    /// Handle keyboard input
    fn handle_input(&mut self, key: KeyCode) -> InputResult {
        // Global keys
        match key {
            KeyCode::Esc => return InputResult::Quit,
            KeyCode::Char('q') if self.current_step == WizardStep::Welcome => {
                return InputResult::Quit
            }
            _ => {}
        }

        // Step-specific handling
        match self.current_step {
            WizardStep::Welcome => self.handle_welcome_input(key),
            WizardStep::ApiKey => self.handle_api_key_input(key),
            WizardStep::Namespace => self.handle_namespace_input(key),
            WizardStep::FeatureTour => self.handle_tour_input(key),
            WizardStep::Complete => self.handle_complete_input(key),
        }
    }

    fn handle_welcome_input(&mut self, key: KeyCode) -> InputResult {
        match key {
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('n') => {
                self.current_step = WizardStep::ApiKey;
            }
            KeyCode::Char('s') => {
                // Skip to tour
                self.skip_api_key = true;
                self.current_step = WizardStep::FeatureTour;
            }
            _ => {}
        }
        InputResult::Continue
    }

    fn handle_api_key_input(&mut self, key: KeyCode) -> InputResult {
        match key {
            KeyCode::Enter => {
                // If using detected auth and no manual input, use detected token
                if self.use_detected_auth && self.api_key_input.is_empty() {
                    if let Some(ref auth) = self.detected_auth {
                        match secrets::set(
                            "ANTHROPIC_API_KEY",
                            &auth.access_token,
                            &SecretScope::Global,
                        ) {
                            Ok(()) => {
                                self.success_message = Some("Claude Code credential added to pool!".into());
                                self.current_step = WizardStep::Namespace;
                            }
                            Err(e) => {
                                self.error_message = Some(format!("Failed to save: {}", e));
                            }
                        }
                    }
                } else if self.api_key_input.is_empty() {
                    self.error_message = Some("API key cannot be empty. Press 's' to skip.".into());
                } else if self.api_key_input.len() < 10 {
                    self.error_message = Some("API key seems too short.".into());
                } else {
                    // Try to store the manually entered API key
                    match secrets::set(
                        "ANTHROPIC_API_KEY",
                        &self.api_key_input,
                        &SecretScope::Global,
                    ) {
                        Ok(()) => {
                            self.success_message = Some("API key saved securely!".into());
                            self.current_step = WizardStep::Namespace;
                        }
                        Err(e) => {
                            self.error_message = Some(format!("Failed to save: {}", e));
                        }
                    }
                }
            }
            KeyCode::Char('s') if self.api_key_input.is_empty() && !self.use_detected_auth => {
                self.skip_api_key = true;
                self.current_step = WizardStep::Namespace;
            }
            KeyCode::Char('d') if self.detected_auth.is_some() => {
                // Toggle between detected and manual input
                self.use_detected_auth = !self.use_detected_auth;
                if self.use_detected_auth {
                    self.api_key_input.clear();
                }
            }
            KeyCode::Char('t') => {
                self.api_key_masked = !self.api_key_masked;
            }
            KeyCode::Backspace => {
                self.api_key_input.pop();
                // If user starts typing, switch to manual mode
                if self.use_detected_auth && !self.api_key_input.is_empty() {
                    self.use_detected_auth = false;
                }
            }
            KeyCode::Char(c) if !c.is_control() => {
                // If user starts typing, switch to manual mode
                if self.use_detected_auth {
                    self.use_detected_auth = false;
                }
                self.api_key_input.push(c);
            }
            KeyCode::Left => {
                if let Some(prev) = self.current_step.prev() {
                    self.current_step = prev;
                }
            }
            _ => {}
        }
        InputResult::Continue
    }

    fn handle_namespace_input(&mut self, key: KeyCode) -> InputResult {
        match key {
            KeyCode::Enter => {
                if self.namespace_input.is_empty() {
                    self.namespace_input = "default".into();
                }
                // Validate namespace name
                if self.namespace_input.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                {
                    self.current_step = WizardStep::FeatureTour;
                } else {
                    self.error_message =
                        Some("Namespace can only contain letters, numbers, - and _".into());
                }
            }
            KeyCode::Backspace => {
                self.namespace_input.pop();
            }
            KeyCode::Char(c) if !c.is_control() => {
                if self.namespace_input.len() < 32 {
                    self.namespace_input.push(c);
                }
            }
            KeyCode::Left => {
                if let Some(prev) = self.current_step.prev() {
                    self.current_step = prev;
                }
            }
            _ => {}
        }
        InputResult::Continue
    }

    fn handle_tour_input(&mut self, key: KeyCode) -> InputResult {
        match key {
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('n') => {
                if self.tour_page < TOUR_PAGES.len() - 1 {
                    self.tour_page += 1;
                } else {
                    self.current_step = WizardStep::Complete;
                }
            }
            KeyCode::Left | KeyCode::Char('p') => {
                if self.tour_page > 0 {
                    self.tour_page -= 1;
                } else if let Some(prev) = self.current_step.prev() {
                    self.current_step = prev;
                }
            }
            KeyCode::Char('s') => {
                self.current_step = WizardStep::Complete;
            }
            _ => {}
        }
        InputResult::Continue
    }

    fn handle_complete_input(&mut self, key: KeyCode) -> InputResult {
        match key {
            KeyCode::Enter | KeyCode::Char('y') => {
                self.save_config();
                InputResult::Complete
            }
            KeyCode::Left => {
                if let Some(prev) = self.current_step.prev() {
                    self.current_step = prev;
                }
                InputResult::Continue
            }
            KeyCode::Char('n') | KeyCode::Esc => InputResult::Quit,
            _ => InputResult::Continue,
        }
    }

    /// Save configuration from wizard
    fn save_config(&self) {
        // Create config directory if needed
        if let Some(config_dir) = dirs_config_dir() {
            let _ = std::fs::create_dir_all(&config_dir);

            // Write minimal config
            let config_content = format!(
                r#"# Kage configuration
# Generated by onboarding wizard

[daemon]
# max_agents = 5

[namespaces.{}]
# Add repositories to this namespace:
# repositories = [{{ name = "my-repo", path = "/path/to/repo" }}]
"#,
                self.namespace_input
            );

            let config_path = config_dir.join("config.toml");
            let _ = std::fs::write(config_path, config_content);

            // Mark first run as complete
            let first_run_marker = config_dir.join(".initialized");
            let _ = std::fs::write(first_run_marker, "");
        }
    }

    /// Render the current step
    fn render(&self, f: &mut Frame) {
        let area = f.size();

        // Main background
        let block = Block::default().style(Style::default().bg(Color::Rgb(24, 24, 27)));
        f.render_widget(block, area);

        // Layout
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // Header
                Constraint::Min(10),    // Content
                Constraint::Length(3),  // Footer
            ])
            .split(area);

        self.render_header(f, chunks[0]);
        self.render_content(f, chunks[1]);
        self.render_footer(f, chunks[2]);
    }

    /// Render the header with progress
    fn render_header(&self, f: &mut Frame, area: Rect) {
        let title = match self.current_step {
            WizardStep::Welcome => " Welcome to Kage ",
            WizardStep::ApiKey => " API Key Setup ",
            WizardStep::Namespace => " Namespace Setup ",
            WizardStep::FeatureTour => " Feature Tour ",
            WizardStep::Complete => " Setup Complete ",
        };

        // Progress indicator
        let progress = format!(
            " Step {} of {} ",
            self.current_step.index() + 1,
            WizardStep::total()
        );

        let block = Block::default()
            .title(Span::styled(title, Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD)))
            .title_alignment(Alignment::Center)
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(theme::DIM));

        // Progress dots
        let dots: String = (0..WizardStep::total())
            .map(|i| {
                if i == self.current_step.index() {
                    "●"
                } else if i < self.current_step.index() {
                    "◉"
                } else {
                    "○"
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        let header = Paragraph::new(Line::from(vec![
            Span::styled(dots, Style::default().fg(theme::PURPLE)),
            Span::raw("  "),
            Span::styled(progress, Style::default().fg(theme::DIM)),
        ]))
        .alignment(Alignment::Center)
        .block(block);

        f.render_widget(header, area);
    }

    /// Render the main content
    fn render_content(&self, f: &mut Frame, area: Rect) {
        match self.current_step {
            WizardStep::Welcome => self.render_welcome(f, area),
            WizardStep::ApiKey => self.render_api_key(f, area),
            WizardStep::Namespace => self.render_namespace(f, area),
            WizardStep::FeatureTour => self.render_tour(f, area),
            WizardStep::Complete => self.render_complete(f, area),
        }

        // Render messages if any
        if let Some(ref msg) = self.error_message {
            self.render_message(f, area, msg, theme::ERROR);
        } else if let Some(ref msg) = self.success_message {
            self.render_message(f, area, msg, theme::SUCCESS);
        }
    }

    /// Render welcome screen
    fn render_welcome(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(12), // Mascot
                Constraint::Length(8),  // Logo
                Constraint::Min(4),     // Description
            ])
            .margin(2)
            .split(area);

        // Mascot
        let mascot_lines: Vec<Line> = MASCOT_SMALL
            .lines()
            .map(|line| {
                let spans: Vec<Span> = line
                    .chars()
                    .map(|c| match c {
                        '☽' => Span::styled(c.to_string(), Style::default().fg(theme::MOON)),
                        '✦' | '✧' | '·' => {
                            Span::styled(c.to_string(), Style::default().fg(theme::SPARKLE))
                        }
                        ')' | '(' => Span::styled(c.to_string(), Style::default().fg(theme::MOON)),
                        '█' | '▄' | '▀' | '▐' | '▌' => {
                            Span::styled(c.to_string(), Style::default().fg(theme::DARK_PURPLE))
                        }
                        '◠' => Span::styled(c.to_string(), Style::default().fg(theme::LIGHT_PURPLE)),
                        _ => Span::raw(c.to_string()),
                    })
                    .collect();
                Line::from(spans)
            })
            .collect();

        let mascot = Paragraph::new(mascot_lines).alignment(Alignment::Center);
        f.render_widget(mascot, chunks[0]);

        // Logo
        let logo = r#"
██╗  ██╗ █████╗  ██████╗ ███████╗
██║ ██╔╝██╔══██╗██╔════╝ ██╔════╝
█████╔╝ ███████║██║  ███╗█████╗
██╔═██╗ ██╔══██║██║   ██║██╔══╝
██║  ██╗██║  ██║╚██████╔╝███████╗
╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝ ╚══════╝"#;

        let logo_widget = Paragraph::new(logo)
            .style(Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD))
            .alignment(Alignment::Center);
        f.render_widget(logo_widget, chunks[1]);

        // Description
        let desc = vec![
            Line::from(""),
            Line::from(Span::styled(
                "Shadow agents for autonomous code work",
                Style::default().fg(theme::LIGHT_PURPLE),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Let Claude Code agents work while you're away.",
                Style::default().fg(theme::TEXT),
            )),
            Line::from(Span::styled(
                "They'll iterate, learn, and share knowledge across your projects.",
                Style::default().fg(theme::DIM),
            )),
        ];

        let description = Paragraph::new(desc).alignment(Alignment::Center);
        f.render_widget(description, chunks[2]);
    }

    /// Render API key setup screen
    fn render_api_key(&self, f: &mut Frame, area: Rect) {
        // Different layout if we detected Claude Code auth
        if let Some(ref auth) = self.detected_auth {
            self.render_api_key_with_detected(f, area, auth);
        } else {
            self.render_api_key_manual(f, area);
        }
    }

    /// Render API key screen when Claude Code auth is detected
    fn render_api_key_with_detected(&self, f: &mut Frame, area: Rect, auth: &ClaudeCodeAuth) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),  // Title
                Constraint::Length(8),  // Detected info
                Constraint::Length(5),  // Input/Confirm
                Constraint::Min(4),     // Info
            ])
            .margin(2)
            .split(area);

        // Title - show success that we found credentials
        let title = Paragraph::new(vec![
            Line::from(Span::styled(
                "✓ Claude Code Detected",
                Style::default().fg(theme::SUCCESS).add_modifier(Modifier::BOLD),
            )),
        ])
        .alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Show detected account info
        let account_name = auth.display_name.as_deref().unwrap_or("Unknown");
        let account_email = auth.email.as_deref().unwrap_or("unknown@example.com");
        let masked_token = secrets::mask_token(&auth.access_token);

        let detected_info = vec![
            Line::from(Span::styled(
                "Found existing Claude Code authentication:",
                Style::default().fg(theme::TEXT),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Account:  ", Style::default().fg(theme::DIM)),
                Span::styled(account_name, Style::default().fg(theme::SPARKLE)),
                Span::styled(format!(" ({})", account_email), Style::default().fg(theme::DIM)),
            ]),
            Line::from(vec![
                Span::styled("  Token:    ", Style::default().fg(theme::DIM)),
                Span::styled(masked_token, Style::default().fg(theme::TEXT)),
            ]),
        ];
        let detected_widget = Paragraph::new(detected_info).alignment(Alignment::Center);
        f.render_widget(detected_widget, chunks[1]);

        // Show either confirmation or manual input
        let input_area = centered_rect(60, 3, chunks[2]);

        if self.use_detected_auth {
            // Show confirmation box
            let confirm_block = Block::default()
                .title(Span::styled(" ✓ Use this credential ", Style::default().fg(theme::SUCCESS)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::SUCCESS));

            let confirm = Paragraph::new(Span::styled(
                "Press Enter to add to subscription pool",
                Style::default().fg(theme::TEXT),
            ))
            .alignment(Alignment::Center)
            .block(confirm_block);

            f.render_widget(confirm, input_area);
        } else {
            // Show manual input
            let display_value = if self.api_key_masked && !self.api_key_input.is_empty() {
                "●".repeat(self.api_key_input.len())
            } else {
                self.api_key_input.clone()
            };

            let input_block = Block::default()
                .title(Span::styled(" Different API Key ", Style::default().fg(theme::PURPLE)))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::PURPLE));

            let input = Paragraph::new(if display_value.is_empty() {
                Span::styled("sk-ant-...", Style::default().fg(theme::DIM))
            } else {
                Span::styled(display_value, Style::default().fg(theme::TEXT))
            })
            .block(input_block);

            f.render_widget(input, input_area);
        }

        // Info text with toggle option
        let info = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("[d]", Style::default().fg(theme::PURPLE)),
                Span::styled(
                    if self.use_detected_auth { " Enter different key  " } else { " Use detected key  " },
                    Style::default().fg(theme::DIM),
                ),
                Span::styled("[t]", Style::default().fg(theme::PURPLE)),
                Span::styled(" Toggle visibility", Style::default().fg(theme::DIM)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Your credential will be added to the subscription pool.",
                Style::default().fg(theme::SPARKLE),
            )),
        ];
        let info_widget = Paragraph::new(info).alignment(Alignment::Center);
        f.render_widget(info_widget, chunks[3]);
    }

    /// Render API key screen for manual entry (no detected auth)
    fn render_api_key_manual(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),  // Title
                Constraint::Length(6),  // Description
                Constraint::Length(5),  // Input
                Constraint::Min(4),     // Info
            ])
            .margin(2)
            .split(area);

        // Title
        let title = Paragraph::new(vec![
            Line::from(Span::styled(
                "  Anthropic API Key",
                Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD),
            )),
        ])
        .alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Description
        let desc = vec![
            Line::from(Span::styled(
                "Kage needs your Anthropic API key to run Claude Code agents.",
                Style::default().fg(theme::TEXT),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Your key is stored securely in your OS keychain,",
                Style::default().fg(theme::DIM),
            )),
            Line::from(Span::styled(
                "never on disk or in config files.",
                Style::default().fg(theme::DIM),
            )),
        ];
        let description = Paragraph::new(desc).alignment(Alignment::Center);
        f.render_widget(description, chunks[1]);

        // Input field
        let input_area = centered_rect(60, 3, chunks[2]);
        let display_value = if self.api_key_masked && !self.api_key_input.is_empty() {
            "●".repeat(self.api_key_input.len())
        } else {
            self.api_key_input.clone()
        };

        let input_block = Block::default()
            .title(Span::styled(" API Key ", Style::default().fg(theme::PURPLE)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::PURPLE));

        let input = Paragraph::new(if display_value.is_empty() {
            Span::styled("sk-ant-...", Style::default().fg(theme::DIM))
        } else {
            Span::styled(display_value, Style::default().fg(theme::TEXT))
        })
        .block(input_block);

        f.render_widget(input, input_area);

        // Info text
        let info = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("[t]", Style::default().fg(theme::PURPLE)),
                Span::styled(" Toggle visibility  ", Style::default().fg(theme::DIM)),
                Span::styled("[s]", Style::default().fg(theme::PURPLE)),
                Span::styled(" Skip for now", Style::default().fg(theme::DIM)),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                "Get your API key at: https://console.anthropic.com/",
                Style::default().fg(theme::SPARKLE),
            )),
        ];
        let info_widget = Paragraph::new(info).alignment(Alignment::Center);
        f.render_widget(info_widget, chunks[3]);
    }

    /// Render namespace setup screen
    fn render_namespace(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),  // Title
                Constraint::Length(6),  // Description
                Constraint::Length(5),  // Input
                Constraint::Min(4),     // Examples
            ])
            .margin(2)
            .split(area);

        // Title
        let title = Paragraph::new(vec![Line::from(Span::styled(
            "  Create Your First Namespace",
            Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD),
        ))])
        .alignment(Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Description
        let desc = vec![
            Line::from(Span::styled(
                "Namespaces help organize your repositories into groups.",
                Style::default().fg(theme::TEXT),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Agents within a namespace share context and coordinate work.",
                Style::default().fg(theme::DIM),
            )),
        ];
        let description = Paragraph::new(desc).alignment(Alignment::Center);
        f.render_widget(description, chunks[1]);

        // Input field
        let input_area = centered_rect(40, 3, chunks[2]);
        let input_block = Block::default()
            .title(Span::styled(" Namespace Name ", Style::default().fg(theme::PURPLE)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::PURPLE));

        let input = Paragraph::new(if self.namespace_input.is_empty() {
            Span::styled("default", Style::default().fg(theme::DIM))
        } else {
            Span::styled(&self.namespace_input, Style::default().fg(theme::TEXT))
        })
        .block(input_block);

        f.render_widget(input, input_area);

        // Examples
        let examples = vec![
            Line::from(""),
            Line::from(Span::styled("Example namespaces:", Style::default().fg(theme::DIM))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  backend", Style::default().fg(theme::SPARKLE)),
                Span::styled("  - API services, databases", Style::default().fg(theme::DIM)),
            ]),
            Line::from(vec![
                Span::styled("  frontend", Style::default().fg(theme::SPARKLE)),
                Span::styled(" - Web apps, mobile apps", Style::default().fg(theme::DIM)),
            ]),
            Line::from(vec![
                Span::styled("  infra", Style::default().fg(theme::SPARKLE)),
                Span::styled("    - Terraform, Kubernetes", Style::default().fg(theme::DIM)),
            ]),
        ];
        let examples_widget = Paragraph::new(examples).alignment(Alignment::Center);
        f.render_widget(examples_widget, chunks[3]);
    }

    /// Render feature tour
    fn render_tour(&self, f: &mut Frame, area: Rect) {
        let page = &TOUR_PAGES[self.tour_page];

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),  // Title with icon
                Constraint::Length(4),  // Description
                Constraint::Min(8),     // Details
                Constraint::Length(3),  // Page indicator
            ])
            .margin(2)
            .split(area);

        // Title with icon
        let title = vec![
            Line::from(""),
            Line::from(Span::styled(
                page.icon,
                Style::default().fg(theme::PURPLE),
            )),
            Line::from(Span::styled(
                page.title,
                Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD),
            )),
        ];
        let title_widget = Paragraph::new(title).alignment(Alignment::Center);
        f.render_widget(title_widget, chunks[0]);

        // Description
        let desc = vec![
            Line::from(""),
            Line::from(Span::styled(
                page.description,
                Style::default().fg(theme::TEXT),
            )),
        ];
        let desc_widget = Paragraph::new(desc).alignment(Alignment::Center);
        f.render_widget(desc_widget, chunks[1]);

        // Details
        let details_area = centered_rect(70, chunks[2].height, chunks[2]);
        let mut detail_lines = vec![Line::from("")];
        for detail in page.details {
            detail_lines.push(Line::from(vec![
                Span::styled("  ◆ ", Style::default().fg(theme::PURPLE)),
                Span::styled(*detail, Style::default().fg(theme::DIM)),
            ]));
        }

        let details_widget = Paragraph::new(detail_lines).alignment(Alignment::Left);
        f.render_widget(details_widget, details_area);

        // Page indicator
        let indicator: String = (0..TOUR_PAGES.len())
            .map(|i| if i == self.tour_page { "●" } else { "○" })
            .collect::<Vec<_>>()
            .join(" ");

        let indicator_line = vec![
            Line::from(""),
            Line::from(Span::styled(indicator, Style::default().fg(theme::PURPLE))),
        ];
        let indicator_widget = Paragraph::new(indicator_line).alignment(Alignment::Center);
        f.render_widget(indicator_widget, chunks[3]);
    }

    /// Render completion screen
    fn render_complete(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(6),  // Icon
                Constraint::Length(4),  // Title
                Constraint::Min(10),    // Summary
            ])
            .margin(2)
            .split(area);

        // Success icon
        let icon = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  ✓  ",
                Style::default().fg(theme::SUCCESS).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        let icon_widget = Paragraph::new(icon).alignment(Alignment::Center);
        f.render_widget(icon_widget, chunks[0]);

        // Title
        let title = vec![
            Line::from(Span::styled(
                "You're all set!",
                Style::default().fg(theme::PURPLE).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        let title_widget = Paragraph::new(title).alignment(Alignment::Center);
        f.render_widget(title_widget, chunks[1]);

        // Summary - show source of API key
        let api_status = if self.skip_api_key {
            Span::styled("Skipped (configure later with 'kage secret set')", Style::default().fg(theme::WARNING))
        } else if self.use_detected_auth && self.detected_auth.is_some() {
            let email = self.detected_auth.as_ref()
                .and_then(|a| a.email.as_deref())
                .unwrap_or("detected");
            Span::styled(format!("Claude Code ({}) → pool", email), Style::default().fg(theme::SUCCESS))
        } else {
            Span::styled("Added to subscription pool", Style::default().fg(theme::SUCCESS))
        };

        let summary = vec![
            Line::from(Span::styled("Configuration Summary:", Style::default().fg(theme::TEXT))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  API Key:    ", Style::default().fg(theme::DIM)),
                api_status,
            ]),
            Line::from(vec![
                Span::styled("  Namespace:  ", Style::default().fg(theme::DIM)),
                Span::styled(&self.namespace_input, Style::default().fg(theme::SPARKLE)),
            ]),
            Line::from(""),
            Line::from(""),
            Line::from(Span::styled("Quick Start:", Style::default().fg(theme::TEXT))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  kage daemon start", Style::default().fg(theme::SPARKLE)),
                Span::styled("        # Start the daemon", Style::default().fg(theme::DIM)),
            ]),
            Line::from(vec![
                Span::styled("  kage agent spawn", Style::default().fg(theme::SPARKLE)),
                Span::styled("         # Spawn an agent", Style::default().fg(theme::DIM)),
            ]),
            Line::from(vec![
                Span::styled("  kage task add \"Fix bugs\"", Style::default().fg(theme::SPARKLE)),
                Span::styled("  # Add a task", Style::default().fg(theme::DIM)),
            ]),
        ];

        let summary_area = centered_rect(70, chunks[2].height, chunks[2]);
        let summary_widget = Paragraph::new(summary).alignment(Alignment::Left);
        f.render_widget(summary_widget, summary_area);
    }

    /// Render footer with navigation hints
    fn render_footer(&self, f: &mut Frame, area: Rect) {
        let hints = match self.current_step {
            WizardStep::Welcome => vec![
                ("[Enter]", "Continue"),
                ("[s]", "Skip setup"),
                ("[Esc]", "Quit"),
            ],
            WizardStep::ApiKey => {
                if self.detected_auth.is_some() {
                    if self.use_detected_auth {
                        vec![
                            ("[Enter]", "Add to pool"),
                            ("[d]", "Different key"),
                            ("[←]", "Back"),
                        ]
                    } else {
                        vec![
                            ("[Enter]", "Save & Continue"),
                            ("[d]", "Use detected"),
                            ("[←]", "Back"),
                        ]
                    }
                } else {
                    vec![
                        ("[Enter]", "Save & Continue"),
                        ("[←]", "Back"),
                        ("[s]", "Skip"),
                    ]
                }
            }
            WizardStep::Namespace => vec![
                ("[Enter]", "Continue"),
                ("[←]", "Back"),
            ],
            WizardStep::FeatureTour => vec![
                ("[→/Enter]", "Next"),
                ("[←]", "Previous"),
                ("[s]", "Skip tour"),
            ],
            WizardStep::Complete => vec![
                ("[Enter/y]", "Finish"),
                ("[←]", "Back"),
                ("[n/Esc]", "Cancel"),
            ],
        };

        let hint_spans: Vec<Span> = hints
            .iter()
            .flat_map(|(key, desc)| {
                vec![
                    Span::styled(*key, Style::default().fg(theme::PURPLE)),
                    Span::styled(format!(" {}  ", desc), Style::default().fg(theme::DIM)),
                ]
            })
            .collect();

        let footer = Paragraph::new(Line::from(hint_spans))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(theme::DIM)),
            );

        f.render_widget(footer, area);
    }

    /// Render a message popup
    fn render_message(&self, f: &mut Frame, area: Rect, message: &str, color: Color) {
        let popup_area = centered_rect(60, 3, area);

        let popup = Paragraph::new(Span::styled(message, Style::default().fg(color)))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(color))
                    .style(Style::default().bg(Color::Rgb(39, 39, 42))),
            );

        f.render_widget(Clear, popup_area);
        f.render_widget(popup, popup_area);
    }
}

/// Input handling result
enum InputResult {
    Continue,
    Quit,
    Complete,
}

/// Get centered rect
fn centered_rect(percent_x: u16, height: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((r.height.saturating_sub(height)) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
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

/// Get config directory
fn dirs_config_dir() -> Option<std::path::PathBuf> {
    directories::ProjectDirs::from("io", "raskell", "kage").map(|d| d.config_dir().to_path_buf())
}

/// Run the onboarding wizard (public entry point)
pub async fn run() -> Result<bool> {
    let mut wizard = OnboardingWizard::new();
    // Run synchronously since TUI is blocking
    tokio::task::spawn_blocking(move || wizard.run()).await?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wizard_step_navigation() {
        assert_eq!(WizardStep::Welcome.next(), Some(WizardStep::ApiKey));
        assert_eq!(WizardStep::ApiKey.prev(), Some(WizardStep::Welcome));
        assert_eq!(WizardStep::Complete.next(), None);
        assert_eq!(WizardStep::Welcome.prev(), None);
    }

    #[test]
    fn test_wizard_default() {
        let wizard = OnboardingWizard::new();
        assert_eq!(wizard.current_step, WizardStep::Welcome);
        assert!(wizard.api_key_input.is_empty());
        assert_eq!(wizard.namespace_input, "default");
    }
}
