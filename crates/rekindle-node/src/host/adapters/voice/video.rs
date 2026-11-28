//! Signed video-plane egress on the Host media roster.

use std::sync::Arc;

use rekindle_codec::community::envelope::CommunityEnvelope;
use rekindle_types::audio::VoiceFailure;
use rekindle_types::subscription_events::VoiceScope;
use rekindle_types::video::MediaCapabilities;
use rekindle_voice::transport::egress::VideoFrame;
use rekindle_voice::transport::roster::MediaRoster;

use crate::host::Host;

fn roster(host: &Host, community_id: &str, channel_id: &str) -> Result<Arc<MediaRoster>, String> {
    host.voice()
        .channel_media(community_id, channel_id)
        .ok_or_else(|| format!("no voice session bound to {community_id}/{channel_id}"))
}

/// `envelope` signed by our pseudonym in `community_id`, for one hop only.
fn sign(host: &Host, community_id: &str, envelope: &CommunityEnvelope) -> Result<Vec<u8>, String> {
    let identity = host
        .ctx
        .identity_secret()
        .ok_or_else(|| VoiceFailure::SignInRequired.notice())?;
    let signing_key = rekindle_secrets::derive::derive_community_pseudonym(&identity, community_id);
    let pseudonym = hex::encode(signing_key.verifying_key().to_bytes());
    let bytes = rekindle_codec::capnp_envelope::encode_community_envelope(envelope)
        .map_err(|e| format!("encode video envelope: {e}"))?;
    let mut signed = rekindle_codec::community::envelope::sign_envelope(
        &signing_key,
        community_id,
        &pseudonym,
        rekindle_utils::random::id_bytes_16_from(host.ctx.rng.as_ref()),
        &bytes,
    );
    // Point to point: a receiver must never gossip it on.
    signed.ttl = 0;
    rekindle_codec::capnp_envelope::encode_signed_envelope(&signed)
        .map_err(|error| format!("encode signed video envelope: {error}"))
}

impl Host {
    /// Sign a video-plane control envelope and queue it for every peer on the
    /// voice roster of `community_id`/`channel_id`.
    ///
    /// # Errors
    /// No voice session is bound to the channel, no identity is loaded, or
    /// the envelope does not encode.
    pub fn send_voice_video_envelope(
        &self,
        community_id: &str,
        channel_id: &str,
        envelope: &CommunityEnvelope,
    ) -> Result<(), String> {
        let roster = roster(self, community_id, channel_id)?;
        roster.send_control_envelope(&sign(self, community_id, envelope)?);
        Ok(())
    }

    /// Sign a built frame's fragments and queue them on every peer's route.
    /// Returns how many routes took the frame: a route whose allocation has
    /// no room for video, or that waits for a keyframe, refuses it.
    ///
    /// # Errors
    /// As [`Self::send_voice_video_envelope`].
    pub fn send_voice_video_frame(
        &self,
        community_id: &str,
        channel_id: &str,
        envelopes: &[CommunityEnvelope],
        keyframe: bool,
        media_bytes: usize,
    ) -> Result<usize, String> {
        let roster = roster(self, community_id, channel_id)?;
        let fragments = envelopes
            .iter()
            .map(|e| sign(self, community_id, e).map(Arc::<[u8]>::from))
            .collect::<Result<Vec<_>, String>>()?;
        Ok(roster.send_video_frame(&VideoFrame {
            keyframe,
            fragments,
            media_bytes,
        }))
    }

    /// The encoder capabilities reported by the frontend for this unlock.
    pub fn set_voice_video_capabilities(&self, caps: MediaCapabilities) {
        *self.voice().video_caps.write() = Some(caps);
        if let Some(VoiceScope::Community { community, channel }) = self.voice().session() {
            let deps = super::HostVoiceDeps::new(Arc::clone(&self.ctx), None);
            super::io::broadcast_media_capabilities(&deps, &community, &channel);
        }
    }

    /// Ask for a missing media key on the held channel session.
    pub fn request_voice_media_key(
        &self,
        community: &str,
        channel: &str,
        sender: &str,
        index: u64,
    ) {
        super::io::request_media_key(
            &super::HostVoiceDeps::new(Arc::clone(&self.ctx), None),
            community,
            channel,
            sender,
            index,
        );
    }
}
