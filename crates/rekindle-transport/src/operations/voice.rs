//! Voice session operations — join, leave.
//!
//! Route allocation routes through `broadcast::route`.

use std::sync::Arc;

use parking_lot::RwLock;
use tracing::info;

use crate::crypto::mek::MekCache;
use crate::crypto::voice_crypto::VoiceSessionKey;
use crate::error::{TransportError, Result};
use crate::broadcast::node::TransportNode;
use crate::session::CommunityMembership;

/// Active voice session state.
pub struct VoiceSession {
    pub community_id: String,
    pub channel_id: String,
    pub session_key: VoiceSessionKey,
    pub our_route_blob: Vec<u8>,
    pub muted: bool,
    pub deafened: bool,
}

/// Join a voice channel.
pub async fn join_voice(
    node: &TransportNode,
    membership: &CommunityMembership,
    channel_id: &str,
    mek_cache: &Arc<RwLock<MekCache>>,
    muted: bool,
    deafened: bool,
) -> Result<VoiceSession> {
    info!(channel = channel_id, community = %membership.community_name, "joining voice channel");

    let session_key = {
        let cache = mek_cache.read();
        let mek = cache.current(&membership.governance_key, channel_id)
            .ok_or_else(|| TransportError::VoiceJoinFailed {
                channel: channel_id.to_string(),
                reason: format!("no MEK cached for {}/{}", membership.community_name, channel_id),
            })?;
        VoiceSessionKey::derive_from_mek(mek.as_bytes())
    };

    let (_route_id, route_blob) = crate::broadcast::route::allocate_voice(node).await
        .map_err(|e| TransportError::VoiceJoinFailed {
            channel: channel_id.to_string(),
            reason: format!("route allocation: {e}"),
        })?;

    info!(channel = channel_id, community = %membership.community_name, "voice session established");

    Ok(VoiceSession {
        community_id: membership.governance_key.clone(),
        channel_id: channel_id.to_string(),
        session_key, our_route_blob: route_blob, muted, deafened,
    })
}

/// Leave the current voice session.
pub fn leave_voice(session: &mut VoiceSession) {
    info!(channel = %session.channel_id, "leaving voice session");
    session.session_key = VoiceSessionKey::derive_from_mek(&[0u8; 32]);
    session.our_route_blob.clear();
    info!("voice session ended");
}
