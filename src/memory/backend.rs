//! Memory backend trait for pluggable storage
//!
//! Supports local filesystem (default), AWS S3, and Azure Blob Storage.

use anyhow::Result;
use async_trait::async_trait;

use super::entry::{MemoryEntry, MemoryId, MemoryScope};
use super::query::MemoryQuery;

/// Backend for long-term memory storage
///
/// Implementations must be thread-safe and async-compatible.
#[async_trait]
pub trait MemoryBackend: Send + Sync {
    /// Append an entry to storage
    async fn append(&self, entry: MemoryEntry, scope: MemoryScope) -> Result<()>;

    /// Query entries matching criteria
    async fn query(&self, query: &MemoryQuery) -> Result<Vec<MemoryEntry>>;

    /// Get a specific entry by ID
    async fn get(&self, id: MemoryId) -> Result<Option<MemoryEntry>>;

    /// Prune old entries
    /// Returns (count of entries deleted, bytes freed)
    async fn prune(&self, older_than_days: u32, dry_run: bool) -> Result<(usize, u64)>;

    /// Check if the backend is available and healthy
    async fn health_check(&self) -> Result<()>;

    /// Get the backend name for logging
    fn name(&self) -> &'static str;
}

/// Configuration for memory backend selection
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MemoryBackendConfig {
    /// Local filesystem storage (default)
    Filesystem {
        /// Directory for event logs (default: ~/.local/share/kage/state/events)
        #[serde(default)]
        log_dir: Option<std::path::PathBuf>,
    },

    /// AWS S3 storage
    #[cfg(feature = "storage-s3")]
    S3 {
        /// S3 bucket name
        bucket: String,
        /// AWS region (default: from AWS_REGION env var)
        #[serde(default)]
        region: Option<String>,
        /// Key prefix for all objects (default: "kage/memory")
        #[serde(default = "default_s3_prefix")]
        prefix: String,
    },

    /// Azure Blob Storage
    #[cfg(feature = "storage-azure")]
    AzureBlob {
        /// Storage account name
        account: String,
        /// Container name
        container: String,
        /// Blob prefix for all objects (default: "kage/memory")
        #[serde(default = "default_azure_prefix")]
        prefix: String,
    },
}

impl Default for MemoryBackendConfig {
    fn default() -> Self {
        Self::Filesystem { log_dir: None }
    }
}

#[cfg(feature = "storage-s3")]
fn default_s3_prefix() -> String {
    "kage/memory".to_string()
}

#[cfg(feature = "storage-azure")]
fn default_azure_prefix() -> String {
    "kage/memory".to_string()
}

/// Create a memory backend from configuration
pub async fn create_backend(
    config: &MemoryBackendConfig,
    state_dir: std::path::PathBuf,
) -> Result<Box<dyn MemoryBackend>> {
    match config {
        MemoryBackendConfig::Filesystem { log_dir } => {
            let dir = log_dir.clone().unwrap_or_else(|| state_dir.join("events"));
            let backend = super::backends::FilesystemBackend::new(dir)?;
            Ok(Box::new(backend))
        }

        #[cfg(feature = "storage-s3")]
        MemoryBackendConfig::S3 { bucket, region, prefix } => {
            let backend = super::backends::S3Backend::new(
                bucket.clone(),
                region.clone(),
                prefix.clone(),
            ).await?;
            Ok(Box::new(backend))
        }

        #[cfg(feature = "storage-azure")]
        MemoryBackendConfig::AzureBlob { account, container, prefix } => {
            let backend = super::backends::AzureBlobBackend::new(
                account.clone(),
                container.clone(),
                prefix.clone(),
            ).await?;
            Ok(Box::new(backend))
        }
    }
}
