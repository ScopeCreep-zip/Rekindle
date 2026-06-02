//! Pure MEK-generation matching + decrypt (drained from
//! `legacy/membership/join.rs`). No AppState / I/O — the caller passes a
//! snapshot of the MEK cache.

use std::collections::HashMap;
use std::hash::BuildHasher;

use rekindle_crypto::group::media_key::MediaEncryptionKey;

/// Outcome of attempting to decrypt a community message with the cached
/// MEK for its community.
pub enum MekDecryptResult {
    /// Decrypted plaintext (lossy UTF-8).
    Decrypted(String),
    /// We hold a MEK but at the wrong generation — caller should refresh.
    NeedRefresh,
    /// No usable MEK / decryption failed.
    Failed,
}

/// Decrypt `ciphertext` for `community_id` using the cached MEK, checking
/// that the cached generation matches `mek_generation`.
#[must_use]
pub fn decrypt_with_cached_mek<S: BuildHasher>(
    mek_cache: &HashMap<String, MediaEncryptionKey, S>,
    community_id: &str,
    ciphertext: &[u8],
    mek_generation: u64,
) -> MekDecryptResult {
    match mek_cache.get(community_id) {
        Some(mek) if mek.generation() == mek_generation => match mek.decrypt(ciphertext) {
            Ok(plaintext) => {
                MekDecryptResult::Decrypted(String::from_utf8(plaintext).unwrap_or_default())
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to decrypt community message");
                MekDecryptResult::Failed
            }
        },
        Some(mek) => {
            tracing::warn!(
                have = mek.generation(),
                need = mek_generation,
                "MEK generation mismatch — fetching updated MEK from DHT vault"
            );
            MekDecryptResult::NeedRefresh
        }
        None => {
            tracing::warn!(community = %community_id, "no MEK cached for community");
            MekDecryptResult::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_with(
        community: &str,
        plaintext: &[u8],
    ) -> (HashMap<String, MediaEncryptionKey>, Vec<u8>) {
        let mek = MediaEncryptionKey::generate(7);
        let ct = mek.encrypt(plaintext).expect("encrypt");
        let mut cache = HashMap::new();
        cache.insert(community.to_string(), mek);
        (cache, ct)
    }

    #[test]
    fn decrypts_on_matching_generation() {
        let (cache, ct) = cache_with("c1", b"hello");
        match decrypt_with_cached_mek(&cache, "c1", &ct, 7) {
            MekDecryptResult::Decrypted(text) => assert_eq!(text, "hello"),
            _ => panic!("expected Decrypted"),
        }
    }

    #[test]
    fn needs_refresh_on_generation_mismatch() {
        let (cache, ct) = cache_with("c1", b"hello");
        assert!(matches!(
            decrypt_with_cached_mek(&cache, "c1", &ct, 9),
            MekDecryptResult::NeedRefresh
        ));
    }

    #[test]
    fn fails_when_no_mek_cached() {
        let (cache, ct) = cache_with("c1", b"hello");
        assert!(matches!(
            decrypt_with_cached_mek(&cache, "other", &ct, 7),
            MekDecryptResult::Failed
        ));
    }
}
