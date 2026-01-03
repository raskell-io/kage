//! Daemon and supervisor implementation
//!
//! The daemon manages agent lifecycles, task scheduling, and IPC.

pub mod client;
pub mod core;
pub mod events;
pub mod protocol;
mod supervisor;

pub use client::DaemonClient;
pub use core::HandlerState;
pub use events::EventBus;
pub use supervisor::Supervisor;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

use crate::config::Config;
use crate::memory::MemorySystem;
use crate::task::{
    CheckpointStore, CriteriaAction, CriteriaContext, Task, TaskConfig, TaskRegistry,
    TaskScheduler, TaskStatus,
};

use protocol::{Request, Response, TaskInfo};

/// Daemon state
pub struct Daemon {
    config: Config,
    supervisor: Arc<RwLock<Supervisor>>,
    socket_path: PathBuf,
    started_at: Instant,
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
    /// Task scheduler
    scheduler: Arc<TaskScheduler>,
    /// Checkpoint store
    checkpoint_store: Arc<CheckpointStore>,
    /// Memory system for context sharing
    memory: Arc<MemorySystem>,
    /// Event bus for real-time streaming to UI clients
    event_bus: Arc<EventBus>,
}

impl Daemon {
    /// Create a new daemon instance
    pub fn new(config: Config) -> Result<Self> {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        let mut supervisor = Supervisor::new(config.clone());

        // Initialize subscription pool
        if let Err(e) = supervisor.init_subscription_pool() {
            tracing::warn!("Failed to initialize subscription pool: {}", e);
        }

        // Initialize task registry and scheduler
        let task_db_path = config.daemon.state_dir.join("tasks.redb");
        let task_registry = Arc::new(TaskRegistry::open(task_db_path)?);
        let scheduler = Arc::new(TaskScheduler::new(task_registry));
        tracing::info!("Task scheduler initialized");

        // Initialize checkpoint store
        let checkpoint_store = Arc::new(CheckpointStore::new(config.daemon.state_dir.clone())?);
        tracing::info!("Checkpoint store initialized");

        // Initialize memory system
        let memory = Arc::new(MemorySystem::new(config.daemon.state_dir.clone())?);
        tracing::info!("Memory system initialized");

        // Initialize event bus for real-time streaming
        let event_bus = Arc::new(EventBus::new());
        tracing::info!("Event bus initialized");

        Ok(Self {
            socket_path: config.daemon.socket_path.clone(),
            supervisor: Arc::new(RwLock::new(supervisor)),
            config,
            started_at: Instant::now(),
            shutdown_tx: Some(shutdown_tx),
            scheduler,
            checkpoint_store,
            memory,
            event_bus,
        })
    }

