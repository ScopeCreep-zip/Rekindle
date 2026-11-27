//! Pinned-attachment set — chunks for these IDs are exempt from LRU
//! eviction (spec §28.9 line 3283). The set is driven by
//! `GovernanceEntry::AttachmentPinned` merge state; this struct is just
//! the in-memory mirror the cache reads on every eviction sweep.

use std::collections::HashSet;

use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct PinnedSet {
    ids: HashSet<Uuid>,
}

impl PinnedSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, id: Uuid) {
        self.ids.insert(id);
    }

    pub fn remove(&mut self, id: &Uuid) {
        self.ids.remove(id);
    }

    pub fn contains(&self, id: &Uuid) -> bool {
        self.ids.contains(id)
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The pinned ids (the cache names them before an eviction sweep).
    pub fn iter(&self) -> impl Iterator<Item = &Uuid> {
        self.ids.iter()
    }

    /// Replace contents with the supplied snapshot — used after every
    /// governance merge so the cache reads from the canonical state.
    pub fn replace<I: IntoIterator<Item = Uuid>>(&mut self, ids: I) {
        self.ids.clear();
        self.ids.extend(ids);
    }
}

/// Architecture §28.9 — moderator-driven pin toggle.
///
/// Caller is gated on `MANAGE_COMMUNITY` via the deps trait; the
/// orchestrator decodes the hex attachment id, bumps the community
/// lamport, and writes a `GovernanceEntry::AttachmentPinned` so every
/// peer's merged state agrees on what's eviction-exempt.
pub async fn set_attachment_pinned<D: crate::deps::FilesDeps + ?Sized>(
    deps: &D,
    community_id: &str,
    attachment_id_hex: &str,
    pinned: bool,
) -> Result<(), crate::error::FilesError> {
    deps.require_permission(
        community_id,
        None,
        rekindle_types::permissions::MANAGE_COMMUNITY,
    )?;
    let attachment_id: [u8; 16] = hex::decode(attachment_id_hex)
        .ok()
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| {
            crate::error::FilesError::InvalidAttachmentId(format!(
                "invalid attachment id hex: {attachment_id_hex}"
            ))
        })?;
    deps.write_attachment_pinned(community_id, attachment_id, pinned)
        .await
}

/// Sync the in-memory pinned set for a community from the merged governance
/// state's `pinned_attachments`. Run after every governance merge.
pub fn sync_pinned_from_governance<D: crate::deps::FilesDeps + ?Sized>(
    deps: &D,
    community_id: &str,
) {
    let ids = deps.governance_pinned_attachments(community_id);
    deps.set_community_pinned_set(community_id, ids);
}

#[cfg(test)]
#[path = "pinned_tests.rs"]
mod tests;
