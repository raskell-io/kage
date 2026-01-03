//! Long-term memory - persistent append-only logs

use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Write};
use std::path::PathBuf;

use anyhow::Result;

use super::entry::{MemoryEntry, MemoryScope};

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
}

/// A record in the log file
#[derive(serde::Serialize, serde::Deserialize)]
struct LogRecord {
    entry: MemoryEntry,
    scope: MemoryScope,
    timestamp: chrono::DateTime<chrono::Utc>,
}