    /// Get the socket path
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    /// Run the daemon (blocking)
    pub async fn run(&mut self) -> Result<()> {
        // Ensure socket directory exists
        if let Some(parent) = self.socket_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Remove stale socket
        let _ = tokio::fs::remove_file(&self.socket_path).await;

        // Bind to Unix socket
        let listener = UnixListener::bind(&self.socket_path)?;
        tracing::info!("Daemon listening on {:?}", self.socket_path);

        // Get shutdown receiver
        let shutdown_tx = self.shutdown_tx.take().unwrap();
        let mut shutdown_rx = shutdown_tx.subscribe();

        // Start task orchestrator - spawns agents for pending tasks
        let orch_supervisor = Arc::clone(&self.supervisor);
        let orch_scheduler = Arc::clone(&self.scheduler);
        let orch_config = self.config.clone();
        let mut orch_shutdown_rx = shutdown_tx.subscribe();
        let orchestrator_handle = tokio::spawn(async move {
            tracing::info!("Task orchestrator started");
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(2)) => {
                        // Get pending tasks that are ready to run
                        let ready_tasks = orch_scheduler.registry().get_ready_tasks();

                        for task in ready_tasks.into_iter().take(orch_config.daemon.max_agents) {
                            // Check if we're at capacity
                            let sup = orch_supervisor.read().await;
                            let running_count = sup.list_all().await.iter()
                                .filter(|a| a.status == "running")
                                .count();
                            drop(sup);

                            if running_count >= orch_config.daemon.max_agents {
                                tracing::debug!("At agent capacity ({}/{}), waiting...",
                                    running_count, orch_config.daemon.max_agents);
                                break;
                            }

                            // Spawn agent for this task
                            let working_dir = task.repository.clone()
                                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

                            tracing::info!("Spawning agent for task '{}' ({})", task.goal, task.id);

                            let mut sup = orch_supervisor.write().await;
                            match sup.spawn(
                                working_dir,
                                task.namespace.clone(),
                                Some(task.goal.clone()),
                                None, // Use default model
                                Some(task.config.max_iterations),
                            ).await {
                                Ok(agent_id) => {
                                    // Assign task to agent
                                    if let Err(e) = orch_scheduler.registry().assign_to_agent(task.id, agent_id) {
                                        tracing::error!("Failed to assign task {} to agent {}: {}", task.id, agent_id, e);
                                    } else {
                                        tracing::info!("Task {} assigned to agent {}", task.id, agent_id);
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Failed to spawn agent for task {}: {}", task.id, e);
                                    // Mark task as failed
                                    let _ = orch_scheduler.registry().fail_task(task.id, Some(&e.to_string()));
                                }
                            }
                        }
                    }
                    _ = orch_shutdown_rx.recv() => {
                        tracing::info!("Task orchestrator shutting down");
                        break;
                    }
                }
            }
        });

        // Start health check loop - updates task status based on agent completion and criteria
        let health_supervisor = Arc::clone(&self.supervisor);
        let health_scheduler = Arc::clone(&self.scheduler);
        let health_memory = Arc::clone(&self.memory);
        let health_event_bus = Arc::clone(&self.event_bus);
        let retention_days = parse_duration_days(&self.config.memory.long_term_retention);
        let mut health_shutdown_rx = shutdown_tx.subscribe();
        let mut prune_counter: u32 = 0;
        let mut heartbeat_counter: u32 = 0;
        let health_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(tokio::time::Duration::from_secs(3)) => {
                        // Run health check on supervisor
                        let mut sup = health_supervisor.write().await;
                        sup.health_check().await;

                        // Publish heartbeat every 15 seconds (5 iterations)
                        heartbeat_counter += 1;
                        if heartbeat_counter >= 5 {
                            heartbeat_counter = 0;
                            health_event_bus.publish(protocol::DaemonEvent::Heartbeat {
                                timestamp: chrono::Utc::now().timestamp(),
                            });
                        }

                        // Extract and store memory events from agent output
                        let memory_events = sup.extract_memory_events();
                        for (entry, scope) in memory_events {
                            if let Err(e) = health_memory.store(entry, scope).await {
                                tracing::warn!("Failed to store memory event: {}", e);
                            }
                        }

                        // Periodic memory pruning (every ~1 hour = 1200 iterations at 3s interval)
                        prune_counter += 1;
                        if prune_counter >= 1200 {
                            prune_counter = 0;
                            match health_memory.longterm.prune(retention_days, false) {
                                Ok((count, bytes)) if count > 0 => {
                                    tracing::info!("Pruned {} old memory entries ({} bytes freed)", count, bytes);
                                }
                                Err(e) => {
                                    tracing::warn!("Memory prune failed: {}", e);
                                }
                                _ => {}
                            }
                        }

                        // Get all agents and update corresponding task statuses
                        let agents = sup.list_all().await;

                        for agent in &agents {
                            // Find tasks assigned to this agent
                            let tasks = health_scheduler.registry().get_by_agent(agent.id);

                            for task in tasks {
                                // Skip if task is already in a terminal state
                                if matches!(task.status, TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled) {
                                    continue;
                                }

                                // Check criteria if agent is still running
                                if agent.status == "running" && (!task.config.success_criteria.is_empty() || !task.config.abort_criteria.is_empty()) {
                                    // Build criteria context
                                    let output_lines = sup.get_output_lines(agent.id);
                                    let working_dir = sup.get_working_dir(agent.id);

                                    let ctx = CriteriaContext {
                                        output_lines,
                                        working_dir,
                                        modified_files: Vec::new(), // TODO: track modified files
                                        iteration: task.iterations,
                                        last_change_iteration: task.iterations, // TODO: track last change
                                    };

                                    // Evaluate criteria
                                    match task.evaluate_criteria(&ctx) {
                                        CriteriaAction::Complete { criterion, details } => {
                                            tracing::info!(
                                                "Task {} auto-completed: {} ({})",
                                                task.id, criterion, details
                                            );
                                            // Kill the agent since task is done
                                            if let Err(e) = sup.kill(agent.id, false).await {
                                                tracing::warn!("Failed to stop agent {}: {}", agent.id, e);
                                            }
                                            if let Err(e) = health_scheduler.registry().complete_task(task.id) {
                                                tracing::error!("Failed to complete task {}: {}", task.id, e);
                                            } else {
                                                // Publish task completed event
                                                health_event_bus.publish(protocol::DaemonEvent::TaskCompleted {
                                                    id: task.id,
                                                    success: true,
                                                    message: Some(format!("{}: {}", criterion, details)),
                                                });
                                            }
                                            continue;
                                        }
                                        CriteriaAction::Abort { criterion, details } => {
                                            tracing::warn!(
                                                "Task {} auto-aborted: {} ({})",
                                                task.id, criterion, details
                                            );
                                            // Kill the agent
                                            if let Err(e) = sup.kill(agent.id, false).await {
                                                tracing::warn!("Failed to stop agent {}: {}", agent.id, e);
                                            }
                                            let error_msg = format!("{}: {}", criterion, details);
                                            if let Err(e) = health_scheduler.registry().fail_task(task.id, Some(&error_msg)) {
                                                tracing::error!("Failed to fail task {}: {}", task.id, e);
                                            } else {
                                                // Publish task completed (failed) event
                                                health_event_bus.publish(protocol::DaemonEvent::TaskCompleted {
                                                    id: task.id,
                                                    success: false,
                                                    message: Some(error_msg),
                                                });
                                            }
                                            continue;
                                        }
                                        CriteriaAction::Continue => {
                                            // Keep running
                                        }
                                    }
                                }

                                // Update task status based on agent status
                                match agent.status.as_str() {
                                    "completed" => {
                                        if let Err(e) = health_scheduler.registry().complete_task(task.id) {
                                            tracing::error!("Failed to complete task {}: {}", task.id, e);
                                        } else {
                                            tracing::info!("Task {} completed (agent {} finished)", task.id, agent.id);
                                            // Publish task completed event
                                            health_event_bus.publish(protocol::DaemonEvent::TaskCompleted {
                                                id: task.id,
                                                success: true,
                                                message: None,
                                            });
                                        }
                                    }
                                    "failed" | "stopped" => {
                                        let error_msg = format!("Agent {} {}", agent.id, agent.status);
                                        if let Err(e) = health_scheduler.registry().fail_task(task.id, Some(&error_msg)) {
                                            tracing::error!("Failed to fail task {}: {}", task.id, e);
                                        } else {
                                            tracing::info!("Task {} failed (agent {} {})", task.id, agent.id, agent.status);
                                            // Publish task completed (failed) event
                                            health_event_bus.publish(protocol::DaemonEvent::TaskCompleted {
                                                id: task.id,
                                                success: false,
                                                message: Some(error_msg),
                                            });
                                        }
                                    }
                                    _ => {
                                        // Agent still running, criteria checked above
                                    }
                                }
                            }
                        }

                        drop(sup);
                    }
                    _ = health_shutdown_rx.recv() => {
                        break;
                    }
                }
            }
        });

        // Start gRPC server if configured
        #[cfg(feature = "server")]
        let grpc_handle = if let Some(grpc_addr) = self.config.daemon.grpc_listen {
            let state = Arc::new(core::HandlerState::new(
                Arc::clone(&self.supervisor),
                Arc::clone(&self.scheduler),
                Arc::clone(&self.memory),
                self.started_at,
            ));
            let grpc_server = crate::rpc::GrpcServer::new(grpc_addr, state);
            let grpc_shutdown_rx = shutdown_tx.subscribe();
            Some(tokio::spawn(async move {
                if let Err(e) = grpc_server.run(grpc_shutdown_rx).await {
                    tracing::error!("gRPC server error: {}", e);
                }
            }))
        } else {
            None
        };

        // Accept connections
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, _addr)) => {
                            let supervisor = Arc::clone(&self.supervisor);
                            let scheduler = Arc::clone(&self.scheduler);
                            let checkpoint_store = Arc::clone(&self.checkpoint_store);
                            let memory = Arc::clone(&self.memory);
                            let event_bus = Arc::clone(&self.event_bus);
                            let started_at = self.started_at;
                            let mut conn_shutdown_rx = shutdown_tx.subscribe();

                            tokio::spawn(async move {
                                tokio::select! {
                                    result = handle_connection(stream, supervisor, scheduler, checkpoint_store, memory, event_bus, started_at) => {
                                        if let Err(e) = result {
                                            tracing::debug!("Connection closed: {}", e);
                                        }
                                    }
                                    _ = conn_shutdown_rx.recv() => {
                                        tracing::debug!("Connection shutdown");
                                    }
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!("Accept error: {}", e);
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    tracing::info!("Received shutdown signal");
                    break;
                }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("Received SIGINT, shutting down");
                    break;
                }
            }
        }

        // Wait for background tasks to stop
        tracing::info!("Stopping background tasks...");
        let _ = orchestrator_handle.await;
        let _ = health_handle.await;
        #[cfg(feature = "server")]
        if let Some(handle) = grpc_handle {
            let _ = handle.await;
        }

        // Graceful shutdown: kill all agents
        tracing::info!("Stopping all agents...");
        {
            let mut supervisor = self.supervisor.write().await;
            supervisor.shutdown_all().await;
        }

        // Cleanup
        let _ = tokio::fs::remove_file(&self.socket_path).await;
        tracing::info!("Daemon stopped");
        Ok(())
    }
}

