//! Splash screen and branding for Kage
//!
//! Displays the Kage mascot ASCII art and project information.

use std::io::{self, Write};

/// ANSI color codes for the splash screen
mod colors {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";

    // Purple/violet theme matching the mascot
    pub const PURPLE: &str = "\x1b[38;5;141m";
    pub const DARK_PURPLE: &str = "\x1b[38;5;99m";
    pub const LIGHT_PURPLE: &str = "\x1b[38;5;183m";
    pub const VIOLET: &str = "\x1b[38;5;135m";

    // Accent colors
    pub const MOON: &str = "\x1b[38;5;189m";
    pub const SPARKLE: &str = "\x1b[38;5;147m";
    pub const WHITE: &str = "\x1b[38;5;255m";
    pub const CYAN: &str = "\x1b[38;5;117m";
}

/// The Kage mascot ASCII art - a friendly shadow ghost with moon and sparkles
/// Based on kage-mascot.png
const MASCOT_ART: &str = r#"
                                        ·  ✦
                    ·                 )
                  ✧                  (
                         ▄▄▄▄▄▄▄▄     )
                      ▄██████████▄
                    ▄██████████████▄    ☽
                   ████████████████▌  ·
                  ▐████  ▀▀  ████████
                  █████  ◠◠  █████████  ✧
                  ██████      █████████
                  ████████▄▄█████████▀
                 ▐████████████████▀▀
                 █████████████▀▀  ·
                ▐███████▀▀ ▀██▌
               ▄████▀▀      ▀██▄
              ▀▀▀▀            ▀▀▀  ·
                   ·    ✦
"#;

/// Compact mascot for smaller terminals
const MASCOT_COMPACT: &str = r#"
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

/// The Kage logo text
const LOGO_TEXT: &str = r#"
██╗  ██╗ █████╗  ██████╗ ███████╗
██║ ██╔╝██╔══██╗██╔════╝ ██╔════╝
█████╔╝ ███████║██║  ███╗█████╗
██╔═██╗ ██╔══██║██║   ██║██╔══╝
██║  ██╗██║  ██║╚██████╔╝███████╗
╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝ ╚══════╝
"#;

/// Compact logo for smaller terminals
const LOGO_COMPACT: &str = r#"
┏┓┏┓┏┓┏┓
┃┫┣┫┃┓┣
┛┗┛┗┗┛┗┛
"#;

/// Project information
const VERSION: &str = env!("CARGO_PKG_VERSION");
const TAGLINE: &str = "Shadow agents for autonomous code work";

/// URLs for the splash screen
const REPO_URL: &str = "https://github.com/raskell-io/kage";
const WEBSITE_URL: &str = "https://kage.raskell.io";
const DOCS_URL: &str = "https://kage.raskell.io/docs/";

/// Display the full splash screen
pub fn display() {
    display_to(&mut io::stdout());
}

/// Display the splash screen to a specific writer
pub fn display_to<W: Write>(w: &mut W) {
    let term_width = terminal_width();
    let use_compact = term_width < 80;

    writeln!(w).ok();

    // Display mascot with colors
    display_mascot(w, use_compact);

    // Display logo
    display_logo(w, use_compact);

    // Tagline
    writeln!(
        w,
        "{}{}    {}{}",
        colors::DIM,
        colors::LIGHT_PURPLE,
        TAGLINE,
        colors::RESET
    )
    .ok();
    writeln!(w).ok();

    // URLs section
    display_urls(w);

    writeln!(w).ok();
}

/// Display only the mascot
fn display_mascot<W: Write>(w: &mut W, compact: bool) {
    let art = if compact { MASCOT_COMPACT } else { MASCOT_ART };

    for line in art.lines() {
        let colored = colorize_mascot_line(line);
        writeln!(w, "{}", colored).ok();
    }
}

/// Apply colors to a mascot line
fn colorize_mascot_line(line: &str) -> String {
    let mut result = String::new();

    for ch in line.chars() {
        let colored = match ch {
            // Moon
            '☽' => format!("{}{}{}", colors::MOON, ch, colors::RESET),
            // Sparkles and stars
            '✦' | '✧' | '·' => format!("{}{}{}", colors::SPARKLE, ch, colors::RESET),
            // Moon curve characters
            ')' | '(' => format!("{}{}{}", colors::MOON, ch, colors::RESET),
            // Ghost body - block characters
            '█' | '▄' | '▀' | '▐' | '▌' => {
                format!("{}{}{}", colors::DARK_PURPLE, ch, colors::RESET)
            }
            // Eyes
            '◠' => format!("{}{}{}", colors::LIGHT_PURPLE, ch, colors::RESET),
            // Space and other
            _ => ch.to_string(),
        };
        result.push_str(&colored);
    }

    result
}

/// Display the text logo
fn display_logo<W: Write>(w: &mut W, compact: bool) {
    let logo = if compact { LOGO_COMPACT } else { LOGO_TEXT };

    for line in logo.lines() {
        if !line.is_empty() {
            writeln!(w, "{}{}    {}{}", colors::BOLD, colors::PURPLE, line, colors::RESET).ok();
        }
    }

    // Version
    writeln!(
        w,
        "{}{}    影 v{}{}",
        colors::DIM,
        colors::VIOLET,
        VERSION,
        colors::RESET
    )
    .ok();
}

/// Display the URLs section
fn display_urls<W: Write>(w: &mut W) {
    writeln!(
        w,
        "    {}{}Website{}   {}{}",
        colors::DIM,
        colors::WHITE,
        colors::RESET,
        colors::CYAN,
        WEBSITE_URL
    )
    .ok();
    writeln!(
        w,
        "    {}{}Docs{}      {}{}",
        colors::DIM,
        colors::WHITE,
        colors::RESET,
        colors::CYAN,
        DOCS_URL
    )
    .ok();
    writeln!(
        w,
        "    {}{}GitHub{}    {}{}",
        colors::DIM,
        colors::WHITE,
        colors::RESET,
        colors::CYAN,
        REPO_URL
    )
    .ok();
}

/// Get terminal width, defaulting to 80 if detection fails
fn terminal_width() -> usize {
    // Try to get terminal size
    #[cfg(feature = "tui")]
    {
        if let Ok((width, _)) = crossterm::terminal::size() {
            return width as usize;
        }
    }

    // Fallback: check COLUMNS env var
    if let Ok(cols) = std::env::var("COLUMNS") {
        if let Ok(width) = cols.parse::<usize>() {
            return width;
        }
    }

    // Default
    80
}

/// Display a minimal version header (for commands that need less output)
pub fn display_header() {
    println!(
        "{}{}Kage{} {}影{} v{}",
        colors::BOLD,
        colors::PURPLE,
        colors::RESET,
        colors::DIM,
        colors::RESET,
        VERSION
    );
}

/// Display version info only
pub fn display_version() {
    println!(
        "{}kage{} {} (影 - shadow)",
        colors::PURPLE,
        colors::RESET,
        VERSION
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_to_buffer() {
        let mut buffer = Vec::new();
        display_to(&mut buffer);
        let output = String::from_utf8_lossy(&buffer);

        // Check key elements are present
        assert!(output.contains("KAGE") || output.contains("┃┫┣┫"));
        assert!(output.contains(VERSION));
        assert!(output.contains(WEBSITE_URL));
        assert!(output.contains(DOCS_URL));
        assert!(output.contains(REPO_URL));
    }

    #[test]
    fn test_colorize_mascot_line() {
        let line = "  ☽ ✦ █▄▀";
        let colored = colorize_mascot_line(line);

        // Should contain ANSI codes
        assert!(colored.contains("\x1b["));
    }
}
