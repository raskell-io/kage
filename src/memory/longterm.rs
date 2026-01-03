//! Long-term memory - persistent append-only logs

use std::fs::{File, OpenOptions};
use std::io::{BufReader, Write};
use std::path::PathBuf;

use anyhow::Result;

use super::entry::{MemoryContent, MemoryEntry, MemoryScope};
use super::query::MemoryQuery;

/// Persistent long-term memory storage
pub struct LongTermMemory {
    /// Directory for event logs
    log_dir: PathBuf,
}

impl LongTermMemory {
    /// Create a new long-term memory store
    pub fn new(state_dir: PathBuf) -> Result<Self> {
        let log_dir = state_dir.join("events");
        std::fs::create_dir_all(&log_dir)?;

        Ok(Self { log_dir })
    }

    /// Append an entry to the log
    pub fn append(&self, entry: MemoryEntry, scope: MemoryScope) -> Result<()> {
        let log_file = self.current_log_file()?;

        let record = LogRecord {
            entry,
            scope,
            timestamp: chrono::Utc::now(),
        };

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)?;

        let bytes = rmp_serde::to_vec(&record)?;
        let len = bytes.len() as u32;

        // Write length-prefixed MessagePack
        file.write_all(&len.to_le_bytes())?;
        file.write_all(&bytes)?;
        file.flush()?;

        Ok(())
    }

    /// Read all entries from a log file
    pub fn read_log(&self, date: chrono::NaiveDate) -> Result<Vec<(MemoryEntry, MemoryScope)>> {
        let log_file = self.log_dir.join(format!("{}.log", date));

        if !log_file.exists() {
            return Ok(vec![]);
        }

        let file = File::open(&log_file)?;
        let mut reader = BufReader::new(file);
        let mut entries = vec![];

        loop {
            let mut len_bytes = [0u8; 4];
            match std::io::Read::read_exact(&mut reader, &mut len_bytes) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e.into()),
            }

            let len = u32::from_le_bytes(len_bytes) as usize;
            let mut bytes = vec![0u8; len];
            std::io::Read::read_exact(&mut reader, &mut bytes)?;

            let record: LogRecord = rmp_serde::from_slice(&bytes)?;
            entries.push((record.entry, record.scope));
        }

        Ok(entries)
    }

    /// Get path to current log file (today's date)
    fn current_log_file(&self) -> Result<PathBuf> {
        let today = chrono::Utc::now().date_naive();
        Ok(self.log_dir.join(format!("{}.log", today)))
    }

    /// List available log files
    pub fn list_logs(&self) -> Result<Vec<chrono::NaiveDate>> {
        let mut dates = vec![];

        for entry in std::fs::read_dir(&self.log_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();

            if let Some(date_str) = name.strip_suffix(".log") {
                if let Ok(date) = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                    dates.push(date);
                }
            }
        }

        dates.sort();
        Ok(dates)
    }

    /// Query entries matching criteria
    pub fn query(&self, query: &MemoryQuery) -> Result<Vec<MemoryEntry>> {
        let mut results = Vec::new();

        // Determine date range to search
        let dates = self.list_logs()?;
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

            // Read and filter entries from this log
            let entries = self.read_log(date)?;
            for (entry, scope) in entries {
                if matches_query(&entry, &scope, query) {
                    results.push(entry);
                }
            }
        }

        Ok(results)
    }

    /// Prune old log files
    /// Returns (count of files deleted, bytes freed)
    pub fn prune(&self, older_than_days: u32, dry_run: bool) -> Result<(usize, u64)> {
        let cutoff = chrono::Utc::now().date_naive()
            - chrono::Duration::days(older_than_days as i64);

        let mut count = 0;
        let mut bytes_freed = 0u64;

        let dates = self.list_logs()?;
        for date in dates {
            if date < cutoff {
                let log_file = self.log_dir.join(format!("{}.log", date));
                if let Ok(metadata) = std::fs::metadata(&log_file) {
                    bytes_freed += metadata.len();
                }

                if !dry_run {
                    if let Err(e) = std::fs::remove_file(&log_file) {
                        tracing::warn!("Failed to remove log file {}: {}", date, e);
                        continue;
                    }
                }
                count += 1;
            }
        }

        Ok((count, bytes_freed))
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

    // Filter by text (simple substring match)
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

/// Get the type name for a MemoryContent variant
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

/// A record in the log file
#[derive(serde::Serialize, serde::Deserialize)]
struct LogRecord {
    entry: MemoryEntry,
    scope: MemoryScope,
    timestamp: chrono::DateTime<chrono::Utc>,
}
