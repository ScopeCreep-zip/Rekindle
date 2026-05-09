//! Safety number ceremony.
//!
//! Distinct from Signal's `NumericFingerprintGenerator` (5200 HMAC-SHA-256
//! iterations). Rekindle uses a single SHA-512 derivation over both
//! X25519 identity keys — cheaper, unambiguously distinct from libsignal,
//! and equally safe at the same digit length since both 32-byte keys are
//! full preimage (no phone-number brute-force surface).
//!
//! Output: 60 ASCII decimal digits (12 groups of 5), matching Signal's
//! display format for user familiarity.

use aws_lc_rs::digest;
use subtle::ConstantTimeEq;

/// A 60-digit safety number for out-of-band verification.
#[derive(Clone)]
pub struct SafetyNumber(pub [u8; 60]);

impl SafetyNumber {
    /// Compute the safety number for a peer pair.
    ///
    /// Canonical ordering: the lexicographically smaller key is first,
    /// so `safety_number(A, B) == safety_number(B, A)`.
    pub fn compute(ik_a: &[u8; 32], ik_b: &[u8; 32]) -> Self {
        let (lo, hi) = if ik_a <= ik_b {
            (ik_a, ik_b)
        } else {
            (ik_b, ik_a)
        };

        let mut ctx = digest::Context::new(&digest::SHA512);
        ctx.update(b"Rekindle Safety Number v1");
        ctx.update(lo);
        ctx.update(hi);
        let hash = ctx.finish(); // 64 bytes

        let bytes = hash.as_ref();
        let mut out = [0u8; 60];

        // Convert 30 bytes (6 groups of 5 bytes) into 60 decimal digits
        for group in 0..12 {
            let base = group * 5;
            // Use 5 bytes per 5-digit group (40 bits → mod 100000)
            let byte_offset = (group * 5) / 2;
            let mut v = 0u64;
            for j in 0..5 {
                let idx = byte_offset + j;
                if idx < bytes.len() {
                    v = (v << 8) | u64::from(bytes[idx]);
                }
            }
            let n = (v % 100_000) as u32;
            for d in 0..5 {
                let digit = (n / 10u32.pow(4 - d)) % 10;
                out[base + d as usize] = b'0' + digit as u8;
            }
        }

        Self(out)
    }

    /// Constant-time comparison to prevent timing oracle on verification.
    pub fn verify(&self, other: &SafetyNumber) -> bool {
        bool::from(self.0.ct_eq(&other.0))
    }

    /// Format as 12 groups of 5 digits separated by spaces.
    pub fn display(&self) -> String {
        self.0
            .chunks(5)
            .map(|chunk| std::str::from_utf8(chunk).unwrap_or("?????"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

impl std::fmt::Debug for SafetyNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safety_number_is_symmetric() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let sn_ab = SafetyNumber::compute(&a, &b);
        let sn_ba = SafetyNumber::compute(&b, &a);
        assert!(sn_ab.verify(&sn_ba));
    }

    #[test]
    fn different_keys_different_numbers() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let c = [3u8; 32];
        let sn_ab = SafetyNumber::compute(&a, &b);
        let sn_ac = SafetyNumber::compute(&a, &c);
        assert!(!sn_ab.verify(&sn_ac));
    }

    #[test]
    fn display_format_is_12_groups_of_5() {
        let a = [42u8; 32];
        let b = [99u8; 32];
        let sn = SafetyNumber::compute(&a, &b);
        let display = sn.display();
        let groups: Vec<&str> = display.split(' ').collect();
        assert_eq!(groups.len(), 12);
        for g in &groups {
            assert_eq!(g.len(), 5);
            assert!(g.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn output_is_60_digits() {
        let a = [0u8; 32];
        let b = [255u8; 32];
        let sn = SafetyNumber::compute(&a, &b);
        assert_eq!(sn.0.len(), 60);
        assert!(sn.0.iter().all(|&b| b >= b'0' && b <= b'9'));
    }
}
