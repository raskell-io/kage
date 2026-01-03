//! Terminal UI components for Kage
//!
//! This module contains:
//! - Splash screen and branding
//! - Theme system for customizable styling
//! - First-run onboarding wizard (requires `tui` feature)
//! - Dashboard for monitoring agents (requires `tui` feature)

pub mod splash;

/// Theme system for TUI styling
#[cfg(feature = "tui")]
pub mod theme;

// Re-export splash functions for convenience
pub use splash::{display as show_splash, display_header, display_version};

/// Re-export theme for convenience
#[cfg(feature = "tui")]
pub use theme::Theme;

/// Onboarding wizard for first-time users
#[cfg(feature = "tui")]
pub mod onboarding;

/// Interactive dashboard
#[cfg(feature = "tui")]
pub mod dashboard;
