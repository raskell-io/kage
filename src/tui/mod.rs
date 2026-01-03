//! Terminal UI components for Kage
//!
//! This module contains:
//! - Splash screen and branding
//! - First-run onboarding wizard (requires `tui` feature)
//! - Dashboard for monitoring agents (requires `tui` feature)

pub mod splash;

// Re-export splash functions for convenience
pub use splash::{display as show_splash, display_header, display_version};

/// Onboarding wizard for first-time users
#[cfg(feature = "tui")]
pub mod onboarding;

/// Interactive dashboard
#[cfg(feature = "tui")]
pub mod dashboard;
