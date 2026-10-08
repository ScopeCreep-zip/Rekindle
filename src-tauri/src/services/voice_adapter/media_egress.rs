//! Video-plane envelopes onto the per-peer media routes (plans E4.3.1,
//! E4.3.3).
//!
//! Video fragments, parity, keyframe requests and topology changes go the
//! way voice does: signed once by our community pseudonym — the same
//! `SignedEnvelope` the receiver already verifies — then queued on each
//! roster peer's route, whose controller stamps that route's
//! `transport_seq` as the pacer releases it. Control is queued unpaced,
//! ahead of media; a frame's fragments are queued paced, behind audio.

use std::sync::Arc;

use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_voice::transport::egress::VideoFrame;
use rekindle_voice::transport::roster::MediaRoster;

use crate::state::AppState;

fn roster(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
) -> Result<Arc<MediaRoster>, String> {
    crate::state_helpers::media_roster_for(state, community_id, channel_id)
        .ok_or_else(|| format!("no voice session bound to {community_id}/{channel_id}"))
}

/// `envelope` signed by our pseudonym in `community_id`, for one hop only.
fn sign(
    state: &Arc<AppState>,
    community_id: &str,
    envelope: &CommunityEnvelope,
) -> Result<Vec<u8>, String> {
    let (pseudonym, signing_key) =
        crate::state_helpers::pseudonym_credentials(state, community_id)?;
    let bytes = rekindle_codec::capnp_envelope::encode_community_envelope(envelope)
        .map_err(|e| format!("encode video envelope: {e}"))?;
    let mut signed = rekindle_codec::community::envelope::sign_envelope(
        &signing_key,
        community_id,
        &hex::encode(pseudonym.0),
        &bytes,
    );
    // Point to point: a receiver must never gossip it on.
    signed.ttl = 0;
    Ok(rekindle_codec::capnp_envelope::encode_signed_envelope(
        &signed,
    ))
}

/// Sign a video-plane control envelope and queue it for every peer on the
/// voice roster of `community_id`/`channel_id`.
///
/// # Errors
/// No voice session is bound to the channel, no identity is loaded, or
/// the envelope does not encode.
pub(crate) fn send_video_envelope(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    envelope: &CommunityEnvelope,
) -> Result<(), String> {
    let roster = roster(state, community_id, channel_id)?;
    roster.send_control_envelope(&sign(state, community_id, envelope)?);
    Ok(())
}

/// Sign a built frame's fragments and queue them on every peer's route.
/// Returns how many routes took the frame: a route whose allocation has
/// no room for video, or that waits for a keyframe, refuses it.
///
/// # Errors
/// As [`send_video_envelope`].
pub(crate) fn send_video_frame(
    state: &Arc<AppState>,
    community_id: &str,
    channel_id: &str,
    frame: &rekindle_video::BuiltVideoFrame,
) -> Result<usize, String> {
    let roster = roster(state, community_id, channel_id)?;
    let fragments = frame
        .envelopes
        .iter()
        .map(|e| sign(state, community_id, e).map(Arc::<[u8]>::from))
        .collect::<Result<Vec<_>, String>>()?;
    Ok(roster.send_video_frame(&VideoFrame {
        keyframe: frame.keyframe,
        fragments,
        media_bytes: frame.media_bytes,
    }))
}
