//! Usage tracking and reporting for subscriptions
//!
//! Tracks detailed usage metrics over time for billing reconciliation,
//! cost analysis, and capacity planning.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use super::SubscriptionId;

/// Time period for usage aggregation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsagePeriod {
    /// Today
    Today,
    /// Last 7 days
    Week,
    /// Last 30 days
    Month,
    /// Custom date range
    Custom { start: NaiveDate, end: NaiveDate },
}

/// Usage record for a single request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    /// Subscription ID
    pub subscription_id: SubscriptionId,
    /// Timestamp of the request
    pub timestamp: i64,
    /// Tokens used (input + output)
    pub tokens: u64,
    /// Input tokens
    pub input_tokens: u64,
    /// Output tokens
    pub output_tokens: u64,
    /// Whether the request succeeded
    pub success: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Namespace (if any)
    pub namespace: Option<String>,
    /// Agent ID (if any)
    pub agent_id: Option<String>,
    /// Model used
    pub model: Option<String>,
}

impl UsageRecord {
    /// Create a new usage record
    pub fn new(subscription_id: SubscriptionId) -> Self {
        Self {
            subscription_id,
            timestamp: Utc::now().timestamp(),
            tokens: 0,
            input_tokens: 0,
            output_tokens: 0,
            success: true,
            duration_ms: 0,
            namespace: None,
            agent_id: None,
            model: None,
        }
    }

    /// Set token counts
    pub fn with_tokens(mut self, input: u64, output: u64) -> Self {
        self.input_tokens = input;
        self.output_tokens = output;
        self.tokens = input + output;
        self
    }

    /// Set success status
    pub fn with_success(mut self, success: bool) -> Self {
        self.success = success;
        self
    }

    /// Set duration
    pub fn with_duration(mut self, duration_ms: u64) -> Self {
        self.duration_ms = duration_ms;
        self
    }

    /// Set namespace
    pub fn with_namespace(mut self, namespace: &str) -> Self {
        self.namespace = Some(namespace.to_string());
        self
    }

    /// Set agent ID
    pub fn with_agent(mut self, agent_id: &str) -> Self {
        self.agent_id = Some(agent_id.to_string());
        self
    }

    /// Set model
    pub fn with_model(mut self, model: &str) -> Self {
        self.model = Some(model.to_string());
        self
    }
}

/// Aggregated usage statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageStats {
    /// Total requests
    pub total_requests: u64,
    /// Successful requests
    pub successful_requests: u64,
    /// Failed requests
    pub failed_requests: u64,
    /// Total tokens used
    pub total_tokens: u64,
    /// Total input tokens
    pub total_input_tokens: u64,
    /// Total output tokens
    pub total_output_tokens: u64,
    /// Total duration in milliseconds
    pub total_duration_ms: u64,
    /// Average tokens per request
    pub avg_tokens_per_request: f64,
    /// Average duration per request (ms)
    pub avg_duration_ms: f64,
    /// Success rate (0.0 - 1.0)
    pub success_rate: f64,
}

impl UsageStats {
    /// Create stats from a collection of records
    pub fn from_records(records: &[UsageRecord]) -> Self {
        if records.is_empty() {
            return Self::default();
        }

        let total_requests = records.len() as u64;
        let successful_requests = records.iter().filter(|r| r.success).count() as u64;
        let failed_requests = total_requests - successful_requests;
        let total_tokens: u64 = records.iter().map(|r| r.tokens).sum();
        let total_input_tokens: u64 = records.iter().map(|r| r.input_tokens).sum();
        let total_output_tokens: u64 = records.iter().map(|r| r.output_tokens).sum();
        let total_duration_ms: u64 = records.iter().map(|r| r.duration_ms).sum();

        Self {
            total_requests,
            successful_requests,
            failed_requests,
            total_tokens,
            total_input_tokens,
            total_output_tokens,
            total_duration_ms,
            avg_tokens_per_request: total_tokens as f64 / total_requests as f64,
            avg_duration_ms: total_duration_ms as f64 / total_requests as f64,
            success_rate: successful_requests as f64 / total_requests as f64,
        }
    }

    /// Merge with another stats object
    pub fn merge(&mut self, other: &UsageStats) {
        self.total_requests += other.total_requests;
        self.successful_requests += other.successful_requests;
        self.failed_requests += other.failed_requests;
        self.total_tokens += other.total_tokens;
        self.total_input_tokens += other.total_input_tokens;
        self.total_output_tokens += other.total_output_tokens;
        self.total_duration_ms += other.total_duration_ms;

        if self.total_requests > 0 {
            self.avg_tokens_per_request = self.total_tokens as f64 / self.total_requests as f64;
            self.avg_duration_ms = self.total_duration_ms as f64 / self.total_requests as f64;
            self.success_rate = self.successful_requests as f64 / self.total_requests as f64;
        }
    }
}

/// Usage tracker for subscriptions
pub struct UsageTracker {
    /// Directory for usage log files
    usage_dir: PathBuf,
    /// In-memory buffer for current day's records
    buffer: Vec<UsageRecord>,
    /// Maximum buffer size before flush
    max_buffer_size: usize,
}

