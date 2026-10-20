//! Random identifier bytes, from one place.
//!
//! Two copies of this existed — `rekindle-channel`'s used
//! `rand::random()` and `src-tauri`'s used `OsRng::fill_bytes`. Both are
//! cryptographically sound sources, which is exactly why the difference
//! went unnoticed: nothing was wrong, there were just two answers to one
//! question, and `check-duplicate-bodies` cannot see it because the
//! bodies genuinely differ.
//!
//! `OsRng` is the surviving one. It reads OS entropy directly rather
//! than through a userspace generator, so there is no reseeding policy
//! to reason about at a call site that only wants an identifier.

use rand::RngCore;

/// Randomness as the engines read it for identifiers and keys (plan C9.2):
/// the production [`OsRandom`], or a seeded generator, so a seed fixes the
/// trace. Every implementation is a cryptographic generator (`CryptoRng`).
/// AEAD nonces never come from this seam: they stay on `OsRng` inside
/// `rekindle-secrets`.
pub trait RandomSource: rand::CryptoRng + Send + Sync {
    /// Fill `dest` with random bytes.
    fn fill(&self, dest: &mut [u8]);

    /// A random `u32`.
    fn next_u32(&self) -> u32 {
        let mut bytes = [0u8; 4];
        self.fill(&mut bytes);
        u32::from_le_bytes(bytes)
    }

    /// A uniform value in `[0, bound)`; `0` when `bound` is `0`. Rejection
    /// sampling, so no value is favoured.
    fn below(&self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        let zone = u64::MAX - (u64::MAX % bound);
        loop {
            let mut bytes = [0u8; 8];
            self.fill(&mut bytes);
            let value = u64::from_le_bytes(bytes);
            if value < zone {
                return value % bound;
            }
        }
    }
}

/// The operating system's entropy.
#[derive(Debug, Clone, Copy, Default)]
pub struct OsRandom;

impl rand::CryptoRng for OsRandom {}

impl RandomSource for OsRandom {
    fn fill(&self, dest: &mut [u8]) {
        rand::rngs::OsRng.fill_bytes(dest);
    }
}

/// A generator of its own, seeded from another's (ChaCha12, `StdRng`): what
/// a unit of work draws from when it runs off the async task that made it
/// (plan E2.4: a ratchet or prekey step inside a database transaction runs
/// on the database's thread). The parent is drawn once, in the caller's
/// order, so a seeded parent fixes every draw whatever the threads do.
#[derive(Debug)]
pub struct ForkedRandom(parking_lot::Mutex<rand::rngs::StdRng>);

impl ForkedRandom {
    /// A generator seeded with 32 bytes from `parent`.
    #[must_use]
    pub fn fork(parent: &dyn RandomSource) -> Self {
        use rand::SeedableRng;
        let mut seed = [0u8; 32];
        parent.fill(&mut seed);
        let rng = rand::rngs::StdRng::from_seed(seed);
        seed.fill(0);
        Self(parking_lot::Mutex::new(rng))
    }
}

impl rand::CryptoRng for ForkedRandom {}

impl RandomSource for ForkedRandom {
    fn fill(&self, dest: &mut [u8]) {
        self.0.lock().fill_bytes(dest);
    }
}

/// 16 random bytes from `rng`, for identifiers an engine mints.
#[must_use]
pub fn id_bytes_16_from(rng: &dyn RandomSource) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    rng.fill(&mut bytes);
    bytes
}

/// 16 random bytes — the shape of a `ChannelId`, `CategoryId` or
/// `RoleId`.
#[must_use]
pub fn id_bytes_16() -> [u8; 16] {
    id_bytes_16_from(&OsRandom)
}

/// `len` random bytes, for nonces and salts.
#[must_use]
pub fn bytes(len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    rand::rngs::OsRng.fill_bytes(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::{OsRandom, RandomSource};

    #[test]
    fn a_fork_follows_its_parent_only() {
        use super::ForkedRandom;
        let fork = |parent: &dyn RandomSource| {
            let child = ForkedRandom::fork(parent);
            (child.next_u32(), child.next_u32())
        };
        let (a, b) = (OsRandom, OsRandom);
        assert_ne!(fork(&a), fork(&b));
    }

    #[test]
    fn below_stays_in_range() {
        for bound in [1u64, 2, 3, 121, 300] {
            for _ in 0..200 {
                assert!(OsRandom.below(bound) < bound);
            }
        }
        assert_eq!(OsRandom.below(0), 0);
    }
}
