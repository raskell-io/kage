//! Themeable TUI styling system
//!
//! Supports popular color schemes like Catppuccin, Dracula, and custom themes.

use ratatui::style::Color;

/// Theme configuration
#[derive(Debug, Clone)]
pub struct Theme {
    /// Theme name
    pub name: String,
    /// Color palette
    pub colors: ColorPalette,
    /// Status symbols
    pub symbols: StatusSymbols,
}

/// Color palette for the TUI
#[derive(Debug, Clone)]
pub struct ColorPalette {
    // Base colors
    pub bg: Color,
    pub bg_surface: Color,
    pub bg_highlight: Color,
    pub border: Color,
    pub border_focused: Color,

    // Text colors
    pub text: Color,
    pub text_dim: Color,
    pub text_muted: Color,

    // Accent colors
    pub accent: Color,
    pub accent_dim: Color,
    pub accent_bright: Color,

    // Status colors
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub info: Color,

    // Agent status colors
    pub status_working: Color,
    pub status_idle: Color,
    pub status_waiting: Color,
    pub status_paused: Color,
    pub status_error: Color,
}

/// Status symbols/icons for the TUI
#[derive(Debug, Clone)]
pub struct StatusSymbols {
    // Agent status
    pub working: &'static str,
    pub idle: &'static str,
    pub waiting: &'static str,
    pub paused: &'static str,
    pub error: &'static str,

    // UI elements
    pub collapsed: &'static str,
    pub expanded: &'static str,
    pub selected: &'static str,
    pub bullet: &'static str,
    pub arrow_right: &'static str,
    pub arrow_left: &'static str,
    pub check: &'static str,
    pub cross: &'static str,
    pub spinner: [&'static str; 4],

    // Panel icons
    pub agents: &'static str,
    pub stream: &'static str,
    pub tasks: &'static str,
    pub logs: &'static str,
}

impl Default for StatusSymbols {
    fn default() -> Self {
        Self {
            // Agent status (using Nerd Font / Unicode)
            working: "●",   // Solid circle - actively working
            idle: "○",      // Empty circle - idle/completed
            waiting: "◉",   // Circle with dot - waiting for input
            paused: "◫",    // Pause symbol
            error: "✖",     // Error X

            // UI elements
            collapsed: "▸",
            expanded: "▾",
            selected: "▶",
            bullet: "•",
            arrow_right: "→",
            arrow_left: "←",
            check: "✓",
            cross: "✗",
            spinner: ["⠋", "⠙", "⠹", "⠸"],

            // Panel icons
            agents: "󰚄",    // Robot icon (Nerd Font)
            stream: "󰞷",    // Terminal icon
            tasks: "󰄬",     // Checklist icon
            logs: "󰍡",      // Log icon
        }
    }
}

impl Theme {
    /// Create terminal-native theme (uses terminal's own colors)
    pub fn terminal() -> Self {
        Self {
            name: "Terminal".to_string(),
            colors: ColorPalette {
                // Use terminal defaults
                bg: Color::Reset,
                bg_surface: Color::Reset,
                bg_highlight: Color::DarkGray,
                border: Color::DarkGray,
                border_focused: Color::White,

                // Text uses terminal colors
                text: Color::Reset,
                text_dim: Color::Gray,
                text_muted: Color::DarkGray,

                // Accent - use magenta/purple from terminal palette
                accent: Color::Magenta,
                accent_dim: Color::Magenta,
                accent_bright: Color::LightMagenta,

                // Status - standard ANSI colors
                success: Color::Green,
                warning: Color::Yellow,
                error: Color::Red,
                info: Color::Blue,

                // Agent status
                status_working: Color::Green,
                status_idle: Color::DarkGray,
                status_waiting: Color::Yellow,
                status_paused: Color::Blue,
                status_error: Color::Red,
            },
            symbols: StatusSymbols::default(),
        }
    }

