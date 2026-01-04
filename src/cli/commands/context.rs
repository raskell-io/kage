//! Context reference command implementations

use anyhow::{Context, Result};

use crate::cli::ContextCommands;
use crate::daemon::client::{default_socket_path, DaemonClient};
use crate::daemon::protocol::{ContextRefInfo, Response};
use crate::memory::ContextRefType;

pub async fn run(cmd: ContextCommands) -> Result<()> {
    match cmd {
        ContextCommands::Attach {
            agent,
            memory,
            r#type,
        } => attach_context(agent, memory, r#type).await,
        ContextCommands::Detach { agent, memory } => detach_context(agent, memory).await,
        ContextCommands::List { agent, r#type, json } => list_context(agent, r#type, json).await,
        ContextCommands::Inherit { from, to, entries } => inherit_context(from, to, entries).await,
    }
}

/// Attach a memory entry to an agent
async fn attach_context(agent_id: String, memory_id: String, ref_type: String) -> Result<()> {
    let mut client = connect_daemon().await?;

    let agent_id = agent_id
        .parse()
        .context("Invalid agent ID format")?;
    let memory_id = memory_id
        .parse()
        .context("Invalid memory ID format")?;

    let ref_type = parse_ref_type(&ref_type)?;

    let response = client.attach_context(agent_id, memory_id, ref_type).await?;

    match response {
        Response::ContextRefAttached { id } => {
            println!("Context attached (ref: {})", id);
        }
        Response::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }

    Ok(())
}

/// Detach a memory entry from an agent
async fn detach_context(agent_id: String, memory_id: String) -> Result<()> {
    let mut client = connect_daemon().await?;

    let agent_id = agent_id
        .parse()
        .context("Invalid agent ID format")?;
    let memory_id = memory_id
        .parse()
        .context("Invalid memory ID format")?;

    let response = client.detach_context(agent_id, memory_id).await?;

    match response {
        Response::ContextRefDetached => {
            println!("Context detached");
        }
        Response::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }

    Ok(())
}

/// List context refs for an agent
async fn list_context(agent_id: String, ref_type: Option<String>, json: bool) -> Result<()> {
    let mut client = connect_daemon().await?;

    let agent_id = agent_id
        .parse()
        .context("Invalid agent ID format")?;

    let response = client.list_context_refs(agent_id, ref_type).await?;

    match response {
        Response::ContextRefList { refs } => {
            if json {
                println!("{}", serde_json::to_string_pretty(&refs)?);
            } else {
                print_context_table(&refs);
            }
        }
        Response::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }

    Ok(())
}

/// Inherit context from parent to child agent
async fn inherit_context(
    from_agent: String,
    to_agent: String,
    entries: Option<String>,
) -> Result<()> {
    let mut client = connect_daemon().await?;

    let from_agent = from_agent
        .parse()
        .context("Invalid parent agent ID format")?;
    let to_agent = to_agent
        .parse()
        .context("Invalid child agent ID format")?;

    let entries = entries.map(|e| {
        e.split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect()
    });

    let response = client.inherit_context(from_agent, to_agent, entries).await?;

    match response {
        Response::ContextInherited { count } => {
            println!("Inherited {} context ref(s)", count);
        }
        Response::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }

    Ok(())
}

/// Parse ref type string to ContextRefType
fn parse_ref_type(s: &str) -> Result<ContextRefType> {
    match s.to_lowercase().as_str() {
        "attached" => Ok(ContextRefType::Attached),
        "pinned" => Ok(ContextRefType::Pinned),
        "bookmarked" => Ok(ContextRefType::Bookmarked),
        _ => anyhow::bail!(
            "Invalid ref type '{}'. Valid types: attached, pinned, bookmarked",
            s
        ),
    }
}

/// Print context refs as a formatted table
fn print_context_table(refs: &[ContextRefInfo]) {
    if refs.is_empty() {
        println!("No context refs found");
        return;
    }

    println!(
        "{:<26}  {:<10}  {:<26}  {:<10}",
        "REF ID", "TYPE", "MEMORY ID", "AUTO-LOAD"
    );
    println!("{}", "-".repeat(80));

    for ctx_ref in refs {
        println!(
            "{:<26}  {:<10}  {:<26}  {:<10}",
            ctx_ref.id.to_string(),
            ctx_ref.ref_type,
            ctx_ref.memory_id.to_string(),
            if ctx_ref.is_auto_load { "yes" } else { "no" },
        );
    }
}

/// Connect to the daemon
async fn connect_daemon() -> Result<DaemonClient> {
    let socket_path = default_socket_path();
    DaemonClient::connect(&socket_path)
        .await
        .context("Failed to connect to daemon. Is it running? Try 'kage daemon start'")
}
