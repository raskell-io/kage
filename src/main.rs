//! Kage (影) - Shadow agents for autonomous code work
//!
//! A local-first agentic work orchestrator that enables Claude Code agents
//! to work autonomously while you're away.

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

mod agent;
mod cli;
mod config;
mod daemon;
mod memory;
mod namespace;
mod secrets;
mod storage;
mod subscription;
mod task;
mod tui;

use cli::Commands;

/// Kage - Shadow agents for autonomous code work
#[derive(Parser)]
#[command(name = "kage")]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    /// Skip the first-run onboarding wizard
    #[arg(long, global = true)]
    skip_onboarding: bool,

    /// Configuration file path
    #[arg(short, long, global = true)]
    config: Option<std::path::PathBuf>,

    /// Enable verbose logging
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing
    let filter = match cli.verbose {
        0 => "kage=info",
        1 => "kage=debug",
        _ => "kage=trace",
    };

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter)))
        .init();

    // Check for first run and show onboarding if needed
    if !cli.skip_onboarding && config::is_first_run() {
        #[cfg(feature = "tui")]
        {
            tracing::info!("First run detected, launching onboarding wizard");
            match tui::onboarding::run().await {
                Ok(completed) => {
                    if completed {
                        tracing::info!("Onboarding completed successfully");
                    } else {
                        tracing::info!("Onboarding skipped");
                        // Still mark as initialized so we don't show again
                        let _ = config::mark_initialized();
                    }
                }
                Err(e) => {
                    tracing::warn!("Onboarding wizard failed: {}. Continuing...", e);
                    let _ = config::mark_initialized();
                }
            }
        }

        #[cfg(not(feature = "tui"))]
        {
            tracing::info!("First run detected. Run with TUI feature for onboarding wizard.");
            tracing::info!("Or configure manually at ~/.config/kage/config.toml");
            // Mark as initialized so we don't keep showing this message
            let _ = config::mark_initialized();
        }
    }

    // Handle commands
    match cli.command {
        Some(cmd) => cli::run(cmd).await,
        None => {
            // No subcommand - show splash screen or launch dashboard
            #[cfg(feature = "tui")]
            {
                // Launch interactive dashboard if terminal is a TTY
                if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                    tui::dashboard::run().await?;
                    return Ok(());
                }
            }

            // Non-interactive mode: show splash and help
            tui::show_splash();
            println!("Run 'kage --help' for usage information");
            println!();
            Ok(())
        }
    }
}