    /// Create Catppuccin Mocha theme
    pub fn catppuccin_mocha() -> Self {
        Self {
            name: "Catppuccin Mocha".to_string(),
            colors: ColorPalette {
                // Base colors (Mocha)
                bg: Color::Rgb(30, 30, 46),           // Base
                bg_surface: Color::Rgb(36, 39, 58),   // Surface0
                bg_highlight: Color::Rgb(49, 50, 68), // Surface1
                border: Color::Rgb(69, 71, 90),       // Surface2
                border_focused: Color::Rgb(137, 180, 250), // Blue

                // Text
                text: Color::Rgb(205, 214, 244),      // Text
                text_dim: Color::Rgb(166, 173, 200),  // Subtext1
                text_muted: Color::Rgb(147, 153, 178), // Subtext0

                // Accent (Mauve/Purple)
                accent: Color::Rgb(203, 166, 247),    // Mauve
                accent_dim: Color::Rgb(180, 142, 223),
                accent_bright: Color::Rgb(245, 194, 231), // Pink

                // Status
                success: Color::Rgb(166, 227, 161),   // Green
                warning: Color::Rgb(249, 226, 175),   // Yellow
                error: Color::Rgb(243, 139, 168),     // Red
                info: Color::Rgb(137, 180, 250),      // Blue

                // Agent status
                status_working: Color::Rgb(166, 227, 161),  // Green
                status_idle: Color::Rgb(147, 153, 178),     // Subtext0
                status_waiting: Color::Rgb(249, 226, 175),  // Yellow
                status_paused: Color::Rgb(137, 180, 250),   // Blue
                status_error: Color::Rgb(243, 139, 168),    // Red
            },
            symbols: StatusSymbols::default(),
        }
    }

    /// Create Catppuccin Latte theme (light)
    pub fn catppuccin_latte() -> Self {
        Self {
            name: "Catppuccin Latte".to_string(),
            colors: ColorPalette {
                bg: Color::Rgb(239, 241, 245),
                bg_surface: Color::Rgb(230, 233, 239),
                bg_highlight: Color::Rgb(220, 224, 232),
                border: Color::Rgb(188, 192, 204),
                border_focused: Color::Rgb(30, 102, 245),

                text: Color::Rgb(76, 79, 105),
                text_dim: Color::Rgb(92, 95, 119),
                text_muted: Color::Rgb(108, 111, 133),

                accent: Color::Rgb(136, 57, 239),
                accent_dim: Color::Rgb(114, 47, 200),
                accent_bright: Color::Rgb(234, 118, 203),

                success: Color::Rgb(64, 160, 43),
                warning: Color::Rgb(223, 142, 29),
                error: Color::Rgb(210, 15, 57),
                info: Color::Rgb(30, 102, 245),

                status_working: Color::Rgb(64, 160, 43),
                status_idle: Color::Rgb(108, 111, 133),
                status_waiting: Color::Rgb(223, 142, 29),
                status_paused: Color::Rgb(30, 102, 245),
                status_error: Color::Rgb(210, 15, 57),
            },
            symbols: StatusSymbols::default(),
        }
    }

    /// Create Dracula theme
    pub fn dracula() -> Self {
        Self {
            name: "Dracula".to_string(),
            colors: ColorPalette {
                bg: Color::Rgb(40, 42, 54),
                bg_surface: Color::Rgb(68, 71, 90),
                bg_highlight: Color::Rgb(98, 114, 164),
                border: Color::Rgb(68, 71, 90),
                border_focused: Color::Rgb(189, 147, 249),

                text: Color::Rgb(248, 248, 242),
                text_dim: Color::Rgb(189, 147, 249),
                text_muted: Color::Rgb(98, 114, 164),

                accent: Color::Rgb(189, 147, 249),  // Purple
                accent_dim: Color::Rgb(139, 97, 199),
                accent_bright: Color::Rgb(255, 121, 198), // Pink

                success: Color::Rgb(80, 250, 123),
                warning: Color::Rgb(241, 250, 140),
                error: Color::Rgb(255, 85, 85),
                info: Color::Rgb(139, 233, 253),

                status_working: Color::Rgb(80, 250, 123),
                status_idle: Color::Rgb(98, 114, 164),
                status_waiting: Color::Rgb(241, 250, 140),
                status_paused: Color::Rgb(139, 233, 253),
                status_error: Color::Rgb(255, 85, 85),
            },
            symbols: StatusSymbols::default(),
        }
    }

