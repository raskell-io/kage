//! Azure Blob Storage backend for memory storage
//!
//! Stores entries as MessagePack blobs, organized by date.
//! Blob name format: {prefix}/{date}/{entry_id}.msgpack

use anyhow::{Context, Result};
use async_trait::async_trait;

use azure_identity::DefaultAzureCredential;
use azure_storage_blobs::prelude::*;
use futures::StreamExt;

use crate::memory::backend::MemoryBackend;
use crate::memory::entry::{MemoryContent, MemoryEntry, MemoryId, MemoryScope};
use crate::memory::query::MemoryQuery;

/// Azure Blob Storage-based memory backend
pub struct AzureBlobBackend {
    container_client: ContainerClient,
    prefix: String,
}

impl AzureBlobBackend {
    /// Create a new Azure Blob backend
    ///
    /// Uses DefaultAzureCredential (env vars, managed identity, Azure CLI, etc.)
    pub async fn new(account: String, container: String, prefix: String) -> Result<Self> {
        let credential = DefaultAzureCredential::default();

        let storage_account_url = format!("https://{}.blob.core.windows.net", account);
        let blob_service_client = BlobServiceClient::new(
            storage_account_url,
            std::sync::Arc::new(credential),
        );

        let container_client = blob_service_client.container_client(&container);

        // Ensure container exists
        match container_client.create().await {
            Ok(_) => tracing::info!("Created Azure container: {}", container),
            Err(e) if e.to_string().contains("ContainerAlreadyExists") => {}
            Err(e) => return Err(e.into()),
        }

        Ok(Self {
            container_client,
            prefix,
        })
    }

    /// Get the blob name for an entry
    fn entry_blob_name(&self, entry: &MemoryEntry) -> String {
        let date = entry.created_at.date_naive();
        format!("{}/{}/{}.msgpack", self.prefix, date, entry.id)
    }

    /// Get the blob prefix for a date
    fn date_prefix(&self, date: chrono::NaiveDate) -> String {
        format!("{}/{}/", self.prefix, date)
    }

    /// List all dates with entries
    async fn list_dates(&self) -> Result<Vec<chrono::NaiveDate>> {
        let mut dates = std::collections::HashSet::new();

        let mut stream = self.container_client
            .list_blobs()
            .prefix(self.prefix.clone())
            .into_stream();

        while let Some(result) = stream.next().await {
            let response = result.context("Failed to list Azure blobs")?;

            for blob in response.blobs.blobs() {
                // Extract date from blob name like "kage/memory/2024-01-15/abc123.msgpack"
                let parts: Vec<&str> = blob.name.split('/').collect();
                if parts.len() >= 3 {
                    if let Ok(date) = chrono::NaiveDate::parse_from_str(parts[parts.len() - 2], "%Y-%m-%d") {
                        dates.insert(date);
                    }
                }
            }
        }

        let mut dates: Vec<_> = dates.into_iter().collect();
        dates.sort();
        Ok(dates)
    }

    /// List entries for a specific date
    async fn list_entries_for_date(&self, date: chrono::NaiveDate) -> Result<Vec<(MemoryEntry, MemoryScope)>> {
        let prefix = self.date_prefix(date);
        let mut entries = Vec::new();

        let mut stream = self.container_client
            .list_blobs()
            .prefix(prefix)
            .into_stream();

        while let Some(result) = stream.next().await {
            let response = result.context("Failed to list Azure blobs")?;

            for blob in response.blobs.blobs() {
                if let Ok(Some((entry, scope))) = self.get_entry_by_name(&blob.name).await {
                    entries.push((entry, scope));
                }
            }
        }

        Ok(entries)
    }

    /// Get an entry by its blob name
    async fn get_entry_by_name(&self, name: &str) -> Result<Option<(MemoryEntry, MemoryScope)>> {
        let blob_client = self.container_client.blob_client(name);

        let response = match blob_client.get_content().await {
            Ok(r) => r,
            Err(e) if e.to_string().contains("BlobNotFound") => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        let record: AzureRecord = rmp_serde::from_slice(&response)?;
        Ok(Some((record.entry, record.scope)))
    }
}

#[async_trait]
impl MemoryBackend for AzureBlobBackend {
    async fn append(&self, entry: MemoryEntry, scope: MemoryScope) -> Result<()> {
        let blob_name = self.entry_blob_name(&entry);
        let blob_client = self.container_client.blob_client(&blob_name);

        let record = AzureRecord {
            entry,
            scope,
            timestamp: chrono::Utc::now(),
        };

        let bytes = rmp_serde::to_vec(&record)?;

        blob_client
            .put_block_blob(bytes)
            .content_type("application/msgpack")
            .await
            .context("Failed to upload blob to Azure")?;

        Ok(())
    }