/// Handle a single client connection
async fn handle_connection(
    mut stream: UnixStream,
    supervisor: Arc<RwLock<Supervisor>>,
    scheduler: Arc<TaskScheduler>,
    checkpoint_store: Arc<CheckpointStore>,
    memory: Arc<MemorySystem>,
    event_bus: Arc<EventBus>,
    started_at: Instant,
) -> Result<()> {
    loop {
        // Read request length
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // Client disconnected
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }

        let len = u32::from_be_bytes(len_buf) as usize;

        // Sanity check
        if len > 10 * 1024 * 1024 {
            tracing::warn!("Request too large: {} bytes", len);
            let resp = Response::Error {
                message: "Request too large".into(),
            };
            send_response(&mut stream, &resp).await?;
            continue;
        }

        // Read request data
        let mut data = vec![0u8; len];
        stream.read_exact(&mut data).await?;

        // Decode request
        let request: Request = match protocol::decode_message(&data) {
            Ok(req) => req,
            Err(e) => {
                tracing::warn!("Failed to decode request: {}", e);
                let resp = Response::Error {
                    message: format!("Invalid request: {}", e),
                };
                send_response(&mut stream, &resp).await?;
                continue;
            }
        };

        tracing::debug!("Received request: {:?}", request);

        // Handle request
        let response = handle_request(
            request,
            &supervisor,
            &scheduler,
            &checkpoint_store,
            &memory,
            &event_bus,
            started_at,
            &mut stream,
        )
        .await;

        // Check if we handled streaming (attach) - in that case response is already sent
        if let Some(resp) = response {
            send_response(&mut stream, &resp).await?;
        }
    }
}

