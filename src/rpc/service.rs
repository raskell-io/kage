//! gRPC service implementation

use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tonic::{Request, Response, Status};

use crate::daemon::core::{self, HandlerState};
use crate::task::ApprovalId;

use super::proto;
use super::proto::kage_service_server::KageService;

/// gRPC service implementation
pub struct KageServiceImpl {
    state: Arc<HandlerState>,
}

impl KageServiceImpl {
    /// Create a new service instance with shared handler state
    pub fn new(state: Arc<HandlerState>) -> Self {
        Self { state }
    }
}

/// Convert anyhow error to tonic Status
fn to_status(e: anyhow::Error) -> Status {
    Status::internal(e.to_string())
}

#[tonic::async_trait]
impl KageService for KageServiceImpl {
    // ========================================================================
    // System RPCs
    // ========================================================================

    async fn ping(
        &self,
        _request: Request<proto::PingRequest>,
    ) -> Result<Response<proto::PingResponse>, Status> {
        let (version, uptime_secs) = core::handle_ping(&self.state);
        Ok(Response::new(proto::PingResponse {
            version,
            uptime_secs,
        }))
    }

    async fn status(
        &self,
        _request: Request<proto::StatusRequest>,
    ) -> Result<Response<proto::StatusResponse>, Status> {
        let (version, uptime_secs, active_agents, pending_tasks, running_tasks) =
            core::handle_status(&self.state).await;
        Ok(Response::new(proto::StatusResponse {
            version,
            uptime_secs,
            active_agents: active_agents as u32,
            pending_tasks: pending_tasks as u32,
            running_tasks: running_tasks as u32,
        }))
    }

    async fn shutdown(
        &self,
        _request: Request<proto::ShutdownRequest>,
    ) -> Result<Response<proto::ShutdownResponse>, Status> {
        tracing::info!("Shutdown requested via gRPC");
        // The actual shutdown is handled by the daemon
        Ok(Response::new(proto::ShutdownResponse {}))
    }

    // ========================================================================
    // Agent RPCs
    // ========================================================================

    async fn spawn_agent(
        &self,
        request: Request<proto::SpawnAgentRequest>,
    ) -> Result<Response<proto::SpawnAgentResponse>, Status> {
        let req = request.into_inner();
        let working_dir = PathBuf::from(&req.working_dir);

        let id = core::handle_spawn_agent(
            &self.state,
            working_dir,
            req.namespace,
            req.prompt,
            req.model,
            req.max_iterations,
        )
        .await
        .map_err(to_status)?;

        Ok(Response::new(proto::SpawnAgentResponse {
            agent_id: id.to_string(),
        }))
    }

    async fn kill_agent(
        &self,
        request: Request<proto::KillAgentRequest>,
    ) -> Result<Response<proto::KillAgentResponse>, Status> {
        let req = request.into_inner();
        let id = req
            .agent_id
            .parse()
            .map_err(|_| Status::invalid_argument("Invalid agent ID"))?;

        core::handle_kill_agent(&self.state, id, req.force)
            .await
            .map_err(to_status)?;

        Ok(Response::new(proto::KillAgentResponse {}))
    }

    async fn list_agents(
        &self,
        request: Request<proto::ListAgentsRequest>,
    ) -> Result<Response<proto::ListAgentsResponse>, Status> {
        let req = request.into_inner();
        let agents = core::handle_list_agents(&self.state, req.namespace, req.include_stopped).await;

        let proto_agents: Vec<proto::AgentInfo> = agents
            .into_iter()
            .map(|a| proto::AgentInfo {
                id: a.id.to_string(),
                namespace: a.namespace.unwrap_or_default(),
                status: a.status,
                working_dir: a.working_dir.to_string_lossy().to_string(),
                prompt: a.name.clone(), // Use name as prompt for display
                iterations: a.iteration,
                started_at: Some(super::convert::to_timestamp_from_secs(a.started_at)),
            })
            .collect();

        Ok(Response::new(proto::ListAgentsResponse {
            agents: proto_agents,
        }))
    }

    async fn get_agent(
        &self,
        request: Request<proto::GetAgentRequest>,
    ) -> Result<Response<proto::GetAgentResponse>, Status> {
        let req = request.into_inner();
        let id = req
            .agent_id
            .parse()
            .map_err(|_| Status::invalid_argument("Invalid agent ID"))?;

        let agent = core::handle_get_agent(&self.state, id)
            .await
            .ok_or_else(|| Status::not_found("Agent not found"))?;

        Ok(Response::new(proto::GetAgentResponse {
            agent: Some(proto::AgentInfo {
                id: agent.id.to_string(),
                namespace: agent.namespace.unwrap_or_default(),
                status: agent.status,
                working_dir: agent.working_dir.to_string_lossy().to_string(),
                prompt: agent.name.clone(), // Use name as prompt for display
                iterations: agent.iteration,
                started_at: Some(super::convert::to_timestamp_from_secs(agent.started_at)),
            }),
        }))
    }

