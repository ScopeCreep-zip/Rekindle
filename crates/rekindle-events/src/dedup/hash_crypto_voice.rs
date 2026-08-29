//! Content hashers for MEK/crypto lifecycle and voice-channel events.

use rekindle_types::subscription_events::{CryptoEvent, VoiceEvent};

pub(super) fn hash_crypto(h: &mut blake3::Hasher, c: &CryptoEvent) {
    match c {
        CryptoEvent::MekRotated {
            community,
            channel,
            generation,
            ..
        } => {
            h.update(b"mek_rot|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_deref().unwrap_or("all").as_bytes());
            h.update(b"|");
            h.update(&generation.to_le_bytes());
        }
        CryptoEvent::MekRequested {
            community,
            channel,
            needed_generation,
            requester_pseudonym,
        } => {
            h.update(b"mek_req|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(&needed_generation.to_le_bytes());
            h.update(b"|");
            h.update(requester_pseudonym.as_bytes());
        }
        CryptoEvent::MekTransferred {
            community,
            channel,
            generation,
            sender_pseudonym,
        } => {
            h.update(b"mek_xfer|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_deref().unwrap_or("all").as_bytes());
            h.update(b"|");
            h.update(&generation.to_le_bytes());
            h.update(b"|");
            h.update(sender_pseudonym.as_bytes());
        }
        CryptoEvent::AdminKeypairGranted { community } => {
            h.update(b"admin_kp|");
            h.update(community.as_bytes());
        }
        CryptoEvent::SlotKeypairGranted {
            community,
            slot_index,
            segment_index,
        } => {
            h.update(b"slot_kp|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(&slot_index.to_le_bytes());
            h.update(b"|");
            h.update(&segment_index.to_le_bytes());
        }
        CryptoEvent::PqBundlePublished { subkey, kind } => {
            h.update(b"pq_bundle|");
            h.update(&subkey.to_le_bytes());
            h.update(b"|");
            h.update(match kind {
                rekindle_types::subscription_events::PqBundleKind::LastResort => b"lr".as_slice(),
                rekindle_types::subscription_events::PqBundleKind::OneTimeBatch => b"ot".as_slice(),
            });
        }
    }
}

pub(super) fn hash_voice(h: &mut blake3::Hasher, v: &VoiceEvent) {
    let now_bucket = rekindle_utils::timestamp_secs() / 5;
    match v {
        VoiceEvent::Joined {
            community,
            channel,
            pseudonym,
        } => {
            h.update(b"join|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
            h.update(b"|");
            h.update(&now_bucket.to_le_bytes());
        }
        VoiceEvent::Left {
            community,
            channel,
            pseudonym,
        } => {
            h.update(b"leave|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(pseudonym.as_bytes());
            h.update(b"|");
            h.update(&now_bucket.to_le_bytes());
        }
        VoiceEvent::ModeChanged {
            community,
            channel,
            mode,
            ..
        } => {
            h.update(b"mode|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(mode.as_bytes());
        }
        VoiceEvent::MuteChanged {
            community,
            channel,
            target_pseudonym,
            muted,
        } => {
            h.update(b"mute|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(target_pseudonym.as_bytes());
            h.update(b"|");
            h.update(&[u8::from(*muted)]);
        }
        VoiceEvent::DeafenChanged {
            community,
            channel,
            target_pseudonym,
            deafened,
        } => {
            h.update(b"deafen|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(target_pseudonym.as_bytes());
            h.update(b"|");
            h.update(&[u8::from(*deafened)]);
        }
        VoiceEvent::RosterUpdated {
            community,
            channel,
            participant_count,
        } => {
            h.update(b"roster|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
            h.update(b"|");
            h.update(&(*participant_count as u64).to_le_bytes());
        }
    }
}
