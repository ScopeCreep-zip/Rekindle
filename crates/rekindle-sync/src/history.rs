//! Lightweight history range advertisements for presence sync.
//!
//! Each member advertises which message ranges they have cached locally
//! (the "Shared Locker" pattern from the Chiral Network model). Newcomers
//! or recovering peers can then target specific peers for message catchup
//! instead of blind-fetching from the DHT.

use rekindle_codec::member_record::history_ad::decode_history_ad;
use rekindle_codec::member_record::RecordScope;
use rekindle_codec::CodecError;
use rekindle_secrets::keys::MediaEncryptionKey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryAd {
    pub channel_id: String,
    pub oldest_lamport: u64,
    pub newest_lamport: u64,
}

impl HistoryAd {
    pub fn contains(&self, lamport: u64) -> bool {
        self.oldest_lamport <= lamport && lamport <= self.newest_lamport
    }
}

/// Select the best peer to request messages from for a given lamport value.
///
/// Takes a slice of `(peer_index, &HistoryAd)` pairs and a needed lamport
/// timestamp. Returns the `peer_index` of the peer whose advertised range
/// covers the needed value. If multiple peers cover it, returns the one
/// with the widest range (most history). Returns `None` if no peer covers
/// the needed value.
pub fn select_best_peer(ads: &[(usize, &HistoryAd)], needed_lamport: u64) -> Option<usize> {
    ads.iter()
        .filter(|(_, ad)| ad.contains(needed_lamport))
        .max_by_key(|(_, ad)| ad.newest_lamport - ad.oldest_lamport)
        .map(|(idx, _)| *idx)
}

/// Select the peer offering the deepest backfill when the caller holds no
/// local history yet (no specific lamport to target).
///
/// Returns the `peer_index` whose advertised range reaches furthest back —
/// the smallest `oldest_lamport`, i.e. the most complete history. `None`
/// when there are no candidates.
#[must_use]
pub fn select_deepest_peer(ads: &[(usize, &HistoryAd)]) -> Option<usize> {
    ads.iter()
        .min_by_key(|(_, ad)| ad.oldest_lamport)
        .map(|(idx, _)| *idx)
}

/// Decode a member's `M`/5 history ad (plan E3.2) into the ads peer
/// selection ranks. The codec verifies the pseudonym's signature for the
/// record `at` and opens the ranges with `key`, the current
/// `KeyScope::Community` key: a member without it reads no ads.
pub fn decode_member_history_ads(
    data: &[u8],
    key: &MediaEncryptionKey,
    at: RecordScope<'_>,
    pseudonym: &[u8; 32],
) -> Result<Vec<HistoryAd>, CodecError> {
    Ok(decode_history_ad(data, key, at, pseudonym)?
        .into_iter()
        .map(|range| HistoryAd {
            channel_id: hex::encode(range.channel_id.0),
            oldest_lamport: range.oldest_lamport,
            newest_lamport: range.newest_lamport,
        })
        .collect())
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
