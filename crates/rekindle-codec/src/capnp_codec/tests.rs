use super::{account, friend, identity, presence, voice};

#[test]
fn round_trip_presence_update() {
    use super::presence::GameInfo;

    let game = GameInfo {
        game_id: 42,
        game_name: "Counter-Strike".to_string(),
        server_info: Some("de_dust2 @ 192.168.1.1:27015".to_string()),
        elapsed_seconds: 3600,
        server_address: Some("192.168.1.1:27015".to_string()),
    };

    let encoded = presence::encode_update(1, Some(&game));
    let (status, game_opt) = presence::decode_update(&encoded).unwrap();
    assert_eq!(status, 1);
    let g = game_opt.unwrap();
    assert_eq!(g.game_id, 42);
    assert_eq!(g.game_name, "Counter-Strike");
    assert_eq!(
        g.server_info,
        Some("de_dust2 @ 192.168.1.1:27015".to_string())
    );
    assert_eq!(g.elapsed_seconds, 3600);
    assert_eq!(g.server_address, Some("192.168.1.1:27015".to_string()));

    // Without game
    let encoded2 = presence::encode_update(3, None);
    let (status2, game2) = presence::decode_update(&encoded2).unwrap();
    assert_eq!(status2, 3);
    assert!(game2.is_none());
}

#[test]
fn round_trip_friend_request() {
    let req = friend::FriendRequest {
        sender_key: vec![0xAA; 32],
        display_name: "FriendlyUser".to_string(),
        message: "Let's be friends!".to_string(),
        prekey_bundle: vec![0xBB; 128],
    };

    let encoded = friend::encode_request(&req);
    let decoded = friend::decode_request(&encoded).unwrap();

    assert_eq!(decoded.sender_key, req.sender_key);
    assert_eq!(decoded.display_name, "FriendlyUser");
    assert_eq!(decoded.message, "Let's be friends!");
    assert_eq!(decoded.prekey_bundle, req.prekey_bundle);
}

#[test]
fn round_trip_friend_list() {
    use crate::friend::FriendEntry;

    let entries = vec![
        FriendEntry {
            public_key: "abc123".to_string(),
            nickname: Some("Buddy".to_string()),
            group: Some("Gaming".to_string()),
            added_at: 1000,
            profile_dht_key: Some("dht_key_1".to_string()),
        },
        FriendEntry {
            public_key: "def456".to_string(),
            nickname: None,
            group: None,
            added_at: 2000,
            profile_dht_key: None,
        },
    ];

    let encoded = friend::encode_friend_list(&entries);
    let decoded = friend::decode_friend_list(&encoded).unwrap();

    assert_eq!(decoded.len(), 2);
    assert_eq!(decoded[0].public_key, "abc123");
    assert_eq!(decoded[0].nickname, Some("Buddy".to_string()));
    assert_eq!(decoded[0].group, Some("Gaming".to_string()));
    assert_eq!(decoded[0].added_at, 1000);
    // The encoder once dropped this: the schema had four fields while the
    // struct had five, so `profile_dht_key` came back `None`.
    assert_eq!(decoded[0].profile_dht_key, Some("dht_key_1".to_string()));

    assert_eq!(decoded[1].public_key, "def456");
    assert_eq!(decoded[1].nickname, None);
    assert_eq!(decoded[1].group, None);
    assert_eq!(decoded[1].profile_dht_key, None);
}

#[test]
fn round_trip_voice_signaling() {
    let sig = voice::VoiceSignaling {
        signal_type: voice::SignalType::Offer,
        channel_id: "voice-room-1".to_string(),
        sender_key: vec![0x42; 32],
        payload: b"SDP data here".to_vec(),
    };

    let encoded = voice::encode_signaling(&sig);
    let decoded = voice::decode_signaling(&encoded).unwrap();

    assert_eq!(decoded.signal_type, voice::SignalType::Offer);
    assert_eq!(decoded.channel_id, "voice-room-1");
    assert_eq!(decoded.sender_key, sig.sender_key);
    assert_eq!(decoded.payload, b"SDP data here");
}

#[test]
fn round_trip_game_status() {
    use super::presence::GameInfo;

    let info = GameInfo {
        game_id: 7,
        game_name: "Team Fortress 2".to_string(),
        server_info: Some("2fort @ 10.0.0.1:27015".to_string()),
        elapsed_seconds: 7200,
        server_address: Some("10.0.0.1:27015".to_string()),
    };

    let encoded = presence::encode_game_status(&info);
    let decoded = presence::decode_game_status(&encoded).unwrap();

    assert_eq!(decoded.game_id, 7);
    assert_eq!(decoded.game_name, "Team Fortress 2");
    assert_eq!(
        decoded.server_info,
        Some("2fort @ 10.0.0.1:27015".to_string())
    );
    assert_eq!(decoded.elapsed_seconds, 7200);
    assert_eq!(decoded.server_address, Some("10.0.0.1:27015".to_string()));
}

