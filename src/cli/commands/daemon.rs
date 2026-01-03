//! Daemon command implementations

use anyhow::Result;

use crate::cli::DaemonCommands;
use crate::config;
use crate::daemon::{client, Daemon, DaemonClient};

pub async fn run(cmd: DaemonCommands) -> Result<()> {
    match cmd {
        DaemonCommands::Start { foreground } => {
            start_daemon(foreground).await
        }
        DaemonCommands::Stop => {
            stop_daemon().await
        }
        DaemonCommands::Status => {
            show_status().await
        }
    }
}

/// Start the daemon
async fn start_daemon(foreground: bool) -> Result<()> {
    let socket_path = client::default_socket_path();

    // Check if daemon is already running
    if client::is_daemon_running(&socket_path).await {
        println!("Daemon is already running");
        return Ok(());
    }

    let config = config::load()?;

    if foreground {
        // Run in foreground (blocking)
        println!("Starting daemon in foreground...");
        println!("Socket: {:?}", socket_path);
        println!("Press Ctrl+C to stop");

        let mut daemon = Daemon::new(config);
        daemon.run().await?;
    } else {
        // Daemonize (fork to background)
        println!("Starting daemon...");

        // On Unix, we can use fork() or spawn a background process
        // For simplicity, we'll use a subprocess approach
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            use std::process::Command;

            // Get the current executable path
            let exe = std::env::current_exe()?;

            // Spawn the daemon process
            let mut cmd = Command::new(&exe);
            cmd.args(["daemon", "start", "--foreground"]);

            // Detach from terminal
            cmd.stdin(std::process::Stdio::null());
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());

            // Create new process group
            unsafe {
                cmd.pre_exec(|| {
                    // Create new session
                    libc::setsid();
                    Ok(())
                });
            }

            cmd.spawn()?;

            // Wait a moment for daemon to start
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            // Check if it's running
            if client::is_daemon_running(&socket_path).await {
                println!("Daemon started successfully");
                println!("Socket: {:?}", socket_path);
            } else {
                println!("Daemon may have failed to start. Check logs for details.");
            }
        }

        #[cfg(not(unix))]
        {
            println!("Background daemon not supported on this platform");
            println!("Use --foreground to run in foreground");
        }
    }

    Ok(())
}

/// Stop the daemon
async fn stop_daemon() -> Result<()> {
    let socket_path = client::default_socket_path();

    if !client::is_daemon_running(&socket_path).await {
        println!("Daemon is not running");
        return Ok(());
    }

    println!("Stopping daemon...");

    let mut client = DaemonClient::connect(&socket_path).await?;
    match client.shutdown().await {
        Ok(_) => {
            println!("Daemon stopped");
        }
        Err(e) => {
            // Connection may be closed before we get a response
            if e.to_string().contains("connection") || e.to_string().contains("EOF") {
                println!("Daemon stopped");
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}

/// Show daemon status
async fn show_status() -> Result<()> {
    let socket_path = client::default_socket_path();

    if !socket_path.exists() {
        println!("Daemon is not running (no socket file)");
        return Ok(());
    }

    match DaemonClient::connect(&socket_path).await {
        Ok(mut client) => {
            match client.status().await? {
                crate::daemon::protocol::Response::Status {
                    version,
                    uptime_secs,
                    active_agents,
                    pending_tasks,
                    running_tasks,
                } => {
                    println!("Daemon Status");
                    println!("─────────────────────────────");
                    println!("Status:         running");
                    println!("Version:        {}", version);
                    println!("Uptime:         {}", format_duration(uptime_secs));
                    println!("Socket:         {:?}", socket_path);
                    println!("");
                    println!("Agents:         {} active", active_agents);
                    println!("Tasks:          {} pending, {} running", pending_tasks, running_tasks);
                }
                crate::daemon::protocol::Response::Error { message } => {
                    println!("Error: {}", message);
                }
                _ => {
                    println!("Unexpected response from daemon");
                }
            }
        }
        Err(_) => {
            println!("Daemon is not running (connection failed)");
        }
    }

    Ok(())
}

/// Format duration in human-readable form
fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d {}h", secs / 86400, (secs % 86400) / 3600)
    }
}