/// Send a response to the client
async fn send_response(stream: &mut UnixStream, response: &Response) -> Result<()> {
    let data = protocol::encode_message(response)?;
    stream.write_all(&data).await?;
    Ok(())
}

/// Handle a single request
async fn handle_request(
    request: Request,
    supervisor: &Arc<RwLock<Supervisor>>,
    scheduler: &Arc<TaskScheduler>,
    _checkpoint_store: &Arc<CheckpointStore>,
    _memory: &Arc<MemorySystem>,
    event_bus: &Arc<EventBus>,
    started_at: Instant,
    stream: &mut UnixStream,
) -> Option<Response> {
    match request {
        Request::Ping => Some(Response::Pong {
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs: started_at.elapsed().as_secs(),
        }),

        Request::Status => {
            let sup = supervisor.read().await;
            let agents = sup.list_all().await;
            let active = agents.iter().filter(|a| a.status == "running").count();

            let counts = scheduler.registry().counts();

            Some(Response::Status {
                version: env!("CARGO_PKG_VERSION").to_string(),
                uptime_secs: started_at.elapsed().as_secs(),
                active_agents: active,
                pending_tasks: counts.pending,
                running_tasks: counts.running,
            })
        }

        Request::SpawnAgent {
            working_dir,
            namespace,
            prompt,
            model,
            max_iterations,
        } => {
            let mut sup = supervisor.write().await;
            match sup
                .spawn(working_dir.clone(), namespace.clone(), prompt, model, max_iterations)
                .await
            {
                Ok(id) => {
                    // Publish event
                    if let Some(agent_info) = sup.get_info(id).await {
                        event_bus.publish(protocol::DaemonEvent::AgentSpawned { agent: agent_info });
                    }
                    Some(Response::AgentSpawned { id })
                }
                Err(e) => Some(Response::Error {
                    message: e.to_string(),
                }),
            }
        }

        Request::KillAgent { id, force } => {
            let mut sup = supervisor.write().await;
            match sup.kill(id, force).await {
                Ok(()) => {
                    // Publish event
                    let reason = if force { "force killed" } else { "killed" };
                    event_bus.publish(protocol::DaemonEvent::AgentStopped {
                        id,
                        reason: reason.to_string(),
                    });
                    Some(Response::Ok)
                }
                Err(e) => Some(Response::Error {
                    message: e.to_string(),
                }),
            }
        }

        Request::ListAgents {
            namespace,
            include_stopped,
        } => {
            let sup = supervisor.read().await;
            let agents = sup.list(namespace, include_stopped).await;
            Some(Response::AgentList { agents })
        }

        Request::GetAgent { id } => {
            let sup = supervisor.read().await;
            match sup.get_info(id).await {
                Some(agent) => Some(Response::AgentDetails { agent }),
                None => Some(Response::Error {
                    message: format!("Agent {} not found", id),
                }),
            }
        }

        Request::SendInput { id, input } => {
            let sup = supervisor.read().await;
            match sup.send_input(id, &input).await {
                Ok(()) => Some(Response::Ok),
                Err(e) => Some(Response::Error {
                    message: e.to_string(),
                }),
            }
        }

        Request::GetOutput { id, lines } => {
            let sup = supervisor.read().await;
            match sup.get_output(id, lines).await {
                Ok((output_lines, has_more)) => Some(Response::AgentOutput {
                    lines: output_lines,
                    has_more,
                }),
                Err(e) => Some(Response::Error {
                    message: e.to_string(),
                }),
            }
        }

        Request::Attach { id } => {
            // Stream output to client
            let sup = supervisor.read().await;

            // First verify agent exists
            if sup.get_info(id).await.is_none() {
                return Some(Response::Error {
                    message: format!("Agent {} not found", id),
                });
            }

            // Get output receiver
            match sup.attach(id).await {
                Ok(mut rx) => {
                    // Drop the read lock before entering the loop
                    drop(sup);

                    // Stream output lines
                    loop {
                        match rx.recv().await {
                            Ok(line) => {
                                let resp = Response::StreamLine {
                                    text: line.text,
                                    is_error: line.is_error,
                                    timestamp: line.timestamp,
                                };
                                if send_response(stream, &resp).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => {
                                // Channel closed or lagged
                                break;
                            }
                        }
                    }

                    let _ = send_response(stream, &Response::StreamEnd).await;
                    None // Response already sent
                }
                Err(e) => Some(Response::Error {
                    message: e.to_string(),
                }),
            }
        }

        Request::Detach { id: _ } => {
            // Detach is handled by closing the stream
            Some(Response::Ok)
        }

        Request::PauseAgent { id } => {
            let mut sup = supervisor.write().await;
            match sup.pause(id).await {
                Ok(()) => Some(Response::Ok),
                Err(e) => Some(Response::Error {
                    message: e.to_string(),
                }),
            }
        }

        Request::ResumeAgent { id } => {
            let mut sup = supervisor.write().await;
            match sup.resume(id).await {
                Ok(()) => Some(Response::Ok),
                Err(e) => Some(Response::Error {
                    message: e.to_string(),
                }),
            }
        }

        Request::AddTask {
            goal,
            namespace,
            repository,
            max_iterations,
        } => {
            let config = TaskConfig {
                max_iterations: max_iterations.unwrap_or(10),
                ..TaskConfig::default()
            };

            let mut task = Task::new(&goal).with_config(config);

            if let Some(ns) = namespace {
                task = task.with_namespace(&ns);
            }

            if let Some(repo) = repository {
                task = task.with_repository(repo);
            }

            match scheduler.registry().add(task.clone()) {
                Ok(id) => {
                    tracing::info!("Task {} added: {}", id, goal);

                    // Publish event
                    event_bus.publish(protocol::DaemonEvent::TaskAdded {
                        task: TaskInfo {
                            id,
                            goal: task.goal,
                            status: "pending".to_string(),
                            agent: None,
                            namespace: task.namespace,
                            iterations: 0,
                            max_iterations: task.config.max_iterations,
                            created_at: task.created_at.timestamp(),
                        },
                    });

                    Some(Response::TaskAdded { id })
                }
                Err(e) => Some(Response::Error {
                    message: e.to_string(),
                }),
            }
        }

        Request::ListTasks { status } => {
            let tasks = if let Some(status_str) = status {
                let task_status = match status_str.to_lowercase().as_str() {
                    "pending" => TaskStatus::Pending,
                    "running" => TaskStatus::Running,
                    "paused" => TaskStatus::Paused,
                    "completed" => TaskStatus::Completed,
                    "failed" => TaskStatus::Failed,
                    "cancelled" => TaskStatus::Cancelled,
                    _ => {
                        return Some(Response::Error {
                            message: format!("Invalid status: {}", status_str),
                        });
                    }
                };
                scheduler.registry().list_by_status(task_status)
            } else {
                scheduler.registry().list()
            };

            let task_infos: Vec<TaskInfo> = tasks
                .into_iter()
                .map(|t| TaskInfo {
                    id: t.id,
                    goal: t.goal,
                    status: format!("{:?}", t.status).to_lowercase(),
                    agent: t.agent,
                    namespace: t.namespace,
                    iterations: t.iterations,
                    max_iterations: t.config.max_iterations,
                    created_at: t.created_at.timestamp(),
                })
                .collect();

            Some(Response::TaskList { tasks: task_infos })
        }

        Request::CancelTask { id } => {
            match scheduler.registry().cancel_task(id) {
                Ok(()) => {
                    tracing::info!("Task {} cancelled", id);
                    Some(Response::Ok)
                }
                Err(e) => Some(Response::Error {
                    message: e.to_string(),
                }),
            }
        }

        Request::ListApprovals => {
            let sup = supervisor.read().await;
            let approvals = sup.list_approvals();
            Some(Response::ApprovalList { approvals })
        }

        Request::Approve { id } => {
            let mut sup = supervisor.write().await;
            match sup.approve(id) {
                Ok(agent_id) => {
                    // Resume the agent after approval
                    if let Err(e) = sup.resume(agent_id).await {
                        tracing::warn!("Failed to resume agent {} after approval: {}", agent_id, e);
                    }
                    tracing::info!("Approved action {} for agent {}", id, agent_id);

                    // Publish event
                    event_bus.publish(protocol::DaemonEvent::ApprovalResolved {
                        id,
                        approved: true,
                    });

                    Some(Response::Ok)
                }
                Err(e) => Some(Response::Error {
                    message: e.to_string(),
                }),
            }
        }

        Request::Reject { id, reason } => {
            let mut sup = supervisor.write().await;
            match sup.reject(id, reason) {
                Ok(agent_id) => {
                    tracing::info!("Rejected action {} for agent {}", id, agent_id);

                    // Publish event
                    event_bus.publish(protocol::DaemonEvent::ApprovalResolved {
                        id,
                        approved: false,
                    });

                    Some(Response::Ok)
                }
                Err(e) => Some(Response::Error {
                    message: e.to_string(),
                }),
            }
        }

        Request::Shutdown => {
            tracing::info!("Shutdown requested by client");
            // The daemon will handle the actual shutdown
            Some(Response::Ok)
        }

        Request::QueryMemory {
            text,
            scope,
            memory_type,
            tags,
            since,
            limit,
        } => {
            use crate::memory::MemoryQuery;

            let mut query = MemoryQuery::new();

            if let Some(t) = text {
                query = query.text(&t);
            }
            if let Some(s) = scope {
                // Parse scope string to MemoryScope
                if let Some(parsed_scope) = parse_scope_string(&s) {
                    query = query.scope(parsed_scope);
                }
            }
            if let Some(mt) = memory_type {
                query = query.memory_type(&mt);
            }
            for tag in tags {
                query = query.tag(&tag);
            }
            if let Some(s) = since {
                if let Some(dt) = chrono::DateTime::from_timestamp(s, 0) {
                    query = query.since(dt);
                }
            }
            if let Some(l) = limit {
                query = query.limit(l);
            }

            let entries = _memory.query(query).await;
            let total = entries.len();

            let infos: Vec<protocol::MemoryInfo> = entries
                .into_iter()
                .map(|e| memory_entry_to_info(&e))
                .collect();

            Some(Response::MemoryList {
                entries: infos,
                total,
            })
        }

        Request::GetMemory { id } => {
            // Query by ID
            use crate::memory::MemoryQuery;

            let query = MemoryQuery::new().text(&id).limit(1);
            let entries = _memory.query(query).await;

            if let Some(entry) = entries.first() {
                Some(Response::MemoryDetails {
                    entry: memory_entry_to_info(entry),
                })
            } else {
                Some(Response::Error {
                    message: format!("Memory entry {} not found", id),
                })
            }
        }

        Request::StoreMemory { entry, scope } => {
            match _memory.store(entry.clone(), scope).await {
                Ok(()) => Some(Response::MemoryStored {
                    id: entry.id.to_string(),
                }),
                Err(e) => Some(Response::Error {
                    message: e.to_string(),
                }),
            }
        }

        Request::PruneMemory {
            older_than_days,
            dry_run,
        } => {
            match _memory.longterm.prune(older_than_days, dry_run) {
                Ok((count, bytes_freed)) => Some(Response::MemoryPruned { count, bytes_freed }),
                Err(e) => Some(Response::Error {
                    message: e.to_string(),
                }),
            }
        }

        Request::Subscribe { event_types: _ } => {
            // Subscribe to daemon events
            let (_id, mut rx) = event_bus.subscribe();
            tracing::debug!("Client subscribed to event stream");

            // Send subscription confirmation
            if send_response(stream, &Response::Subscribed).await.is_err() {
                return None;
            }

            // Stream events to client
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let resp = Response::Event(event);
                        if send_response(stream, &resp).await.is_err() {
                            tracing::debug!("Event stream: client disconnected");
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("Event stream: client lagged {} events", n);
                        // Continue streaming
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::debug!("Event stream: channel closed");
                        break;
                    }
                }
            }

            None // Response already sent via stream
        }

        Request::Unsubscribe => {
            // Client can simply disconnect to unsubscribe
            Some(Response::Ok)
        }
    }
}

