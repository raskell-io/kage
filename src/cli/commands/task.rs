//! Task command implementations

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::Result;
use ulid::Ulid;

use crate::cli::{CheckpointCommands, TaskCommands};
use crate::config;
use crate::task::{ApprovalLevel, CheckpointStore, Criterion, Task, TaskConfig, TaskId, TaskRegistry, TaskStatus};

pub async fn run(cmd: TaskCommands) -> Result<()> {
    match cmd {
        TaskCommands::Add {
            goal,
            namespace,
            repo,
            max_iterations,
            checkpoint_every,
            approval,
            priority,
            depends_on,
            success_on,
            abort_on,
        } => {
            add_task(
                goal,
                namespace,
                repo,
                max_iterations,
                checkpoint_every,
                approval,
                priority,
                depends_on,
                success_on,
                abort_on,
            )
            .await
        }
        TaskCommands::List {
            status,
            namespace,
            json,
        } => list_tasks(status, namespace, json).await,
        TaskCommands::Resume {
            id,
            extend_iterations,
            guidance,
        } => resume_task(id, extend_iterations, guidance).await,
        TaskCommands::Show { id } => show_task(id).await,
        TaskCommands::Pause { id } => pause_task(id).await,
        TaskCommands::Cancel { id, force } => cancel_task(id, force).await,
        TaskCommands::Checkpoint { command } => run_checkpoint_command(command).await,
    }
}

/// Get the task registry
fn get_registry() -> Result<TaskRegistry> {
    let config = config::load()?;
    let db_path = config.daemon.state_dir.join("tasks.redb");
    TaskRegistry::open(db_path)
}

/// Get the checkpoint store
fn get_checkpoint_store() -> Result<CheckpointStore> {
    let config = config::load()?;
    CheckpointStore::new(config.daemon.state_dir)
}

/// Parse approval level from string
fn parse_approval(s: &str) -> Result<ApprovalLevel> {
    match s.to_lowercase().as_str() {
        "none" => Ok(ApprovalLevel::None),
        "on-write" | "onwrite" => Ok(ApprovalLevel::OnWrite),
        "on-commit" | "oncommit" => Ok(ApprovalLevel::OnCommit),
        "always" => Ok(ApprovalLevel::Always),
        _ => anyhow::bail!(
            "Invalid approval level: {}. Use: none, on-write, on-commit, always",
            s
        ),
    }
}

/// Parse status from string
fn parse_status(s: &str) -> Result<TaskStatus> {
    match s.to_lowercase().as_str() {
        "pending" => Ok(TaskStatus::Pending),
        "running" => Ok(TaskStatus::Running),
        "paused" => Ok(TaskStatus::Paused),
        "completed" => Ok(TaskStatus::Completed),
        "failed" => Ok(TaskStatus::Failed),
        "cancelled" => Ok(TaskStatus::Cancelled),
        _ => anyhow::bail!(
            "Invalid status: {}. Use: pending, running, paused, completed, failed, cancelled",
            s
        ),
    }
}

/// Parse a success criterion from string
/// Formats:
///   pattern:REGEX              - Match regex in output
///   file-exists:PATH           - Check if file exists
///   file-contains:PATH:PATTERN - Check if file contains pattern
///   command:CMD                - Run command and check exit code
///   tests-pass                 - Run tests and check they pass
fn parse_success_criterion(s: &str) -> Result<Criterion> {
    let lower = s.to_lowercase();

    if lower == "tests-pass" {
        return Ok(Criterion::TestsPass);
    }

    if let Some(rest) = s.strip_prefix("pattern:") {
        return Ok(Criterion::PatternMatch {
            pattern: rest.to_string(),
            last_n_lines: 0,
        });
    }

    if let Some(rest) = s.strip_prefix("file-exists:") {
        return Ok(Criterion::FileExists {
            path: PathBuf::from(rest),
        });
    }

    if let Some(rest) = s.strip_prefix("file-contains:") {
        let parts: Vec<&str> = rest.splitn(2, ':').collect();
        if parts.len() != 2 {
            anyhow::bail!("file-contains format: file-contains:PATH:PATTERN");
        }
        return Ok(Criterion::FileContains {
            path: PathBuf::from(parts[0]),
            pattern: parts[1].to_string(),
        });
    }

    if let Some(rest) = s.strip_prefix("command:") {
        return Ok(Criterion::CommandSucceeds {
            command: rest.to_string(),
            working_dir: None,
        });
    }

    anyhow::bail!(
        "Unknown success criterion: {}. Use: pattern:REGEX, file-exists:PATH, \
        file-contains:PATH:PATTERN, command:CMD, tests-pass",
        s
    )
}

