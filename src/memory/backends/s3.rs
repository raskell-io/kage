//! AWS S3 backend for memory storage
//!
//! Stores entries as MessagePack objects in S3, organized by date.
//! Object key format: {prefix}/{date}/{entry_id}.msgpack

use anyhow::{Context, Result};
use async_trait::async_trait;

use aws_sdk_s3::Client;

use crate::memory::backend::MemoryBackend;
use crate::memory::entry::{MemoryContent, MemoryEntry, MemoryId, MemoryScope};
use crate::memory::query::MemoryQuery;

/// AWS S3-based memory backend
pub struct S3Backend {
    client: Client,
    bucket: String,
    prefix: String,
}

impl S3Backend {
    /// Create a new S3 backend
    ///
    /// Uses the default AWS credential chain (env vars, IAM role, etc.)
    pub async fn new(bucket: String, region: Option<String>, prefix: String) -> Result<Self> {
        let mut config_loader = aws_config::from_env();

        if let Some(region) = region {
            config_loader = config_loader.region(aws_sdk_s3::config::Region::new(region));
        }

        let config = config_loader.load().await;
        let client = Client::new(&config);

        Ok(Self {
            client,
            bucket,
            prefix,
        })
    }

    /// Get the S3 key for an entry
    fn entry_key(&self, entry: &MemoryEntry) -> String {
        let date = entry.created_at.date_naive();
        format!("{}/{}/{}.msgpack", self.prefix, date, entry.id)
    }

    /// Get the S3 key prefix for a date
    fn date_prefix(&self, date: chrono::NaiveDate) -> String {
        format!("{}/{}/", self.prefix, date)
    }

    /// List all dates with entries
    async fn list_dates(&self) -> Result<Vec<chrono::NaiveDate>> {
        let mut dates = Vec::new();
        let mut continuation_token = None;

        loop {
            let mut request = self.client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(&self.prefix)
                .delimiter("/");

            if let Some(token) = continuation_token {
                request = request.continuation_token(token);
            }

            let response = request.send().await
                .context("Failed to list S3 objects")?;

            // Parse date prefixes from common prefixes
            if let Some(prefixes) = response.common_prefixes {
                for prefix in prefixes {
                    if let Some(prefix_str) = prefix.prefix {
                        // Extract date from prefix like "kage/memory/2024-01-15/"
                        if let Some(date_str) = prefix_str
                            .trim_end_matches('/')
                            .rsplit('/')
                            .next()
                        {
                            if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                                dates.push(date);
                            }
                        }
                    }
                }
            }

            if response.is_truncated == Some(true) {
                continuation_token = response.next_continuation_token;
            } else {
                break;
            }
        }

        dates.sort();
        Ok(dates)
    }

    /// List entries for a specific date
    async fn list_entries_for_date(&self, date: chrono::NaiveDate) -> Result<Vec<(MemoryEntry, MemoryScope)>> {
        let prefix = self.date_prefix(date);
        let mut entries = Vec::new();
        let mut continuation_token = None;

        loop {
            let mut request = self.client
                .list_objects_v2()
                .bucket(&self.bucket)
                .prefix(&prefix);

            if let Some(token) = continuation_token {
                request = request.continuation_token(token);
            }

            let response = request.send().await
                .context("Failed to list S3 objects")?;

            if let Some(contents) = response.contents {
                for object in contents {
                    if let Some(key) = object.key {
                        // Fetch and deserialize the object
                        if let Ok(Some((entry, scope))) = self.get_entry_by_key(&key).await {
                            entries.push((entry, scope));
                        }
                    }
                }
            }

            if response.is_truncated == Some(true) {
                continuation_token = response.next_continuation_token;
            } else {
                break;
            }
        }

        Ok(entries)
    }

    /// Get an entry by its S3 key
    async fn get_entry_by_key(&self, key: &str) -> Result<Option<(MemoryEntry, MemoryScope)>> {
        let response = match self.client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                if e.to_string().contains("NoSuchKey") {
                    return Ok(None);
                }
                return Err(e.into());
            }
        };

        let bytes = response.body.collect().await?.into_bytes();
        let record: S3Record = rmp_serde::from_slice(&bytes)?;

        Ok(Some((record.entry, record.scope)))
    }
}

#[async_trait]
impl MemoryBackend for S3Backend {
    async fn append(&self, entry: MemoryEntry, scope: MemoryScope) -> Result<()> {
        let key = self.entry_key(&entry);

        let record = S3Record {
            entry,
            scope,
            timestamp: chrono::Utc::now(),
        };

        let bytes = rmp_serde::to_vec(&record)?;

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(bytes.into())
            .content_type("application/msgpack")
            .send()
            .await
            .context("Failed to put object to S3")?;

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
        // This is inefficient - consider adding an index
        let dates = self.list_dates().await?;

        for date in dates.into_iter().rev() {
            let key = format!("{}/{}/{}.msgpack", self.prefix, date, id);
            if let Some((entry, _)) = self.get_entry_by_key(&key).await? {
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

                // List all objects with this prefix
                let mut continuation_token = None;

                loop {
                    let mut request = self.client
                        .list_objects_v2()
                        .bucket(&self.bucket)
                        .prefix(&prefix);

                    if let Some(token) = continuation_token {
                        request = request.continuation_token(token);
                    }

                    let response = request.send().await?;

                    if let Some(contents) = &response.contents {
                        for object in contents {
                            if let Some(size) = object.size {
                                bytes_freed += size as u64;
                            }
                            count += 1;

                            if !dry_run {
                                if let Some(key) = &object.key {
                                    self.client
                                        .delete_object()
                                        .bucket(&self.bucket)
                                        .key(key)
                                        .send()
                                        .await?;
                                }
                            }
                        }
                    }

                    if response.is_truncated == Some(true) {
                        continuation_token = response.next_continuation_token.clone();
                    } else {
                        break;
                    }
                }
            }
        }

        Ok((count, bytes_freed))
    }

    async fn health_check(&self) -> Result<()> {
        // Try to list objects (will fail if bucket doesn't exist or no access)
        self.client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(&self.prefix)
            .max_keys(1)
            .send()
            .await
            .context("S3 health check failed")?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        "s3"
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

/// A record stored in S3
#[derive(serde::Serialize, serde::Deserialize)]
struct S3Record {
    entry: MemoryEntry,
    scope: MemoryScope,
    timestamp: chrono::DateTime<chrono::Utc>,
}
