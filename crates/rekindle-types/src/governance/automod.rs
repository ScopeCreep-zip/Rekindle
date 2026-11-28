//! The one automod action vocabulary (plan E3.1 D26): what a matching
//! `GovernanceEntry::AutoModRule` asks every receiving peer's ingest gate
//! to do with the post.

use serde::{Deserialize, Serialize};

/// A rule's action. The gate maps `BlockLocally` to a drop, `BlurContent`
/// to a hidden (blurred) row and `AlertModerators` to an admitted post
/// reported to the moderators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AutoModAction {
    BlockLocally,
    BlurContent,
    AlertModerators,
}