/// Parse an abort criterion from string
/// Formats:
///   error-count:MAX:PATTERN        - Abort if pattern appears MAX times
///   repeated-output:MIN:WINDOW     - Abort if same output repeats MIN times in WINDOW lines
///   output-contains:PHRASE1,PHRASE2 - Abort if any phrase appears in output
///   no-progress:ITERATIONS         - Abort if no file changes for N iterations
fn parse_abort_criterion(s: &str) -> Result<Criterion> {
    if let Some(rest) = s.strip_prefix("error-count:") {
        let parts: Vec<&str> = rest.splitn(2, ':').collect();
        if parts.len() != 2 {
            anyhow::bail!("error-count format: error-count:MAX:PATTERN");
        }
        let max_count: usize = parts[0]
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid max count: {}", parts[0]))?;
        return Ok(Criterion::ErrorCount {
            pattern: parts[1].to_string(),
            max_count,
        });
    }

    if let Some(rest) = s.strip_prefix("repeated-output:") {
        let parts: Vec<&str> = rest.splitn(2, ':').collect();
        if parts.len() != 2 {
            anyhow::bail!("repeated-output format: repeated-output:MIN_REPEATS:WINDOW_SIZE");
        }
        let min_repeats: usize = parts[0]
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid min repeats: {}", parts[0]))?;
        let window_size: usize = parts[1]
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid window size: {}", parts[1]))?;
        return Ok(Criterion::RepeatedOutput {
            min_repeats,
            window_size,
        });
    }

    if let Some(rest) = s.strip_prefix("output-contains:") {
        let phrases: Vec<String> = rest.split(',').map(|s| s.trim().to_string()).collect();
        if phrases.is_empty() {
            anyhow::bail!("output-contains requires at least one phrase");
        }
        return Ok(Criterion::OutputContains { phrases });
    }

    if let Some(rest) = s.strip_prefix("no-progress:") {
        let iterations: u32 = rest
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid iteration count: {}", rest))?;
        return Ok(Criterion::NoProgress { iterations });
    }

    anyhow::bail!(
        "Unknown abort criterion: {}. Use: error-count:MAX:PATTERN, \
        repeated-output:MIN:WINDOW, output-contains:PHRASE1,PHRASE2, no-progress:ITERATIONS",
        s
    )
}

/// Add a new task
async fn add_task(
    goal: String,
    namespace: Option<String>,
    repo: Option<std::path::PathBuf>,
    max_iterations: u32,
    checkpoint_every: u32,
    approval: String,
    priority: u8,
    depends_on: Vec<String>,
    success_on: Vec<String>,
    abort_on: Vec<String>,
) -> Result<()> {
    let registry = get_registry()?;

    // Parse approval level
    let approval_level = parse_approval(&approval)?;

    // Parse dependencies
    let mut dependencies = Vec::new();
    for dep in depends_on {
        let task_id: TaskId = dep
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid task ID: {}", dep))?;
        dependencies.push(task_id);
    }

    // Parse success criteria
    let mut success_criteria = Vec::new();
    for criterion_str in &success_on {
        let criterion = parse_success_criterion(criterion_str)?;
        success_criteria.push(criterion);
    }

    // Parse abort criteria
    let mut abort_criteria = Vec::new();
    for criterion_str in &abort_on {
        let criterion = parse_abort_criterion(criterion_str)?;
        abort_criteria.push(criterion);
    }

    // Create task config
    let config = TaskConfig {
        max_iterations,
        checkpoint_every,
        approval: approval_level,
        success_criteria,
        abort_criteria,
    };

    // Create task
    let mut task = Task::new(&goal).with_config(config).with_priority(priority);

    if let Some(ns) = namespace {
        task = task.with_namespace(&ns);
    }

    if let Some(r) = repo {
        task = task.with_repository(r);
    }

    for dep in dependencies {
        task = task.with_dependency(dep);
    }

    let id = registry.add(task)?;

    println!("Task created successfully");
    println!("ID: {}", id);
    println!("Goal: {}", goal);
    println!("Max iterations: {}", max_iterations);
    println!("Approval: {}", approval);

    if !success_on.is_empty() {
        println!("Success criteria: {}", success_on.join(", "));
    }

    if !abort_on.is_empty() {
        println!("Abort criteria: {}", abort_on.join(", "));
    }

    Ok(())
}

