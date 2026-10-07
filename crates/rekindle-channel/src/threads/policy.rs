//! Pure thread policy: archive rules, defaults, the message view.

use crate::error::ChannelError;

pub fn validate_auto_archive_seconds(secs: u64) -> Result<u64, ChannelError> {
    if matches!(secs, 3600 | 86_400 | 259_200 | 604_800) {
        Ok(secs)
    } else {
        Err(ChannelError::InvalidId(format!(
            "auto_archive_seconds must be 3600, 86400, 259200, or 604800 — got {secs}"
        )))
    }
}

#[must_use]
pub fn default_auto_archive_seconds(thread_type: &str) -> u64 {
    match thread_type {
        "forum_post" => 604_800,
        "announcement" => 259_200,
        _ => 86_400,
    }
}

/// Architecture §14 — a thread is archived when it was manually
/// archived at a lamport >= last activity, OR when it has been idle
/// past its auto-archive window.
#[must_use]
pub fn is_thread_archived(
    archived_lamport: Option<u64>,
    last_lamport: u64,
    last_activity_secs: u64,
    auto_archive_seconds: u64,
    now_secs: u64,
) -> bool {
    let manually_archived = archived_lamport.is_some_and(|archived| last_lamport <= archived);
    let auto_archived = last_activity_secs > 0
        && last_activity_secs.saturating_add(auto_archive_seconds) < now_secs;
    manually_archived || auto_archived
}

/// One thread message decrypted and ready for adapter-side display
/// assembly. Crate-side counterpart of src-tauri `Message` — adapter
/// wraps these into the full Message DTO.
#[derive(Debug, Clone)]
pub struct ThreadMessageView {
    pub sender_pseudonym: String,
    pub body: String,
    /// The body did not open under its key generation at its position
    /// (generation not held yet, or not authentic there).
    pub decryption_failed: bool,
    pub timestamp_ms: u64,
    pub is_own: bool,
    pub server_message_id: Option<String>,
    pub mek_generation: u64,
    pub subkey_index: u32,
    pub lamport_ts: u64,
}

#[cfg(test)]
mod tests;
