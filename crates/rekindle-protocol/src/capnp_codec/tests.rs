use super::{account, conversation, friend, identity, message, presence, voice};

#[test]
fn round_trip_message_envelope() {
    use crate::messaging::envelope::MessageEnvelope;

    let env = MessageEnvelope {
        sender_key: vec![1u8; 32],
        timestamp: 1_234_567_890,
        nonce: vec![42u8; 16],
        payload: b"encrypted content".to_vec(),
        signature: vec![99u8; 64],
    };

    let encoded = message::encode_envelope(&env);
    let decoded = message::decode_envelope(&encoded).unwrap();

    assert_eq!(env.sender_key, decoded.sender_key);
    assert_eq!(env.timestamp, decoded.timestamp);
    assert_eq!(env.nonce, decoded.nonce);
    assert_eq!(env.payload, decoded.payload);
    assert_eq!(env.signature, decoded.signature);
}

#[test]
fn round_trip_chat_message() {
    let (body, reply) = message::decode_chat_message(&message::encode_chat_message(
        "hello world",
        Some(&[1, 2, 3]),
    ))
    .unwrap();
    assert_eq!(body, "hello world");
    assert_eq!(reply, Some(vec![1, 2, 3]));

    let (body2, reply2) =
        message::decode_chat_message(&message::encode_chat_message("no reply", None)).unwrap();
    assert_eq!(body2, "no reply");
    assert_eq!(reply2, None);
}

#[test]
fn round_trip_presence_update() {
    use crate::messaging::envelope::GameInfo;

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
fn round_trip_user_profile() {
    use crate::messaging::envelope::GameInfo;

    let profile = identity::UserProfile {
        display_name: "xXGamerXx".to_string(),
        status_message: "Playing games".to_string(),
        status: 0,
        avatar_hash: vec![0xDE, 0xAD],
        game_status: Some(GameInfo {
            game_id: 1,
            game_name: "Halo".to_string(),
            server_info: None,
            elapsed_seconds: 120,
            server_address: None,
        }),
    };

    let encoded = identity::encode_profile(&profile);
    let decoded = identity::decode_profile(&encoded).unwrap();

    assert_eq!(decoded.display_name, "xXGamerXx");
    assert_eq!(decoded.status_message, "Playing games");
    assert_eq!(decoded.status, 0);
    assert_eq!(decoded.avatar_hash, vec![0xDE, 0xAD]);
    let g = decoded.game_status.unwrap();
    assert_eq!(g.game_name, "Halo");
    assert_eq!(g.elapsed_seconds, 120);
}

#[test]
fn round_trip_prekey_bundle() {
    let bundle = identity::PreKeyBundle {
        identity_key: vec![1u8; 32],
        signed_pre_key: vec![2u8; 32],
        signed_pre_key_sig: vec![3u8; 64],
        one_time_pre_key: vec![4u8; 32],
        one_time_pre_key_id: 7,
        registration_id: 12345,
        pqpk_lr: vec![5u8; 1184],
        pqpk_lr_sig: vec![6u8; 64],
        pqpk_ot: vec![8u8; 1184],
        pqpk_ot_sig: vec![9u8; 64],
        pqpk_ot_id: 99,
    };

    let encoded = identity::encode_prekey_bundle(&bundle);
    let decoded = identity::decode_prekey_bundle(&encoded).unwrap();

    assert_eq!(decoded.identity_key, bundle.identity_key);
    assert_eq!(decoded.signed_pre_key, bundle.signed_pre_key);
    assert_eq!(decoded.signed_pre_key_sig, bundle.signed_pre_key_sig);
    assert_eq!(decoded.one_time_pre_key, bundle.one_time_pre_key);
    assert_eq!(decoded.one_time_pre_key_id, 7);
    assert_eq!(decoded.registration_id, 12345);
    assert_eq!(decoded.pqpk_lr, bundle.pqpk_lr);
    assert_eq!(decoded.pqpk_lr_sig, bundle.pqpk_lr_sig);
    assert_eq!(decoded.pqpk_ot, bundle.pqpk_ot);
    assert_eq!(decoded.pqpk_ot_sig, bundle.pqpk_ot_sig);
    assert_eq!(decoded.pqpk_ot_id, 99);
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
    use crate::dht::friends::FriendEntry;

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
    // profile_dht_key not in capnp schema
    assert_eq!(decoded[0].profile_dht_key, None);

    assert_eq!(decoded[1].public_key, "def456");
    assert_eq!(decoded[1].nickname, None);
    assert_eq!(decoded[1].group, None);
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
    use crate::messaging::envelope::GameInfo;

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
        contact_list_key: "VLD0:abc123".to_string(),
        chat_list_key: "VLD0:def456".to_string(),
        invitation_list_key: "VLD0:ghi789".to_string(),
        display_name: "xXGamerXx".to_string(),
        status_message: "Playing games".to_string(),
        avatar_hash: vec![0xDE, 0xAD],
        created_at: 1000,
        updated_at: 2000,
        contact_list_keypair: Some("VLD0:contacts_kp".to_string()),
        chat_list_keypair: Some("VLD0:chats_kp".to_string()),
        invitation_list_keypair: None,
    };

    let encoded = account::encode_account_header(&header);
    let decoded = account::decode_account_header(&encoded).unwrap();

    assert_eq!(decoded.contact_list_key, "VLD0:abc123");
    assert_eq!(decoded.chat_list_key, "VLD0:def456");
    assert_eq!(decoded.invitation_list_key, "VLD0:ghi789");
    assert_eq!(decoded.display_name, "xXGamerXx");
    assert_eq!(decoded.status_message, "Playing games");
    assert_eq!(decoded.avatar_hash, vec![0xDE, 0xAD]);
    assert_eq!(decoded.created_at, 1000);
    assert_eq!(decoded.updated_at, 2000);
    assert_eq!(
        decoded.contact_list_keypair,
        Some("VLD0:contacts_kp".to_string())
    );
    assert_eq!(decoded.chat_list_keypair, Some("VLD0:chats_kp".to_string()));
    assert_eq!(decoded.invitation_list_keypair, None);
}