/// Parse a scope string like "global", "namespace:backend", "agent:xxx"
fn parse_scope_string(s: &str) -> Option<crate::memory::MemoryScope> {
    use crate::memory::MemoryScope;

    if s == "global" {
        return Some(MemoryScope::Global);
    }

    if let Some(ns) = s.strip_prefix("namespace:") {
        return Some(MemoryScope::Namespace(ns.to_string()));
    }

    if let Some(agent_str) = s.strip_prefix("agent:") {
        if let Ok(agent_id) = agent_str.parse() {
            return Some(MemoryScope::Agent(agent_id));
        }
    }

    None
}

/// Convert a MemoryEntry to MemoryInfo for wire format
fn memory_entry_to_info(entry: &crate::memory::MemoryEntry) -> protocol::MemoryInfo {
    use crate::memory::entry::MemoryContent;

    let (content_type, content_summary) = match &entry.content {
        MemoryContent::FileDiscovered { path, summary, .. } => {
            ("file_discovered".to_string(), format!("{}: {}", path.display(), summary))
        }
        MemoryContent::PatternLearned { pattern, confidence, .. } => {
            ("pattern_learned".to_string(), format!("{} (confidence: {:.0}%)", pattern, confidence * 100.0))
        }
        MemoryContent::DependencyMapped { from, to, relationship } => {
            ("dependency_mapped".to_string(), format!("{} {} {}", from, relationship, to))
        }
        MemoryContent::ErrorEncountered { error, worked, .. } => {
            let status = if *worked { "resolved" } else { "unresolved" };
            ("error_encountered".to_string(), format!("[{}] {}", status, error))
        }
        MemoryContent::DecisionMade { decision, .. } => {
            ("decision_made".to_string(), decision.clone())
        }
        MemoryContent::TaskCompleted { task_id, summary, .. } => {
            ("task_completed".to_string(), format!("{}: {}", task_id, summary))
        }
        MemoryContent::InsightShared { topic, content } => {
            ("insight_shared".to_string(), format!("{}: {}", topic, content))
        }
        MemoryContent::QuestionAsked { question, answer } => {
            let status = if answer.is_some() { "answered" } else { "unanswered" };
            ("question_asked".to_string(), format!("[{}] {}", status, question))
        }
    };

    // Serialize full content to JSON for storage
    let content_json = serde_json::to_string(&entry.content).unwrap_or_default();

    protocol::MemoryInfo {
        id: entry.id.to_string(),
        created_at: entry.created_at.timestamp(),
        created_by: entry.created_by.to_string(),
        content_type,
        content_summary,
        content: content_json,
        tags: entry.tags.clone(),
        scope: String::new(), // Will be set by caller if needed
    }
}

/// Parse a duration string (e.g., "90d", "1w", "2m") to days
fn parse_duration_days(s: &str) -> u32 {
    let s = s.trim().to_lowercase();

    if let Some(days) = s.strip_suffix('d') {
        return days.parse().unwrap_or(90);
    }
    if let Some(weeks) = s.strip_suffix('w') {
        return weeks.parse::<u32>().unwrap_or(13) * 7;
    }
    if let Some(months) = s.strip_suffix('m') {
        return months.parse::<u32>().unwrap_or(3) * 30;
    }

    // Try parsing as plain number (days)
    s.parse().unwrap_or(90)
}