/// List tasks
async fn list_tasks(status: Option<String>, namespace: Option<String>, json: bool) -> Result<()> {
    let registry = get_registry()?;

    let tasks = if let Some(status_str) = status {
        let status = parse_status(&status_str)?;
        registry.list_by_status(status)
    } else if let Some(ns) = namespace {
        registry.list_by_namespace(&ns)
    } else {
        registry.list()
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&tasks)?);
        return Ok(());
    }

    if tasks.is_empty() {
        println!("No tasks found");
        println!();
        println!("Add a task with: kage task add \"<goal>\"");
        return Ok(());
    }

    println!(
        "{:<26} {:<12} {:<10} {:<8} {}",
        "ID", "STATUS", "ITER", "PRIORITY", "GOAL"
    );
    println!("{}", "─".repeat(80));

    for task in tasks {
        let status_str = match task.status {
            TaskStatus::Pending => "\x1b[33mpending\x1b[0m".to_string(),
            TaskStatus::Running => "\x1b[32mrunning\x1b[0m".to_string(),
            TaskStatus::Paused => "\x1b[34mpaused\x1b[0m".to_string(),
            TaskStatus::Completed => "\x1b[90mcompleted\x1b[0m".to_string(),
            TaskStatus::Failed => "\x1b[31mfailed\x1b[0m".to_string(),
            TaskStatus::Cancelled => "\x1b[90mcancelled\x1b[0m".to_string(),
        };

        let iter_str = format!("{}/{}", task.iterations, task.config.max_iterations);

        let goal_display = if task.goal.len() > 30 {
            format!("{}...", &task.goal[..27])
        } else {
            task.goal.clone()
        };

        println!(
            "{:<26} {:<22} {:<10} {:<8} {}",
            task.id, status_str, iter_str, task.priority, goal_display
        );
    }

    Ok(())
}

