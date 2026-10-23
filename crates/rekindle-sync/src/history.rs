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
mod tests {
    use super::{decode_member_history_ads, select_best_peer, HistoryAd};
    use rekindle_codec::member_record::history_ad::encode_history_ad;
    use rekindle_codec::member_record::RecordScope;
    use rekindle_secrets::ed25519_dalek::SigningKey;
    use rekindle_secrets::keys::MediaEncryptionKey;
    use rekindle_types::id::ChannelId;
    use rekindle_types::presence::HistoryRange;
    use rekindle_utils::random::OsRandom;

    #[test]
    fn a_member_history_ad_decodes_into_ads() {
        let at = RecordScope {
            community_id: "community",
            member_record: "VLD0:member",
        };
        let me = SigningKey::from_bytes(&[1; 32]);
        let key = MediaEncryptionKey::from_bytes([2; 32], 4);
        let range = HistoryRange {
            channel_id: ChannelId([0xAB; 16]),
            oldest_lamport: 3,
            newest_lamport: 9,
        };
        let bytes = encode_history_ad(&[range], &key, at, &me, &OsRandom).unwrap();
        let ads =
            decode_member_history_ads(&bytes, &key, at, &me.verifying_key().to_bytes()).unwrap();
        assert_eq!(
            ads,
            vec![HistoryAd {
                channel_id: "ab".repeat(16),
                oldest_lamport: 3,
                newest_lamport: 9,
            }]
        );
        assert_eq!(select_best_peer(&[(0, &ads[0])], 5), Some(0));
    }

    #[test]
    fn range_contains_endpoints() {
        let ad = HistoryAd {
            channel_id: "chan".into(),
            oldest_lamport: 10,
            newest_lamport: 20,
        };
        assert!(ad.contains(10));
        assert!(ad.contains(15));
        assert!(ad.contains(20));
        assert!(!ad.contains(9));
    }

    #[test]
    fn select_best_peer_finds_covering_range() {
        let ad0 = HistoryAd {
            channel_id: "ch".into(),
            oldest_lamport: 1,
            newest_lamport: 50,
        };
        let ad1 = HistoryAd {
            channel_id: "ch".into(),
            oldest_lamport: 40,
            newest_lamport: 100,
        };
        // Peer 1 covers lamport 75, peer 0 does not
        assert_eq!(select_best_peer(&[(0, &ad0), (1, &ad1)], 75), Some(1));
        // Peer 0 covers lamport 25, peer 1 does not
        assert_eq!(select_best_peer(&[(0, &ad0), (1, &ad1)], 25), Some(0));
    }

    #[test]
    fn select_best_peer_prefers_widest_range() {
        let narrow = HistoryAd {
            channel_id: "ch".into(),
            oldest_lamport: 10,
            newest_lamport: 20,
        };
        let wide = HistoryAd {
            channel_id: "ch".into(),
            oldest_lamport: 5,
            newest_lamport: 30,
        };
        // Both cover lamport 15, but peer 1 has wider range
        assert_eq!(select_best_peer(&[(0, &narrow), (1, &wide)], 15), Some(1));
    }

    #[test]
    fn select_best_peer_returns_none_when_no_coverage() {
        let ad = HistoryAd {
            channel_id: "ch".into(),
            oldest_lamport: 1,
            newest_lamport: 50,
        };
        assert_eq!(select_best_peer(&[(0, &ad)], 75), None);
    }

    #[test]
    fn select_best_peer_empty_ads() {
        assert_eq!(select_best_peer(&[], 10), None);
    }

    #[test]
    fn select_deepest_peer_picks_smallest_oldest_lamport() {
        let shallow = HistoryAd {
            channel_id: "ch".into(),
            oldest_lamport: 40,
            newest_lamport: 100,
        };
        let deep = HistoryAd {
            channel_id: "ch".into(),
            oldest_lamport: 1,
            newest_lamport: 50,
        };
        assert_eq!(
            super::select_deepest_peer(&[(0, &shallow), (1, &deep)]),
            Some(1)
        );
        assert_eq!(super::select_deepest_peer(&[]), None);
    }
}
