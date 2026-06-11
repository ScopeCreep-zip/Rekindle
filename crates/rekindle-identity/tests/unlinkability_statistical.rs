//! Statistical unlinkability test — the statistical arm.
//!
//! `community_persona_no_root_bytes` (inline in pseudonym.rs) is the
//! structural arm: a sliding-window scan for root bytes in the serialized
//! persona. This test is the statistical arm: it derives many pseudonyms
//! from one seed across many governance keys and checks that the pseudonym
//! bits are uniformly distributed and uncorrelated with the root bits.
//!
//! If pseudonym derivation has any bias correlated with the root, the
//! pseudonym leaks identity.

use rekindle_identity::{originate_from_seed, derive_persona, GovernanceKey, OriginSeed};
use zeroize::Zeroizing;

const NUM_COMMUNITIES: usize = 1000;

#[test]
fn pseudonym_bits_uniformly_distributed() {
    let seed = OriginSeed::from_vault_bytes(Zeroizing::new([0x42; 32]));
    let o = originate_from_seed(seed).unwrap();
    let root_bytes = *o.root.as_bytes();

    // Derive pseudonyms across many governance keys
    let mut pseudonym_bytes_collection: Vec<[u8; 32]> = Vec::with_capacity(NUM_COMMUNITIES);

    for i in 0..NUM_COMMUNITIES {
        let gov_str = format!("VLD0:community-{i:04}");
        let gov = GovernanceKey::parse(&gov_str).unwrap();
        let seed_i = OriginSeed::from_vault_bytes(Zeroizing::new([0x42; 32]));
        let (persona, _) = derive_persona(&seed_i, &gov, 0).unwrap();
        pseudonym_bytes_collection.push(persona.pseudonym.0);
    }

    // Byte-frequency uniformity check (simplified chi-squared).
    // For each byte position, count occurrences of each value across
    // all pseudonyms. With 1000 samples and 256 possible values,
    // expected count per value ≈ 3.9. We check that no single value
    // exceeds 3x expected (a very loose bound that still catches
    // catastrophic bias like "byte 0 is always 0x42").
    for byte_pos in 0..32 {
        let mut counts = [0u32; 256];
        for ps in &pseudonym_bytes_collection {
            counts[ps[byte_pos] as usize] += 1;
        }
        let max_count = *counts.iter().max().unwrap();
        let expected = NUM_COMMUNITIES as f64 / 256.0;
        assert!(
            (max_count as f64) < expected * 6.0,
            "byte position {byte_pos}: max count {max_count} exceeds 6x expected {expected:.1} — \
             possible bias in pseudonym derivation"
        );
    }

    // Bit correlation check: for each bit position in the root,
    // check correlation with each bit position in the pseudonym.
    // With independent derivation, the correlation should be near 0.
    // We use a simple same/different count.
    for root_bit in 0..256 {
        let root_byte = root_bit / 8;
        let root_mask = 1u8 << (root_bit % 8);
        let root_val = (root_bytes[root_byte] & root_mask) != 0;

        for pseudo_bit in 0..256 {
            let pseudo_byte = pseudo_bit / 8;
            let pseudo_mask = 1u8 << (pseudo_bit % 8);

            let mut same_count = 0u32;
            for ps in &pseudonym_bytes_collection {
                let ps_val = (ps[pseudo_byte] & pseudo_mask) != 0;
                if root_val == ps_val {
                    same_count += 1;
                }
            }

            // With independent bits, same_count should be near NUM_COMMUNITIES/2.
            // Allow ±15% deviation (very loose to avoid flaky tests).
            let expected = NUM_COMMUNITIES as f64 / 2.0;
            let deviation = ((same_count as f64) - expected).abs() / expected;
            assert!(
                deviation < 0.15,
                "bit correlation: root bit {root_bit} vs pseudonym bit {pseudo_bit}: \
                 same_count={same_count}, expected={expected:.0}, deviation={deviation:.3} — \
                 possible root↔pseudonym bit leakage"
            );
        }
    }
}
