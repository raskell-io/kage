//! Approval command implementations

use anyhow::Result;

use crate::cli::ApprovalCommands;
use crate::daemon::client::{default_socket_path, DaemonClient};
use crate::daemon::protocol::Response;
use crate::task::ApprovalId;

/// Execute an approval command
pub async fn execute(command: ApprovalCommands) -> Result<()> {
    match command {
        ApprovalCommands::List { json } => list_approvals(json).await,
        ApprovalCommands::Approve { id } => approve_action(&id).await,
        ApprovalCommands::Reject { id, reason } => reject_action(&id, reason).await,
        ApprovalCommands::Show { id } => show_approval(&id).await,
    }
}

/// List pending approvals
async fn list_approvals(json: bool) -> Result<()> {
    let socket = default_socket_path();
    let mut client = DaemonClient::connect(&socket).await?;

    match client.list_approvals().await? {
        Response::ApprovalList { approvals } => {
            if json {
                println!("{}", serde_json::to_string_pretty(&approvals)?);
            } else if approvals.is_empty() {
                println!("No pending approvals");
            } else {
                println!("Pending Approvals ({}):", approvals.len());
                println!("{}", "─".repeat(70));
                println!(
                    "{:<26} {:<12} {:<30}",
                    "ID", "AGENT", "ACTION"
                );
                println!("{}", "─".repeat(70));

                for approval in &approvals {
                    let short_id = &approval.id.to_string()[..8];
                    let short_agent = &approval.agent_id.to_string()[..8];
                    println!(
                        "{:<26} {:<12} {:<30}",
                        short_id,
                        short_agent,
                        truncate(&approval.summary, 28)
                    );
                }

                println!();
                println!("Use 'kage approval approve <id>' to approve an action");
                println!("Use 'kage approval reject <id>' to reject an action");
            }
        }
        Response::Error { message } => {
            anyhow::bail!("Error: {}", message);
        }
        _ => {
            anyhow::bail!("Unexpected response from daemon");
        }
    }

    Ok(())
}

/// Approve an action
async fn approve_action(id: &str) -> Result<()> {
    let socket = default_socket_path();
    let mut client = DaemonClient::connect(&socket).await?;

    if id == "all" {
        // Approve all pending
        match client.list_approvals().await? {
            Response::ApprovalList { approvals } => {
                if approvals.is_empty() {
                    println!("No pending approvals");
                    return Ok(());
                }

                let count = approvals.len();
                for approval in approvals {
                    match client.approve(approval.id).await? {
                        Response::Ok => {
                            println!("✓ Approved: {}", approval.summary);
                        }
                        Response::Error { message } => {
                            println!("✗ Failed to approve {}: {}", approval.id, message);
                        }
                        _ => {}
                    }
                }
                println!();
                println!("Approved {} action(s)", count);
            }
            Response::Error { message } => {
                anyhow::bail!("Error: {}", message);
            }
            _ => {
                anyhow::bail!("Unexpected response from daemon");
            }
        }
    } else {
        // Approve single
        let approval_id: ApprovalId = id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid approval ID: {}", id))?;

        match client.approve(approval_id).await? {
            Response::Ok => {
                println!("✓ Approved action {}", &id[..8.min(id.len())]);
            }
            Response::Error { message } => {
                anyhow::bail!("Error: {}", message);
            }
            _ => {
                anyhow::bail!("Unexpected response from daemon");
            }
        }
    }

    Ok(())
}

/// Reject an action
async fn reject_action(id: &str, reason: Option<String>) -> Result<()> {
    let socket = default_socket_path();
    let mut client = DaemonClient::connect(&socket).await?;

    let approval_id: ApprovalId = id
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid approval ID: {}", id))?;

    match client.reject(approval_id, reason.clone()).await? {
        Response::Ok => {
            let reason_str = reason.map(|r| format!(" ({})", r)).unwrap_or_default();
            println!("✗ Rejected action {}{}", &id[..8.min(id.len())], reason_str);
        }
        Response::Error { message } => {
            anyhow::bail!("Error: {}", message);
        }
        _ => {
            anyhow::bail!("Unexpected response from daemon");
        }
    }

    Ok(())
}

/// Show approval details
async fn show_approval(id: &str) -> Result<()> {
    let socket = default_socket_path();
    let mut client = DaemonClient::connect(&socket).await?;

    let approval_id: ApprovalId = id
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid approval ID: {}", id))?;

    match client.list_approvals().await? {
        Response::ApprovalList { approvals } => {
            if let Some(approval) = approvals.iter().find(|a| a.id == approval_id) {
                println!("Approval Details");
                println!("{}", "═".repeat(50));
                println!();
                println!("ID:        {}", approval.id);
                println!("Agent:     {}", approval.agent_id);
                if let Some(task_id) = &approval.task_id {
                    println!("Task:      {}", task_id);
                }
                println!("Action:    {}", approval.summary);
                println!(
                    "Created:   {}",
                    chrono::DateTime::from_timestamp(approval.created_at, 0)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| "Unknown".to_string())
                );

                if !approval.context.is_empty() {
                    println!();
                    println!("Context:");
                    println!("{}", "─".repeat(50));
                    for line in &approval.context {
                        println!("  {}", line);
                    }
                }

                // Show action details
                println!();
                println!("Action Details:");
                println!("{}", "─".repeat(50));
                println!("{}", serde_json::to_string_pretty(&approval.action)?);

                println!();
                println!("Commands:");
                println!("  kage approval approve {}", &id[..8.min(id.len())]);
                println!("  kage approval reject {} --reason \"...\"", &id[..8.min(id.len())]);
            } else {
                anyhow::bail!("Approval {} not found", id);
            }
        }
        Response::Error { message } => {
            anyhow::bail!("Error: {}", message);
        }
        _ => {
            anyhow::bail!("Unexpected response from daemon");
        }
    }

    Ok(())
}

/// Truncate string with ellipsis
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len.saturating_sub(1)])
    }
}
