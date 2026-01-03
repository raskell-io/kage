//! Daemon and supervisor implementation
//!
//! The daemon manages agent lifecycles, task scheduling, and IPC.

pub mod client;
pub mod protocol;
mod supervisor;

pub use client::DaemonClient;
pub use supervisor::Supervisor;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;

use crate::config::Config;

use protocol::{Request, Response};

/// Daemon state
pub struct Daemon {
    config: Config,
    supervisor: Arc<RwLock<Supervisor>>,
    socket_path: PathBuf,
    started_at: Instant,
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
}

impl Daemon {
    /// Create a new daemon instance
    pub fn new(config: Config) -> Self {
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);
        Self {
            socket_path: config.daemon.socket_path.clone(),
            supervisor: Arc::new(RwLock::new(Supervisor::new(config.clone()))),
            config,
            started_at: Instant::now(),
            shutdown_tx: Some(shutdown_tx),
        }
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

        // Accept connections
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((stream, _addr)) => {
                            let supervisor = Arc::clone(&self.supervisor);
                            let started_at = self.started_at;
                            let mut conn_shutdown_rx = shutdown_tx.subscribe();

                            tokio::spawn(async move {
                                tokio::select! {
                                    result = handle_connection(stream, supervisor, started_at) => {
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
        let response = handle_request(request, &supervisor, started_at, &mut stream).await;

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

            Some(Response::Status {
                version: env!("CARGO_PKG_VERSION").to_string(),
                uptime_secs: started_at.elapsed().as_secs(),
                active_agents: active,
                pending_tasks: 0, // TODO: implement task tracking
                running_tasks: 0,
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
                .spawn(working_dir, namespace, prompt, model, max_iterations)
                .await
            {
                Ok(id) => Some(Response::AgentSpawned { id }),
                Err(e) => Some(Response::Error {
                    message: e.to_string(),
                }),
            }
        }

        Request::KillAgent { id, force } => {
            let mut sup = supervisor.write().await;
            match sup.kill(id, force).await {
                Ok(()) => Some(Response::Ok),
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

        Request::AddTask { .. } => {
            // TODO: Implement task queue
            Some(Response::Error {
                message: "Task queue not yet implemented".into(),
            })
        }

        Request::ListTasks { .. } => {
            // TODO: Implement task queue
            Some(Response::TaskList { tasks: vec![] })
        }

        Request::CancelTask { .. } => {
            // TODO: Implement task queue
            Some(Response::Error {
                message: "Task queue not yet implemented".into(),
            })
        }

        Request::Shutdown => {
            tracing::info!("Shutdown requested by client");
            // The daemon will handle the actual shutdown
            Some(Response::Ok)
        }
    }
}
