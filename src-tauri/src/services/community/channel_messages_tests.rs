use rekindle_protocol::dht::community::channel_record::ChannelMessage;
use rekindle_protocol::dht::community::envelope::CommunityEnvelope;

use super::build_message_notification;

fn sample_message() -> ChannelMessage {
    ChannelMessage {
        sequence: 7,
        sender_pseudonym: "sender".into(),
        ciphertext: b"ciphertext".to_vec(),
        mek_generation: 3,
        timestamp: 1234,
        reply_to: None,
        lamport_ts: 88,
        message_id: Some("msg-1".into()),
    }
}

#[test]
fn gossip_only_notification_core_is_independent_of_smpl_retry_state() {
    let smpl_write_failed_and_was_queued = true;
    assert!(smpl_write_failed_and_was_queued);

    let notification = build_message_notification("chan-1", &sample_message(), 4)
        .expect("notification should still be buildable");

    match notification {
        CommunityEnvelope::MessageNotification {
            channel_id,
            message_id,
            subkey_index,
            content_hash,
            ..
        } => {
            assert_eq!(channel_id, "chan-1");
            assert_eq!(message_id, "msg-1");
            assert_eq!(
                subkey_index,
                crate::services::community::channel_message_subkey(4)
            );
            assert_eq!(
                content_hash,
                blake3::hash(b"ciphertext").to_hex().to_string()
            );
        }
        other => panic!("expected MessageNotification, got {other:?}"),
    }
}