#[test]
fn round_trip_contact_entry() {
    let entry = account::ContactEntry {
        public_key: vec![0xAA; 32],
        display_name: "Bob".to_string(),
        nickname: "Bobby".to_string(),
        group: "Gaming".to_string(),
        local_conversation_key: "VLD0:local123".to_string(),
        remote_conversation_key: "VLD0:remote456".to_string(),
        added_at: 1000,
        updated_at: 2000,
    };

    let encoded = account::encode_contact_entry(&entry);
    let decoded = account::decode_contact_entry(&encoded).unwrap();

    assert_eq!(decoded.public_key, vec![0xAA; 32]);
    assert_eq!(decoded.display_name, "Bob");
    assert_eq!(decoded.nickname, "Bobby");
    assert_eq!(decoded.group, "Gaming");
    assert_eq!(decoded.local_conversation_key, "VLD0:local123");
    assert_eq!(decoded.remote_conversation_key, "VLD0:remote456");
    assert_eq!(decoded.added_at, 1000);
    assert_eq!(decoded.updated_at, 2000);
}

#[test]
fn round_trip_chat_entry() {
    let entry = account::ChatEntry {
        contact_public_key: vec![0xBB; 32],
        local_conversation_key: "VLD0:chat789".to_string(),
        last_message_timestamp: 3000,
        unread_count: 5,
        is_pinned: true,
        is_muted: false,
    };

    let encoded = account::encode_chat_entry(&entry);
    let decoded = account::decode_chat_entry(&encoded).unwrap();

    assert_eq!(decoded.contact_public_key, vec![0xBB; 32]);
    assert_eq!(decoded.local_conversation_key, "VLD0:chat789");
    assert_eq!(decoded.last_message_timestamp, 3000);
    assert_eq!(decoded.unread_count, 5);
    assert!(decoded.is_pinned);
    assert!(!decoded.is_muted);
}

#[test]
fn round_trip_conversation_header() {
    use crate::messaging::envelope::GameInfo;

    let header = conversation::ConversationHeader {
        identity_public_key: vec![0xCC; 32],
        profile: identity::UserProfile {
            display_name: "Alice".to_string(),
            status_message: "In a meeting".to_string(),
            status: 2, // busy
            avatar_hash: vec![0x01, 0x02],
            game_status: Some(GameInfo {
                game_id: 42,
                game_name: "Portal 2".to_string(),
                server_info: None,
                elapsed_seconds: 600,
                server_address: None,
            }),
        },
        message_log_key: "VLD0:msglog123".to_string(),
        route_blob: vec![0xDD; 64],
        prekey_bundle: identity::PreKeyBundle {
            identity_key: vec![1u8; 32],
            signed_pre_key: vec![2u8; 32],
            signed_pre_key_sig: vec![3u8; 64],
            one_time_pre_key: vec![4u8; 32],
            one_time_pre_key_id: 3,
            registration_id: 42,
            pqpk_lr: vec![5u8; 1184],
            pqpk_lr_sig: vec![6u8; 64],
            pqpk_ot: vec![7u8; 1184],
            pqpk_ot_sig: vec![8u8; 64],
            pqpk_ot_id: 11,
        },
        created_at: 5000,
        updated_at: 6000,
    };

    let encoded = conversation::encode_conversation_header(&header);
    let decoded = conversation::decode_conversation_header(&encoded).unwrap();

    assert_eq!(decoded.identity_public_key, vec![0xCC; 32]);
    assert_eq!(decoded.profile.display_name, "Alice");
    assert_eq!(decoded.profile.status_message, "In a meeting");
    assert_eq!(decoded.profile.status, 2);
    assert_eq!(decoded.profile.avatar_hash, vec![0x01, 0x02]);
    let g = decoded.profile.game_status.unwrap();
    assert_eq!(g.game_name, "Portal 2");
    assert_eq!(g.elapsed_seconds, 600);
    assert_eq!(decoded.message_log_key, "VLD0:msglog123");
    assert_eq!(decoded.route_blob, vec![0xDD; 64]);
    assert_eq!(decoded.prekey_bundle.identity_key, vec![1u8; 32]);
    assert_eq!(decoded.prekey_bundle.registration_id, 42);
    assert_eq!(decoded.created_at, 5000);
    assert_eq!(decoded.updated_at, 6000);
}
