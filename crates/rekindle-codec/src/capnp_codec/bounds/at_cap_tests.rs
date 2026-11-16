//! The other side of every bound: the largest legitimate value of each type
//! that its carrier delivers still decodes. Each value has the real schema
//! shape and realistic field data (random key and ciphertext bytes, hex and
//! record-key text, small timestamps and offsets), and is grown until one
//! more element or byte would no longer fit the carrier.

use rekindle_types::dht_layout::{prekeys, MAX_APP_MESSAGE_LEN, MAX_SUBKEY_SIZE};
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::video::Codec;

use crate::capnp_codec::{account, chain, friend, identity, presence, voice};
use crate::capnp_codec::{transport_feedback::TransportFeedback, voice_packet::VoicePacket};
use crate::capnp_envelope::{
    decode_community_envelope, decode_governance_entry, decode_signed_envelope,
    encode_community_envelope, encode_governance_entry, encode_signed_envelope,
};
use crate::community::envelope::{
    CommunityEnvelope, ControlPayload, SignedEnvelope, VideoFragmentPayload,
};

/// Deterministic high-entropy bytes (`SplitMix64`), the shape of keys,
/// signatures and ciphertext.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        out.extend_from_slice(&(z ^ (z >> 31)).to_le_bytes());
    }
    out.truncate(len);
    out
}

fn array<const N: usize>(seed: u64) -> [u8; N] {
    noise(seed, N).try_into().expect("N bytes")
}

/// Printable ASCII, the shape of names, SDP and policy text.
fn text(seed: u64, len: usize) -> String {
    noise(seed, len)
        .into_iter()
        .map(|b| char::from(b' ' + b % 95))
        .collect()
}

fn hex(seed: u64, bytes: usize) -> String {
    use std::fmt::Write as _;
    noise(seed, bytes).iter().fold(String::new(), |mut out, b| {
        write!(out, "{b:02x}").expect("a String takes writes");
        out
    })
}

/// A typed Veilid record key as the app spells it.
fn record_key(seed: u64) -> String {
    format!("VLD0:{}", &hex(seed, 32)[..43])
}

