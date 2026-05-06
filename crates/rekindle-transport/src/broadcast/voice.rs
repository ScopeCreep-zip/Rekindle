//! Voice packet send — single peer and mesh broadcast.
//!
//! Voice packets are already MEK-encrypted and HMAC-authenticated by
//! the caller. This module handles framing and Veilid transport only.

use tracing::{debug, warn};

use crate::error::Result;
use super::node::TransportNode;
use crate::payload::voice::{VoiceAuthMode, VoicePayload};
use super::peer_registry::PeerTarget;
use super::send::BroadcastReport;

/// Send an encrypted voice packet to a single peer.
pub async fn send_voice_packet(
    node: &TransportNode,
    target: &PeerTarget,
    packet: &VoicePayload,
) -> Result<()> {
    debug!(
        sender = %packet.sender_key_hex, seq = packet.sequence,
        audio_bytes = packet.encrypted_audio.len(),
        signed = !packet.signature.is_empty(),
        "voice: send_packet"
    );
    let payload_bytes = postcard::to_stdvec(packet)
        .map_err(|e| crate::error::TransportError::SerializationFailed {
            reason: format!("voice: {e}"),
        })?;
    let result = node.sender().send_voice(target, &payload_bytes).await;
    if let Err(ref e) = result {
        warn!(sender = %packet.sender_key_hex, seq = packet.sequence, error = %e, "voice: send failed");
    }
    result
}

/// Broadcast an encrypted voice packet to all voice channel participants.
pub async fn broadcast_voice_packet(
    node: &TransportNode,
    targets: &[PeerTarget],
    packet: &VoicePayload,
) -> BroadcastReport {
    debug!(
        sender = %packet.sender_key_hex, seq = packet.sequence,
        targets = targets.len(), "voice: broadcast_packet"
    );
    let payload_bytes = match postcard::to_stdvec(packet) {
        Ok(b) => b,
        Err(e) => {
            warn!(error = %e, "voice: broadcast serialize failed");
            return BroadcastReport {
                delivered: 0,
                failures: vec![("*".into(), format!("voice serialize: {e}"))],
            };
        }
    };
    let report = node.sender().broadcast_voice(targets, &payload_bytes).await;
    debug!(
        delivered = report.delivered, failed = report.failures.len(),
        "voice: broadcast complete"
    );
    report
}

/// Build a voice packet from raw encrypted audio.
pub fn build_voice_packet(
    sender_key_hex: &str,
    sequence: u32,
    timestamp: u64,
    encrypted_audio: Vec<u8>,
    hmac: [u8; 16],
    auth_mode: VoiceAuthMode,
    signing_key: Option<&[u8; 32]>,
) -> VoicePayload {
    debug!(
        sender = sender_key_hex, seq = sequence,
        audio_bytes = encrypted_audio.len(),
        auth = ?auth_mode, "voice: build_packet"
    );
    let mut packet = VoicePayload {
        sender_key_hex: sender_key_hex.into(),
        sequence, timestamp, encrypted_audio, hmac,
        signature: Vec::new(),
    };

    if auth_mode == VoiceAuthMode::Signed {
        if let Some(key_bytes) = signing_key {
            let signing_key = ed25519_dalek::SigningKey::from_bytes(key_bytes);
            let sig_data = packet.signature_data();
            use ed25519_dalek::Signer;
            let sig = signing_key.sign(&sig_data);
            packet.signature = sig.to_bytes().to_vec();
            debug!(sender = sender_key_hex, seq = sequence, "voice: packet signed");
        }
    }

    packet
}
