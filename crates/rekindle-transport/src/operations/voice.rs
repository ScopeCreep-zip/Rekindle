//! Voice session operations — join, leave.
//!
//! Route allocation routes through `broadcast::route`.

use rekindle_types::channel_keys::ChannelKeyProvider;
use tracing::info;

use crate::broadcast::node::TransportNode;
use crate::error::{Result, TransportError};
use crate::session::CommunityMembership;

/// Active voice session state.
pub struct VoiceSession {
    pub community_id: String,
    pub channel_id: String,
    pub our_route_blob: Vec<u8>,
    pub muted: bool,
    pub deafened: bool,
}

/// Join a voice channel.
pub fn join_voice(
    node: &TransportNode,
    membership: &CommunityMembership,
    channel_id: &str,
    keys: &dyn ChannelKeyProvider,
    muted: bool,
    deafened: bool,
) -> Result<VoiceSession> {
    info!(channel = channel_id, community = %membership.community_name, "joining voice channel");

    // Voice frames are SFrame-sealed under the channel's media key
    // (`rekindle-voice::media_crypto`); without one there is nothing to
    // join with.
    let scope = rekindle_types::id::ChannelId::from_hex(channel_id)
        .map(|channel| keys.scope_for_media(&membership.governance_key, channel))
        .ok_or_else(|| TransportError::VoiceJoinFailed {
            channel: channel_id.to_string(),
            reason: "channel id is not a 32-hex channel id".to_string(),
        })?;
    if keys
        .current_epoch(&membership.governance_key, scope)
        .is_none()
    {
        return Err(TransportError::VoiceJoinFailed {
            channel: channel_id.to_string(),
            reason: format!(
                "no MEK cached for {}/{}",
                membership.community_name, channel_id
            ),
        });
    }

    // Our media-class route, which peers send our media over; the personal
    // route is never substituted, so without one the join fails (plan
    // C7.9c, C7.9d).
    let route_blob = node
        .media_route_blob()
        .ok_or_else(|| TransportError::VoiceJoinFailed {
            channel: channel_id.to_string(),
            reason: "media route unavailable".to_string(),
        })?;

    info!(channel = channel_id, community = %membership.community_name, "voice session established");

    Ok(VoiceSession {
        community_id: membership.governance_key.clone(),
        channel_id: channel_id.to_string(),
        our_route_blob: route_blob,
        muted,
        deafened,
    })
}

/// Leave the current voice session.
pub fn leave_voice(session: &mut VoiceSession) {
    info!(channel = %session.channel_id, "leaving voice session");
    session.our_route_blob.clear();
    info!("voice session ended");
}