    async fn query(&self, query: &MemoryQuery) -> Result<Vec<MemoryEntry>> {
        let mut results = Vec::new();

        // Determine date range to search
        let dates = self.list_dates().await?;
        let start_date = query.since.map(|dt| dt.date_naive());
        let end_date = query.until.map(|dt| dt.date_naive());

        for date in dates {
            // Skip dates outside range
            if let Some(start) = start_date {
                if date < start {
                    continue;
                }
            }
            if let Some(end) = end_date {
                if date > end {
                    continue;
                }
            }

            // Fetch and filter entries from this date
            let entries = self.list_entries_for_date(date).await?;
            for (entry, scope) in entries {
                if matches_query(&entry, &scope, query) {
                    results.push(entry);
                }
            }

            // Early exit if we have enough results
            if let Some(limit) = query.limit {
                if results.len() >= limit {
                    results.truncate(limit);
                    break;
                }
            }
        }

        Ok(results)
    }

    async fn get(&self, id: MemoryId) -> Result<Option<MemoryEntry>> {
        // Search all dates for the entry
        let dates = self.list_dates().await?;

        for date in dates.into_iter().rev() {
            let blob_name = format!("{}/{}/{}.msgpack", self.prefix, date, id);
            if let Some((entry, _)) = self.get_entry_by_name(&blob_name).await? {
                return Ok(Some(entry));
            }
        }

        Ok(None)
    }

    async fn prune(&self, older_than_days: u32, dry_run: bool) -> Result<(usize, u64)> {
        let cutoff = chrono::Utc::now().date_naive()
            - chrono::Duration::days(older_than_days as i64);

        let mut count = 0;
        let mut bytes_freed = 0u64;

        let dates = self.list_dates().await?;

        for date in dates {
            if date < cutoff {
                let prefix = self.date_prefix(date);

                let mut stream = self.container_client
                    .list_blobs()
                    .prefix(prefix)
                    .into_stream();

                while let Some(result) = stream.next().await {
                    let response = result?;

                    for blob in response.blobs.blobs() {
                        bytes_freed += blob.properties.content_length as u64;
                        count += 1;

                        if !dry_run {
                            let blob_client = self.container_client.blob_client(&blob.name);
                            blob_client.delete().await?;
                        }
                    }
                }
            }
        }

        Ok((count, bytes_freed))
    }

    async fn health_check(&self) -> Result<()> {
        // Try to list blobs (will fail if container doesn't exist or no access)
        let mut stream = self.container_client
            .list_blobs()
            .prefix(self.prefix.clone())
            .max_results(std::num::NonZeroU32::new(1).unwrap())
            .into_stream();

        if let Some(result) = stream.next().await {
            result.context("Azure health check failed")?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "azure_blob"
    }
}

/// Check if an entry matches a query
fn matches_query(entry: &MemoryEntry, scope: &MemoryScope, query: &MemoryQuery) -> bool {
    // Filter by scope
    if let Some(ref query_scope) = query.scope {
        if scope != query_scope {
            return false;
        }
    }

    // Filter by time range
    if let Some(since) = query.since {
        if entry.created_at < since {
            return false;
        }
    }
    if let Some(until) = query.until {
        if entry.created_at > until {
            return false;
        }
    }

    // Filter by memory type
    if let Some(ref memory_type) = query.memory_type {
        let entry_type = content_type_name(&entry.content);
        if entry_type != memory_type {
            return false;
        }
    }

    // Filter by tags (all must match)
    for tag in &query.tags {
        if !entry.tags.contains(tag) {
            return false;
        }
    }

    // Filter by text
    if let Some(ref text) = query.text {
        let text_lower = text.to_lowercase();
        let content_str = format!("{:?}", entry.content).to_lowercase();
        let id_str = entry.id.to_string().to_lowercase();

        if !content_str.contains(&text_lower) && !id_str.contains(&text_lower) {
            return false;
        }
    }

    true
}

fn content_type_name(content: &MemoryContent) -> &'static str {
    match content {
        MemoryContent::FileDiscovered { .. } => "file_discovered",
        MemoryContent::PatternLearned { .. } => "pattern_learned",
        MemoryContent::DependencyMapped { .. } => "dependency_mapped",
        MemoryContent::ErrorEncountered { .. } => "error_encountered",
        MemoryContent::DecisionMade { .. } => "decision_made",
        MemoryContent::TaskCompleted { .. } => "task_completed",
        MemoryContent::InsightShared { .. } => "insight_shared",
        MemoryContent::QuestionAsked { .. } => "question_asked",
    }
}

/// A record stored in Azure Blob
#[derive(serde::Serialize, serde::Deserialize)]
struct AzureRecord {
    entry: MemoryEntry,
    scope: MemoryScope,
    timestamp: chrono::DateTime<chrono::Utc>,
}
