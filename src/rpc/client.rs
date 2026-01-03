//! gRPC client implementation

use anyhow::{Context, Result};
use tonic::transport::Channel;

use super::proto::kage_service_client::KageServiceClient;
use super::proto::{
    ListAgentsRequest, ListTasksRequest, PingRequest, ShutdownRequest, StatusRequest,
};

/// gRPC client for connecting to remote Kage daemon
pub struct GrpcClient {
    client: KageServiceClient<Channel>,
}

impl GrpcClient {
    /// Connect to a remote Kage daemon
    pub async fn connect(addr: &str) -> Result<Self> {
        // Ensure the address has a scheme
        let url = if addr.starts_with("http://") || addr.starts_with("https://") {
            addr.to_string()
        } else {
            format!("http://{}", addr)
        };

        let client = KageServiceClient::connect(url)
            .await
            .context("Failed to connect to Kage server")?;

        Ok(Self { client })
    }

    /// Ping the server to check connectivity
    pub async fn ping(&mut self) -> Result<(String, u64)> {
        let response = self
            .client
            .ping(PingRequest {})
            .await
            .context("Ping failed")?;

        let resp = response.into_inner();
        Ok((resp.version, resp.uptime_secs))
    }

    /// Get server status
    pub async fn status(&mut self) -> Result<ServerStatus> {
        let response = self
            .client
            .status(StatusRequest {})
            .await
            .context("Status request failed")?;

        let resp = response.into_inner();
        Ok(ServerStatus {
            version: resp.version,
            uptime_secs: resp.uptime_secs,
            active_agents: resp.active_agents,
            pending_tasks: resp.pending_tasks,
            running_tasks: resp.running_tasks,
        })
    }

    /// Request server shutdown
    pub async fn shutdown(&mut self) -> Result<()> {
        self.client
            .shutdown(ShutdownRequest {})
            .await
            .context("Shutdown request failed")?;

        Ok(())
    }

    /// List agents
    pub async fn list_agents(&mut self, namespace: Option<String>, include_stopped: bool) -> Result<Vec<AgentInfo>> {
        let response = self
            .client
            .list_agents(ListAgentsRequest {
                namespace,
                include_stopped,
            })
            .await
            .context("List agents failed")?;

        let resp = response.into_inner();
        Ok(resp
            .agents
            .into_iter()
            .map(|a| AgentInfo {
                id: a.id,
                namespace: a.namespace,
                status: a.status,
                working_dir: a.working_dir,
                prompt: a.prompt,
                iterations: a.iterations,
            })
            .collect())
    }

    /// List tasks
    pub async fn list_tasks(&mut self, status: Option<String>) -> Result<Vec<TaskInfo>> {
        let response = self
            .client
            .list_tasks(ListTasksRequest { status })
            .await
            .context("List tasks failed")?;

        let resp = response.into_inner();
        Ok(resp
            .tasks
            .into_iter()
            .map(|t| TaskInfo {
                id: t.id,
                goal: t.goal,
                status: t.status,
                agent_id: t.agent_id,
                namespace: t.namespace,
                iterations: t.iterations,
                max_iterations: t.max_iterations,
            })
            .collect())
    }
}

/// Server status information
#[derive(Debug)]
pub struct ServerStatus {
    pub version: String,
    pub uptime_secs: u64,
    pub active_agents: u32,
    pub pending_tasks: u32,
    pub running_tasks: u32,
}

/// Agent info from server
#[derive(Debug)]
pub struct AgentInfo {
    pub id: String,
    pub namespace: String,
    pub status: String,
    pub working_dir: String,
    pub prompt: String,
    pub iterations: u32,
}

/// Task info from server
#[derive(Debug)]
pub struct TaskInfo {
    pub id: String,
    pub goal: String,
    pub status: String,
    pub agent_id: Option<String>,
    pub namespace: Option<String>,
    pub iterations: u32,
    pub max_iterations: u32,
}
