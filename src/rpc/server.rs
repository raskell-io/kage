//! gRPC server implementation

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::broadcast;
use tonic::transport::Server;

use crate::daemon::HandlerState;

use super::proto::kage_service_server::KageServiceServer;
use super::service::KageServiceImpl;

/// gRPC server wrapper
pub struct GrpcServer {
    addr: SocketAddr,
    state: Arc<HandlerState>,
}

impl GrpcServer {
    /// Create a new gRPC server
    pub fn new(addr: SocketAddr, state: Arc<HandlerState>) -> Self {
        Self { addr, state }
    }

    /// Run the gRPC server until shutdown signal
    pub async fn run(self, mut shutdown: broadcast::Receiver<()>) -> Result<()> {
        let service = KageServiceImpl::new(self.state);
        let svc = KageServiceServer::new(service);

        tracing::info!("gRPC server listening on {}", self.addr);

        Server::builder()
            .add_service(svc)
            .serve_with_shutdown(self.addr, async move {
                let _ = shutdown.recv().await;
                tracing::info!("gRPC server shutting down");
            })
            .await?;

        Ok(())
    }
}
