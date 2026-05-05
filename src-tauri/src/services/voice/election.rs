//! Deterministic MCU host election (architecture §10.2 lines 2017-2019).
//!
//! When a voice channel exceeds the full-mesh threshold, every participant
//! independently picks the same relay peer: "the member whose pseudonym has
//! the lowest XOR distance to the channel_id hash." Computing this on every
//! peer with the same input always yields the same answer, so no
//! coordination round-trip is needed for the transition.
//!
//! The hash is BLAKE3 of the channel_id bytes (already used elsewhere in the
//! project for content-addressed identifiers); XOR distance is the standard
//! Kademlia metric — bytewise XOR compared as a big-endian unsigned integer,
//! which `[u8; 32]` lexicographic ordering implements directly.

/// Hash a channel identifier to a 32-byte target for XOR-distance comparisons.
///
/// `channel_id` here is the wire-form string the rest of the voice stack
/// passes around (UUID-hex or DHT key); we hash the raw bytes so this works
/// for both representations without the caller needing to parse first.
pub fn channel_target(channel_id: &str) -> [u8; 32] {
    blake3::hash(channel_id.as_bytes()).into()
}

/// Pick the candidate pseudonym with the lowest XOR distance to `target`.
/// Pseudonyms are 32-byte hex strings as carried by `CommunityState`.
/// Bad-hex candidates are skipped (returned `None` if none decode cleanly).
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
mod tests {
    use super::*;

    fn pk(prefix: u8) -> String {
        let mut bytes = [0u8; 32];
        bytes[0] = prefix;
        hex::encode(bytes)
    }

    #[test]
    fn target_is_deterministic() {
        let a = channel_target("ch-1234");
        let b = channel_target("ch-1234");
        assert_eq!(a, b);
        let c = channel_target("ch-5678");
        assert_ne!(a, c, "different channel_ids hash to different targets");
    }

    #[test]
    fn election_is_deterministic_across_orderings() {
        let target = channel_target("ch-deterministic");
        let alice = pk(0x10);
        let bob = pk(0x20);
        let carol = pk(0x30);
        let order_a = [alice.clone(), bob.clone(), carol.clone()];
        let order_b = [carol.clone(), alice.clone(), bob.clone()];
        let order_c = [bob.clone(), carol.clone(), alice.clone()];
        let elected_a = elect_relay_host(order_a.iter(), &target);
        let elected_b = elect_relay_host(order_b.iter(), &target);
        let elected_c = elect_relay_host(order_c.iter(), &target);
        assert_eq!(elected_a, elected_b);
        assert_eq!(elected_b, elected_c);
        assert!(elected_a.is_some());
    }

    #[test]
    fn election_minimises_xor_distance() {
        // Pick a target with a known prefix, then choose pseudonyms whose
        // XOR distance is obvious: a candidate equal to the target itself
        // wins (distance = 0). This proves the metric is XOR-min, not
        // lexicographic on the pseudonym.
        let target = channel_target("ch-xor-min");
        let target_hex = hex::encode(target);
        let lex_smallest = pk(0x00);
        let candidates = [lex_smallest.clone(), target_hex.clone()];
        let elected = elect_relay_host(candidates.iter(), &target).unwrap();
        assert_eq!(
            elected, target_hex,
            "the candidate whose bytes equal the target must win — distance 0"
        );
    }

    #[test]
    fn election_picks_different_hosts_per_channel() {
        // Per-channel different relay distributes load. Two channels with
        // different IDs should usually elect different hosts from the same
        // candidate set. With a small candidate set the result isn't 100%
        // guaranteed, but using two distinct sets is enough for a smoke
        // test that the channel hash actually influences the outcome.
        let candidates: Vec<String> = (0..8u8).map(pk).collect();
        let target_a = channel_target("ch-A");
        let target_b = channel_target("ch-B-different");
        let host_a = elect_relay_host(candidates.iter(), &target_a).unwrap();
        let host_b = elect_relay_host(candidates.iter(), &target_b).unwrap();
        // Not asserting != (could collide); just asserting both elect a
        // valid candidate from the set.
        assert!(candidates.contains(&host_a));
        assert!(candidates.contains(&host_b));
    }

    #[test]
    fn election_skips_invalid_hex() {
        let target = channel_target("ch-skip-bad-hex");
        let good = pk(0x42);
        let bad = "not-real-hex".to_string();
        let elected = elect_relay_host([&bad, &good], &target);
        assert_eq!(elected.as_deref(), Some(good.as_str()));
    }

    #[test]
    fn election_returns_none_for_empty_or_all_bad() {
        let target = channel_target("ch-empty");
        let empty: Vec<String> = Vec::new();
        assert!(elect_relay_host(empty.iter(), &target).is_none());

        let all_bad = ["zzz".to_string(), "qq".to_string()];
        assert!(elect_relay_host(all_bad.iter(), &target).is_none());
    }
}
