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

use chacha20::cipher::{KeyIvInit, StreamCipher};
use chacha20::ChaCha20Legacy;
use rand::RngCore;
use zeroize::Zeroizing;

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

/// A generator of its own, seeded from another's: what a unit of work
/// draws from when it runs off the async task that made it (plan E2.4: a
/// ratchet or prekey step inside a database transaction runs on the
/// database's thread). The parent is drawn once, in the caller's order, so
/// a seeded parent fixes every draw whatever the threads do.
///
/// The generator is ChaCha20 (`chacha20`, the RustCrypto implementation,
/// its `zeroize` feature wiping the cipher state on drop) with fast-key
/// erasure (Bernstein, "Fast-key-erasure random-number generators", 2017):
/// every fill runs the keystream from the current key, hands out the bytes
/// and replaces the key with the next 32 keystream bytes. The only state
/// kept between fills is that 32-byte key, held in `Zeroizing`, so a copy
/// of the state taken after a fill cannot regenerate any byte already
/// handed out (audit C-1: these bytes become ratchet and prekey secrets,
/// I7 / ADR 0002 forward secrecy).
pub struct ForkedRandom(parking_lot::Mutex<Zeroizing<[u8; 32]>>);

impl ForkedRandom {
    /// A generator seeded with 32 bytes from `parent`.
    #[must_use]
    pub fn fork(parent: &dyn RandomSource) -> Self {
        let mut seed = Zeroizing::new([0u8; 32]);
        parent.fill(&mut *seed);
        Self(parking_lot::Mutex::new(seed))
    }
}

impl std::fmt::Debug for ForkedRandom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForkedRandom").finish_non_exhaustive()
    }
}

impl rand::CryptoRng for ForkedRandom {}

impl RandomSource for ForkedRandom {
    fn fill(&self, dest: &mut [u8]) {
        let mut key = self.0.lock();
        // A key is used for one fill only, so the fixed nonce never repeats
        // under a key.
        let mut cipher = ChaCha20Legacy::new(chacha20::Key::from_slice(&**key), &[0u8; 8].into());
        dest.fill(0);
        cipher.apply_keystream(dest);
        let mut next = Zeroizing::new([0u8; 32]);
        cipher.apply_keystream(&mut *next);
        **key = *next;
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

    /// A parent that hands out the same bytes every time.
    struct Fixed(u8);

    impl rand::CryptoRng for Fixed {}

    impl RandomSource for Fixed {
        fn fill(&self, dest: &mut [u8]) {
            dest.fill(self.0);
        }
    }

    fn draw(rng: &dyn RandomSource) -> [u8; 64] {
        let mut out = [0u8; 64];
        rng.fill(&mut out);
        out
    }

    #[test]
    fn one_parent_seed_gives_one_sequence() {
        use super::ForkedRandom;
        let (a, b) = (ForkedRandom::fork(&Fixed(7)), ForkedRandom::fork(&Fixed(7)));
        for _ in 0..4 {
            assert_eq!(draw(&a), draw(&b));
        }
        assert_ne!(draw(&ForkedRandom::fork(&Fixed(8))), draw(&b));
    }

    #[test]
    fn captured_state_cannot_regenerate_earlier_output() {
        use super::ForkedRandom;
        let rng = ForkedRandom::fork(&Fixed(7));
        let earlier = draw(&rng);
        // Everything the generator keeps after the fill, copied out as an
        // attacker who reads its memory would.
        let captured = rng.0.lock().clone();
        assert_ne!(*captured, [7u8; 32], "the seed was replaced");
        let copy = ForkedRandom::fork(&Fixed(0));
        *copy.0.lock() = captured;
        let mut replay = vec![0u8; 64 * 64];
        copy.fill(&mut replay);
        assert!(replay.windows(64).all(|w| w != earlier));
        // The copy can only follow the original forward.
        let copy = ForkedRandom::fork(&Fixed(0));
        *copy.0.lock() = rng.0.lock().clone();
        assert_eq!(draw(&copy), draw(&rng));
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