impl UsageTracker {
    /// Create a new usage tracker
    pub fn new(usage_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&usage_dir)
            .with_context(|| format!("Failed to create usage directory: {:?}", usage_dir))?;

        Ok(Self {
            usage_dir,
            buffer: Vec::new(),
            max_buffer_size: 100,
        })
    }

    /// Record a usage event
    pub fn record(&mut self, record: UsageRecord) -> Result<()> {
        self.buffer.push(record);

        // Flush if buffer is full
        if self.buffer.len() >= self.max_buffer_size {
            self.flush()?;
        }

        Ok(())
    }

    /// Flush buffer to disk
    pub fn flush(&mut self) -> Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        // Group records by date
        let mut by_date: HashMap<NaiveDate, Vec<UsageRecord>> = HashMap::new();
        for record in self.buffer.drain(..) {
            let date = DateTime::from_timestamp(record.timestamp, 0)
                .map(|dt| dt.date_naive())
                .unwrap_or_else(|| Utc::now().date_naive());
            by_date.entry(date).or_default().push(record);
        }

        // Append to each day's file
        for (date, records) in by_date {
            self.append_to_log(date, &records)?;
        }

        Ok(())
    }

    /// Append records to a daily log file
    fn append_to_log(&self, date: NaiveDate, records: &[UsageRecord]) -> Result<()> {
        let path = self.log_path(date);

        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("Failed to open usage log: {:?}", path))?;

        let mut writer = BufWriter::new(file);

        for record in records {
            let json = serde_json::to_string(record)?;
            writeln!(writer, "{}", json)?;
        }

        writer.flush()?;
        Ok(())
    }

    /// Get path for a daily log file
    fn log_path(&self, date: NaiveDate) -> PathBuf {
        self.usage_dir.join(format!(
            "usage-{:04}-{:02}-{:02}.jsonl",
            date.year(),
            date.month(),
            date.day()
        ))
    }

    /// Read records for a specific date
    pub fn read_date(&self, date: NaiveDate) -> Result<Vec<UsageRecord>> {
        let path = self.log_path(date);

        if !path.exists() {
            return Ok(vec![]);
        }

        let file = File::open(&path)
            .with_context(|| format!("Failed to open usage log: {:?}", path))?;
        let reader = BufReader::new(file);

        let mut records = Vec::new();
        for line in std::io::BufRead::lines(reader) {
            let line = line?;
            if !line.is_empty() {
                let record: UsageRecord = serde_json::from_str(&line)?;
                records.push(record);
            }
        }

        Ok(records)
    }

    /// Get usage statistics for a period
    pub fn stats(&mut self, period: UsagePeriod) -> Result<UsageStats> {
        // Flush buffer first to ensure we have all data
        self.flush()?;

        let (start, end) = match period {
            UsagePeriod::Today => {
                let today = Utc::now().date_naive();
                (today, today)
            }
            UsagePeriod::Week => {
                let today = Utc::now().date_naive();
                let start = today - chrono::Duration::days(6);
                (start, today)
            }
            UsagePeriod::Month => {
                let today = Utc::now().date_naive();
                let start = today - chrono::Duration::days(29);
                (start, today)
            }
            UsagePeriod::Custom { start, end } => (start, end),
        };

        let mut all_records = Vec::new();
        let mut current = start;
        while current <= end {
            let records = self.read_date(current)?;
            all_records.extend(records);
            current = current.succ_opt().unwrap_or(current);
        }

        Ok(UsageStats::from_records(&all_records))
    }

    /// Get usage statistics per subscription
    pub fn stats_by_subscription(
        &mut self,
        period: UsagePeriod,
    ) -> Result<HashMap<SubscriptionId, UsageStats>> {
        // Flush buffer first
        self.flush()?;

        let (start, end) = match period {
            UsagePeriod::Today => {
                let today = Utc::now().date_naive();
                (today, today)
            }
            UsagePeriod::Week => {
                let today = Utc::now().date_naive();
                let start = today - chrono::Duration::days(6);
                (start, today)
            }
            UsagePeriod::Month => {
                let today = Utc::now().date_naive();
                let start = today - chrono::Duration::days(29);
                (start, today)
            }
            UsagePeriod::Custom { start, end } => (start, end),
        };

        let mut by_sub: HashMap<SubscriptionId, Vec<UsageRecord>> = HashMap::new();

        let mut current = start;
        while current <= end {
            for record in self.read_date(current)? {
                by_sub.entry(record.subscription_id).or_default().push(record);
            }
            current = current.succ_opt().unwrap_or(current);
        }

        Ok(by_sub
            .into_iter()
            .map(|(id, records)| (id, UsageStats::from_records(&records)))
            .collect())
    }

    /// Get usage statistics per namespace
    pub fn stats_by_namespace(
        &mut self,
        period: UsagePeriod,
    ) -> Result<HashMap<String, UsageStats>> {
        // Flush buffer first
        self.flush()?;

        let (start, end) = match period {
            UsagePeriod::Today => {
                let today = Utc::now().date_naive();
                (today, today)
            }
            UsagePeriod::Week => {
                let today = Utc::now().date_naive();
                let start = today - chrono::Duration::days(6);
                (start, today)
            }
            UsagePeriod::Month => {
                let today = Utc::now().date_naive();
                let start = today - chrono::Duration::days(29);
                (start, today)
            }
            UsagePeriod::Custom { start, end } => (start, end),
        };

        let mut by_ns: HashMap<String, Vec<UsageRecord>> = HashMap::new();

        let mut current = start;
        while current <= end {
            for record in self.read_date(current)? {
                let ns = record.namespace.clone().unwrap_or_else(|| "(none)".to_string());
                by_ns.entry(ns).or_default().push(record);
            }
            current = current.succ_opt().unwrap_or(current);
        }

        Ok(by_ns
            .into_iter()
            .map(|(ns, records)| (ns, UsageStats::from_records(&records)))
            .collect())
    }

    /// Export usage data to JSON
    pub fn export_json(&mut self, period: UsagePeriod) -> Result<String> {
        self.flush()?;

        let (start, end) = match period {
            UsagePeriod::Today => {
                let today = Utc::now().date_naive();
                (today, today)
            }
            UsagePeriod::Week => {
                let today = Utc::now().date_naive();
                let start = today - chrono::Duration::days(6);
                (start, today)
            }
            UsagePeriod::Month => {
                let today = Utc::now().date_naive();
                let start = today - chrono::Duration::days(29);
                (start, today)
            }
            UsagePeriod::Custom { start, end } => (start, end),
        };

        let mut all_records = Vec::new();
        let mut current = start;
        while current <= end {
            let records = self.read_date(current)?;
            all_records.extend(records);
            current = current.succ_opt().unwrap_or(current);
        }

        Ok(serde_json::to_string_pretty(&all_records)?)
    }

    /// List available log files
    pub fn list_logs(&self) -> Result<Vec<NaiveDate>> {
        let mut dates = Vec::new();

        for entry in fs::read_dir(&self.usage_dir)? {
            let entry = entry?;
            let path = entry.path();

            if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                if filename.starts_with("usage-") && filename.ends_with(".jsonl") {
                    // Parse date from filename: usage-YYYY-MM-DD.jsonl
                    let date_str = &filename[6..16];
                    if let Ok(date) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                        dates.push(date);
                    }
                }
            }
        }

        dates.sort();
        Ok(dates)
    }

    /// Prune old log files
    pub fn prune(&self, older_than_days: u32) -> Result<usize> {
        let cutoff = Utc::now().date_naive() - chrono::Duration::days(older_than_days as i64);
        let mut removed = 0;

        for date in self.list_logs()? {
            if date < cutoff {
                let path = self.log_path(date);
                fs::remove_file(&path)?;
                removed += 1;
                tracing::info!("Pruned usage log: {:?}", path);
            }
        }

        Ok(removed)
    }
}

