//! BLAKE3 hash-chained audit log for destructive operations.
//!
//! Best-effort — audit failures are logged but don't block the operation.
//! Path: `${XDG_STATE_HOME}/rekindle/audit.jsonl`
//!
//! Security note: The hash chain is UNKEYED (blake3::hash, not blake3::keyed_hash).
//! The CLI is an IPC client and does not have access to vault keys. This chain
//! detects accidental corruption and naive log truncation but does NOT provide
//! tamper evidence against a motivated attacker with file write access. The
//! daemon's own audit log (rekindle-node/src/state/audit.rs) uses a keyed HMAC
//! derived from the session MAC key for actual tamper detection.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use super::sanitize::sanitize_for_display;

/// Append a destructive action to the audit log.
pub fn audit_log(action: &str, target: &str, outcome: &str) {
    if let Err(e) = audit_log_inner(action, target, outcome) {
        tracing::warn!(error = %e, action, target, "audit log write failed (non-fatal)");
    }
}

fn audit_log_inner(action: &str, target: &str, outcome: &str) -> anyhow::Result<()> {
    let log_dir = dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local/state")
        })
        .join("rekindle");
    std::fs::create_dir_all(&log_dir)?;

    let path = log_dir.join("audit.jsonl");

    // Read the last line efficiently — seek to end, read last 4KB, find last newline.
    // Avoids reading the entire audit file into memory (M10 fix).
    let prev_hash = read_last_line_hash(&path);

    let ts = chrono::Utc::now().to_rfc3339();
    let entry = serde_json::json!({
        "ts": ts,
        "action": action,
        "target": sanitize_for_display(target),
        "outcome": outcome,
        "prev_hash": prev_hash,
    });

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{}", serde_json::to_string(&entry)?)?;

    Ok(())
}

/// Read the BLAKE3 hash of the last line in the audit file.
/// Uses tail-read (last 4KB) instead of reading the entire file.
fn read_last_line_hash(path: &std::path::Path) -> String {
    let Ok(mut file) = std::fs::File::open(path) else {
        return "genesis".to_string();
    };

    let Ok(metadata) = file.metadata() else {
        return "genesis".to_string();
    };

    let file_len = metadata.len();
    if file_len == 0 {
        return "genesis".to_string();
    }

    // Read the last 4KB (audit entries are typically ~200 bytes)
    const TAIL_SIZE: u64 = 4096;
    let seek_pos = file_len.saturating_sub(TAIL_SIZE);
    if file.seek(SeekFrom::Start(seek_pos)).is_err() {
        return "genesis".to_string();
    }

    let mut raw = Vec::new();
    if file.read_to_end(&mut raw).is_err() {
        return "genesis".to_string();
    }

    // Seek may land mid-UTF-8 character — lossy conversion drops the partial byte
    let buf = String::from_utf8_lossy(&raw);

    buf.lines()
        .last()
        .filter(|line| !line.is_empty())
        .map_or_else(
            || "genesis".to_string(),
            |line| {
                let hash = blake3::hash(line.as_bytes());
                hex::encode(&hash.as_bytes()[..16])
            },
        )
}
