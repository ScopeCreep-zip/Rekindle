//! A channel's history backfill (plan E3.1 E3f), offline: a record
//! message stored from the channel record keeps its attachment's record.

use rekindle_channel::history::RecordMessage;
use rekindle_db::repo::messages::channel as rows;
use rekindle_types::attachment::AttachmentOffer;
use rekindle_types::dto::channel_message::ChannelMessageView;
use rekindle_types::message::SendState;

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::channel::history::stored_view;
use crate::host::adapters::channel::HostChannel;

const BACKFILL_CHANNEL: &str = "0123456789abcdef0123456789abcdef";

fn offer() -> AttachmentOffer {
    AttachmentOffer {
        attachment_id: [0x5a; 16],
        filename: "map.png".into(),
        mime_type: "image/png".into(),
        total_size: 3,
        chunk_count: 1,
        chunk_size: 28 * 1024,
        merkle_root: [1; 32],
        chunk_hashes: vec![[2; 32]],
        wrapped_fek: vec![3; 60],
        fek_mek_generation: 0,
    }
}

fn record_message() -> RecordMessage {
    RecordMessage {
        view: ChannelMessageView {
            id: 0,
            sender_id: hex::encode([0xbe; 32]),
            body: "the map".into(),
            decryption_failed: false,
            automod_blurred: false,
            timestamp: 1_000,
            is_own: false,
            server_message_id: Some("m1".into()),
            reactions: None,
            pinned: None,
            poll: None,
            forwarded_from_author: None,
            attachment: None,
            flags: 0,
            send_state: SendState::Delivered,
        },
        reply_to: None,
        mek_generation: 0,
        lamport_ts: 1,
        mentions_me: false,
        attachment: Some(offer()),
    }
}

#[tokio::test]
async fn a_backfilled_message_keeps_its_attachment() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    HostChannel::new(&host.ctx)
        .store_record_messages(&owner, BACKFILL_CHANNEL, &[record_message()])
        .await
        .unwrap();
    let db = host.ctx.db.current().unwrap();
    let key = owner.clone();
    let stored = db
        .call(move |conn| rows::page(conn, &key, BACKFILL_CHANNEL, None, 10))
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    let view = stored_view(stored.into_iter().next().unwrap(), &owner, "");
    let attachment = view.attachment.expect("the row keeps the attachment");
    assert_eq!(attachment.attachment_id, hex::encode([0x5a; 16]));
    assert_eq!(attachment.filename, "map.png");
    assert_eq!(attachment.chunk_count, 1);
    assert!(!attachment.downloaded);
    host.shutdown().await.unwrap();
}