#[test]
fn round_trip_account_header() {
    let header = account::AccountHeader {
        display_name: "xXGamerXx".to_string(),
        status_message: "Playing games".to_string(),
        avatar_hash: vec![0xDE, 0xAD],
        created_at: 1000,
        updated_at: 2000,
    };

    let encoded = account::encode_account_header(&header);
    let decoded = account::decode_account_header(&encoded).unwrap();

    assert_eq!(decoded.display_name, "xXGamerXx");
    assert_eq!(decoded.status_message, "Playing games");
    assert_eq!(decoded.avatar_hash, vec![0xDE, 0xAD]);
    assert_eq!(decoded.created_at, 1000);
    assert_eq!(decoded.updated_at, 2000);
}

/// An account with no avatar set must still decode.
///
/// `avatar_hash` is the one optional field left in the header — the
/// encoder skips it when empty, so this is the shape every account has
/// before the user picks a picture.
#[test]
fn account_header_without_avatar() {
    let header = account::AccountHeader {
        display_name: "no_avatar".to_string(),
        status_message: String::new(),
        avatar_hash: Vec::new(),
        created_at: 7,
        updated_at: 7,
    };

    let decoded = account::decode_account_header(&account::encode_account_header(&header)).unwrap();

    assert_eq!(decoded.display_name, "no_avatar");
    assert!(decoded.status_message.is_empty());
    assert!(decoded.avatar_hash.is_empty());
}

/// A head at its worst case: maximal ids, a full ML-KEM-768 key.
fn full_head() -> identity::PrekeyRecordHead {
    identity::PrekeyRecordHead {
        spk_id: 0x7FFF_FFFF,
        spk: [1; 32],
        spk_sig: [2; 64],
        pq_lr_id: 0x7FFF_FFFF,
        pq_lr: vec![3; 1_184],
        pq_lr_sig: [4; 64],
        issued_at: i64::MAX,
    }
}

/// A slot at its worst case.
fn full_slot() -> identity::OpkSlot {
    identity::OpkSlot {
        opk_id: 0x7FFF_FFFF,
        opk: [5; 32],
        opk_sig: [6; 64],
        pq_opk_id: 0x7FFF_FFFF,
        pq_opk: vec![7; 1_184],
        pq_opk_sig: [8; 64],
    }
}

#[test]
fn the_prekey_record_round_trips() {
    let head = full_head();
    assert_eq!(
        identity::decode_prekey_head(&identity::encode_prekey_head(&head)).unwrap(),
        head
    );
    let slot = full_slot();
    assert_eq!(
        identity::decode_opk_slot(&identity::encode_opk_slot(&slot)).unwrap(),
        slot
    );
    let list = identity::PrekeyRecordList {
        devices: vec![identity::PrekeyDevice {
            device_id: [9; 16],
            record_key: format!("VLD0:{}", "A".repeat(43)),
        }],
    };
    assert_eq!(
        identity::decode_prekey_record_list(&identity::encode_prekey_record_list(&list)).unwrap(),
        list
    );
    let empty = identity::PrekeyRecordList::default();
    assert_eq!(
        identity::decode_prekey_record_list(&identity::encode_prekey_record_list(&empty)).unwrap(),
        empty
    );
}

/// Each subkey value fits the DFLT(101) record's per-subkey cap.
#[test]
fn every_prekey_record_value_fits_its_subkey() {
    use rekindle_types::dht_layout::prekeys::MAX_SUBKEY_VALUE_LEN;
    let head = identity::encode_prekey_head(&full_head()).len();
    let slot = identity::encode_opk_slot(&full_slot()).len();
    let list = identity::encode_prekey_record_list(&identity::PrekeyRecordList {
        devices: vec![identity::PrekeyDevice {
            device_id: [0xFF; 16],
            record_key: format!("VLD0:{}", "A".repeat(43)),
        }],
    })
    .len();
    for (what, len) in [("head", head), ("slot", slot), ("device list", list)] {
        assert!(len <= MAX_SUBKEY_VALUE_LEN, "{what} {len} B");
    }
}

#[test]
fn a_short_prekey_or_signature_is_refused() {
    let mut builder = capnp::message::Builder::new_default();
    {
        let mut root = builder.init_root::<crate::identity_capnp::opk_slot::Builder<'_>>();
        root.set_opk(&[1; 31]);
        root.set_opk_sig(&[2; 64]);
        root.set_pq_opk_sig(&[3; 64]);
    }
    assert!(identity::decode_opk_slot(&super::pack(&builder)).is_err());
    assert!(identity::decode_prekey_head(b"not a head").is_err());
}
