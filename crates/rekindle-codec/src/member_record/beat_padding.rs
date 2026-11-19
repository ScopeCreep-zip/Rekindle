//! The beat's padding: random bytes whose *packed* cost is known exactly,
//! so the beat can be brought to its fixed size (plan E3.4 Q2: zero
//! padding packs away, so the padding is sized after packing).
//!
//! Cap'n Proto packing (the encoding spec, "Packing") writes each 8-byte
//! word as a tag byte plus its nonzero bytes. A word with eight nonzero
//! bytes (tag `0xff`) is followed by a count of raw words that each hold at
//! most one zero byte, and an all-zero word by a count of zero words. A
//! padding word here holds `n` random nonzero bytes (1 ≤ n ≤ 6) and
//! `8 - n` zero bytes: it never starts or joins either run, so it packs to
//! exactly `1 + n` bytes whatever precedes it. A padding of `k` such words
//! therefore packs to the sum of its words' costs, any total from `2k` to
//! `7k`, and [`padding`] picks the words that sum to the cost asked.

use rekindle_utils::random::RandomSource;

/// The smallest padding cost: one word with one nonzero byte.
pub(super) const MIN_COST: usize = 2;
/// The most a word can cost and still never take part in a run.
const MAX_WORD_COST: usize = 7;
/// Re-measurements before the encoder gives up. Each re-signs, and only a
/// signature whose zero bytes pack differently from the last moves the
/// size, so two or three suffice; the bound only stops a loop.
pub(super) const MAX_ATTEMPTS: usize = 32;

/// Padding words that pack to exactly `cost` bytes (`cost ≥ MIN_COST`),
/// their nonzero bytes drawn from `rng`.
pub(super) fn padding(cost: usize, rng: &dyn RandomSource) -> Vec<u8> {
    let cost = cost.max(MIN_COST);
    let words = cost.div_ceil(MAX_WORD_COST);
    let mut out = vec![0u8; words * 8];
    // Spread the cost evenly: every word costs `cost / words` or one more,
    // which stays within [MIN_COST, MAX_WORD_COST] since
    // 2·words ≤ cost ≤ 7·words.
    let (base, extra) = (cost / words, cost % words);
    for (i, word) in out.chunks_exact_mut(8).enumerate() {
        let nonzero = base + usize::from(i < extra) - 1;
        let bytes = &mut word[..nonzero];
        rng.fill(bytes);
        for byte in bytes.iter_mut() {
            // A zero drawn would pack away; 1..=255 keeps the cost exact.
            if *byte == 0 {
                *byte = 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rekindle_utils::random::OsRandom;

    fn pack_list(bytes: &[u8]) -> usize {
        let mut builder = capnp::message::Builder::new_default();
        builder.set_root::<capnp::data::Owned>(bytes).unwrap();
        crate::capnp_codec::pack(&builder).len()
    }

    /// The padding's packed cost, measured against a list of the same
    /// length (so the same header and list pointer) whose every word holds
    /// one nonzero byte, which packs to 2 bytes a word.
    fn packed_cost(padding: &[u8]) -> usize {
        let mut reference = vec![0u8; padding.len()];
        for word in reference.chunks_exact_mut(8) {
            word[0] = 0x5a;
        }
        pack_list(padding) + 2 * (padding.len() / 8) - pack_list(&reference)
    }

    #[test]
    fn every_padding_packs_to_exactly_its_cost() {
        for cost in MIN_COST..=400 {
            let pad = padding(cost, &OsRandom);
            assert_eq!(pad.len() % 8, 0);
            assert_eq!(packed_cost(&pad), cost, "cost {cost}");
        }
    }
}
