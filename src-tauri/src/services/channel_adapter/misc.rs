//! Phase 23.D.7 — small bodies extracted from `deps_impl.rs` to keep
//! that trait impl under the 500-LoC cap (Invariant 1). All of these
//! delegate to existing src-tauri service modules; no protocol logic
//! lives here.

use rekindle_channel::error::ChannelError;
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;
use rekindle_protocol::dht::community::permissions_v2::Permissions;
use rekindle_types::attachment::AttachmentOffer;
use rekindle_types::governance::GovernanceEntry;

use super::ChannelAdapter;

pub(super) fn upload_expression_to_cache_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    expression_id: [u8; 16],
    bytes: &[u8],
    filename: String,
    mime_type: String,
) -> Result<AttachmentOffer, ChannelError> {
    crate::services::community::expression_assets::upload_to_cache(
        &adapter.state,
        community_id,
        expression_id,
        bytes,
        filename,
        mime_type,
    )
    .map_err(ChannelError::Adapter)
}

pub(super) fn read_expression_bytes_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    offer: &AttachmentOffer,
) -> Option<Vec<u8>> {
    crate::services::community::expression_assets::read_bytes_from_cache(
        &adapter.state,
        community_id,
        offer,
    )
}

pub(super) fn send_to_mesh_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    envelope: &CommunityEnvelope,
) -> Result<(), ChannelError> {
    crate::services::community::send_to_mesh(&adapter.state, community_id, envelope)
        .map_err(ChannelError::Adapter)
}

pub(super) async fn write_governance_entry_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    entry: GovernanceEntry,
) -> Result<(), ChannelError> {
    crate::services::community::write_entry(&adapter.state, community_id, entry)
        .await
        .map_err(ChannelError::Adapter)
}

pub(super) fn require_channel_permission_impl(
    adapter: &ChannelAdapter,
    community_id: &str,
    perm_bits: u64,
) -> Result<(), ChannelError> {
    let perms = Permissions::from_bits_truncate(perm_bits);
    crate::commands::community::require_permission(&adapter.state, community_id, perms)
        .map_err(ChannelError::PermissionDenied)
}