    /// Create Tokyo Night theme
    pub fn tokyo_night() -> Self {
        Self {
            name: "Tokyo Night".to_string(),
            colors: ColorPalette {
                bg: Color::Rgb(26, 27, 38),
                bg_surface: Color::Rgb(36, 40, 59),
                bg_highlight: Color::Rgb(41, 46, 66),
                border: Color::Rgb(59, 66, 97),
                border_focused: Color::Rgb(122, 162, 247),

                text: Color::Rgb(192, 202, 245),
                text_dim: Color::Rgb(169, 177, 214),
                text_muted: Color::Rgb(86, 95, 137),

                accent: Color::Rgb(187, 154, 247),  // Purple
                accent_dim: Color::Rgb(157, 124, 217),
                accent_bright: Color::Rgb(255, 117, 127), // Pink

                success: Color::Rgb(158, 206, 106),
                warning: Color::Rgb(224, 175, 104),
                error: Color::Rgb(247, 118, 142),
                info: Color::Rgb(122, 162, 247),

                status_working: Color::Rgb(158, 206, 106),
                status_idle: Color::Rgb(86, 95, 137),
                status_waiting: Color::Rgb(224, 175, 104),
                status_paused: Color::Rgb(122, 162, 247),
                status_error: Color::Rgb(247, 118, 142),
            },
            symbols: StatusSymbols::default(),
        }
    }

    /// Create Kage default theme (purple-focused)
    pub fn kage() -> Self {
        Self {
            name: "Kage".to_string(),
            colors: ColorPalette {
                bg: Color::Rgb(24, 24, 27),
                bg_surface: Color::Rgb(32, 32, 36),
                bg_highlight: Color::Rgb(39, 39, 42),
                border: Color::Rgb(63, 63, 70),
                border_focused: Color::Rgb(167, 139, 250),

                text: Color::Rgb(244, 244, 245),
                text_dim: Color::Rgb(161, 161, 170),
                text_muted: Color::Rgb(113, 113, 122),

                accent: Color::Rgb(167, 139, 250),  // Purple
                accent_dim: Color::Rgb(139, 92, 246),
                accent_bright: Color::Rgb(196, 181, 253),

                success: Color::Rgb(134, 239, 172),
                warning: Color::Rgb(253, 224, 71),
                error: Color::Rgb(252, 165, 165),
                info: Color::Rgb(147, 197, 253),

                status_working: Color::Rgb(134, 239, 172),
                status_idle: Color::Rgb(113, 113, 122),
                status_waiting: Color::Rgb(253, 224, 71),
                status_paused: Color::Rgb(147, 197, 253),
                status_error: Color::Rgb(252, 165, 165),
            },
            symbols: StatusSymbols::default(),
        }
    }

    /// Get theme by name
    pub fn by_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "terminal" | "native" | "default" => Self::terminal(),
            "catppuccin" | "catppuccin-mocha" | "mocha" => Self::catppuccin_mocha(),
            "catppuccin-latte" | "latte" => Self::catppuccin_latte(),
            "dracula" => Self::dracula(),
            "tokyo-night" | "tokyonight" => Self::tokyo_night(),
            "kage" => Self::kage(),
            _ => Self::terminal(),
        }
    }

    /// List available themes
    pub fn available() -> Vec<&'static str> {
        vec![
            "terminal",
            "kage",
            "catppuccin-mocha",
            "catppuccin-latte",
            "dracula",
            "tokyo-night",
        ]
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::terminal()
    }
}

/// Simple ASCII-only symbols for terminals without Unicode support
pub fn ascii_symbols() -> StatusSymbols {
    StatusSymbols {
        working: "*",
        idle: "o",
        waiting: "@",
        paused: "=",
        error: "x",

        collapsed: ">",
        expanded: "v",
        selected: ">",
        bullet: "-",
        arrow_right: "->",
        arrow_left: "<-",
        check: "+",
        cross: "x",
        spinner: ["-", "\\", "|", "/"],

        agents: "[A]",
        stream: "[S]",
        tasks: "[T]",
        logs: "[L]",
    }
}
