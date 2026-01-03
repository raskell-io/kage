//! Server command implementations (gRPC mode)

use std::net::SocketAddr;

use anyhow::{Context, Result};

use crate::cli::ServerCommands;
use crate::config;
use crate::daemon::Daemon;
use crate::rpc::GrpcClient;

/// Default gRPC server address
const DEFAULT_GRPC_ADDR: &str = "127.0.0.1:50051";

pub async fn run(cmd: ServerCommands) -> Result<()> {
    match cmd {
        ServerCommands::Start {
            listen,
            tls: _,
            multiuser: _,
        } => {
            start_server(&listen).await
        }
        ServerCommands::Stop => {
            stop_server().await
        }
        ServerCommands::Status => {
            show_status().await
        }
        ServerCommands::EnableMultiuser => {
            enable_multiuser().await
        }
    }
}

/// Start the daemon in server mode with gRPC enabled
async fn start_server(listen: &str) -> Result<()> {
    // Parse the listen address
    let grpc_addr: SocketAddr = listen
        .parse()
        .context("Invalid listen address")?;

    // Load config and set gRPC address
    let mut cfg = config::load()?;
    cfg.daemon.grpc_listen = Some(grpc_addr);

    println!("Starting Kage server on {}...", grpc_addr);
    println!("  Unix socket: {:?}", cfg.daemon.socket_path);
    println!("  gRPC endpoint: {}", grpc_addr);
    println!();
    println!("Press Ctrl+C to stop.");
    println!();

    // Start the daemon
    let mut daemon = Daemon::new(cfg)?;
    daemon.run().await?;

    println!("Server stopped.");
    Ok(())
}

/// Stop a running server
async fn stop_server() -> Result<()> {
    // First try to connect via gRPC
    let addr = get_server_address();

    match GrpcClient::connect(&addr).await {
        Ok(mut client) => {
            println!("Connecting to server at {}...", addr);

            // Get status first to confirm connection
            match client.status().await {
                Ok(status) => {
                    println!("Connected to Kage v{}", status.version);
                    println!("Sending shutdown request...");

                    if let Err(e) = client.shutdown().await {
                        // Shutdown might close connection before response
                        if !e.to_string().contains("connection") {
                            return Err(e);
                        }
                    }

                    println!("Server shutdown initiated.");
                    Ok(())
                }
                Err(e) => {
                    Err(e).context("Failed to get server status")
                }
            }
        }
        Err(_) => {
            // Try Unix socket as fallback
            try_unix_socket_stop().await
        }
    }
}

/// Show server status
async fn show_status() -> Result<()> {
    let addr = get_server_address();

    match GrpcClient::connect(&addr).await {
        Ok(mut client) => {
            let status = client.status().await?;

            println!("Kage Server Status");
            println!("══════════════════════════════════════");
            println!("  Version:        {}", status.version);
            println!("  Uptime:         {}", format_uptime(status.uptime_secs));
            println!("  gRPC endpoint:  {}", addr);
            println!();
            println!("Workload");
            println!("──────────────────────────────────────");
            println!("  Active agents:  {}", status.active_agents);
            println!("  Pending tasks:  {}", status.pending_tasks);
            println!("  Running tasks:  {}", status.running_tasks);

            // Get agent list if any are active
            if status.active_agents > 0 {
                println!();
                println!("Active Agents");
                println!("──────────────────────────────────────");

                if let Ok(agents) = client.list_agents(None, false).await {
                    for agent in agents {
                        println!("  {} [{}] - {}",
                            &agent.id[..8],
                            agent.status,
                            if agent.prompt.is_empty() { "(no prompt)" } else { &agent.prompt }
                        );
                    }
                }
            }

            Ok(())
        }
        Err(_) => {
            // Try Unix socket as fallback
            try_unix_socket_status().await
        }
    }
}

/// Enable multi-user mode
async fn enable_multiuser() -> Result<()> {
    println!("Multi-user mode is not yet implemented.");
    println!();
    println!("This feature will enable:");
    println!("  - User authentication via OAuth/OIDC");
    println!("  - Per-user agent isolation");
    println!("  - Role-based access control");
    println!("  - Audit logging");
    println!();
    println!("Coming in a future release.");
    Ok(())
}

/// Get the server address from environment or default
fn get_server_address() -> String {
    std::env::var("KAGE_SERVER")
        .unwrap_or_else(|_| DEFAULT_GRPC_ADDR.to_string())
}

/// Try to stop via Unix socket (local daemon)
async fn try_unix_socket_stop() -> Result<()> {
    use crate::daemon::DaemonClient;

    let cfg = config::load()?;
    let mut client = DaemonClient::connect(&cfg.daemon.socket_path).await?;

    println!("Connected to local daemon via Unix socket");
    println!("Sending shutdown request...");

    client.shutdown().await?;

    println!("Daemon shutdown initiated.");
    Ok(())
}

/// Try to get status via Unix socket (local daemon)
async fn try_unix_socket_status() -> Result<()> {
    use crate::daemon::DaemonClient;

    let cfg = config::load()?;
    let mut client = DaemonClient::connect(&cfg.daemon.socket_path).await?;

    let response = client.status().await?;

    println!("Kage Daemon Status (Local)");
    println!("══════════════════════════════════════");

    if let crate::daemon::protocol::Response::Status {
        version,
        uptime_secs,
        active_agents,
        pending_tasks,
        running_tasks,
    } = response {
        println!("  Version:        {}", version);
        println!("  Uptime:         {}", format_uptime(uptime_secs));
        println!("  Socket:         {:?}", cfg.daemon.socket_path);
        println!();
        println!("Workload");
        println!("──────────────────────────────────────");
        println!("  Active agents:  {}", active_agents);
        println!("  Pending tasks:  {}", pending_tasks);
        println!("  Running tasks:  {}", running_tasks);
    }

    Ok(())
}

/// Format uptime in human-readable form
fn format_uptime(secs: u64) -> String {
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
