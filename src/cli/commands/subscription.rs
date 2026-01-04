//! Subscription command implementations

use std::io::{self, Write};

use anyhow::Result;

use crate::cli::SubscriptionCommands;
use crate::config;
use crate::daemon::{client, DaemonClient};
use crate::daemon::protocol::Response;
use crate::subscription::registry::SubscriptionRegistry;
use crate::subscription::usage::{UsagePeriod, UsageTracker};
use crate::subscription::{ProviderType, Subscription, SubscriptionStatus};

pub async fn run(cmd: SubscriptionCommands) -> Result<()> {
    match cmd {
        SubscriptionCommands::Add {
            name,
            provider,
            priority,
            tags,
            description,
        } => add_subscription(name, provider, priority, tags, description).await,
        SubscriptionCommands::List { status, json } => list_subscriptions(status, json).await,
        SubscriptionCommands::Show { name } => show_subscription(name).await,
        SubscriptionCommands::Remove { name, force } => remove_subscription(name, force).await,
        SubscriptionCommands::Enable { name } => enable_subscription(name).await,
        SubscriptionCommands::Disable { name } => disable_subscription(name).await,
        SubscriptionCommands::Status => show_status().await,
        SubscriptionCommands::Usage {
            period,
            subscription,
            json,
        } => show_usage(period, subscription, json).await,
        SubscriptionCommands::Route {
            namespace,
            priority,
            subscriptions,
        } => set_route(namespace, priority, subscriptions).await,
    }
}

/// Get the subscription registry
fn get_registry() -> Result<SubscriptionRegistry> {
    let config = config::load()?;
    let state_dir = config.daemon.state_dir.clone();
    let db_path = state_dir.join("subscriptions.redb");
    SubscriptionRegistry::open(db_path)
}

/// Parse provider type from string
fn parse_provider(s: &str) -> Result<ProviderType> {
    match s.to_lowercase().as_str() {
        "claude-code" | "claude" => Ok(ProviderType::ClaudeCode),
        "anthropic-api" | "anthropic" => Ok(ProviderType::AnthropicApi),
        "openai" => Ok(ProviderType::OpenAi),
        "gemini" => Ok(ProviderType::Gemini),
        other => Ok(ProviderType::Custom(other.to_string())),
    }
}

/// Add a new subscription
async fn add_subscription(
    name: String,
    provider: String,
    priority: u8,
    tags: Option<String>,
    description: Option<String>,
) -> Result<()> {
    // Prompt for API key
    print!("Enter API key for '{}': ", name);
    io::stdout().flush()?;

    let mut api_key = String::new();
    io::stdin().read_line(&mut api_key)?;
    let api_key = api_key.trim().to_string();

    if api_key.is_empty() {
        anyhow::bail!("API key cannot be empty");
    }

    let socket_path = client::default_socket_path();

    // Use daemon if running (preferred - avoids DB lock issues and stores key in database)
    if client::is_daemon_running(&socket_path).await {
        let mut daemon_client = DaemonClient::connect(&socket_path).await?;
        match daemon_client.add_subscription(name.clone(), api_key).await? {
            Response::SubscriptionAdded { name: added_name } => {
                println!("Subscription '{}' added successfully", added_name);
                println!("API key stored securely in subscription database");
                return Ok(());
            }
            Response::Error { message } => {
                anyhow::bail!("Daemon error: {}", message);
            }
            _ => {
                anyhow::bail!("Unexpected response from daemon");
            }
        }
    }

    // Fallback: open database directly (only works if daemon is not running)
    let registry = get_registry()?;
    let provider_type = parse_provider(&provider)?;

    // Create subscription with API key stored directly in database
    let key_ref = format!("subscription:{}", name);
    let mut subscription = Subscription::new(&name, &key_ref)
        .with_provider(provider_type)
        .with_priority(priority)
        .with_api_key(&api_key);

    if let Some(tags_str) = tags {
        let tag_list: Vec<String> = tags_str.split(',').map(|s| s.trim().to_string()).collect();
        subscription = subscription.with_tags(tag_list);
    }

    if let Some(desc) = description {
        subscription = subscription.with_description(&desc);
    }

    let id = registry.add(subscription)?;

    println!("Subscription '{}' added successfully", name);
    println!("ID: {}", id);
    println!("API key stored securely in subscription database");

    Ok(())
}

