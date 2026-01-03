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

#[cfg(feature = "server")]
mod rpc;

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
            // No subcommand - launch dashboard (or show help if not a TTY)
            #[cfg(feature = "tui")]
            {
                // Launch interactive dashboard if terminal is a TTY
                if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
                    // Ensure daemon is running before launching dashboard
                    ensure_daemon_running().await?;
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

/// Ensure the daemon is running, starting it in the background if needed
async fn ensure_daemon_running() -> Result<()> {
    use daemon::client::{default_socket_path, is_daemon_running};

    let socket_path = default_socket_path();

    if is_daemon_running(&socket_path).await {
        tracing::debug!("Daemon already running");
        return Ok(());
    }

    tracing::info!("Starting daemon in background...");

    // Load config
    let cfg = config::load()?;

    // Spawn daemon in background
    let daemon_cfg = cfg.clone();
    tokio::spawn(async move {
        match daemon::Daemon::new(daemon_cfg) {
            Ok(mut d) => {
                if let Err(e) = d.run().await {
                    tracing::error!("Daemon error: {}", e);
                }
            }
            Err(e) => {
                tracing::error!("Failed to create daemon: {}", e);
            }
        }
    });

    // Wait briefly for daemon to start
    for _ in 0..10 {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        if is_daemon_running(&socket_path).await {
            tracing::info!("Daemon started successfully");
            return Ok(());
        }
    }

    // Continue anyway - dashboard will show disconnected status
    tracing::warn!("Daemon may not have started, continuing with dashboard");
    Ok(())
}
