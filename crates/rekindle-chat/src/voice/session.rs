//! Voice session lifecycle — join, leave, mute, deafen, gossip signaling.
//!
//! Joining a voice channel: derive session key from MEK, allocate a
//! voice-specific route, broadcast VoiceJoin gossip to mesh peers.
//! Leaving: broadcast VoiceLeave, zeroize key, release route.

use std::sync::Arc;

use parking_lot::RwLock;
use rekindle_types::gossip_payload::{GossipPayload, ControlPayload};

use crate::crypto::mek::MekCache;
use crate::io::PlatformIO;
use crate::ChatError;
use super::VoiceSessionKey;

/// Active voice session state.
pub struct VoiceSession {
    pub community: String,
    pub channel_id: String,
    pub session_key: VoiceSessionKey,
    pub route_blob: Vec<u8>,
    pub muted: bool,
    pub deafened: bool,
}

/// Voice session manager — holds the active session (if any).
pub struct VoiceService {
    pub(crate) io: Arc<PlatformIO>,
    pub(crate) mek_cache: Arc<MekCache>,
    pub(crate) active: RwLock<Option<VoiceSession>>,
}

impl VoiceService {
    pub fn new(
        io: Arc<PlatformIO>,
        mek_cache: Arc<MekCache>,
    ) -> Self {
        Self {
            io, mek_cache,
            active: RwLock::new(None),
        }
    }

    /// Join a voice channel.
    ///
    /// Derives voice session key from the channel MEK, allocates a
    /// voice-optimized route, broadcasts VoiceJoin to mesh peers.
    pub async fn join_voice(
        &self,
        community: &str,
        channel_id: &str,
        muted: bool,
        deafened: bool,
    ) -> Result<(), ChatError> {
        // Check not already in a voice session
        {
            let active = self.active.read();
            if active.is_some() {
                return Err(ChatError::Internal(
                    "already in a voice session — leave current session before joining another".into()
                ));
            }
        }

        // Derive session key from channel MEK
        let (mek_key, _gen) = self.mek_cache
            .current(community, channel_id)
            .ok_or_else(|| ChatError::MekNotCached {
                community: community.into(),
                channel: channel_id.into(),
            })?;
        let session_key = VoiceSessionKey::derive_from_mek(&mek_key);

        // Allocate a route for voice
        let (_route_id, route_blob) = self.io.allocate_route().await?;

        // Broadcast VoiceJoin to mesh peers
        if let Err(e) = self.io.broadcast_gossip_dedup(
            community,
            GossipPayload::Control(ControlPayload::VoiceJoin {
                channel_id: channel_id.into(),
                route_blob: route_blob.clone(),
            }),
        ).await {
            tracing::warn!(
                community = &community[..12.min(community.len())],
                channel = channel_id,
                error = %e,
                "VoiceJoin gossip failed — peers may not see us in the voice channel"
            );
        }

        // Set active session
        {
            let mut active = self.active.write();
            *active = Some(VoiceSession {
                community: community.to_string(),
                channel_id: channel_id.to_string(),
                session_key,
                route_blob,
                muted,
                deafened,
            });
        }

        tracing::info!(
            community = &community[..12.min(community.len())],
            channel = channel_id,
            "voice session joined"
        );

        Ok(())
    }

    /// Leave the current voice session.
    pub async fn leave_voice(&self) -> Result<(), ChatError> {
        let session = {
            let mut active = self.active.write();
            active.take().ok_or_else(|| ChatError::Internal(
                "not in a voice session".into()
            ))?
        };

        // Broadcast VoiceLeave
        if let Err(e) = self.io.broadcast_gossip_dedup(
            &session.community,
            GossipPayload::Control(ControlPayload::VoiceLeave {
                channel_id: session.channel_id.clone(),
            }),
        ).await {
            tracing::debug!(error = %e, "VoiceLeave gossip failed");
        }

        tracing::info!(
            community = &session.community[..12.min(session.community.len())],
            channel = %session.channel_id,
            "voice session left"
        );
        // VoiceSessionKey is ZeroizeOnDrop — key material zeroized on drop

        Ok(())
    }

    /// Toggle mute and broadcast to mesh peers.
    pub async fn set_mute(&self, muted: bool) -> Result<(), ChatError> {
        let (community, channel_id, pseudonym_hex) = {
            let mut active = self.active.write();
            let session = active.as_mut().ok_or_else(|| ChatError::Internal("not in voice".into()))?;
            session.muted = muted;
            (session.community.clone(), session.channel_id.clone(), self.io.pseudonym_hex(&session.community)?)
        };

        if let Err(e) = self.io.broadcast_gossip_dedup(
            &community,
            GossipPayload::Control(ControlPayload::VoiceMute {
                channel_id, target_pseudonym: pseudonym_hex, muted,
            }),
        ).await {
            tracing::debug!(error = %e, "VoiceMute gossip failed");
        }

        Ok(())
    }

    /// Toggle deafen and broadcast to mesh peers.
    pub async fn set_deafen(&self, deafened: bool) -> Result<(), ChatError> {
        let (community, channel_id, pseudonym_hex) = {
            let mut active = self.active.write();
            let session = active.as_mut().ok_or_else(|| ChatError::Internal("not in voice".into()))?;
            session.deafened = deafened;
            (session.community.clone(), session.channel_id.clone(), self.io.pseudonym_hex(&session.community)?)
        };

        if let Err(e) = self.io.broadcast_gossip_dedup(
            &community,
            GossipPayload::Control(ControlPayload::VoiceDeafen {
                channel_id, target_pseudonym: pseudonym_hex, deafened,
            }),
        ).await {
            tracing::debug!(error = %e, "VoiceDeafen gossip failed");
        }

        Ok(())
    }

    /// Whether we're currently in a voice session.
    pub fn is_active(&self) -> bool {
        self.active.read().is_some()
    }

    /// Get the active session's channel info.
    pub fn active_channel(&self) -> Option<(String, String)> {
        let active = self.active.read();
        active.as_ref().map(|s| (s.community.clone(), s.channel_id.clone()))
    }

    /// Get the active session key for packet encryption.
    /// Returns None if not in a voice session.
    pub fn session_key(&self) -> Option<VoiceSessionKey> {
        let active = self.active.read();
        active.as_ref().map(|s| s.session_key.clone())
    }
}
