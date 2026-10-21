//! A conversation's history keeps each sender's stream order whatever
//! order the entries arrive in (E2.B F3, D23): a friend's messages applied
//! out of order, some written in the same millisecond, read back in the
//! order they were written.

use rekindle_codec::capnp_codec::direct::{ContentHint, DmContent};
use rekindle_db::repo::unprocessed::UnprocessedKey;
use rekindle_transport::operations::direct::replay_unprocessed;

use super::direct_tests::{ann, befriend, text};
use crate::host::adapters::friends::HostFriends;
use crate::host::Host;

/// Bob's stream entry `seq` (written at `sent_at_ms`), decrypted and
/// waiting for its apply.
async fn arrived(host: &Host, owner: [u8; 32], bob: [u8; 32], seq: u8, sent_at_ms: u64) {
    let content = DmContent {
        stream_seq: u64::from(seq),
        content_hint: ContentHint::Default,
        sent_at_ms,
        payload: text(seq).encode(),
    }
    .encode();
    let key = UnprocessedKey {
        peer_identity: bob,
        header_dh: [7; 32],
        header_n: u32::from(seq),
    };
    let type_id = text(seq).type_id().as_u16();
    let now = rekindle_utils::timestamp_ms_i64();
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| {
            rekindle_db::repo::unprocessed::insert(conn, &owner, &key, type_id, &content, now)
        })
        .await
        .unwrap();
}

async fn bodies(host: &Host, owner: [u8; 32], bob: [u8; 32]) -> Vec<String> {
    let (o, p) = (hex::encode(owner), hex::encode(bob));
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::messages::dm_history(conn, &o, &p, 50, None))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.body)
        .collect()
}

#[tokio::test]
async fn a_senders_entries_applied_out_of_order_read_back_in_stream_order() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let bob = rekindle_crypto::Identity::generate().public_key_bytes();
    befriend(&host, owner, bob).await;
    let t = rekindle_utils::timestamp_ms();
    // Entries 2, 3 and 4 were written in one millisecond, 5 later; they
    // arrive (and are applied) 4, 2, 5, 3, 1.
    for (seq, at) in [(4, t), (2, t), (5, t + 10), (3, t), (1, t - 10)] {
        arrived(&host, owner, bob, seq, at).await;
        replay_unprocessed(&HostFriends::of(&host.ctx)).await;
    }
    let expected: Vec<String> = (1..=5).map(|n| format!("hello {n}")).collect();
    assert_eq!(bodies(&host, owner, bob).await, expected);
}
