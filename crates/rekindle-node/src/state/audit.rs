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

/// Append-only BLAKE3 hash-chained audit logger.
pub struct AuditLogger {
    /// File handle for append writes.
    file: std::fs::File,
    /// Sequence counter.
    next_sequence: u64,
    /// Hash of the last written entry (chain link).
    prev_hash: [u8; 32],
}

impl AuditLogger {
    /// Open or create the audit log file.
    ///
    /// If the file exists, reads the last entry to recover the chain state
    /// (sequence number and prev_hash). If the file is empty or doesn't exist,
    /// starts a fresh chain.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        // Ensure parent directory exists.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Read existing entries to recover chain state.
        let (next_sequence, prev_hash) = if path.exists() {
            recover_chain_state(path)?
        } else {
            (0, [0u8; 32])
        };

        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        // [RC-6] Set file permissions to 0600 (append-only sensitive data).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(Self {
            file,
            next_sequence,
            prev_hash,
        })
    }

    /// Append an audit entry for a routed IPC message.
    ///
    /// Computes the BLAKE3 hash of the payload, creates the entry with
    /// the chain link, writes it as a JSON line, and fsyncs.
    pub fn append(
        &mut self,
        payload: &[u8],
        sender_name: Option<String>,
        security_level: SecurityLevel,
        event_type: String,
        community_scope: Option<String>,
    ) -> anyhow::Result<()> {
        let msg_hash = *blake3::hash(payload).as_bytes();

        let timestamp_ms = rekindle_utils::timestamp_ms();

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

        let json = serde_json::to_string(&entry)?;
        writeln!(self.file, "{json}")?;
        self.file.flush()?;
        // fsync to ensure durability before we update chain state.
        self.file.sync_data()?;

        // Update chain state for next entry.
        self.prev_hash = *blake3::hash(json.as_bytes()).as_bytes();
        self.next_sequence += 1;

        Ok(())
    }

    /// Current sequence number (number of entries written).
    pub fn sequence(&self) -> u64 {
        self.next_sequence
    }
}

/// Recover chain state from an existing audit log file.
///
/// Reads the last line, parses it, and extracts the sequence number
/// and computes the hash for chain linking.
fn recover_chain_state(path: &Path) -> anyhow::Result<(u64, [u8; 32])> {
    let content = std::fs::read_to_string(path)?;
    let last_line = content.lines().rfind(|l| !l.is_empty());

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

        // Write 3 entries.
        for i in 0..3 {
            logger
                .append(
                    format!("payload-{i}").as_bytes(),
                    Some("test-agent".into()),
                    SecurityLevel::Authenticated,
                    format!("Event{i}"),
                    None,
                )
                .unwrap();
        }
        assert_eq!(logger.sequence(), 3);

        // Verify chain: each entry's prev_hash matches the hash of the previous line.
        let content = std::fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 3);

        // First entry's prev_hash should be all zeros.
        let entry0: AuditEntry = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(entry0.prev_hash, [0u8; 32]);
        assert_eq!(entry0.sequence, 0);

        // Second entry's prev_hash should be BLAKE3(first line).
        let entry1: AuditEntry = serde_json::from_str(lines[1]).unwrap();
        let expected_hash = *blake3::hash(lines[0].as_bytes()).as_bytes();
        assert_eq!(entry1.prev_hash, expected_hash);
        assert_eq!(entry1.sequence, 1);
    }

    #[test]
    fn audit_log_recovery_after_reopen() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        // Write 2 entries.
        {
            let mut logger = AuditLogger::open(path).unwrap();
            logger.append(b"a", None, SecurityLevel::Open, "A".into(), None).unwrap();
            logger.append(b"b", None, SecurityLevel::Open, "B".into(), None).unwrap();
        }

        // Reopen and verify chain continues.
        let mut logger = AuditLogger::open(path).unwrap();
        assert_eq!(logger.sequence(), 2);

        logger.append(b"c", None, SecurityLevel::Open, "C".into(), None).unwrap();
        assert_eq!(logger.sequence(), 3);

        // Verify third entry chains from second.
        let content = std::fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        let entry2: AuditEntry = serde_json::from_str(lines[2]).unwrap();
        let expected = *blake3::hash(lines[1].as_bytes()).as_bytes();
        assert_eq!(entry2.prev_hash, expected);
    }
}
