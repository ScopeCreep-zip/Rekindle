//! An author's edit or delete that arrives before its message waits in
//! the bounded cache (plan E3.3 E5) and applies, re-validated, when the
//! message lands.

use rekindle_codec::community::channel_record::{
    ChannelDelete, ChannelMessage, ChannelRecordEntry,
};
use rekindle_secrets::channel_body::{encrypt_channel_body, BodyPosition};

use super::channel_change_tests::{
    apply, edit, reader, reload, AUTHOR, EDIT_CHANNEL, RECORD, SUBKEY, TEXT_KEY,
};
use crate::host::adapters::channel::ingest::{Ingested, ReceivedPost};
use crate::host::adapters::channel::HostChannel;
use crate::host::Host;

/// `AUTHOR`'s post `message_id` with `body`, as a record read hands it on.
fn post(message_id: &str, body: &str) -> ReceivedPost {
    let lamport_ts = 3;
    let at = BodyPosition {
        channel_record_key: RECORD,
        subkey_index: SUBKEY,
        lamport_ts,
    };
    ReceivedPost {
        community_id: "c1".into(),
        channel_id: EDIT_CHANNEL.into(),
        record_key: RECORD.into(),
        subkey_index: SUBKEY,
        sequence: 1,
        message: ChannelMessage {
            sequence: 1,
            sender_pseudonym: hex::encode(AUTHOR.0),
            ciphertext: encrypt_channel_body(&TEXT_KEY, at, body.as_bytes()).unwrap(),
            mek_generation: 1,
            timestamp: 2_000,
            reply_to: None,
            lamport_ts,
            message_id: Some(message_id.into()),
            attachment: None,
            flags: 0,
            mentioned_pseudonyms: Vec::new(),
            mentioned_roles: Vec::new(),
        },
        forwarded_from_author: None,
    }
}

fn retarget(entry: ChannelRecordEntry, message_id: &str) -> ChannelRecordEntry {
    match entry {
        ChannelRecordEntry::Edit(mut e) => {
            e.message_id = message_id.into();
            ChannelRecordEntry::Edit(e)
        }
        other => other,
    }
}

async fn land(host: &Host, message_id: &str, body: &str) {
    let ingested = HostChannel::new(&host.ctx)
        .ingest(post(message_id, body))
        .await
        .unwrap();
    assert!(matches!(ingested, Ingested::Stored { .. }), "{ingested:?}");
}

#[tokio::test]
async fn an_edit_before_its_message_applies_once_the_message_lands() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = reader(&dir).await;
    apply(&host, &AUTHOR, &[retarget(edit("edited early", 7), "m2")]).await;
    assert_eq!(reload(&host, &owner).await.len(), 1, "nothing to edit yet");

    land(&host, "m2", "original").await;

    assert_eq!(
        reload(&host, &owner).await,
        [("first".into(), None), ("edited early".into(), Some(5_007))],
        "the held edit applied when its message landed"
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_delete_before_its_message_applies_once_the_message_lands() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = reader(&dir).await;
    let delete = ChannelRecordEntry::Delete(ChannelDelete {
        message_id: "m2".into(),
        deleted_at: 9_000,
        lamport_ts: 8,
    });
    apply(
        &host,
        &AUTHOR,
        &[retarget(edit("edited early", 7), "m2"), delete],
    )
    .await;

    land(&host, "m2", "original").await;

    assert_eq!(
        reload(&host, &owner).await,
        [("first".into(), None)],
        "delete wins over the held edit: the message is deleted for members"
    );
    host.shutdown().await.unwrap();
}
