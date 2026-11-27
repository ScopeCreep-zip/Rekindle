//! What an attachment carries out of the app, for the intent review that
//! precedes its upload (plan U1.4, escalation E3; ADR 0019).

use std::path::Path;

use rekindle_types::intent::AttachmentFacts;

/// The facts about the file at `path` about to be offered: its name, its
/// size, and whether embedded metadata is removed. Nothing strips
/// metadata yet (step E5.1 adds it), so the review says it leaves as is.
///
/// # Errors
/// The file cannot be read.
pub fn attachment_facts(path: &Path) -> std::io::Result<AttachmentFacts> {
    let size_bytes = std::fs::metadata(path)?.len();
    let file_name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    Ok(AttachmentFacts {
        file_name: crate::sanitize::sanitize_filename(&file_name),
        size_bytes,
        metadata_removed: false,
    })
}

/// The same facts for bytes about to be offered under `file_name`.
#[must_use]
pub fn attachment_facts_for_bytes(file_name: &str, bytes: &[u8]) -> AttachmentFacts {
    AttachmentFacts {
        file_name: crate::sanitize::sanitize_filename(file_name),
        size_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        metadata_removed: false,
    }
}

#[cfg(test)]
#[path = "intent_facts_tests.rs"]
mod tests;