/// Show task details
async fn show_task(id: String) -> Result<()> {
    let registry = get_registry()?;
    let checkpoint_store = get_checkpoint_store()?;

    let task_id: TaskId = id
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid task ID: {}", id))?;

    let task = registry
        .get(task_id)
        .ok_or_else(|| anyhow::anyhow!("Task {} not found", id))?;

    println!("Task Details");
    println!("{}", "─".repeat(50));
    println!("ID:           {}", task.id);
    println!("Goal:         {}", task.goal);

    let status_str = match task.status {
        TaskStatus::Pending => "Pending",
        TaskStatus::Running => "Running",
        TaskStatus::Paused => "Paused",
        TaskStatus::Completed => "Completed",
        TaskStatus::Failed => "Failed",
        TaskStatus::Cancelled => "Cancelled",
    };
    println!("Status:       {}", status_str);

    println!(
        "Iterations:   {}/{}",
        task.iterations, task.config.max_iterations
    );
    println!("Priority:     {}", task.priority);

    if let Some(ref ns) = task.namespace {
        println!("Namespace:    {}", ns);
    }

    if let Some(ref repo) = task.repository {
        println!("Repository:   {}", repo.display());
    }

    if let Some(ref agent) = task.agent {
        println!("Agent:        {}", agent);
    }

    let approval_str = match task.config.approval {
        ApprovalLevel::None => "None",
        ApprovalLevel::OnWrite => "On Write",
        ApprovalLevel::OnCommit => "On Commit",
        ApprovalLevel::Always => "Always",
    };
    println!("Approval:     {}", approval_str);

    if !task.depends_on.is_empty() {
        let deps: Vec<String> = task.depends_on.iter().map(|d| d.to_string()).collect();
        println!("Dependencies: {}", deps.join(", "));
    }

    if !task.config.success_criteria.is_empty() {
        println!();
        println!("Success Criteria");
        println!("{}", "─".repeat(50));
        for criterion in &task.config.success_criteria {
            println!("  - {:?}", criterion);
        }
    }

    if !task.config.abort_criteria.is_empty() {
        println!();
        println!("Abort Criteria");
        println!("{}", "─".repeat(50));
        for criterion in &task.config.abort_criteria {
            println!("  - {:?}", criterion);
        }
    }

    if let Some(ref guidance) = task.guidance {
        println!();
        println!("Guidance:");
        println!("  {}", guidance);
    }

    if let Some(ref error) = task.error {
        println!();
        println!("Error:");
        println!("  {}", error);
    }

    println!();
    println!("Timestamps");
    println!("{}", "─".repeat(50));
    println!("Created:      {}", task.created_at.format("%Y-%m-%d %H:%M:%S UTC"));

    if let Some(started) = task.started_at {
        println!("Started:      {}", started.format("%Y-%m-%d %H:%M:%S UTC"));
    }

    if let Some(completed) = task.completed_at {
        println!(
            "Completed:    {}",
            completed.format("%Y-%m-%d %H:%M:%S UTC")
        );
    }

    if let Some(duration) = task.duration() {
        let secs = duration.num_seconds();
        let mins = secs / 60;
        let hours = mins / 60;
        if hours > 0 {
            println!("Duration:     {}h {}m {}s", hours, mins % 60, secs % 60);
        } else if mins > 0 {
            println!("Duration:     {}m {}s", mins, secs % 60);
        } else {
            println!("Duration:     {}s", secs);
        }
    }

    // Show checkpoint info
    let checkpoint_info = checkpoint_store.info(task_id)?;
    if checkpoint_info.count > 0 {
        println!();
        println!("Checkpoints");
        println!("{}", "─".repeat(50));
        println!("Count:        {}", checkpoint_info.count);
        println!("Size:         {} bytes", checkpoint_info.size);
        if let Some(newest) = checkpoint_info.newest {
            println!("Latest:       {}", newest.format("%Y-%m-%d %H:%M:%S UTC"));
        }
    }

    Ok(())
}

/// Pause a task
async fn pause_task(id: String) -> Result<()> {
    let registry = get_registry()?;

    let task_id: TaskId = id
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid task ID: {}", id))?;

    registry.pause_task(task_id)?;

    println!("Task {} paused", id);
    println!("Resume with: kage task resume {} --extend-iterations 5", id);

    Ok(())
}

/// Resume a task
async fn resume_task(id: String, extend_iterations: u32, guidance: Option<String>) -> Result<()> {
    let registry = get_registry()?;

    let task_id: TaskId = id
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid task ID: {}", id))?;

    let task = registry
        .get(task_id)
        .ok_or_else(|| anyhow::anyhow!("Task {} not found", id))?;

    if task.status != TaskStatus::Paused && task.status != TaskStatus::Failed {
        anyhow::bail!(
            "Task {} is not paused or failed (current status: {:?})",
            id,
            task.status
        );
    }

    registry.resume_task(task_id, guidance.as_deref(), extend_iterations)?;

    println!("Task {} resumed", id);
    println!(
        "New iteration limit: {}",
        task.iterations + extend_iterations
    );

    if let Some(ref g) = guidance {
        println!("Guidance injected: {}", g);
    }

    Ok(())
}

/// Cancel a task
async fn cancel_task(id: String, force: bool) -> Result<()> {
    let registry = get_registry()?;

    let task_id: TaskId = id
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid task ID: {}", id))?;

    let task = registry
        .get(task_id)
        .ok_or_else(|| anyhow::anyhow!("Task {} not found", id))?;

    if !force {
        print!("Are you sure you want to cancel task '{}'? [y/N]: ", task.goal);
        io::stdout().flush()?;

        let mut response = String::new();
        io::stdin().read_line(&mut response)?;

        if !response.trim().eq_ignore_ascii_case("y") {
            println!("Cancelled");
            return Ok(());
        }
    }

    registry.cancel_task(task_id)?;

    println!("Task {} cancelled", id);

    Ok(())
}