    async fn pause_agent(
        &self,
        request: Request<proto::PauseAgentRequest>,
    ) -> Result<Response<proto::PauseAgentResponse>, Status> {
        let req = request.into_inner();
        let id = req
            .agent_id
            .parse()
            .map_err(|_| Status::invalid_argument("Invalid agent ID"))?;

        core::handle_pause_agent(&self.state, id)
            .await
            .map_err(to_status)?;

        Ok(Response::new(proto::PauseAgentResponse {}))
    }

    async fn resume_agent(
        &self,
        request: Request<proto::ResumeAgentRequest>,
    ) -> Result<Response<proto::ResumeAgentResponse>, Status> {
        let req = request.into_inner();
        let id = req
            .agent_id
            .parse()
            .map_err(|_| Status::invalid_argument("Invalid agent ID"))?;

        core::handle_resume_agent(&self.state, id)
            .await
            .map_err(to_status)?;

        Ok(Response::new(proto::ResumeAgentResponse {}))
    }

    // ========================================================================
    // Agent I/O RPCs
    // ========================================================================

    async fn send_input(
        &self,
        request: Request<proto::SendInputRequest>,
    ) -> Result<Response<proto::SendInputResponse>, Status> {
        let req = request.into_inner();
        let id = req
            .agent_id
            .parse()
            .map_err(|_| Status::invalid_argument("Invalid agent ID"))?;

        core::handle_send_input(&self.state, id, &req.input)
            .await
            .map_err(to_status)?;

        Ok(Response::new(proto::SendInputResponse {}))
    }

    async fn get_output(
        &self,
        request: Request<proto::GetOutputRequest>,
    ) -> Result<Response<proto::GetOutputResponse>, Status> {
        let req = request.into_inner();
        let id = req
            .agent_id
            .parse()
            .map_err(|_| Status::invalid_argument("Invalid agent ID"))?;

        let (output_lines, has_more) = core::handle_get_output(&self.state, id, req.lines as usize)
            .await
            .map_err(to_status)?;

        let proto_lines: Vec<proto::OutputLine> = output_lines
            .into_iter()
            .map(|l| proto::OutputLine {
                text: l.text,
                is_error: l.is_error,
                timestamp: Some(super::convert::to_timestamp_from_secs(l.timestamp)),
            })
            .collect();

        Ok(Response::new(proto::GetOutputResponse {
            lines: proto_lines,
            has_more,
        }))
    }

    type AttachStream = std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<proto::OutputLine, Status>> + Send>,
    >;

    async fn attach(
        &self,
        request: Request<proto::AttachRequest>,
    ) -> Result<Response<Self::AttachStream>, Status> {
        let req = request.into_inner();
        let id = req
            .agent_id
            .parse()
            .map_err(|_| Status::invalid_argument("Invalid agent ID"))?;

        let rx = core::handle_attach(&self.state, id)
            .await
            .map_err(to_status)?;

        // Convert broadcast receiver to gRPC stream
        let stream = BroadcastStream::new(rx).filter_map(|result| async move {
            match result {
                Ok(line) => Some(Ok(proto::OutputLine {
                    text: line.text,
                    is_error: line.is_error,
                    timestamp: Some(super::convert::to_timestamp_from_secs(line.timestamp)),
                })),
                Err(_) => None, // Channel lagged, skip
            }
        });

        Ok(Response::new(Box::pin(stream)))
    }

    async fn detach(
        &self,
        _request: Request<proto::DetachRequest>,
    ) -> Result<Response<proto::DetachResponse>, Status> {
        // Detach is handled by closing the stream
        Ok(Response::new(proto::DetachResponse {}))
    }

    // ========================================================================
    // Task RPCs
    // ========================================================================

    async fn add_task(
        &self,
        request: Request<proto::AddTaskRequest>,
    ) -> Result<Response<proto::AddTaskResponse>, Status> {
        let req = request.into_inner();

        let id = core::handle_add_task(
            &self.state,
            req.goal,
            req.namespace,
            req.repository.map(PathBuf::from),
            req.max_iterations,
        )
        .map_err(to_status)?;

        Ok(Response::new(proto::AddTaskResponse {
            task_id: id.to_string(),
        }))
    }

    async fn list_tasks(
        &self,
        request: Request<proto::ListTasksRequest>,
    ) -> Result<Response<proto::ListTasksResponse>, Status> {
        let req = request.into_inner();

        let tasks = core::handle_list_tasks(&self.state, req.status).map_err(to_status)?;

        let proto_tasks: Vec<proto::TaskInfo> = tasks
            .into_iter()
            .map(|t| proto::TaskInfo {
                id: t.id.to_string(),
                goal: t.goal,
                status: t.status,
                agent_id: t.agent.map(|a| a.to_string()),
                namespace: t.namespace,
                iterations: t.iterations,
                max_iterations: t.max_iterations,
                created_at: Some(super::convert::to_timestamp_from_secs(t.created_at)),
            })
            .collect();

        Ok(Response::new(proto::ListTasksResponse { tasks: proto_tasks }))
    }

