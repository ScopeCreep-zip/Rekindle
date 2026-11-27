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
    let ads = decode_member_history_ads(&bytes, &key, at, &me.verifying_key().to_bytes()).unwrap();
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
