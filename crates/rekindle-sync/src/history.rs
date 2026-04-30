//! Lightweight history range advertisements for presence sync.
//!
//! Each member advertises which message ranges they have cached locally
//! (the "Shared Locker" pattern from the Chiral Network model). Newcomers
//! or recovering peers can then target specific peers for message catchup
//! instead of blind-fetching from the DHT.

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

#[cfg(test)]
mod tests {
    use super::{select_best_peer, HistoryAd};

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
}
