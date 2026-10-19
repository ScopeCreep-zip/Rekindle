//! Group DM MEK ratchet (architecture §27.2).
//!
//! A group DM's genesis MEK is random and wrapped per recipient in its
//! `GroupDmInvite`. Rotation uses a forward-secure ratchet:
//!
//! ```text
//! mek_n+1 = HKDF-SHA256(mek_n, salt: empty, info: b"rekindle-dm-ratchet-v1")
//! ```
//!
//! Triggered every 100 messages or 24 hours, whichever comes first
//! (the caller is responsible for invoking `ratchet_dm_mek`).

use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroize;

use crate::error::DmError;
use rekindle_types::domains;

pub const MEK_LEN: usize = 32;

/// 32-byte symmetric MEK material. Caller passes this to a
/// MediaEncryptionKey wrapper (in `rekindle-crypto`) for actual
/// AES-GCM operations.
#[derive(Clone, Zeroize)]
pub struct DmMek(pub [u8; MEK_LEN]);

impl DmMek {
    pub fn as_bytes(&self) -> &[u8; MEK_LEN] {
        &self.0
    }
}

impl Drop for DmMek {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for DmMek {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DmMek").field("len", &MEK_LEN).finish()
    }
}

/// Forward-secure ratchet: derive next-generation MEK from the previous.
pub fn ratchet_dm_mek(prev: &DmMek) -> Result<DmMek, DmError> {
    let hk = Hkdf::<Sha256>::new(None, prev.as_bytes());
    let mut next = [0u8; MEK_LEN];
    hk.expand(domains::DM_MEK_RATCHET.as_bytes(), &mut next)
        .map_err(|e| DmError::Hkdf(e.to_string()))?;
    Ok(DmMek(next))
}

/// Lazily-materialized chain of ratchet generations starting from a
/// genesis MEK. Forward architecture for the spec at §27.1: each peer
/// ratchets *independently* at the 100-message / 24-hour mark, but
/// every receiver must still be able to decrypt messages from any
/// generation the sender has ever used (architecture §5.2 line 1100,
/// §5.3 line 1186 — generation is in the envelope, receiver caches
/// historical MEKs).
///
/// The chain stores generation N at index N. On `for_generation`, any
/// missing generations are derived forward from the highest known and
/// memoized so the next lookup is O(1).
#[derive(Clone)]
pub struct DmMekChain {
    cache: Vec<DmMek>,
    current_gen: u64,
}

impl DmMekChain {
    /// Build a fresh chain rooted at the group's genesis MEK.
    pub fn new(genesis: DmMek) -> Self {
        Self {
            cache: vec![genesis],
            current_gen: 0,
        }
    }

    /// Build a chain whose current generation is already advanced to
    /// `current_gen`. Used when restoring from SQLite — the persisted
    /// `mek_generation` column is the chain's tip.
    pub fn restore(genesis: DmMek, current_gen: u64) -> Result<Self, DmError> {
        let mut chain = Self::new(genesis);
        if current_gen > 0 {
            chain.materialize_through(current_gen)?;
            chain.current_gen = current_gen;
        }
        Ok(chain)
    }

    /// Get the MEK for `gen`, deriving any missing generations.
    pub fn for_generation(&mut self, gen: u64) -> Result<&DmMek, DmError> {
        self.materialize_through(gen)?;
        let idx = usize::try_from(gen).unwrap_or(usize::MAX);
        self.cache
            .get(idx)
            .ok_or_else(|| DmError::Hkdf(format!("generation {gen} out of range")))
    }

    /// Current outbound (writer) generation and its MEK.
    pub fn current(&mut self) -> Result<(u64, &DmMek), DmError> {
        let gen = self.current_gen;
        let mek = self.for_generation(gen)?;
        Ok((gen, mek))
    }

    /// Advance our writer generation by one (architecture §27.1 trigger).
    /// Returns the new generation.
    pub fn advance(&mut self) -> Result<u64, DmError> {
        let next = self.current_gen.saturating_add(1);
        self.materialize_through(next)?;
        self.current_gen = next;
        Ok(next)
    }

    /// React to a received message tagged at `observed`. Forward-locks
    /// our outbound generation to at least `observed` so future writes
    /// never go backward (monotonic convergence — the highest peer's
    /// ratchet wins). Also pre-derives the MEK so a future decrypt is
    /// O(1).
    pub fn observed_generation(&mut self, observed: u64) -> Result<(), DmError> {
        self.materialize_through(observed)?;
        if observed > self.current_gen {
            self.current_gen = observed;
        }
        Ok(())
    }

    fn materialize_through(&mut self, gen: u64) -> Result<(), DmError> {
        let target = usize::try_from(gen).unwrap_or(usize::MAX);
        while self.cache.len() <= target {
            let prev = self.cache.last().expect("genesis always present");
            let next = ratchet_dm_mek(prev)?;
            self.cache.push(next);
        }
        Ok(())
    }
}

impl std::fmt::Debug for DmMekChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DmMekChain")
            .field("current_gen", &self.current_gen)
            .field("cached_through", &self.cache.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn genesis() -> DmMek {
        DmMek([7u8; MEK_LEN])
    }

    #[test]
    fn ratchet_advances_mek() {
        let mek0 = genesis();
        let mek1 = ratchet_dm_mek(&mek0).unwrap();
        assert_ne!(mek0.as_bytes(), mek1.as_bytes());
        let mek2 = ratchet_dm_mek(&mek1).unwrap();
        assert_ne!(mek1.as_bytes(), mek2.as_bytes());
    }

    #[test]
    fn chain_lazy_materialize() {
        let expected = ratchet_dm_mek(&ratchet_dm_mek(&genesis()).unwrap()).unwrap();
        let mut chain = DmMekChain::new(genesis());
        let got = chain.for_generation(2).unwrap();
        assert_eq!(got.as_bytes(), expected.as_bytes());
    }

    #[test]
    fn chain_observed_locks_forward() {
        let mut chain = DmMekChain::new(genesis());
        chain.observed_generation(5).unwrap();
        assert_eq!(chain.current().unwrap().0, 5);
        // Observing a lower gen does NOT roll us back.
        chain.observed_generation(2).unwrap();
        assert_eq!(chain.current().unwrap().0, 5);
    }

    #[test]
    fn chain_advance_is_monotonic() {
        let mut chain = DmMekChain::new(genesis());
        assert_eq!(chain.advance().unwrap(), 1);
        assert_eq!(chain.advance().unwrap(), 2);
        assert_eq!(chain.current().unwrap().0, 2);
    }

    #[test]
    fn chain_restore_resumes_at_persisted_generation() {
        let chain = DmMekChain::restore(genesis(), 3).unwrap();
        let derived = {
            let mut c = DmMekChain::new(genesis());
            c.observed_generation(3).unwrap();
            c
        };
        assert_eq!(chain.current_gen, derived.current_gen);
    }
}