/// List subscriptions - queries the daemon if running, otherwise opens DB directly
async fn list_subscriptions(status: Option<String>, json: bool) -> Result<()> {
    let socket_path = client::default_socket_path();

    // Try to query daemon first (preferred - avoids DB lock issues)
    if client::is_daemon_running(&socket_path).await {
        let mut daemon_client = DaemonClient::connect(&socket_path).await?;
        match daemon_client.list_subscriptions().await? {
            Response::SubscriptionList { subscriptions } => {
                if json {
                    println!("{}", serde_json::to_string_pretty(&subscriptions)?);
                    return Ok(());
                }

                if subscriptions.is_empty() {
                    println!("No subscriptions registered");
                    println!("");
                    println!("Add a subscription with: kage subscription add --name <name>");
                    return Ok(());
                }

                println!("{:<20} {:<15} {:<12}", "NAME", "PROVIDER", "STATUS");
                println!("{}", "─".repeat(50));

                for sub in subscriptions {
                    let status_str = match sub.status.as_str() {
                        "active" => "\x1b[32mactive\x1b[0m".to_string(),
                        "disabled" => "\x1b[90mdisabled\x1b[0m".to_string(),
                        s if s.starts_with("rate") => "\x1b[33mrate-limited\x1b[0m".to_string(),
                        s if s.starts_with("exhaust") => "\x1b[31mexhausted\x1b[0m".to_string(),
                        s if s.starts_with("unhealthy") => "\x1b[31munhealthy\x1b[0m".to_string(),
                        other => other.to_string(),
                    };

                    let name_display = if sub.name.len() > 18 {
                        format!("{}...", &sub.name[..15])
                    } else {
                        sub.name.clone()
                    };

                    println!("{:<20} {:<15} {}", name_display, sub.provider, status_str);
                }
                return Ok(());
            }
            Response::Error { message } => {
                anyhow::bail!("Daemon error: {}", message);
            }
            _ => {
                anyhow::bail!("Unexpected response from daemon");
            }
        }
    }

    // Fallback: open database directly (only works if daemon is not running)
    let registry = get_registry()?;

    let subscriptions = if let Some(status_filter) = status {
        registry.list_by_status(&status_filter)
    } else {
        registry.list()
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&subscriptions)?);
        return Ok(());
    }

    if subscriptions.is_empty() {
        println!("No subscriptions registered");
        println!("");
        println!("Add a subscription with: kage subscription add --name <name>");
        return Ok(());
    }

    println!(
        "{:<20} {:<15} {:<12} {:<8} {:<10}",
        "NAME", "PROVIDER", "STATUS", "PRIORITY", "REQUESTS"
    );
    println!("{}", "─".repeat(70));

    for sub in subscriptions {
        let status_str = match &sub.status {
            SubscriptionStatus::Active => "\x1b[32mactive\x1b[0m".to_string(),
            SubscriptionStatus::Disabled => "\x1b[90mdisabled\x1b[0m".to_string(),
            SubscriptionStatus::RateLimited { .. } => "\x1b[33mrate-limited\x1b[0m".to_string(),
            SubscriptionStatus::Exhausted { .. } => "\x1b[31mexhausted\x1b[0m".to_string(),
            SubscriptionStatus::Unhealthy { .. } => "\x1b[31munhealthy\x1b[0m".to_string(),
        };

        let name_display = if sub.name.len() > 18 {
            format!("{}...", &sub.name[..15])
        } else {
            sub.name.clone()
        };

        println!(
            "{:<20} {:<15} {:<22} {:<8} {}",
            name_display,
            sub.provider.to_string(),
            status_str,
            sub.priority,
            sub.usage.total_requests
        );
    }

    Ok(())
}