/// The encoding of the largest `n` whose encoding fits `cap`, which must
/// fill at least nine tenths of it.
fn largest_fitting(cap: usize, encode: impl Fn(usize) -> Vec<u8>) -> Vec<u8> {
    let (mut lo, mut hi) = (0, cap);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if encode(mid).len() <= cap {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let bytes = encode(lo);
    assert!(
        bytes.len() > cap - cap / 10,
        "{} of {cap} bytes",
        bytes.len()
    );
    bytes
}

#[test]
fn a_full_signed_envelope_decodes() {
    let bytes = largest_fitting(MAX_APP_MESSAGE_LEN, |n| {
        encode_signed_envelope(&SignedEnvelope {
            community_id: hex(1, 32),
            sender_pseudonym: hex(2, 32),
            envelope_bytes: noise(3, n),
            signature: noise(4, 64),
            ttl: 5,
        })
    });
    decode_signed_envelope(&bytes).unwrap();
}

#[test]
fn a_full_community_envelope_decodes() {
    let bytes = largest_fitting(MAX_APP_MESSAGE_LEN, |n| {
        let fragment = VideoFragmentPayload {
            channel_id: hex(5, 16),
            stream_id: array(6),
            frame_seq: 48_211,
            frag_index: 3,
            frag_total: 7,
            keyframe: true,
            codec: Codec::Vp9,
            timestamp: 3_600_000,
            key_index: 12,
            payload: noise(7, n),
            signature: noise(8, 64),
        };
        encode_community_envelope(&CommunityEnvelope::Control(ControlPayload::VideoFragment(
            fragment,
        )))
        .unwrap()
    });
    decode_community_envelope(&bytes).unwrap();
}

#[test]
fn a_full_governance_entry_decodes() {
    let bytes = largest_fitting(MAX_APP_MESSAGE_LEN, |n| {
        encode_governance_entry(&GovernanceEntry::CommunityPolicy {
            policy_text: Some(text(9, n)),
            max_joins_per_interval: 20,
            join_interval_seconds: 3_600,
        })
    });
    decode_governance_entry(&bytes).unwrap();
}

#[test]
fn a_full_voice_packet_decodes() {
    let bytes = largest_fitting(MAX_APP_MESSAGE_LEN, |n| {
        VoicePacket {
            sender_key: noise(10, 32),
            sequence: 90_210,
            timestamp: 1_792_860_061_000,
            sframe: noise(11, n),
            sig: noise(12, 64),
        }
        .encode()
    });
    VoicePacket::decode(&bytes).unwrap();
}

/// Arrival offsets are small: milliseconds from the report's first
/// arrival, so most of each word's high bytes are zero.
#[test]
fn a_full_transport_feedback_decodes() {
    let bytes = largest_fitting(MAX_APP_MESSAGE_LEN, |n| {
        TransportFeedback {
            reporter_key: noise(13, 32),
            begin_seq: 7_001,
            report_time_ms: 1_792_860_061_000,
            arrivals: (0..n)
                .map(|i| {
                    let i = u16::try_from(i % 3_000).unwrap();
                    if i % 17 == 0 {
                        crate::capnp_codec::transport_feedback::NOT_RECEIVED
                    } else {
                        i * 20 % 997
                    }
                })
                .collect(),
            sig: noise(14, 64),
        }
        .encode()
    });
    TransportFeedback::decode(&bytes).unwrap();
}

#[test]
fn a_full_voice_signaling_message_decodes() {
    let bytes = largest_fitting(MAX_APP_MESSAGE_LEN, |n| {
        voice::encode_signaling(&voice::VoiceSignaling {
            signal_type: voice::SignalType::Offer,
            channel_id: hex(15, 16),
            sender_key: noise(16, 32),
            payload: text(17, n).into_bytes(),
        })
    });
    voice::decode_signaling(&bytes).unwrap();
}

/// The chain head is bounded by its schema: the longest record key
/// spelling and the largest sequence numbers and retire mark fit.
#[test]
fn the_largest_chain_head_decodes() {
    let pointer = chain::PagePointer {
        page_key: format!("VLD0:{}", hex(18, 48)),
        first_seq: u64::MAX - 1,
        last_seq: u64::MAX,
    };
    let head = chain::ChainHead {
        newest: pointer,
        retired_through: u64::MAX,
    };
    let bytes = chain::encode_chain_head(&head);
    assert_eq!(chain::decode_chain_head(&bytes).unwrap(), head);
}

/// A page of the smallest entries a chain holds, sealed frames of a few
/// dozen bytes, packs worst; larger entries pack nearer one to one.
#[test]
fn a_full_chain_page_decodes() {
    for entry_len in [24, 96, 1_200] {
        let bytes = largest_fitting(MAX_SUBKEY_SIZE, |n| {
            chain::encode_chain_page(&chain::ChainPage {
                prev: Some(chain::PagePointer {
                    page_key: record_key(19),
                    first_seq: 4_096,
                    last_seq: 4_351,
                }),
                first_seq: 4_352,
                entries: (0..n).map(|i| noise(20 + i as u64, entry_len)).collect(),
            })
        });
        chain::decode_chain_page(&bytes).unwrap();
    }
}

fn game(seed: u64, name_len: usize) -> presence::GameInfo {
    presence::GameInfo {
        game_id: 4_101,
        game_name: text(seed, name_len),
        server_info: Some(text(seed + 1, 64)),
        elapsed_seconds: 7_260,
        server_address: Some("203.0.113.40:27015".into()),
    }
}

#[test]
fn a_full_presence_update_and_game_status_decode() {
    let update = largest_fitting(MAX_SUBKEY_SIZE, |n| {
        presence::encode_update(2, Some(&game(21, n)), 1_792_860_061_000)
    });
    presence::decode_update(&update).unwrap();
    let status = largest_fitting(MAX_SUBKEY_SIZE, |n| {
        presence::encode_game_status(&game(22, n))
    });
    presence::decode_game_status(&status).unwrap();
}

/// The prekey head and slot have a fixed shape: an X25519 key, an
/// ML-KEM-768 key (1,184 bytes) and two signatures.
#[test]
fn the_prekey_head_slot_and_a_full_device_list_decode() {
    let head = identity::PrekeyRecordHead {
        spk_id: u32::MAX,
        spk: array(23),
        spk_sig: array(24),
        pq_lr_id: u32::MAX,
        pq_lr: noise(25, 1_184),
        pq_lr_sig: array(26),
        issued_at: 1_792_860_061_000,
    };
    let bytes = identity::encode_prekey_head(&head);
    assert!(bytes.len() <= prekeys::MAX_SUBKEY_VALUE_LEN);
    assert_eq!(identity::decode_prekey_head(&bytes).unwrap(), head);
    let slot = identity::OpkSlot {
        opk_id: u32::MAX,
        opk: array(27),
        opk_sig: array(28),
        pq_opk_id: u32::MAX,
        pq_opk: noise(29, 1_184),
        pq_opk_sig: array(30),
    };
    let bytes = identity::encode_opk_slot(&slot);
    assert!(bytes.len() <= prekeys::MAX_SUBKEY_VALUE_LEN);
    assert_eq!(identity::decode_opk_slot(&bytes).unwrap(), slot);
    let list = largest_fitting(prekeys::MAX_SUBKEY_VALUE_LEN, |n| {
        identity::encode_prekey_record_list(&identity::PrekeyRecordList {
            devices: (0..n as u64)
                .map(|i| identity::PrekeyDevice {
                    device_id: array(31 + i),
                    record_key: record_key(10_000 + i),
                })
                .collect(),
        })
    });
    identity::decode_prekey_record_list(&list).unwrap();
}

#[test]
fn a_full_friend_list_decodes() {
    let bytes = largest_fitting(MAX_SUBKEY_SIZE, |n| {
        let entries: Vec<_> = (0..n as u64)
            .map(|i| crate::friend::FriendEntry {
                public_key: hex(i, 32),
                nickname: (i % 3 == 0).then(|| text(i + 1, 12)),
                group: (i % 2 == 0).then(|| "Gaming".to_owned()),
                added_at: 1_792_860_061_000 + i,
                profile_dht_key: Some(record_key(i + 2)),
            })
            .collect();
        friend::encode_friend_list(&entries)
    });
    friend::decode_friend_list(&bytes).unwrap();
}

#[test]
fn a_full_account_header_decodes() {
    let bytes = largest_fitting(MAX_SUBKEY_SIZE, |n| {
        account::encode_account_header(&account::AccountHeader {
            display_name: text(40, 32),
            status_message: text(41, n),
            avatar_hash: noise(42, 32),
            created_at: 1_792_860_061_000,
            updated_at: 1_792_860_062_000,
        })
    });
    account::decode_account_header(&bytes).unwrap();
}
