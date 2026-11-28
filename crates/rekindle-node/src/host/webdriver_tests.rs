//! The real-app suite's damaged-post switch (`host::webdriver`): armed,
//! the next received post fails its real AEAD and is stored damaged; the
//! one after it opens normally.

use super::channel_change_tests::{reader, reload, AUTHOR, EDIT_CHANNEL, TEXT_KEY};
use crate::host::adapters::channel::ingest::{Ingested, ReceivedAt, ReceivedPost};
use crate::host::adapters::channel::HostChannel;
use rekindle_codec::community::channel_record::ChannelMessage;
use rekindle_secrets::channel_body::{encrypt_channel_body, BodyPosition};

/// `AUTHOR`'s post `message_id` at `stream_seq`, sealed under the held key.
fn post(message_id: &str, stream_seq: u64, body: &str) -> ReceivedPost {
    let at = BodyPosition {
        anchor: "c1",
        channel_id: EDIT_CHANNEL,
        author: &AUTHOR.0,
        stream_seq,
    };
    ReceivedPost {
        community_id: "c1".into(),
        channel_id: EDIT_CHANNEL.into(),
        at: ReceivedAt::Outbox {
            stream_seq,
            departure_head: None,
        },
        sequence: stream_seq,
        message: ChannelMessage {
            sequence: stream_seq,
            sender_pseudonym: hex::encode(AUTHOR.0),
            ciphertext: encrypt_channel_body(&TEXT_KEY, at, body.as_bytes()).unwrap(),
            mek_generation: 1,
            timestamp: 2_000 + stream_seq,
            reply_to: None,
            revives: None,
            lamport_ts: 3 + stream_seq,
            message_id: Some(message_id.into()),
            attachment: None,
            flags: 0,
            mentioned_pseudonyms: Vec::new(),
            mentioned_roles: Vec::new(),
        },
        forwarded_from_author: None,
    }
}

#[tokio::test]
async fn an_armed_switch_damages_only_the_next_post() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = reader(&dir).await;
    let channel = HostChannel::new(&host.ctx);
    host.arm_damage_next_post();

    let damaged = post("m2", 2, "flipped");
    let ingested = channel.ingest(damaged.clone()).await.unwrap();
    assert_eq!(ingested, Ingested::Undecryptable, "the real AEAD failed");
    assert!(
        channel.store_undecryptable(damaged).await.unwrap(),
        "its key is held: stored damaged"
    );

    let ingested = channel.ingest(post("m3", 3, "intact")).await.unwrap();
    assert_eq!(
        ingested,
        Ingested::Stored {
            body: "intact".into(),
            blurred: false
        },
        "the switch is spent: the next post opens"
    );
    assert_eq!(
        reload(&host, &owner).await,
        [
            ("first".into(), None),
            (String::new(), None),
            ("intact".into(), None)
        ],
        "the damaged post is a row without its body, between the others"
    );
    host.shutdown().await.unwrap();
}
