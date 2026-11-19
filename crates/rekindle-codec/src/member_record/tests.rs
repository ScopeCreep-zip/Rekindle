use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_secrets::keys::MediaEncryptionKey;
use rekindle_types::id::ChannelId;
use rekindle_types::presence::{
    GameInfo, HistoryRange, SessionExtras, SessionLocation, SessionStatus,
};
use rekindle_utils::random::OsRandom;

use super::beat::{decode_presence_beat, MAX_ACTIVITY, MAX_CHANNEL_ID};
use super::card::{
    decode_member_card, verify_card_hpke, MAX_GAME_NAME_LEN, MAX_SERVER_ADDRESS_LEN,
};
use super::history_ad::{decode_history_ad, encode_history_ad};
use super::*;
use rekindle_types::game_server::MAX_GAME_ID_LEN;
use rekindle_types::key_format::MAX_STATUS_LEN;
use rekindle_types::presence::limits::{
    MAX_BADGES, MAX_BADGE_LEN, MAX_BIO_LEN, MAX_CONTENT_REF_LEN, MAX_DISPLAY_NAME_LEN,
    MAX_PRONOUNS_LEN,
};

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

/// `n` characters of four UTF-8 bytes each: free text's worst case.
fn wide(n: usize) -> String {
    "\u{1F525}".repeat(n)
}

fn max_card() -> MemberCard {
    MemberCard {
        display_name: Some(wide(MAX_DISPLAY_NAME_LEN)),
        bio: Some(wide(MAX_BIO_LEN)),
        avatar_ref: Some(s(MAX_CONTENT_REF_LEN)),
        banner_ref: Some(s(MAX_CONTENT_REF_LEN)),
        pronouns: Some(wide(MAX_PRONOUNS_LEN)),
        theme_color: Some(u32::MAX),
        badges: vec![s(MAX_BADGE_LEN); MAX_BADGES],
        custom_status: Some(wide(MAX_STATUS_LEN)),
        game: Some(GameInfo {
            game_name: wide(MAX_GAME_NAME_LEN),
            game_id: Some(wide(MAX_GAME_ID_LEN)),
            elapsed_seconds: Some(u64::MAX),
            server_address: Some(s(MAX_SERVER_ADDRESS_LEN)),
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
    card.bio = Some(wide(MAX_BIO_LEN + 1));
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

fn beat(extras: Option<SealedExtras>, voice: Option<String>) -> PresenceBeat {
    PresenceBeat {
        seq: 7,
        state: BeatState::Busy,
        voice_channel: voice,
        beat_number: 5_871_000,
        extras,
    }
}

#[test]
fn beat_round_trips_at_exactly_512_bytes_at_the_maximum_field_sizes() {
    let me = key(1);
    let mek = MediaEncryptionKey::from_bytes([9; 32], u64::MAX);
    let extras = max_extras();
    let beat = PresenceBeat {
        seq: u64::MAX,
        beat_number: u64::MAX,
        ..beat(
            Some(SealedExtras::seal(&extras, &mek, AT, &OsRandom).unwrap()),
            Some(s(MAX_CHANNEL_ID)),
        )
    };
    let bytes = beat.encode(AT, &me, &OsRandom).unwrap();
    assert_eq!(bytes.len(), PRESENCE_BEAT_BYTES);
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

/// Every beat packs to exactly 512 B, whatever its fields hold: an empty
/// beat, every voice-channel length, extras sealed over every activity
/// length, and the extreme counters (plan E3.4 Q2).
#[test]
fn every_beat_packs_to_exactly_512_bytes_whatever_its_field_lengths() {
    let me = key(3);
    let mek = MediaEncryptionKey::from_bytes([4; 32], 2);
    let mut sizes = std::collections::BTreeSet::new();
    for voice_len in 0..=MAX_CHANNEL_ID {
        let voice = (voice_len > 0).then(|| s(voice_len));
        for activity in [None, Some(0), Some(1), Some(17), Some(MAX_ACTIVITY)] {
            let extras = activity.map(|n| {
                let body = SessionExtras {
                    location: Some(SessionLocation::Text {
                        channel_id: s(voice_len.max(1)),
                    }),
                    activity: Some(s(n)),
                };
                SealedExtras::seal(&body, &mek, AT, &OsRandom).unwrap()
            });
            for (seq, number) in [(0, 0), (1, 300), (u64::MAX, u64::MAX)] {
                let b = PresenceBeat {
                    seq,
                    beat_number: number,
                    ..beat(extras.clone(), voice.clone())
                };
                let bytes = b.encode(AT, &me, &OsRandom).unwrap();
                sizes.insert(bytes.len());
                assert_eq!(decode_presence_beat(&bytes, AT, &pk(&me)).unwrap(), b);
            }
        }
    }
    assert_eq!(sizes.into_iter().collect::<Vec<_>>(), [PRESENCE_BEAT_BYTES]);
}

/// A beat of any other size is refused before it is read, and a beat
/// whose signed bytes changed (its padding included) is refused.
#[test]
fn a_beat_of_the_wrong_size_or_signature_is_refused() {
    let me = key(1);
    let bytes = beat(None, None).encode(AT, &me, &OsRandom).unwrap();
    let mut short = bytes.clone();
    short.pop();
    assert!(decode_presence_beat(&short, AT, &pk(&me)).is_err());
    let mut long = bytes.clone();
    long.push(0);
    assert!(decode_presence_beat(&long, AT, &pk(&me)).is_err());
    // The last byte is padding: a flip there breaks the signature.
    let mut flipped = bytes.clone();
    *flipped.last_mut().unwrap() ^= 0x01;
    assert!(decode_presence_beat(&flipped, AT, &pk(&me)).is_err());
    assert!(decode_presence_beat(&bytes, AT, &pk(&key(2))).is_err());
}

#[test]
fn an_invisible_beat_is_written_offline_and_departed_round_trips() {
    let me = key(1);
    let invisible = PresenceBeat {
        state: SessionStatus::Invisible.into(),
        ..beat(None, None)
    };
    let decoded =
        decode_presence_beat(&invisible.encode(AT, &me, &OsRandom).unwrap(), AT, &pk(&me)).unwrap();
    assert_eq!(decoded.state, BeatState::Offline);
    assert!(!decoded.state.is_present());
    assert_eq!(decoded.extras, None);
    let departed = PresenceBeat {
        state: BeatState::Departed,
        ..beat(None, None)
    };
    let decoded =
        decode_presence_beat(&departed.encode(AT, &me, &OsRandom).unwrap(), AT, &pk(&me)).unwrap();
    assert_eq!(decoded.state, BeatState::Departed);
    assert!(!decoded.state.is_present());
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
