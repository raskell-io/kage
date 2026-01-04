//! Google Cloud Storage backend for memory storage
//!
//! Stores entries as MessagePack objects in GCS, organized by date.
//! Object name format: {prefix}/{date}/{entry_id}.msgpack

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::PathBuf;

use google_cloud_storage::client::{Client, ClientConfig};
use google_cloud_storage::http::objects::delete::DeleteObjectRequest;
use google_cloud_storage::http::objects::download::Range;
use google_cloud_storage::http::objects::get::GetObjectRequest;
use google_cloud_storage::http::objects::list::ListObjectsRequest;
use google_cloud_storage::http::objects::upload::{Media, UploadObjectRequest, UploadType};

use crate::memory::backend::MemoryBackend;
use crate::memory::entry::{MemoryContent, MemoryEntry, MemoryId, MemoryScope};
use crate::memory::query::MemoryQuery;

/// Google Cloud Storage-based memory backend
pub struct GcsBackend {
    client: Client,
    bucket: String,
    prefix: String,
}

impl GcsBackend {
    /// Create a new GCS backend
    ///
    /// Uses Application Default Credentials by default.
    /// Set GOOGLE_APPLICATION_CREDENTIALS env var or pass credentials_file.
    pub async fn new(
        bucket: String,
        prefix: String,
        credentials_file: Option<PathBuf>,
    ) -> Result<Self> {
        // Set credentials file if provided
        if let Some(creds_path) = credentials_file {
            std::env::set_var("GOOGLE_APPLICATION_CREDENTIALS", creds_path);
        }

        let config = ClientConfig::default()
            .with_auth()
            .await
            .context("Failed to configure GCS authentication")?;

        let client = Client::new(config);

        Ok(Self {
            client,
            bucket,
            prefix,
        })
    }

    /// Get the GCS object name for an entry
    fn entry_object_name(&self, entry: &MemoryEntry) -> String {
        let date = entry.created_at.date_naive();
        format!("{}/{}/{}.msgpack", self.prefix, date, entry.id)
    }

    /// Get the object prefix for a date
    fn date_prefix(&self, date: chrono::NaiveDate) -> String {
        format!("{}/{}/", self.prefix, date)
    }

    /// List all dates with entries
    async fn list_dates(&self) -> Result<Vec<chrono::NaiveDate>> {
        let mut dates = std::collections::HashSet::new();
        let mut page_token: Option<String> = None;

        loop {
            let request = ListObjectsRequest {
                bucket: self.bucket.clone(),
                prefix: Some(self.prefix.clone()),
                page_token: page_token.clone(),
                ..Default::default()
            };

            let response = self.client
                .list_objects(&request)
                .await
                .context("Failed to list GCS objects")?;

            if let Some(items) = response.items {
                for object in items {
                    // Extract date from object name like "kage/memory/2024-01-15/abc123.msgpack"
                    let parts: Vec<&str> = object.name.split('/').collect();
                    if parts.len() >= 3 {
                        if let Ok(date) = chrono::NaiveDate::parse_from_str(
                            parts[parts.len() - 2],
                            "%Y-%m-%d",
                        ) {
                            dates.insert(date);
                        }
                    }
                }
            }

            page_token = response.next_page_token;
            if page_token.is_none() {
                break;
            }
        }

        let mut dates: Vec<_> = dates.into_iter().collect();
        dates.sort();
        Ok(dates)
    }

    /// List entries for a specific date
    async fn list_entries_for_date(
        &self,
        date: chrono::NaiveDate,
    ) -> Result<Vec<(MemoryEntry, MemoryScope)>> {
        let prefix = self.date_prefix(date);
        let mut entries = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let request = ListObjectsRequest {
                bucket: self.bucket.clone(),
                prefix: Some(prefix.clone()),
                page_token: page_token.clone(),
                ..Default::default()
            };

            let response = self.client
                .list_objects(&request)
                .await
                .context("Failed to list GCS objects")?;

            if let Some(items) = response.items {
                for object in items {
                    if let Ok(Some((entry, scope))) = self.get_entry_by_name(&object.name).await {
                        entries.push((entry, scope));
                    }
                }
            }

            page_token = response.next_page_token;
            if page_token.is_none() {
                break;
            }
        }

