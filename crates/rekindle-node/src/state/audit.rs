//! BLAKE3 hash-chained audit log.
//!
//! Every IPC message routed through the bus is hashed into a tamper-evident
//! chain. Each entry links to the previous via `prev_hash`, so deleting or
//! modifying a middle entry breaks the chain for all subsequent entries.
//!
//! Entries are appended as single JSON lines to `audit.jsonl`.


use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ipc::message::SecurityLevel;

/// A single audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Monotonically increasing sequence number.
    pub sequence: u64,
    /// Wall clock timestamp (milliseconds since Unix epoch).
    pub timestamp_ms: u64,
    /// BLAKE3 hash of the previous entry (chain link). All zeros for the first entry.
    pub prev_hash: [u8; 32],
    /// BLAKE3 hash of the stamped IPC payload.
    pub msg_hash: [u8; 32],
    /// Verified sender name (from Noise IK registry lookup).
    pub sender_name: Option<String>,
    /// Security level of the message.
    pub security_level: SecurityLevel,
    /// IPC event type name (e.g., "ChannelSend", "FriendAdd").
    pub event_type: String,
    /// Community scope (governance key) if applicable.
    pub community_scope: Option<String>,
}

/// Batched BLAKE3 hash-chained audit logger.
///
/// Buffers entries and flushes in batches, reducing fsync calls from
/// per-entry to per-batch. Chain integrity is preserved across batches.
pub struct AuditLogger {
    file: std::fs::File,
    next_sequence: u64,
    prev_hash: [u8; 32],
    /// Pre-serialized JSON lines — serialized once in append(), written in flush().
    /// Storing strings (not structs) guarantees the chain hash matches what's on disk.
    buffer: Vec<String>,
    last_flush: std::time::Instant,
}

/// Buffer capacity before automatic flush.
const AUDIT_BUFFER_CAPACITY: usize = 64;
/// Time-based flush interval.
const AUDIT_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

impl AuditLogger {
    /// Open or create the audit log file.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let (next_sequence, prev_hash) = if path.exists() {
            recover_chain_state(path)?
        } else {
            (0, [0u8; 32])
        };

        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(Self {
            file,
            next_sequence,
            prev_hash,
            buffer: Vec::with_capacity(AUDIT_BUFFER_CAPACITY),
            last_flush: std::time::Instant::now(),
        })
    }

    /// Append an audit entry. Buffered — call `flush()` or `maybe_flush()` to persist.
    pub fn append(
        &mut self,
        payload: &[u8],
        sender_name: Option<String>,
        security_level: SecurityLevel,
        event_type: String,
        community_scope: Option<String>,
    ) -> anyhow::Result<()> {
        let msg_hash = *blake3::hash(payload).as_bytes();

        #[allow(clippy::cast_possible_truncation)]
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let entry = AuditEntry {
            sequence: self.next_sequence,
            timestamp_ms,
            prev_hash: self.prev_hash,
            msg_hash,
            sender_name,
            security_level,
            event_type,
            community_scope,
        };

        // Serialize once. Store the JSON string — flush() writes exactly this,
        // guaranteeing the chain hash matches what lands on disk.
        let json = serde_json::to_string(&entry)?;
        self.prev_hash = *blake3::hash(json.as_bytes()).as_bytes();
        self.next_sequence += 1;

        self.buffer.push(json);

        if self.buffer.len() >= AUDIT_BUFFER_CAPACITY {
            self.flush()?;
        }

        Ok(())
    }

    /// Flush if buffer has entries and the flush interval has elapsed.
    pub fn maybe_flush(&mut self) -> anyhow::Result<()> {
        if !self.buffer.is_empty() && self.last_flush.elapsed() >= AUDIT_FLUSH_INTERVAL {
            self.flush()?;
        }
        Ok(())
    }

    /// Flush all buffered entries to disk with a single fsync.
    pub fn flush(&mut self) -> anyhow::Result<()> {
        if self.buffer.is_empty() { return Ok(()); }

        let mut output = String::with_capacity(self.buffer.len() * 256);
        for json in &self.buffer {
            output.push_str(json);
            output.push('\n');
        }

        self.file.write_all(output.as_bytes())?;
        self.file.flush()?;
        self.file.sync_data()?;

        self.buffer.clear();
        self.last_flush = std::time::Instant::now();
        Ok(())
    }

    /// Current sequence number (number of entries written, including buffered).
    pub fn sequence(&self) -> u64 {
        self.next_sequence
    }
}