impl Drop for UsageTracker {
    fn drop(&mut self) {
        // Try to flush remaining buffer on drop
        let _ = self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_usage_record() {
        let id = SubscriptionId::new();
        let record = UsageRecord::new(id)
            .with_tokens(100, 200)
            .with_success(true)
            .with_duration(500)
            .with_namespace("backend");

        assert_eq!(record.tokens, 300);
        assert_eq!(record.input_tokens, 100);
        assert_eq!(record.output_tokens, 200);
        assert!(record.success);
    }

    #[test]
    fn test_usage_stats() {
        let id = SubscriptionId::new();
        let records = vec![
            UsageRecord::new(id).with_tokens(100, 200).with_success(true),
            UsageRecord::new(id).with_tokens(50, 150).with_success(true),
            UsageRecord::new(id).with_tokens(0, 0).with_success(false),
        ];

        let stats = UsageStats::from_records(&records);
        assert_eq!(stats.total_requests, 3);
        assert_eq!(stats.successful_requests, 2);
        assert_eq!(stats.failed_requests, 1);
        assert_eq!(stats.total_tokens, 500);
    }

    #[test]
    fn test_tracker_record_and_read() {
        let dir = tempdir().unwrap();
        let mut tracker = UsageTracker::new(dir.path().to_path_buf()).unwrap();

        let id = SubscriptionId::new();
        tracker
            .record(UsageRecord::new(id).with_tokens(100, 200))
            .unwrap();
        tracker
            .record(UsageRecord::new(id).with_tokens(150, 250))
            .unwrap();
        tracker.flush().unwrap();

        let today = Utc::now().date_naive();
        let records = tracker.read_date(today).unwrap();
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn test_stats_period() {
        let dir = tempdir().unwrap();
        let mut tracker = UsageTracker::new(dir.path().to_path_buf()).unwrap();

        let id = SubscriptionId::new();
        for _ in 0..5 {
            tracker
                .record(UsageRecord::new(id).with_tokens(100, 100))
                .unwrap();
        }
        tracker.flush().unwrap();

        let stats = tracker.stats(UsagePeriod::Today).unwrap();
        assert_eq!(stats.total_requests, 5);
        assert_eq!(stats.total_tokens, 1000);
    }
}
