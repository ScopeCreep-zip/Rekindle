use rekindle_crypto::group::media_key::MediaEncryptionKey;
use rekindle_protocol::dht::community::channel_record::ChannelMessage;

use super::{verify_notification_message, PendingMessageFetch};

fn pending(ciphertext: &[u8]) -> PendingMessageFetch {
    PendingMessageFetch {
        community_id: "community".into(),
        channel_id: "channel".into(),
        message_id: "msg-1".into(),
        subkey_index: 9,
        sequence: 2,
        content_hash: blake3::hash(ciphertext).to_hex().to_string(),
        attempt: 0,
    }
}

#[test]
fn gossip_only_fetch_core_accepts_matching_dht_message() {
    let mek = MediaEncryptionKey::generate(4);
    let ciphertext = mek.encrypt(b"hello world").expect("encrypt");
    let message = ChannelMessage {
        sequence: 2,
        sender_pseudonym: "sender".into(),
        ciphertext: ciphertext.clone(),
        mek_generation: mek.generation(),
        timestamp: 55,
        reply_to: None,
        lamport_ts: 10,
        message_id: Some("msg-1".into()),
    };

    assert!(verify_notification_message(&pending(&ciphertext), &message).is_ok());
}
