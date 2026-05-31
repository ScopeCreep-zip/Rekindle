//! Phase 16 — deterministic stream_id derivation per
//! architecture §10.6 line 2063.
//!
//! Pure blake3-keyed hash over `(channel_id, sender_pseudonym,
//! track_label)`. Same member streaming twice (camera + screen
//! share concurrently) passes distinct labels — typically `"camera"`
//! and `"screen"` — to avoid stream_id collisions.

/// Compute the deterministic stream_id for
/// `(channel_id, sender_pseudonym, track_label)` per architecture §10.6
/// line 2063. Two members streaming in the same channel get distinct
/// stream_ids automatically (different `sender_pseudonym`); the same
/// member streaming twice (e.g. camera + screen share at once) passes
/// distinct `track_label`s — typically `"camera"` and `"screen"`. The
/// label is hashed in alongside the other inputs so it can be any UTF-8
/// string the caller picks.
#[must_use]
pub fn derive_stream_id(
    channel_id: &str,
    sender_pseudonym_hex: &str,
    track_label: &str,
) -> [u8; 16] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(channel_id.as_bytes());
    hasher.update(b"|");
    hasher.update(sender_pseudonym_hex.as_bytes());
    hasher.update(b"|");
    hasher.update(track_label.as_bytes());
    let digest = hasher.finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&digest.as_bytes()[..16]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAMERA: &str = "camera";
    const SCREEN: &str = "screen";

    #[test]
    fn camera_and_screen_share_get_distinct_stream_ids() {
        let camera = derive_stream_id("ch1", "alice", CAMERA);
        let screen = derive_stream_id("ch1", "alice", SCREEN);
        assert_ne!(camera, screen);
    }

    #[test]
    fn same_label_is_deterministic_across_calls() {
        let a = derive_stream_id("ch1", "alice", CAMERA);
        let b = derive_stream_id("ch1", "alice", CAMERA);
        assert_eq!(a, b);
    }

    #[test]
    fn distinct_senders_get_distinct_stream_ids() {
        let alice = derive_stream_id("ch1", "alice", CAMERA);
        let bob = derive_stream_id("ch1", "bob", CAMERA);
        assert_ne!(alice, bob);
    }
}