/// Show subscription details
async fn show_subscription(name: String) -> Result<()> {
    let registry = get_registry()?;

    let sub = registry
        .get_by_name(&name)
        .ok_or_else(|| anyhow::anyhow!("Subscription '{}' not found", name))?;

    println!("Subscription Details");
    println!("{}", "─".repeat(40));
    println!("Name:         {}", sub.name);
    println!("ID:           {}", sub.id);
    println!("Provider:     {}", sub.provider);
    println!("Priority:     {}", sub.priority);

    let status_str = match &sub.status {
        SubscriptionStatus::Active => "Active".to_string(),
        SubscriptionStatus::Disabled => "Disabled".to_string(),
        SubscriptionStatus::RateLimited { until } => format!("Rate limited until {}", until),
        SubscriptionStatus::Exhausted { resets_at } => format!("Exhausted (resets at {})", resets_at),
        SubscriptionStatus::Unhealthy { reason, since } => {
            format!("Unhealthy since {}: {}", since, reason)
        }
    };
    println!("Status:       {}", status_str);

    if !sub.tags.is_empty() {
        println!("Tags:         {}", sub.tags.join(", "));
    }

    if let Some(desc) = &sub.description {
        println!("Description:  {}", desc);
    }

    println!("");
    println!("Usage Statistics");
    println!("{}", "─".repeat(40));
    println!("Total requests:    {}", sub.usage.total_requests);
    println!("Successful:        {}", sub.usage.successful_requests);
    println!("Failed:            {}", sub.usage.failed_requests);
    println!("Total tokens:      {}", sub.usage.total_tokens);
    println!(
        "Success rate:      {:.1}%",
        sub.usage.success_rate() * 100.0
    );
    println!("Rate limit hits:   {}", sub.usage.rate_limit_hits);

    if let Some(last_used) = sub.usage.last_used_at {
        let dt = chrono::DateTime::from_timestamp(last_used, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        println!("Last used:         {}", dt);
    }

    println!("");
    println!("Rate Limit State");
    println!("{}", "─".repeat(40));
    println!(
        "Requests in window: {}/{}",
        sub.rate_limit_state.requests_in_window, sub.rate_limit_state.max_requests_per_window
    );
    println!("Tokens today:       {}", sub.rate_limit_state.tokens_today);
    println!(
        "Consecutive errors: {}",
        sub.rate_limit_state.consecutive_errors
    );

    Ok(())
}

/// Remove a subscription
async fn remove_subscription(name: String, force: bool) -> Result<()> {
    let registry = get_registry()?;

    let sub = registry
        .get_by_name(&name)
        .ok_or_else(|| anyhow::anyhow!("Subscription '{}' not found", name))?;

    if !force {
        print!(
            "Are you sure you want to remove subscription '{}'? [y/N]: ",
            name
        );
        io::stdout().flush()?;

        let mut response = String::new();
        io::stdin().read_line(&mut response)?;

        if !response.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled");
            return Ok(());
        }
    }

    // Remove API key from keychain
    let _ = crate::secrets::delete(&sub.api_key_ref, &crate::secrets::SecretScope::Global);

    // Remove subscription
    registry.remove(sub.id)?;

    println!("Subscription '{}' removed", name);

    Ok(())
}

/// Enable a subscription
async fn enable_subscription(name: String) -> Result<()> {
    let registry = get_registry()?;

    let sub = registry
        .get_by_name(&name)
        .ok_or_else(|| anyhow::anyhow!("Subscription '{}' not found", name))?;

    registry.enable(sub.id)?;

    println!("Subscription '{}' enabled", name);

    Ok(())
}

/// Disable a subscription
async fn disable_subscription(name: String) -> Result<()> {
    let registry = get_registry()?;

    let sub = registry
        .get_by_name(&name)
        .ok_or_else(|| anyhow::anyhow!("Subscription '{}' not found", name))?;

    registry.disable(sub.id)?;

    println!("Subscription '{}' disabled", name);

    Ok(())
}

