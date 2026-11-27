//! Phase 14 — deterministic MCU host election (architecture §10.2 lines 2017-2019).
//!
//! When a voice channel exceeds the full-mesh threshold, every
//! participant independently picks the same relay peer: "the member
//! whose pseudonym has the lowest XOR distance to the channel_id hash."
//! Computing this on every peer with the same input always yields the
//! same answer, so no coordination round-trip is needed.
//!
//! The hash is BLAKE3 of the channel_id bytes; XOR distance is the
//! standard Kademlia metric — bytewise XOR compared as a big-endian
//! unsigned integer, which `[u8; 32]` lexicographic ordering implements
//! directly.
//!
//! Pre-Phase-14 lived in `src-tauri/services/voice/election.rs` (pure
//! logic, no Tauri/AppState deps — moved verbatim).

/// Hash a channel identifier to a 32-byte target for XOR-distance
/// comparisons. `channel_id` is the wire-form string the voice stack
/// passes around (UUID-hex or DHT key); we hash the raw bytes so this
/// works for both representations without the caller needing to parse.
pub fn channel_target(channel_id: &str) -> [u8; 32] {
    blake3::hash(channel_id.as_bytes()).into()
}

/// Pick the candidate pseudonym with the lowest XOR distance to
/// `target`. Pseudonyms are 32-byte hex strings as carried by
/// `CommunityState`. Bad-hex candidates are skipped (returns `None` if
/// none decode cleanly).
pub fn elect_relay_host<'a, I>(candidates: I, target: &[u8; 32]) -> Option<String>
where
    I: IntoIterator<Item = &'a String>,
{
    candidates
        .into_iter()
        .filter_map(|hex_pk| decode_pseudonym(hex_pk).map(|bytes| (hex_pk, bytes)))
        .min_by_key(|(_, bytes)| xor_distance(bytes, target))
        .map(|(hex_pk, _)| hex_pk.clone())
}

fn decode_pseudonym(hex_pk: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(hex_pk).ok()?;
    bytes.as_slice().try_into().ok()
}

fn xor_distance(a: &[u8; 32], b: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (slot, (left, right)) in out.iter_mut().zip(a.iter().zip(b.iter())) {
        *slot = left ^ right;
    }
    out
}

#[cfg(test)]
#[path = "election_tests.rs"]
mod tests;
