use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::keys::MediaEncryptionKey;
use rekindle_types::id::ChannelId;
use rekindle_types::presence::{
    GameInfo, HistoryRange, SessionExtras, SessionLocation, SessionStatus,
};
use rekindle_utils::random::OsRandom;

use super::beat::{decode_presence_beat, MAX_ACTIVITY, MAX_CHANNEL_ID};
use super::card::{
    decode_member_card, verify_card_hpke, MAX_BADGE, MAX_BADGES, MAX_BIO, MAX_CUSTOM_STATUS,
    MAX_DISPLAY_NAME, MAX_GAME_ID, MAX_GAME_NAME, MAX_PRONOUNS, MAX_REF, MAX_SERVER_ADDRESS,
};
use super::history_ad::{decode_history_ad, encode_history_ad};
use super::*;

const AT: RecordScope<'static> = RecordScope {
    community_id: "community-0123456789abcdef0123456789abcdef",
    member_record: "VLD0:abcdefghijklmnopqrstuvwxyz0123456789ABCDEFG",
};
const OTHER_RECORD: RecordScope<'static> = RecordScope {
    community_id: AT.community_id,
    member_record: "VLD0:another-member-record",
};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn pk(key: &SigningKey) -> [u8; 32] {
    key.verifying_key().to_bytes()
}

fn s(n: usize) -> String {
    "z".repeat(n)
}

fn max_card() -> MemberCard {
    MemberCard {
        display_name: Some(s(MAX_DISPLAY_NAME)),
        bio: Some(s(MAX_BIO)),
        avatar_ref: Some(s(MAX_REF)),
        banner_ref: Some(s(MAX_REF)),
        pronouns: Some(s(MAX_PRONOUNS)),
        theme_color: Some(u32::MAX),
        badges: vec![s(MAX_BADGE); MAX_BADGES],
        custom_status: Some(s(MAX_CUSTOM_STATUS)),
        game: Some(GameInfo {
            game_name: s(MAX_GAME_NAME),
            game_id: Some(s(MAX_GAME_ID)),
            elapsed_seconds: Some(u64::MAX),
            server_address: Some(s(MAX_SERVER_ADDRESS)),
        }),
        hpke_key: [0xAB; 32],
    }
}

#[test]
fn card_round_trips_within_its_bound_at_the_maximum_field_sizes() {
    let me = key(1);
    let card = max_card();
    let bytes = card.encode(AT, &me).unwrap();
    assert!(
        bytes.len() <= MEMBER_CARD_MAX_BYTES,
        "{} bytes",
        bytes.len()
    );
    let signed = decode_member_card(&bytes, AT, &pk(&me)).unwrap();
    assert_eq!(signed.card, card);
    verify_card_hpke(AT.community_id, &card.hpke_key, &signed.hpke_sig, &pk(&me)).unwrap();
    assert!(verify_card_hpke("elsewhere", &card.hpke_key, &signed.hpke_sig, &pk(&me)).is_err());
}

#[test]
fn a_minimal_card_round_trips_and_a_card_is_bound_to_its_record_and_signer() {
    let me = key(1);
    let card = MemberCard {
        display_name: None,
        bio: None,
        avatar_ref: None,
        banner_ref: None,
        pronouns: None,
        theme_color: None,
        badges: Vec::new(),
        custom_status: None,
        game: None,
        hpke_key: [3; 32],
    };
    let bytes = card.encode(AT, &me).unwrap();
    assert_eq!(decode_member_card(&bytes, AT, &pk(&me)).unwrap().card, card);
    assert!(decode_member_card(&bytes, OTHER_RECORD, &pk(&me)).is_err());
    assert!(decode_member_card(&bytes, AT, &pk(&key(2))).is_err());
}

#[test]
fn a_card_field_over_its_bound_is_refused() {
    let mut card = max_card();
    card.bio = Some(s(MAX_BIO + 1));
    assert!(card.encode(AT, &key(1)).is_err());
    let mut card = max_card();
    card.badges.push(s(1));
    assert!(card.encode(AT, &key(1)).is_err());
}

fn max_extras() -> SessionExtras {
    SessionExtras {
        location: Some(SessionLocation::Voice {
            channel_id: s(MAX_CHANNEL_ID),
        }),
        activity: Some(s(MAX_ACTIVITY)),
    }
}

