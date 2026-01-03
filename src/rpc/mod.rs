//! gRPC server and client implementation
//!
//! This module provides gRPC transport for remote daemon communication.
//! Only compiled when the `server` feature is enabled.

mod client;
mod convert;
mod server;
mod service;

pub use client::GrpcClient;
pub use server::GrpcServer;
pub use service::KageServiceImpl;

/// Generated protobuf types
pub mod proto {
    tonic::include_proto!("kage.v1");
}