/// Show pool status
async fn show_status() -> Result<()> {
    let registry = get_registry()?;

    let all = registry.list();
    let available = registry.list_available();

    println!("Subscription Pool Status");
    println!("{}", "─".repeat(40));
    println!("Total subscriptions:     {}", all.len());
    println!("Available:               {}", available.len());
    println!(
        "Disabled:                {}",
        all.iter()
            .filter(|s| matches!(s.status, SubscriptionStatus::Disabled))
            .count()
    );
    println!(
        "Rate limited:            {}",
        all.iter()
            .filter(|s| matches!(s.status, SubscriptionStatus::RateLimited { .. }))
            .count()
    );
    println!(
        "Unhealthy:               {}",
        all.iter()
            .filter(|s| matches!(s.status, SubscriptionStatus::Unhealthy { .. }))
            .count()
    );

    // Health score
    let health_score = if all.is_empty() {
        0.0
    } else {
        available.len() as f64 / all.len() as f64 * 100.0
    };

    let health_str = if health_score >= 80.0 {
        format!("\x1b[32m{:.0}%\x1b[0m", health_score)
    } else if health_score >= 50.0 {
        format!("\x1b[33m{:.0}%\x1b[0m", health_score)
    } else {
        format!("\x1b[31m{:.0}%\x1b[0m", health_score)
    };

    println!("");
    println!("Health Score:            {}", health_str);

    // Aggregate usage
    let total_requests: u64 = all.iter().map(|s| s.usage.total_requests).sum();
    let total_tokens: u64 = all.iter().map(|s| s.usage.total_tokens).sum();

    println!("");
    println!("Aggregate Usage");
    println!("{}", "─".repeat(40));
    println!("Total requests:          {}", total_requests);
    println!("Total tokens:            {}", total_tokens);

    Ok(())
}

/// Show usage statistics
async fn show_usage(period: String, subscription: Option<String>, json: bool) -> Result<()> {
    let config = config::load()?;
    let usage_dir = config.daemon.state_dir.join("usage");

    let mut tracker = UsageTracker::new(usage_dir)?;

    let usage_period = match period.to_lowercase().as_str() {
        "today" => UsagePeriod::Today,
        "week" => UsagePeriod::Week,
        "month" => UsagePeriod::Month,
        _ => {
            anyhow::bail!("Invalid period. Use 'today', 'week', or 'month'");
        }
    };

    if json {
        let stats = if subscription.is_some() {
            tracker.stats(usage_period)?
        } else {
            tracker.stats(usage_period)?
        };
        println!("{}", serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }

    let stats = tracker.stats(usage_period)?;

    println!("Usage Statistics ({})", period);
    println!("{}", "─".repeat(40));
    println!("Total requests:        {}", stats.total_requests);
    println!("Successful:            {}", stats.successful_requests);
    println!("Failed:                {}", stats.failed_requests);
    println!("Total tokens:          {}", stats.total_tokens);
    println!("  Input tokens:        {}", stats.total_input_tokens);
    println!("  Output tokens:       {}", stats.total_output_tokens);
    println!(
        "Avg tokens/request:    {:.0}",
        stats.avg_tokens_per_request
    );
    println!("Avg duration:          {:.0}ms", stats.avg_duration_ms);
    println!("Success rate:          {:.1}%", stats.success_rate * 100.0);

    // Show per-subscription breakdown
    println!("");
    println!("Per-Subscription Breakdown");
    println!("{}", "─".repeat(40));

    let by_sub = tracker.stats_by_subscription(usage_period)?;

    if by_sub.is_empty() {
        println!("No usage data available");
    } else {
        let registry = get_registry()?;
        for (sub_id, sub_stats) in by_sub {
            let name = registry
                .get(sub_id)
                .map(|s| s.name)
                .unwrap_or_else(|| sub_id.to_string());
            println!(
                "  {}: {} requests, {} tokens",
                name, sub_stats.total_requests, sub_stats.total_tokens
            );
        }
    }

    Ok(())
}

/// Set routing configuration
async fn set_route(
    namespace: Option<String>,
    priority: Option<String>,
    subscriptions: String,
) -> Result<()> {
    if namespace.is_none() && priority.is_none() {
        anyhow::bail!("Must specify either --namespace or --priority");
    }

    let sub_names: Vec<String> = subscriptions
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();

    // Verify subscriptions exist
    let registry = get_registry()?;
    for name in &sub_names {
        if registry.get_by_name(name).is_none() {
            anyhow::bail!("Subscription '{}' not found", name);
        }
    }

    // For now, just print what would be configured
    // In a full implementation, this would update the routing config
    if let Some(ns) = namespace {
        println!(
            "Route configured: namespace '{}' -> [{}]",
            ns,
            sub_names.join(", ")
        );
    }

    if let Some(prio) = priority {
        println!(
            "Route configured: priority '{}' -> [{}]",
            prio,
            sub_names.join(", ")
        );
    }

    println!("");
    println!("Note: Routing configuration will be persisted in future versions");

    Ok(())
}
