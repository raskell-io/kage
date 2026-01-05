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
    /// Colors from: https://github.com/catppuccin/catppuccin
    pub fn catppuccin_mocha() -> Self {
        // Official Catppuccin Mocha palette
        // Base layers
        const CRUST: Color = Color::Rgb(17, 17, 27);     // #11111b
        const MANTLE: Color = Color::Rgb(24, 24, 37);    // #181825
        const BASE: Color = Color::Rgb(30, 30, 46);      // #1e1e2e
        const SURFACE0: Color = Color::Rgb(49, 50, 68);  // #313244
        const SURFACE1: Color = Color::Rgb(69, 71, 90);  // #45475a
        const SURFACE2: Color = Color::Rgb(88, 91, 112); // #585b70
        const OVERLAY0: Color = Color::Rgb(108, 112, 134); // #6c7086
        const OVERLAY1: Color = Color::Rgb(127, 132, 156); // #7f849c
        const OVERLAY2: Color = Color::Rgb(147, 153, 178); // #9399b2
        // Text
        const SUBTEXT0: Color = Color::Rgb(166, 173, 200); // #a6adc8
        const SUBTEXT1: Color = Color::Rgb(186, 194, 222); // #bac2de
        const TEXT: Color = Color::Rgb(205, 214, 244);     // #cdd6f4
        // Accent colors
        const LAVENDER: Color = Color::Rgb(180, 190, 254); // #b4befe
        const BLUE: Color = Color::Rgb(137, 180, 250);     // #89b4fa
        const SAPPHIRE: Color = Color::Rgb(116, 199, 236); // #74c7ec
        const SKY: Color = Color::Rgb(137, 220, 235);      // #89dceb
        const TEAL: Color = Color::Rgb(148, 226, 213);     // #94e2d5
        const GREEN: Color = Color::Rgb(166, 227, 161);    // #a6e3a1
        const YELLOW: Color = Color::Rgb(249, 226, 175);   // #f9e2af
        const PEACH: Color = Color::Rgb(250, 179, 135);    // #fab387
        const MAROON: Color = Color::Rgb(235, 160, 172);   // #eba0ac
        const RED: Color = Color::Rgb(243, 139, 168);      // #f38ba8
        const MAUVE: Color = Color::Rgb(203, 166, 247);    // #cba6f7
        const PINK: Color = Color::Rgb(245, 194, 231);     // #f5c2e7
        const FLAMINGO: Color = Color::Rgb(242, 205, 205); // #f2cdcd
        const ROSEWATER: Color = Color::Rgb(245, 224, 220); // #f5e0dc

        Self {
            name: "Catppuccin Mocha".to_string(),
            colors: ColorPalette {
                // Base colors
                bg: BASE,
                bg_surface: MANTLE,
                bg_highlight: SURFACE0,
                border: SURFACE1,
                border_focused: LAVENDER,

                // Text
                text: TEXT,
                text_dim: SUBTEXT1,
                text_muted: OVERLAY1,

                // Accent (Mauve/Purple)
                accent: MAUVE,
                accent_dim: LAVENDER,
                accent_bright: PINK,

                // Status
                success: GREEN,
                warning: YELLOW,
                error: RED,
                info: BLUE,

                // Agent status
                status_working: GREEN,
                status_idle: OVERLAY1,
                status_waiting: PEACH,
                status_paused: BLUE,
                status_error: RED,
            },
            symbols: StatusSymbols::default(),
        }
    }

    /// Create Catppuccin Latte theme (light)
    /// Colors from: https://github.com/catppuccin/catppuccin
    pub fn catppuccin_latte() -> Self {
        // Official Catppuccin Latte palette
        const CRUST: Color = Color::Rgb(220, 224, 232);    // #dce0e8
        const MANTLE: Color = Color::Rgb(230, 233, 239);   // #e6e9ef
        const BASE: Color = Color::Rgb(239, 241, 245);     // #eff1f5
        const SURFACE0: Color = Color::Rgb(204, 208, 218); // #ccd0da
        const SURFACE1: Color = Color::Rgb(188, 192, 204); // #bcc0cc
        const SURFACE2: Color = Color::Rgb(172, 176, 190); // #acb0be
        const OVERLAY0: Color = Color::Rgb(156, 160, 176); // #9ca0b0
        const OVERLAY1: Color = Color::Rgb(140, 143, 161); // #8c8fa1
        const OVERLAY2: Color = Color::Rgb(124, 127, 147); // #7c7f93
        const SUBTEXT0: Color = Color::Rgb(108, 111, 133); // #6c6f85
        const SUBTEXT1: Color = Color::Rgb(92, 95, 119);   // #5c5f77
        const TEXT: Color = Color::Rgb(76, 79, 105);       // #4c4f69
        // Accent colors
        const LAVENDER: Color = Color::Rgb(114, 135, 253); // #7287fd
        const BLUE: Color = Color::Rgb(30, 102, 245);      // #1e66f5
        const SAPPHIRE: Color = Color::Rgb(32, 159, 181);  // #209fb5
        const SKY: Color = Color::Rgb(4, 165, 229);        // #04a5e5
        const TEAL: Color = Color::Rgb(23, 146, 153);      // #179299
        const GREEN: Color = Color::Rgb(64, 160, 43);      // #40a02b
        const YELLOW: Color = Color::Rgb(223, 142, 29);    // #df8e1d
        const PEACH: Color = Color::Rgb(254, 100, 11);     // #fe640b
        const MAROON: Color = Color::Rgb(230, 69, 83);     // #e64553
        const RED: Color = Color::Rgb(210, 15, 57);        // #d20f39
        const MAUVE: Color = Color::Rgb(136, 57, 239);     // #8839ef
        const PINK: Color = Color::Rgb(234, 118, 203);     // #ea76cb
        const FLAMINGO: Color = Color::Rgb(221, 120, 120); // #dd7878
        const ROSEWATER: Color = Color::Rgb(220, 138, 120); // #dc8a78

        Self {
            name: "Catppuccin Latte".to_string(),
            colors: ColorPalette {
                bg: BASE,
                bg_surface: MANTLE,
                bg_highlight: SURFACE0,
                border: SURFACE1,
                border_focused: LAVENDER,

                text: TEXT,
                text_dim: SUBTEXT1,
                text_muted: OVERLAY1,

                accent: MAUVE,
                accent_dim: LAVENDER,
                accent_bright: PINK,

                success: GREEN,
                warning: YELLOW,
                error: RED,
                info: BLUE,

                status_working: GREEN,
                status_idle: OVERLAY1,
                status_waiting: PEACH,
                status_paused: BLUE,
                status_error: RED,
            },
            symbols: StatusSymbols::default(),
        }
    }

    /// Create Dracula theme
    /// Colors from: https://draculatheme.com/contribute
    pub fn dracula() -> Self {
        // Official Dracula palette
        const BACKGROUND: Color = Color::Rgb(40, 42, 54);    // #282a36
        const CURRENT_LINE: Color = Color::Rgb(68, 71, 90);  // #44475a
        const FOREGROUND: Color = Color::Rgb(248, 248, 242); // #f8f8f2
        const COMMENT: Color = Color::Rgb(98, 114, 164);     // #6272a4
        const CYAN: Color = Color::Rgb(139, 233, 253);       // #8be9fd
        const GREEN: Color = Color::Rgb(80, 250, 123);       // #50fa7b
        const ORANGE: Color = Color::Rgb(255, 184, 108);     // #ffb86c
        const PINK: Color = Color::Rgb(255, 121, 198);       // #ff79c6
        const PURPLE: Color = Color::Rgb(189, 147, 249);     // #bd93f9
        const RED: Color = Color::Rgb(255, 85, 85);          // #ff5555
        const YELLOW: Color = Color::Rgb(241, 250, 140);     // #f1fa8c

        Self {
            name: "Dracula".to_string(),
            colors: ColorPalette {
                bg: BACKGROUND,
                bg_surface: CURRENT_LINE,
                bg_highlight: Color::Rgb(55, 58, 72), // Slightly lighter than bg
                border: COMMENT,
                border_focused: PURPLE,

                text: FOREGROUND,
                text_dim: Color::Rgb(200, 200, 194), // Slightly dimmer foreground
                text_muted: COMMENT,

                accent: PURPLE,
                accent_dim: Color::Rgb(159, 117, 219), // Dimmer purple
                accent_bright: PINK,

                success: GREEN,
                warning: ORANGE,
                error: RED,
                info: CYAN,

                status_working: GREEN,
                status_idle: COMMENT,
                status_waiting: ORANGE,
                status_paused: CYAN,
                status_error: RED,
            },
            symbols: StatusSymbols::default(),
        }
    }

    /// Create Tokyo Night theme
    /// Colors from: https://github.com/folke/tokyonight.nvim
    pub fn tokyo_night() -> Self {
        // Official Tokyo Night (Night variant) palette
        const BG: Color = Color::Rgb(26, 27, 38);            // #1a1b26
        const BG_DARK: Color = Color::Rgb(22, 22, 30);       // #16161e
        const BG_HIGHLIGHT: Color = Color::Rgb(41, 46, 66);  // #292e42
        const TERMINAL_BLACK: Color = Color::Rgb(65, 72, 104); // #414868
        const FG: Color = Color::Rgb(192, 202, 245);         // #c0caf5
        const FG_DARK: Color = Color::Rgb(169, 177, 214);    // #a9b1d6
        const FG_GUTTER: Color = Color::Rgb(59, 66, 97);     // #3b4261
        const DARK3: Color = Color::Rgb(84, 91, 112);        // #545c7e
        const COMMENT: Color = Color::Rgb(86, 95, 137);      // #565f89
        const DARK5: Color = Color::Rgb(115, 124, 159);      // #737aa2
        // Accent colors
        const BLUE: Color = Color::Rgb(122, 162, 247);       // #7aa2f7
        const CYAN: Color = Color::Rgb(125, 207, 255);       // #7dcfff
        const BLUE1: Color = Color::Rgb(45, 127, 215);       // #2d7fd7 (for info)
        const BLUE0: Color = Color::Rgb(61, 89, 161);        // #3d59a1
        const MAGENTA: Color = Color::Rgb(187, 154, 247);    // #bb9af7
        const MAGENTA2: Color = Color::Rgb(255, 0, 124);     // #ff007c
        const PURPLE: Color = Color::Rgb(157, 124, 216);     // #9d7cd8
        const ORANGE: Color = Color::Rgb(255, 158, 100);     // #ff9e64
        const YELLOW: Color = Color::Rgb(224, 175, 104);     // #e0af68
        const GREEN: Color = Color::Rgb(158, 206, 106);      // #9ece6a
        const GREEN1: Color = Color::Rgb(115, 218, 202);     // #73daca
        const TEAL: Color = Color::Rgb(30, 215, 210);        // #1ed7d2
        const RED: Color = Color::Rgb(247, 118, 142);        // #f7768e
        const RED1: Color = Color::Rgb(219, 75, 75);         // #db4b4b

        Self {
            name: "Tokyo Night".to_string(),
            colors: ColorPalette {
                bg: BG,
                bg_surface: BG_DARK,
                bg_highlight: BG_HIGHLIGHT,
                border: TERMINAL_BLACK,
                border_focused: BLUE,

                text: FG,
                text_dim: FG_DARK,
                text_muted: COMMENT,

                accent: MAGENTA,
                accent_dim: PURPLE,
                accent_bright: CYAN,

                success: GREEN,
                warning: YELLOW,
                error: RED,
                info: BLUE,

                status_working: GREEN,
                status_idle: COMMENT,
                status_waiting: ORANGE,
                status_paused: BLUE,
                status_error: RED,
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
            "terminal" | "native" => Self::terminal(),
            "catppuccin" | "catppuccin-mocha" | "mocha" => Self::catppuccin_mocha(),
            "catppuccin-latte" | "latte" => Self::catppuccin_latte(),
            "dracula" => Self::dracula(),
            "tokyo-night" | "tokyonight" => Self::tokyo_night(),
            "kage" | "default" => Self::kage(),
            _ => Self::kage(),
        }
    }

    /// List available themes
    pub fn available() -> Vec<&'static str> {
        vec![
            "kage",
            "catppuccin-mocha",
            "catppuccin-latte",
            "dracula",
            "tokyo-night",
            "terminal",
        ]
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::kage()
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