    async fn cancel_task(
        &self,
        request: Request<proto::CancelTaskRequest>,
    ) -> Result<Response<proto::CancelTaskResponse>, Status> {
        let req = request.into_inner();
        let id = req
            .task_id
            .parse()
            .map_err(|_| Status::invalid_argument("Invalid task ID"))?;

        core::handle_cancel_task(&self.state, id).map_err(to_status)?;

        Ok(Response::new(proto::CancelTaskResponse {}))
    }

    // ========================================================================
    // Approval RPCs
    // ========================================================================

    async fn list_approvals(
        &self,
        _request: Request<proto::ListApprovalsRequest>,
    ) -> Result<Response<proto::ListApprovalsResponse>, Status> {
        let approvals = core::handle_list_approvals(&self.state).await;

        let proto_approvals: Vec<proto::ApprovalInfo> = approvals
            .into_iter()
            .map(|a| proto::ApprovalInfo {
                id: a.id.to_string(),
                agent_id: a.agent_id.to_string(),
                task_id: a.task_id.map(|t| t.to_string()),
                action: None, // TODO: Convert ApprovalAction to proto
                summary: a.summary,
                created_at: Some(super::convert::to_timestamp_from_secs(a.created_at)),
                context: a.context,
            })
            .collect();

        Ok(Response::new(proto::ListApprovalsResponse {
            approvals: proto_approvals,
        }))
    }

    async fn approve(
        &self,
        request: Request<proto::ApproveRequest>,
    ) -> Result<Response<proto::ApproveResponse>, Status> {
        let req = request.into_inner();
        let id: ApprovalId = req
            .approval_id
            .parse()
            .map_err(|_| Status::invalid_argument("Invalid approval ID"))?;

        core::handle_approve(&self.state, id)
            .await
            .map_err(to_status)?;

        Ok(Response::new(proto::ApproveResponse {}))
    }

    async fn reject(
        &self,
        request: Request<proto::RejectRequest>,
    ) -> Result<Response<proto::RejectResponse>, Status> {
        let req = request.into_inner();
        let id: ApprovalId = req
            .approval_id
            .parse()
            .map_err(|_| Status::invalid_argument("Invalid approval ID"))?;

        core::handle_reject(&self.state, id, req.reason)
            .await
            .map_err(to_status)?;

        Ok(Response::new(proto::RejectResponse {}))
    }

    // ========================================================================
    // Memory RPCs
    // ========================================================================

    async fn query_memory(
        &self,
        request: Request<proto::QueryMemoryRequest>,
    ) -> Result<Response<proto::QueryMemoryResponse>, Status> {
        let req = request.into_inner();

        let (entries, total) = core::handle_query_memory(
            &self.state,
            req.text,
            req.scope,
            req.memory_type,
            req.tags,
            req.since,
            req.limit.map(|l| l as usize),
        )
        .await;

        let proto_entries: Vec<proto::MemoryInfo> = entries
            .into_iter()
            .map(|e| proto::MemoryInfo {
                id: e.id,
                created_at: Some(super::convert::to_timestamp_from_secs(e.created_at)),
                created_by: e.created_by,
                content_type: e.content_type,
                content_summary: e.content_summary,
                content: e.content,
                tags: e.tags,
                scope: e.scope,
            })
            .collect();

        Ok(Response::new(proto::QueryMemoryResponse {
            entries: proto_entries,
            total: total as u32,
        }))
    }

    async fn get_memory(
        &self,
        request: Request<proto::GetMemoryRequest>,
    ) -> Result<Response<proto::GetMemoryResponse>, Status> {
        let req = request.into_inner();

        let entry = core::handle_get_memory(&self.state, req.id)
            .await
            .map_err(to_status)?;

        Ok(Response::new(proto::GetMemoryResponse {
            entry: Some(proto::MemoryInfo {
                id: entry.id,
                created_at: Some(super::convert::to_timestamp_from_secs(entry.created_at)),
                created_by: entry.created_by,
                content_type: entry.content_type,
                content_summary: entry.content_summary,
                content: entry.content,
                tags: entry.tags,
                scope: entry.scope,
            }),
        }))
    }

    async fn store_memory(
        &self,
        _request: Request<proto::StoreMemoryRequest>,
    ) -> Result<Response<proto::StoreMemoryResponse>, Status> {
        // Store memory requires deserializing the proto MemoryEntry
        // This is more complex and will be implemented later
        Err(Status::unimplemented(
            "StoreMemory via gRPC not yet implemented",
        ))
    }

    async fn prune_memory(
        &self,
        request: Request<proto::PruneMemoryRequest>,
    ) -> Result<Response<proto::PruneMemoryResponse>, Status> {
        let req = request.into_inner();

        let (count, bytes_freed) =
            core::handle_prune_memory(&self.state, req.older_than_days, req.dry_run)
                .map_err(to_status)?;

        Ok(Response::new(proto::PruneMemoryResponse {
            count: count as u32,
            bytes_freed,
        }))
    }
}
