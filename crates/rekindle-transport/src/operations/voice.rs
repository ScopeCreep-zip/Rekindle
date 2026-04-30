//! Voice session operations — join, leave.

use std::sync::Arc;

use parking_lot::RwLock;
use tracing::info;

use crate::crypto::mek::MekCache;
use crate::crypto::voice_crypto::VoiceSessionKey;
use crate::error::{TransportError, Result};
use crate::node::TransportNode;
use crate::session::CommunityMembership;

/// Active voice session state.
pub struct VoiceSession {
    /// Community governance key.
    pub community_id: String,
    /// Voice channel ID.
    pub channel_id: String,
    /// Derived voice session encryption key.
    pub session_key: VoiceSessionKey,
    /// Our allocated voice route blob.
    pub our_route_blob: Vec<u8>,
    /// Whether we're currently muted.
    pub muted: bool,
    /// Whether we're currently deafened.
    pub deafened: bool,
}

/// Join a voice channel.
///
/// Steps:
/// 1. Look up the channel MEK to derive the voice session key
/// 2. Allocate a voice-specific private route
/// 3. Return the session state (the CLI/TUI broadcasts VoiceJoin gossip)
pub async fn join_voice(
    node: &TransportNode,
    membership: &CommunityMembership,
    channel_id: &str,
    mek_cache: &Arc<RwLock<MekCache>>,
    muted: bool,
    deafened: bool,
) -> Result<VoiceSession> {
    info!(
        channel = channel_id,
        community = %membership.community_name,
        "joining voice channel"
    );

    // Step 1: Derive voice session key from channel MEK
    let session_key = {
        let cache = mek_cache.read();
        let mek = cache
            .current(&membership.governance_key, channel_id)
            .ok_or_else(|| TransportError::VoiceJoinFailed {
                channel: channel_id.to_string(),
                reason: format!(
                    "no MEK cached for {}/{}",
                    membership.community_name, channel_id
                ),
            })?;
        VoiceSessionKey::derive_from_mek(mek.as_bytes())
    };

    // Step 2: Allocate voice route
    let (_route_id, route_blob) = node.allocate_route().await.map_err(|e| {
        TransportError::VoiceJoinFailed {
            channel: channel_id.to_string(),
            reason: format!("route allocation: {e}"),
        }
    })?;

    info!(
        channel = channel_id,
        community = %membership.community_name,
        "voice session established"
    );

    Ok(VoiceSession {
        community_id: membership.governance_key.clone(),
        channel_id: channel_id.to_string(),
        session_key,
        our_route_blob: route_blob,
        muted,
        deafened,
    })
}

/// Leave the current voice session.
///
/// The caller (CLI/TUI) is responsible for broadcasting `VoiceLeave`
/// gossip to the community mesh. This function cleans up local state only.
pub fn leave_voice(session: &mut VoiceSession) {
    info!(
        channel = %session.channel_id,
        "leaving voice session"
    );

    // Zeroize the session key by overwriting with zeros
    session.session_key = VoiceSessionKey::derive_from_mek(&[0u8; 32]);
    session.our_route_blob.clear();

    info!("voice session ended");
}
