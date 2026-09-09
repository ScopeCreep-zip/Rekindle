//! Community-envelope dedup-key extraction.

use rekindle_protocol::capnp_envelope::encode_community_envelope;
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;

/// Phase 20 — pure community-envelope dedup-key extractor.
///
/// Mirrors src-tauri's `extract_mesh_dedup_key`: returns a stable
/// string the dedup cache uses to gate duplicate broadcasts. Different
/// envelope variants use different bucketing strategies:
///
/// - `MessageNotification` — use the message_id directly
/// - `TypingIndicator` — 5-second buckets per channel + sender
/// - `PresenceUpdate` — 30-second buckets per sender
/// - `Control` — full 16-byte BLAKE2b hash of the encoded envelope
/// - `WatchRelay` — (record_key, subkey, content_hash) tuple
#[must_use]
pub fn extract_mesh_dedup_key(envelope: &CommunityEnvelope) -> String {
    match envelope {
        CommunityEnvelope::MessageNotification { message_id, .. } => message_id.clone(),
        CommunityEnvelope::TypingIndicator {
            channel_id,
            pseudonym_key,
        } => {
            let bucket = rekindle_utils::timestamp_secs() / 5;
            format!("typing:{channel_id}:{pseudonym_key}:{bucket}")
        }
        CommunityEnvelope::PresenceUpdate { pseudonym_key, .. } => {
            let bucket = rekindle_utils::timestamp_secs() / 30;
            format!("presence:{pseudonym_key}:{bucket}")
        }
        CommunityEnvelope::Control(_) => {
            use blake2::{digest::consts::U16, Blake2b, Digest};
            let bytes = encode_community_envelope(envelope).unwrap_or_default();
            let mut hash = Blake2b::<U16>::new();
            hash.update(&bytes);
            hex::encode(hash.finalize())
        }
        CommunityEnvelope::WatchRelay {
            record_key,
            subkey,
            content_hash,
            ..
        } => format!("watch:{record_key}:{subkey}:{content_hash}"),
    }
}

// `broadcast()` lived here: a generic, transport-agnostic fan-out that
// took closures instead of a `GossipDeps` impl. It predates the Deps
// trait and had no callers, and the Deps path is strictly more capable
// — `send_to_mesh_raw` ranks peers by reliability, applies the same
// `fanout_degree`, and queues for the next presence-poll cycle when the
// peer list is empty rather than dropping (architecture A1/P4.1). It
// was also the only thing importing `rekindle_types::error::GossipError`
// into this crate, which meant two different `GossipError`s were in
// scope here at once while `lib.rs` exported only one of them.
//
// This module now holds just the dedup-key extractor, which the
// transport dispatch path uses.
