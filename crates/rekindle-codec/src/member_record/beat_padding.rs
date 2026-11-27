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

use std::num::NonZeroUsize;

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
    // ⌈cost / 7⌉, at least one word since cost ≥ MIN_COST.
    let words = NonZeroUsize::MIN.saturating_add(cost.saturating_sub(1) / MAX_WORD_COST);
    let mut out = vec![[0u8; 8]; words.get()];
    // Spread the cost evenly: every word costs `cost / words` or one more,
    // which stays within [MIN_COST, MAX_WORD_COST] since
    // 2·words ≤ cost ≤ 7·words, so the word's `cost - 1` nonzero bytes
    // (1 to 6) neither underflow nor overflow it.
    let (base, extra) = (cost / words, cost % words);
    for (i, word) in out.iter_mut().enumerate() {
        let nonzero = base.saturating_sub(1).saturating_add(usize::from(i < extra));
        let mut drawn = vec![0u8; nonzero];
        rng.fill(&mut drawn);
        for (byte, value) in word.iter_mut().zip(drawn) {
            // A zero drawn would pack away; 1..=255 keeps the cost exact.
            *byte = value.max(1);
        }
    }
    out.into_flattened()
}

#[cfg(test)]
#[path = "beat_padding_tests.rs"]
mod tests;