        Ok(entries)
    }

    /// Get an entry by its GCS object name
    async fn get_entry_by_name(&self, name: &str) -> Result<Option<(MemoryEntry, MemoryScope)>> {
        let request = GetObjectRequest {
            bucket: self.bucket.clone(),
            object: name.to_string(),
            ..Default::default()
        };

        let bytes = match self.client.download_object(&request, &Range::default()).await {
            Ok(bytes) => bytes,
            Err(e) => {
                if e.to_string().contains("404") || e.to_string().contains("Not Found") {
                    return Ok(None);
                }
                return Err(e.into());
            }
        };

        let record: GcsRecord = rmp_serde::from_slice(&bytes)?;
        Ok(Some((record.entry, record.scope)))
    }
}

#[async_trait]
impl MemoryBackend for GcsBackend {
    async fn append(&self, entry: MemoryEntry, scope: MemoryScope) -> Result<()> {
        let object_name = self.entry_object_name(&entry);

        let record = GcsRecord {
            entry,
            scope,
            timestamp: chrono::Utc::now(),
        };

        let bytes = rmp_serde::to_vec(&record)?;

        let upload_type = UploadType::Simple(Media::new(object_name));
        let request = UploadObjectRequest {
            bucket: self.bucket.clone(),
            ..Default::default()
        };

        self.client
            .upload_object(&request, bytes, &upload_type)
            .await
            .context("Failed to upload object to GCS")?;

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
        // We need to search since we don't know the date
        let dates = self.list_dates().await?;

        for date in dates.into_iter().rev() {
            let name = format!("{}/{}/{}.msgpack", self.prefix, date, id);
            if let Some((entry, _)) = self.get_entry_by_name(&name).await? {
                return Ok(Some(entry));
            }
        }

        Ok(None)
    }

    async fn prune(&self, older_than_days: u32, dry_run: bool) -> Result<(usize, u64)> {
        let cutoff =
            chrono::Utc::now().date_naive() - chrono::Duration::days(older_than_days as i64);

        let mut count = 0;
        let mut bytes_freed = 0u64;

        let dates = self.list_dates().await?;

        for date in dates {
            if date < cutoff {
                let prefix = self.date_prefix(date);
                let mut page_token: Option<String> = None;

                loop {
                    let request = ListObjectsRequest {
                        bucket: self.bucket.clone(),
                        prefix: Some(prefix.clone()),
                        page_token: page_token.clone(),
                        ..Default::default()
                    };

                    let response = self.client.list_objects(&request).await?;

                    if let Some(items) = &response.items {
                        for object in items {
                            if let Some(size) = object.size {
                                bytes_freed += size as u64;
                            }
                            count += 1;

                            if !dry_run {
                                let delete_request = DeleteObjectRequest {
                                    bucket: self.bucket.clone(),
                                    object: object.name.clone(),
                                    ..Default::default()
                                };
                                self.client.delete_object(&delete_request).await?;
                            }
                        }
                    }

                    page_token = response.next_page_token.clone();
                    if page_token.is_none() {
                        break;
                    }
                }
            }
        }

        Ok((count, bytes_freed))
    }

    async fn health_check(&self) -> Result<()> {
        // Try to list objects (will fail if bucket doesn't exist or no access)
        let request = ListObjectsRequest {
            bucket: self.bucket.clone(),
            prefix: Some(self.prefix.clone()),
            max_results: Some(1),
            ..Default::default()
        };

        self.client
            .list_objects(&request)
            .await
            .context("GCS health check failed")?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        "gcs"
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

/// A record stored in GCS
#[derive(serde::Serialize, serde::Deserialize)]
struct GcsRecord {
    entry: MemoryEntry,
    scope: MemoryScope,
    timestamp: chrono::DateTime<chrono::Utc>,
}