/// Run checkpoint subcommand
async fn run_checkpoint_command(cmd: CheckpointCommands) -> Result<()> {
    match cmd {
        CheckpointCommands::List { task_id } => list_checkpoints(task_id).await,
        CheckpointCommands::Show {
            task_id,
            checkpoint_id,
        } => show_checkpoint(task_id, checkpoint_id).await,
        CheckpointCommands::Prune { task_id, keep } => prune_checkpoints(task_id, keep).await,
    }
}

/// List checkpoints for a task
async fn list_checkpoints(task_id_str: String) -> Result<()> {
    let checkpoint_store = get_checkpoint_store()?;

    let task_id: TaskId = task_id_str
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid task ID: {}", task_id_str))?;

    let checkpoints = checkpoint_store.list(task_id)?;

    if checkpoints.is_empty() {
        println!("No checkpoints for task {}", task_id);
        return Ok(());
    }

    println!("Checkpoints for task {}", task_id);
    println!("{}", "─".repeat(60));
    println!("{:<28} {:<10} {}", "ID", "ITERATION", "CREATED");
    println!("{}", "─".repeat(60));

    for checkpoint_id in checkpoints {
        let checkpoint = checkpoint_store.load(task_id, checkpoint_id)?;
        println!(
            "{:<28} {:<10} {}",
            checkpoint_id,
            checkpoint.iteration,
            checkpoint.created_at.format("%Y-%m-%d %H:%M:%S")
        );
    }

    Ok(())
}

/// Show checkpoint details
async fn show_checkpoint(task_id_str: String, checkpoint_id_str: Option<String>) -> Result<()> {
    let checkpoint_store = get_checkpoint_store()?;

    let task_id: TaskId = task_id_str
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid task ID: {}", task_id_str))?;

    let checkpoint = if let Some(cp_id_str) = checkpoint_id_str {
        let checkpoint_id = Ulid::from_string(&cp_id_str)
            .map_err(|_| anyhow::anyhow!("Invalid checkpoint ID: {}", cp_id_str))?;
        checkpoint_store.load(task_id, checkpoint_id)?
    } else {
        checkpoint_store
            .load_latest(task_id)?
            .ok_or_else(|| anyhow::anyhow!("No checkpoints for task {}", task_id))?
    };

    println!("Checkpoint Details");
    println!("{}", "─".repeat(50));
    println!("Checkpoint ID: {}", checkpoint.checkpoint_id);
    println!("Task ID:       {}", checkpoint.task_id);
    println!("Iteration:     {}", checkpoint.iteration);
    println!(
        "Created:       {}",
        checkpoint.created_at.format("%Y-%m-%d %H:%M:%S UTC")
    );

    println!();
    println!("Task State at Checkpoint");
    println!("{}", "─".repeat(50));
    println!("Goal:          {}", checkpoint.task_state.goal);
    println!("Status:        {:?}", checkpoint.task_state.status);
    println!(
        "Iterations:    {}/{}",
        checkpoint.task_state.iterations, checkpoint.task_state.config.max_iterations
    );

    if let Some(ref context) = checkpoint.context {
        println!();
        println!("Conversation Context");
        println!("{}", "─".repeat(50));
        // Print first 500 chars
        let display = if context.len() > 500 {
            format!("{}...", &context[..500])
        } else {
            context.clone()
        };
        println!("{}", display);
    }

    Ok(())
}

/// Prune old checkpoints
async fn prune_checkpoints(task_id_str: String, keep: usize) -> Result<()> {
    let checkpoint_store = get_checkpoint_store()?;

    let task_id: TaskId = task_id_str
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid task ID: {}", task_id_str))?;

    let deleted = checkpoint_store.prune(task_id, keep)?;

    println!("Pruned {} checkpoints, kept {}", deleted, keep);

    Ok(())
}
