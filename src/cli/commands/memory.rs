//! Memory command implementations

use std::io::Write;

use anyhow::{Context, Result};

use crate::cli::MemoryCommands;
use crate::daemon::client::{default_socket_path, DaemonClient};
use crate::daemon::protocol::{MemoryInfo, Response};

pub async fn run(cmd: MemoryCommands) -> Result<()> {
    match cmd {
        MemoryCommands::List {
            namespace,
            r#type,
            json,
        } => {
            list_memory(namespace, r#type, json).await
        }
        MemoryCommands::Search { query, namespace } => {
            search_memory(query, namespace).await
        }
        MemoryCommands::Show { id } => {
            show_memory(id).await
        }
        MemoryCommands::Share { id, scope } => {
            share_memory(id, scope).await
        }
        MemoryCommands::Watch { namespace } => {
            watch_memory(namespace).await
        }
        MemoryCommands::Export { format, output } => {
            export_memory(format, output).await
        }
        MemoryCommands::Prune { older_than, dry_run } => {
            prune_memory(older_than, dry_run).await
        }
    }
}

/// List memory entries with optional filters
async fn list_memory(namespace: Option<String>, memory_type: Option<String>, json: bool) -> Result<()> {
    let mut client = connect_daemon().await?;

    let scope = namespace.map(|ns| format!("namespace:{}", ns));

    let response = client
        .query_memory(None, scope, memory_type, vec![], None, Some(100))
        .await?;

    match response {
        Response::MemoryList { entries, .. } => {
            if json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else {
                print_memory_table(&entries);
            }
        }
        Response::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }

    Ok(())
}

/// Search memory entries by text
async fn search_memory(query: String, namespace: Option<String>) -> Result<()> {
    let mut client = connect_daemon().await?;

    let scope = namespace.map(|ns| format!("namespace:{}", ns));

    let response = client
        .query_memory(Some(query), scope, None, vec![], None, Some(50))
        .await?;

    match response {
        Response::MemoryList { entries, .. } => {
            if entries.is_empty() {
                println!("No matching memory entries found.");
            } else {
                print_memory_table(&entries);
            }
        }
        Response::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }

    Ok(())
}

/// Show details of a specific memory entry
async fn show_memory(id: String) -> Result<()> {
    let mut client = connect_daemon().await?;

    let response = client.get_memory(id).await?;

    match response {
        Response::MemoryDetails { entry } => {
            println!("ID:          {}", entry.id);
            println!("Type:        {}", entry.content_type);
            println!("Created:     {}", format_timestamp(entry.created_at));
            println!("Created By:  {}", entry.created_by);
            println!("Scope:       {}", entry.scope);
            if !entry.tags.is_empty() {
                println!("Tags:        {}", entry.tags.join(", "));
            }
            println!();
            println!("Content:");
            println!("{}", entry.content_summary);
            println!();
            println!("Raw:");
            println!("{}", entry.content);
        }
        Response::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }

    Ok(())
}

/// Share a memory entry to a wider scope
async fn share_memory(id: String, scope: String) -> Result<()> {
    let mut client = connect_daemon().await?;

    // First get the entry
    let response = client.get_memory(id.clone()).await?;

    let entry = match response {
        Response::MemoryDetails { entry } => entry,
        Response::Error { message } => anyhow::bail!("{}", message),
        _ => anyhow::bail!("Unexpected response from daemon"),
    };

    // Parse target scope
    let new_scope = parse_scope(&scope)?;

    // Create a new entry with updated scope
    let created_by: crate::agent::AgentId = entry
        .created_by
        .parse()
        .context("Invalid agent ID in memory entry")?;

    let memory_entry = crate::memory::MemoryEntry {
        id: crate::memory::MemoryId::new(),
        created_at: chrono::Utc::now(),
        created_by,
        content: serde_json::from_str(&entry.content)
            .context("Failed to parse content")?,
        tags: entry.tags.clone(),
    };

    let response = client.store_memory(memory_entry, new_scope).await?;

    match response {
        Response::MemoryStored { id } => {
            println!("Memory shared to {} scope with new ID: {}", scope, id);
        }
        Response::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }

    Ok(())
}

/// Watch memory stream in real-time
async fn watch_memory(_namespace: Option<String>) -> Result<()> {
    // Real-time streaming requires WebSocket or persistent connection
    // For now, we'll poll periodically
    println!("Memory watch not yet implemented (requires streaming support).");
    println!("Use 'kage memory list' to view current entries.");
    Ok(())
}

/// Export memory to file
async fn export_memory(format: String, output: std::path::PathBuf) -> Result<()> {
    let mut client = connect_daemon().await?;

    let response = client
        .query_memory(None, None, None, vec![], None, Some(1000))
        .await?;

    let entries = match response {
        Response::MemoryList { entries, .. } => entries,
        Response::Error { message } => anyhow::bail!("{}", message),
        _ => anyhow::bail!("Unexpected response from daemon"),
    };

    let content = match format.as_str() {
        "json" => serde_json::to_string_pretty(&entries)?,
        "markdown" | "md" => format_as_markdown(&entries),
        _ => anyhow::bail!("Unsupported format: {}. Use 'json' or 'markdown'.", format),
    };

    let mut file = std::fs::File::create(&output)
        .with_context(|| format!("Failed to create output file: {:?}", output))?;
    file.write_all(content.as_bytes())?;

    println!("Exported {} entries to {:?}", entries.len(), output);
    Ok(())
}

/// Prune old memory entries
async fn prune_memory(older_than: String, dry_run: bool) -> Result<()> {
    let days = parse_duration(&older_than)?;
    let mut client = connect_daemon().await?;

    let response = client.prune_memory(days, dry_run).await?;

    match response {
        Response::MemoryPruned { count, bytes_freed } => {
            if dry_run {
                println!(
                    "Would prune {} entries ({} freed)",
                    count,
                    format_bytes(bytes_freed)
                );
            } else {
                println!(
                    "Pruned {} entries ({} freed)",
                    count,
                    format_bytes(bytes_freed)
                );
            }
        }
        Response::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => anyhow::bail!("Unexpected response from daemon"),
    }

    Ok(())
}

// --- Helpers ---

async fn connect_daemon() -> Result<DaemonClient> {
    let socket_path = default_socket_path();
    DaemonClient::connect(&socket_path)
        .await
        .context("Failed to connect to daemon. Is it running?")
}

fn print_memory_table(entries: &[MemoryInfo]) {
    if entries.is_empty() {
        println!("No memory entries found.");
        return;
    }

    // Header
    println!(
        "{:<26} {:<18} {:<12} {}",
        "ID", "TYPE", "AGE", "SUMMARY"
    );
    println!("{}", "-".repeat(80));

    for entry in entries {
        let id_short = if entry.id.len() > 24 {
            format!("{}…", &entry.id[..24])
        } else {
            entry.id.clone()
        };

        let summary = if entry.content_summary.len() > 30 {
            format!("{}…", &entry.content_summary[..30])
        } else {
            entry.content_summary.clone()
        };

        println!(
            "{:<26} {:<18} {:<12} {}",
            id_short,
            entry.content_type,
            format_age(entry.created_at),
            summary
        );
    }
}

fn format_timestamp(ts: i64) -> String {
    use chrono::{DateTime, Utc};
    let dt = DateTime::<Utc>::from_timestamp(ts, 0)
        .unwrap_or_else(|| Utc::now());
    dt.format("%Y-%m-%d %H:%M:%S UTC").to_string()
}

fn format_age(ts: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let diff = now - ts;

    if diff < 60 {
        format!("{}s ago", diff)
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn parse_duration(s: &str) -> Result<u32> {
    let s = s.trim().to_lowercase();

    if let Some(days) = s.strip_suffix('d') {
        return days.parse().context("Invalid number of days");
    }
    if let Some(weeks) = s.strip_suffix('w') {
        let w: u32 = weeks.parse().context("Invalid number of weeks")?;
        return Ok(w * 7);
    }
    if let Some(months) = s.strip_suffix('m') {
        let m: u32 = months.parse().context("Invalid number of months")?;
        return Ok(m * 30);
    }

    // Try parsing as plain number (days)
    s.parse().context("Invalid duration. Use format like '30d', '1w', or '2m'")
}

fn parse_scope(s: &str) -> Result<crate::memory::MemoryScope> {
    let s = s.trim().to_lowercase();

    if s == "global" {
        return Ok(crate::memory::MemoryScope::Global);
    }

    if let Some(ns) = s.strip_prefix("namespace:") {
        return Ok(crate::memory::MemoryScope::Namespace(ns.to_string()));
    }

    // Just a namespace name
    if !s.contains(':') {
        return Ok(crate::memory::MemoryScope::Namespace(s.to_string()));
    }

    anyhow::bail!("Invalid scope: {}. Use 'global' or 'namespace:name'", s)
}

fn format_as_markdown(entries: &[MemoryInfo]) -> String {
    let mut md = String::new();
    md.push_str("# Memory Export\n\n");
    md.push_str(&format!("Exported at: {}\n\n", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")));
    md.push_str(&format!("Total entries: {}\n\n", entries.len()));
    md.push_str("---\n\n");

    for entry in entries {
        md.push_str(&format!("## {}\n\n", entry.id));
        md.push_str(&format!("- **Type:** {}\n", entry.content_type));
        md.push_str(&format!("- **Created:** {}\n", format_timestamp(entry.created_at)));
        md.push_str(&format!("- **Created By:** {}\n", entry.created_by));
        md.push_str(&format!("- **Scope:** {}\n", entry.scope));
        if !entry.tags.is_empty() {
            md.push_str(&format!("- **Tags:** {}\n", entry.tags.join(", ")));
        }
        md.push_str("\n### Summary\n\n");
        md.push_str(&entry.content_summary);
        md.push_str("\n\n### Content\n\n```json\n");
        md.push_str(&entry.content);
        md.push_str("\n```\n\n---\n\n");
    }

    md
}