#[test]
fn beat_round_trips_within_512_bytes_at_the_maximum_field_sizes() {
    let me = key(1);
    let mek = MediaEncryptionKey::from_bytes([9; 32], u64::MAX);
    let extras = max_extras();
    let beat = PresenceBeat {
        seq: u64::MAX,
        status: SessionStatus::Busy,
        voice_channel: Some(s(MAX_CHANNEL_ID)),
        expires_at: u64::MAX,
        extras: Some(SealedExtras::seal(&extras, &mek, AT, &OsRandom).unwrap()),
    };
    let bytes = beat.encode(AT, &me).unwrap();
    assert!(
        bytes.len() <= PRESENCE_BEAT_MAX_BYTES,
        "{} bytes",
        bytes.len()
    );
    let decoded = decode_presence_beat(&bytes, AT, &pk(&me)).unwrap();
    assert_eq!(decoded, beat);
    let sealed = decoded.extras.unwrap();
    assert_eq!(sealed.open(&mek, AT).unwrap(), extras);
    // Bound to its record, and unreadable after a rotation.
    assert!(sealed.open(&mek, OTHER_RECORD).is_err());
    let rotated = MediaEncryptionKey::from_bytes([9; 32], 0);
    assert!(sealed.open(&rotated, AT).is_err());
    assert!(decode_presence_beat(&bytes, OTHER_RECORD, &pk(&me)).is_err());
    assert!(decode_presence_beat(&bytes, AT, &pk(&key(2))).is_err());
}

#[test]
fn an_invisible_beat_is_written_offline_and_extras_are_optional() {
    let me = key(1);
    let beat = PresenceBeat {
        seq: 1,
        status: SessionStatus::Invisible,
        voice_channel: None,
        expires_at: 10,
        extras: None,
    };
    let decoded = decode_presence_beat(&beat.encode(AT, &me).unwrap(), AT, &pk(&me)).unwrap();
    assert_eq!(decoded.status, SessionStatus::Offline);
    assert_eq!(decoded.extras, None);
    let mut over = max_extras();
    over.activity = Some(s(MAX_ACTIVITY + 1));
    let mek = MediaEncryptionKey::from_bytes([9; 32], 1);
    assert!(SealedExtras::seal(&over, &mek, AT, &OsRandom).is_err());
}

fn ranges(n: usize) -> Vec<HistoryRange> {
    (0..n)
        .map(|i| HistoryRange {
            channel_id: ChannelId([u8::try_from(i % 255).unwrap() + 1; 16]),
            oldest_lamport: u64::MAX - 1,
            newest_lamport: u64::MAX,
        })
        .collect()
}

#[test]
fn history_ad_round_trips_within_4_kib_at_the_maximum_range_count() {
    let me = key(1);
    let mek = MediaEncryptionKey::from_bytes([5; 32], u64::MAX);
    let all = ranges(HISTORY_AD_MAX_RANGES);
    let bytes = encode_history_ad(&all, &mek, AT, &me, &OsRandom).unwrap();
    assert!(bytes.len() <= HISTORY_AD_MAX_BYTES, "{} bytes", bytes.len());
    assert_eq!(decode_history_ad(&bytes, &mek, AT, &pk(&me)).unwrap(), all);
    assert!(
        encode_history_ad(&ranges(HISTORY_AD_MAX_RANGES + 1), &mek, AT, &me, &OsRandom).is_err()
    );
}

#[test]
fn a_history_ad_is_bound_to_its_record_signer_and_generation() {
    let me = key(1);
    let mek = MediaEncryptionKey::from_bytes([5; 32], 7);
    let bytes = encode_history_ad(&ranges(3), &mek, AT, &me, &OsRandom).unwrap();
    assert!(decode_history_ad(&bytes, &mek, OTHER_RECORD, &pk(&me)).is_err());
    assert!(decode_history_ad(&bytes, &mek, AT, &pk(&key(2))).is_err());
    // A member left holding the replaced key after a rotation reads nothing.
    let replaced = MediaEncryptionKey::from_bytes([5; 32], 6);
    assert!(decode_history_ad(&bytes, &replaced, AT, &pk(&me)).is_err());
    let forged_key = MediaEncryptionKey::from_bytes([6; 32], 7);
    assert!(decode_history_ad(&bytes, &forged_key, AT, &pk(&me)).is_err());
    let empty = encode_history_ad(&[], &mek, AT, &me, &OsRandom).unwrap();
    assert!(decode_history_ad(&empty, &mek, AT, &pk(&me))
        .unwrap()
        .is_empty());
}
