//! Connect command implementation (remote daemon connection)

use anyhow::{Context, Result};

use crate::rpc::GrpcClient;

pub async fn run(address: String) -> Result<()> {
    println!("Connecting to remote Kage server at {}...", address);

    let mut client = GrpcClient::connect(&address)
        .await
        .context("Failed to connect to server")?;

    // Ping to verify connection
    let (version, uptime_secs) = client.ping().await?;

    println!();
    println!("Connected to Kage v{}", version);
    println!("Server uptime: {}", format_uptime(uptime_secs));

    // Get full status
    let status = client.status().await?;

    println!();
    println!("Server Status");
    println!("──────────────────────────────────────");
    println!("  Active agents:  {}", status.active_agents);
    println!("  Pending tasks:  {}", status.pending_tasks);
    println!("  Running tasks:  {}", status.running_tasks);

    // Set the server address for future commands
    println!();
    println!("To use this server for future commands, set:");
    println!("  export KAGE_SERVER={}", address);

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
