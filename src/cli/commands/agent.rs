//! Agent command implementations

use std::io::{self, Write};

use anyhow::Result;

use crate::agent::AgentId;
use crate::cli::AgentCommands;
use crate::daemon::{client, protocol::Response, DaemonClient};

pub async fn run(cmd: AgentCommands) -> Result<()> {
    match cmd {
        AgentCommands::Spawn {
            repo,
            namespace,
            prompt,
        } => spawn_agent(repo, namespace, prompt).await,
        AgentCommands::List { namespace, json } => list_agents(namespace, json).await,
        AgentCommands::Attach { id } => attach_agent(id).await,
        AgentCommands::Detach => {
            println!("Use Ctrl+C to detach from an agent");
            Ok(())
        }
        AgentCommands::Kill { id, force } => kill_agent(id, force).await,
        AgentCommands::Status { id } => show_agent_status(id).await,
    }
}

/// Get connected daemon client or fail with helpful message
async fn get_client() -> Result<DaemonClient> {
    let socket_path = client::default_socket_path();

    if !client::is_daemon_running(&socket_path).await {
        anyhow::bail!(
            "Daemon is not running. Start it with: kage daemon start"
        );
    }

    DaemonClient::connect(&socket_path).await
}

/// Spawn a new agent
async fn spawn_agent(
    repo: std::path::PathBuf,
    namespace: Option<String>,
    prompt: Option<String>,
) -> Result<()> {
    let mut client = get_client().await?;

    // Use provided repo path
    let working_dir = repo;

    // Verify directory exists
    if !working_dir.exists() {
        anyhow::bail!("Directory does not exist: {:?}", working_dir);
    }

    println!("Spawning agent in {:?}...", working_dir);

    let response = client
        .spawn_agent(working_dir, namespace.clone(), prompt, None, None)
        .await?;

    match response {
        Response::AgentSpawned { id } => {
            println!("Agent spawned: {}", id);
            if let Some(ns) = namespace {
                println!("Namespace: {}", ns);
            }
            println!("");
            println!("To attach: kage agent attach {}", id);
            println!("To kill:   kage agent kill {}", id);
        }
        Response::Error { message } => {
            anyhow::bail!("Failed to spawn agent: {}", message);
        }
        _ => {
            anyhow::bail!("Unexpected response from daemon");
        }
    }

    Ok(())
}

/// List agents
async fn list_agents(namespace: Option<String>, json: bool) -> Result<()> {
    let mut client = get_client().await?;

    let response = client.list_agents(namespace, false).await?;

    match response {
        Response::AgentList { agents } => {
            if json {
                println!("{}", serde_json::to_string_pretty(&agents)?);
            } else if agents.is_empty() {
                println!("No agents running");
            } else {
                println!(
                    "{:<14} {:<12} {:<10} {:<20} {:<10}",
                    "ID", "NAME", "STATUS", "WORKING DIR", "ITERATIONS"
                );
                println!("{}", "─".repeat(70));

                for agent in agents {
                    let id_short = agent.id.to_string();
                    let id_display = if id_short.len() > 12 {
                        format!("{}...", &id_short[..9])
                    } else {
                        id_short
                    };

                    let dir_display = agent
                        .working_dir
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| agent.working_dir.to_string_lossy().to_string());

                    let dir_short = if dir_display.len() > 18 {
                        format!("...{}", &dir_display[dir_display.len() - 15..])
                    } else {
                        dir_display
                    };

                    let status_colored = match agent.status.as_str() {
                        "running" => format!("\x1b[32m{}\x1b[0m", agent.status),
                        "paused" => format!("\x1b[33m{}\x1b[0m", agent.status),
                        "stopped" | "failed" => format!("\x1b[31m{}\x1b[0m", agent.status),
                        _ => agent.status.clone(),
                    };

                    println!(
                        "{:<14} {:<12} {:<10} {:<20} {}/{}",
                        id_display,
                        agent.name,
                        status_colored,
                        dir_short,
                        agent.iteration,
                        agent.max_iterations
                    );
                }
            }
        }
        Response::Error { message } => {
            anyhow::bail!("Failed to list agents: {}", message);
        }
        _ => {
            anyhow::bail!("Unexpected response from daemon");
        }
    }

    Ok(())
}

/// Attach to an agent
async fn attach_agent(id: String) -> Result<()> {
    let mut client = get_client().await?;

    // Parse agent ID
    let agent_id: AgentId = id.parse().map_err(|_| anyhow::anyhow!("Invalid agent ID: {}", id))?;

    println!("Attaching to agent {}...", id);
    println!("Press Ctrl+C to detach");
    println!("{}", "─".repeat(50));

    // Start attach
    client.attach(agent_id).await?;

    // Read and display output
    loop {
        match client.read_stream_line().await? {
            Some(Response::StreamLine { text, is_error, .. }) => {
                if is_error {
                    eprint!("{}", text);
                    io::stderr().flush()?;
                } else {
                    print!("{}", text);
                    io::stdout().flush()?;
                }
            }
            Some(Response::StreamEnd) => {
                println!("\n{}", "─".repeat(50));
                println!("Agent stream ended");
                break;
            }
            Some(Response::Error { message }) => {
                anyhow::bail!("Error: {}", message);
            }
            None => {
                println!("\n{}", "─".repeat(50));
                println!("Connection closed");
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

/// Kill an agent
async fn kill_agent(id: String, force: bool) -> Result<()> {
    let mut client = get_client().await?;

    // Parse agent ID
    let agent_id: AgentId = id.parse().map_err(|_| anyhow::anyhow!("Invalid agent ID: {}", id))?;

    if force {
        println!("Force killing agent {}...", id);
    } else {
        println!("Stopping agent {}...", id);
    }

    let response = client.kill_agent(agent_id, force).await?;

    match response {
        Response::Ok => {
            println!("Agent stopped");
        }
        Response::Error { message } => {
            anyhow::bail!("Failed to kill agent: {}", message);
        }
        _ => {
            anyhow::bail!("Unexpected response from daemon");
        }
    }

    Ok(())
}

/// Show agent status
async fn show_agent_status(id: String) -> Result<()> {
    let mut client = get_client().await?;

    // Parse agent ID
    let agent_id: AgentId = id.parse().map_err(|_| anyhow::anyhow!("Invalid agent ID: {}", id))?;

    let response = client.get_agent(agent_id).await?;

    match response {
        Response::AgentDetails { agent } => {
            println!("Agent Details");
            println!("{}", "─".repeat(40));
            println!("ID:           {}", agent.id);
            println!("Name:         {}", agent.name);
            println!("Status:       {}", agent.status);
            println!("Working Dir:  {:?}", agent.working_dir);
            if let Some(ns) = &agent.namespace {
                println!("Namespace:    {}", ns);
            }
            println!("Iterations:   {}/{}", agent.iteration, agent.max_iterations);
            if let Some(pid) = agent.pid {
                println!("PID:          {}", pid);
            }

            // Format started time
            let started = chrono::DateTime::from_timestamp(agent.started_at, 0)
                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            println!("Started:      {}", started);
        }
        Response::Error { message } => {
            anyhow::bail!("Agent not found: {}", message);
        }
        _ => {
            anyhow::bail!("Unexpected response from daemon");
        }
    }

    Ok(())
}
