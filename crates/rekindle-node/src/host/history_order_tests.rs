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

/// A sender's claimed time cannot reorder their own messages: seq 2
/// claiming a time before seq 1 is stored at seq 1's, and seq 3 claiming
/// a far-future time is stored no later than our receive time plus the
/// allowed skew; all three read back in stream order (threat model).
#[tokio::test]
async fn a_false_claimed_time_is_clamped_into_stream_order() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = ann(&dir).await;
    let bob = rekindle_crypto::Identity::generate().public_key_bytes();
    befriend(&host, owner, bob).await;
    let t = rekindle_utils::timestamp_ms();
    let far = t + 365 * 24 * 60 * 60 * 1000;
    for (seq, at) in [(1, t), (2, t - 60_000), (3, far)] {
        arrived(&host, owner, bob, seq, at).await;
        replay_unprocessed(&HostFriends::of(&host.ctx)).await;
    }
    // Bob's 4th, written after; a far-future 3rd must not stay below it.
    arrived(&host, owner, bob, 4, t + 1).await;
    replay_unprocessed(&HostFriends::of(&host.ctx)).await;
    let expected: Vec<String> = (1..=4).map(|n| format!("hello {n}")).collect();
    assert_eq!(bodies(&host, owner, bob).await, expected);
    let times = times(&host, owner, bob).await;
    let (t, done) = (
        i64::try_from(t).unwrap(),
        rekindle_utils::timestamp_ms_i64(),
    );
    assert_eq!(times[0], t);
    assert_eq!(times[1], t, "seq 2 never before seq 1");
    assert!(
        times[2]
            <= done + i64::try_from(rekindle_types::message::ENVELOPE_MAX_FUTURE_SKEW_MS).unwrap(),
        "seq 3 within the skew"
    );
    assert!(times.windows(2).all(|w| w[0] <= w[1]), "{times:?}");
}

async fn times(host: &Host, owner: [u8; 32], bob: [u8; 32]) -> Vec<i64> {
    let (o, p) = (hex::encode(owner), hex::encode(bob));
    host.ctx
        .db
        .current()
        .unwrap()
        .call(move |conn| rekindle_db::repo::messages::dm_history(conn, &o, &p, 50, None))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.timestamp)
        .collect()
}