impl Drop for AuditLogger {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

/// Recover chain state from an existing audit log file.
///
/// Reads the last line, parses it, and extracts the sequence number
/// and computes the hash for chain linking.
fn recover_chain_state(path: &Path) -> anyhow::Result<(u64, [u8; 32])> {
    let content = std::fs::read_to_string(path)?;
    let last_line = content.lines().filter(|l| !l.is_empty()).next_back();

    match last_line {
        Some(line) => {
            let entry: AuditEntry = serde_json::from_str(line)?;
            let prev_hash = *blake3::hash(line.as_bytes()).as_bytes();
            Ok((entry.sequence + 1, prev_hash))
        }
        None => Ok((0, [0u8; 32])), // Empty file, start fresh.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn audit_log_chain_integrity() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        let mut logger = AuditLogger::open(path).unwrap();

        for i in 0..3 {
            logger.append(
                format!("payload-{i}").as_bytes(),
                Some("test-agent".into()),
                SecurityLevel::Authenticated,
                format!("Event{i}"),
                None,
            ).unwrap();
        }
        logger.flush().unwrap();
        assert_eq!(logger.sequence(), 3);

        let content = std::fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 3);

        let entry0: AuditEntry = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(entry0.prev_hash, [0u8; 32]);
        assert_eq!(entry0.sequence, 0);

        let entry1: AuditEntry = serde_json::from_str(lines[1]).unwrap();
        let expected_hash = *blake3::hash(lines[0].as_bytes()).as_bytes();
        assert_eq!(entry1.prev_hash, expected_hash);
        assert_eq!(entry1.sequence, 1);
    }

    #[test]
    fn audit_log_recovery_after_reopen() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        {
            let mut logger = AuditLogger::open(path).unwrap();
            logger.append(b"a", None, SecurityLevel::Open, "A".into(), None).unwrap();
            logger.append(b"b", None, SecurityLevel::Open, "B".into(), None).unwrap();
            logger.flush().unwrap();
        }

        let mut logger = AuditLogger::open(path).unwrap();
        assert_eq!(logger.sequence(), 2);

        logger.append(b"c", None, SecurityLevel::Open, "C".into(), None).unwrap();
        logger.flush().unwrap();
        assert_eq!(logger.sequence(), 3);

        let content = std::fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        let entry2: AuditEntry = serde_json::from_str(lines[2]).unwrap();
        let expected = *blake3::hash(lines[1].as_bytes()).as_bytes();
        assert_eq!(entry2.prev_hash, expected);
    }

    #[test]
    fn audit_log_chain_integrity_across_batches() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        let mut logger = AuditLogger::open(path).unwrap();

        // Batch 1: 3 entries
        for i in 0..3 {
            logger.append(format!("p-{i}").as_bytes(), None, SecurityLevel::Open, format!("E{i}"), None).unwrap();
        }
        logger.flush().unwrap();

        // Batch 2: 2 entries
        for i in 3..5 {
            logger.append(format!("p-{i}").as_bytes(), None, SecurityLevel::Open, format!("E{i}"), None).unwrap();
        }
        logger.flush().unwrap();

        assert_eq!(logger.sequence(), 5);

        let content = std::fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 5);

        // Entry 3 (first of batch 2) chains from entry 2 (last of batch 1)
        let entry3: AuditEntry = serde_json::from_str(lines[3]).unwrap();
        let expected = *blake3::hash(lines[2].as_bytes()).as_bytes();
        assert_eq!(entry3.prev_hash, expected);
    }
}
