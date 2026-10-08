//! Content hashers for MEK/crypto lifecycle and voice-channel events.

use rekindle_types::subscription_events::{CryptoEvent, VoiceEvent, VoiceScope};

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

    // Scope first, so every variant is keyed to its call without each
    // one restating community + channel — and so a DM call, which has
    // no community at all, hashes on its peer key instead.
    match v.scope() {
        Some(VoiceScope::Community { community, channel }) => {
            h.update(b"c|");
            h.update(community.as_bytes());
            h.update(b"|");
            h.update(channel.as_bytes());
        }
        Some(VoiceScope::Dm { peer_key }) => {
            h.update(b"d|");
            h.update(peer_key.as_bytes());
        }
        // Device changes are machine-wide.
        None => {
            h.update(b"-|");
        }
    }
    h.update(b"|");

    match v {
        VoiceEvent::Joined { pseudonym, .. } => {
            h.update(b"join|");
            h.update(pseudonym.as_bytes());
            h.update(b"|");
            h.update(&now_bucket.to_le_bytes());
        }
        VoiceEvent::Left { pseudonym, .. } => {
            h.update(b"leave|");
            h.update(pseudonym.as_bytes());
            h.update(b"|");
            h.update(&now_bucket.to_le_bytes());
        }
        VoiceEvent::ModeChanged { mode, .. } => {
            h.update(b"mode|");
            h.update(mode.as_bytes());
        }
        VoiceEvent::MuteChanged {
            target_pseudonym,
            muted,
            ..
        } => {
            h.update(b"mute|");
            h.update(target_pseudonym.as_bytes());
            h.update(b"|");
            h.update(&[u8::from(*muted)]);
        }
        VoiceEvent::DeafenChanged {
            target_pseudonym,
            deafened,
            ..
        } => {
            h.update(b"deafen|");
            h.update(target_pseudonym.as_bytes());
            h.update(b"|");
            h.update(&[u8::from(*deafened)]);
        }
        VoiceEvent::RosterUpdated { participants, .. } => {
            // Membership, not just the count: two rosters of the same
            // size with different people in them are different rosters,
            // and hashing the count alone would dedup the second away.
            h.update(b"roster|");
            for p in participants {
                h.update(p.pseudonym_key.as_bytes());
                h.update(b",");
            }
        }
        VoiceEvent::JoinHandshake { state, peer, .. } => {
            h.update(b"handshake|");
            h.update(state.as_bytes());
            h.update(b"|");
            h.update(peer.as_deref().unwrap_or("").as_bytes());
        }
        VoiceEvent::PeerConfirmed { pseudonym, .. } => {
            h.update(b"peer_ok|");
            h.update(pseudonym.as_bytes());
        }
        VoiceEvent::MediaReady { ready, reason, .. } => {
            h.update(b"media_ready|");
            h.update(&[u8::from(*ready)]);
            h.update(b"|");
            h.update(reason.as_bytes());
        }
        VoiceEvent::StageUpdated {
            topic, speakers, ..
        } => {
            h.update(b"stage|");
            h.update(topic.as_deref().unwrap_or("").as_bytes());
            h.update(b"|");
            for s in speakers {
                h.update(s.as_bytes());
                h.update(b",");
            }
        }
        VoiceEvent::SpeakRequested {
            requester_pseudonym,
            ..
        } => {
            h.update(b"speak_req|");
            h.update(requester_pseudonym.as_bytes());
            h.update(b"|");
            // Bucketed: a listener may re-request after a denial, and
            // that is a new request rather than a duplicate.
            h.update(&now_bucket.to_le_bytes());
        }
        VoiceEvent::SoundboardPlayed {
            expression_id,
            actor_pseudonym,
            ..
        } => {
            // No time bucket: the same member firing the same sound
            // twice is two plays, not a duplicate — that is the whole
            // point of a soundboard.
            h.update(b"soundboard|");
            h.update(expression_id.as_bytes());
            h.update(b"|");
            h.update(actor_pseudonym.as_bytes());
            h.update(b"|");
            h.update(&rekindle_utils::timestamp_ms().to_le_bytes());
        }
        VoiceEvent::SpeakResponded {
            requester_pseudonym,
            granted,
            ..
        } => {
            h.update(b"speak_res|");
            h.update(requester_pseudonym.as_bytes());
            h.update(b"|");
            h.update(&[u8::from(*granted)]);
        }
        VoiceEvent::LocalJoined { .. } => {
            h.update(b"localjoin|");
            h.update(&now_bucket.to_le_bytes());
        }
        VoiceEvent::SpeakingChanged {
            pseudonym,
            speaking,
            ..
        } => {
            // No time bucket: speaking flips many times a second and
            // every flip is meaningful, so bucketing would swallow the
            // stop that follows a start inside the same window.
            h.update(b"speaking|");
            h.update(pseudonym.as_bytes());
            h.update(b"|");
            h.update(&[u8::from(*speaking)]);
        }
        VoiceEvent::DeviceChanged {
            device_type,
            device_name,
            reason,
        } => {
            h.update(b"device|");
            h.update(device_type.as_bytes());
            h.update(b"|");
            h.update(device_name.as_bytes());
            h.update(b"|");
            h.update(reason.as_bytes());
        }
        VoiceEvent::PacketsDropped { reason, .. } => {
            // Not the count: it climbs monotonically, so including it
            // would make every report unique and defeat the dedup.
            h.update(b"drops|");
            h.update(reason.as_bytes());
            h.update(b"|");
            h.update(&now_bucket.to_le_bytes());
        }
        VoiceEvent::ConnectionQuality { quality, .. } => {
            // The verdict, not the counters — same reason as above.
            h.update(b"quality|");
            h.update(quality.as_bytes());
            h.update(b"|");
            h.update(&now_bucket.to_le_bytes());
        }
    }
}
