//! A reader applies an author's edits and deletes from the author's record
//! (plan E3.3, rules E1–E3) to the stored row, so they survive a reload
//! (walk M47: every edit reverted on a re-read), and publishes each change
//! the row takes.

use rekindle_codec::community::channel_record::{ChannelDelete, ChannelEdit, ChannelRecordEntry};
use rekindle_db::repo::messages::channel::{self as rows, ChannelMessageInsert};
use rekindle_governance::state::GovernanceState;
use rekindle_secrets::channel_body::{encrypt_channel_body, BodyPosition};
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::PseudonymKey;
use rekindle_types::message::SendState;
use rekindle_types::subscription_events::{ChannelMessageEvent, StreamFrame, SubscriptionEvent};

use super::{create_ann, offline_host, seed_registry_community};
use crate::host::adapters::channel::changes::ChangeSource;
use crate::host::adapters::channel::HostChannel;
use crate::host::Host;

pub(super) const EDIT_CHANNEL: &str = "fedcba9876543210fedcba9876543210";
pub(super) const RECORD: &str = "VLD0:channel";
pub(super) const SUBKEY: u32 = 9;
pub(super) const TEXT_KEY: [u8; 32] = [7; 32];
pub(super) const AUTHOR: PseudonymKey = PseudonymKey([1; 32]);
const OTHER_AUTHOR: PseudonymKey = PseudonymKey([2; 32]);

/// The channel-message events published within a short wait.
pub(super) async fn channel_events(
    stream: &mut rekindle_events::EventStream,
) -> Vec<ChannelMessageEvent> {
    let mut out = Vec::new();
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), stream.next()).await
    {
        if let StreamFrame::Event(s) = frame {
            if let SubscriptionEvent::ChannelMessage(e) = s.event {
                out.push(e);
            }
        }
    }
    out
}

/// Ann's Host holding `AUTHOR`'s message `m1` ("first") in `c1`, the
/// community's text key, and a merged state that refuses nobody.
pub(super) async fn reader(dir: &tempfile::TempDir) -> (std::sync::Arc<Host>, String) {
    let host = offline_host(dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let cache = crate::daemon::mek_rotation::MekCacheAdapter::new(std::sync::Arc::clone(
        &host.ctx.mek_cache,
    ));
    let key = rekindle_transport::crypto::mek::Mek::from_bytes(TEXT_KEY, 1);
    rekindle_mek_rotation::ChannelMekCache::insert(&cache, "c1", KeyScope::Community, key);
    (host.ctx.communities).set_governance_state("c1", GovernanceState::default());
    let o = owner.clone();
    (host.ctx.db.current().unwrap())
        .call(move |conn| {
            rows::insert(
                conn,
                &ChannelMessageInsert {
                    owner_key: &o,
                    channel_id: EDIT_CHANNEL,
                    sender_key: &hex::encode(AUTHOR.0),
                    body: "first",
                    timestamp: 1_000,
                    is_read: false,
                    mek_generation: Some(1),
                    message_id: "m1",
                    lamport_ts: 2,
                    automod_blurred: false,
                    forwarded_from_author: None,
                    flags: 0,
                    attachment_json: None,
                    send_state: SendState::Delivered,
                    reply_to: None,
                    mentions_me: false,
                },
            )
        })
        .await
        .unwrap();
    (host, owner)
}

pub(super) fn edit(body: &str, seq: u64) -> ChannelRecordEntry {
    let at = BodyPosition {
        channel_record_key: RECORD,
        subkey_index: SUBKEY,
        lamport_ts: seq,
    };
    ChannelRecordEntry::Edit(ChannelEdit {
        message_id: "m1".into(),
        ciphertext: encrypt_channel_body(&TEXT_KEY, at, body.as_bytes()).unwrap(),
        mek_generation: 1,
        edited_at: 5_000 + seq,
        lamport_ts: seq,
    })
}

pub(super) async fn apply(host: &Host, author: &PseudonymKey, entries: &[ChannelRecordEntry]) {
    let source = ChangeSource {
        community_id: "c1",
        channel_id: EDIT_CHANNEL,
        record_key: RECORD,
        subkey_index: SUBKEY,
        author,
    };
    HostChannel::new(&host.ctx)
        .apply_changes(source, entries)
        .await;
}

/// What a reload shows: each row's body and edit time.
pub(super) async fn reload(host: &Host, owner: &str) -> Vec<(String, Option<i64>)> {
    let owner = owner.to_owned();
    (host.ctx.db.current().unwrap())
        .call(move |conn| rows::page(conn, &owner, EDIT_CHANNEL, None, 10))
        .await
        .unwrap()
        .into_iter()
        .map(|m| (m.body, m.edited_at))
        .collect()
}

#[tokio::test]
async fn the_authors_edit_is_stored_and_another_authors_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = reader(&dir).await;
    let mut events = host.ctx.journal.subscribe(None);

    apply(&host, &OTHER_AUTHOR, &[edit("forged", 9)]).await;
    assert_eq!(reload(&host, &owner).await, [("first".into(), None)], "E1");
    apply(&host, &AUTHOR, &[edit("second", 4), edit("third", 6)]).await;
    apply(&host, &AUTHOR, &[edit("stale", 5)]).await;

    assert_eq!(
        reload(&host, &owner).await,
        [("third".into(), Some(5_006))],
        "the latest edit by the author's seq survives the reload"
    );
    let got = channel_events(&mut events).await;
    let bodies: Vec<_> = (got.iter())
        .filter_map(|e| match e {
            ChannelMessageEvent::Edited { body, .. } => body.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(
        bodies,
        ["second", "third"],
        "each applied edit is published once"
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_reader_takes_ten_edits_of_a_message() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = reader(&dir).await;
    let edits: Vec<_> = (1..=11)
        .map(|n| edit(&format!("edit {n}"), 10 + n))
        .collect();
    apply(&host, &AUTHOR, &edits).await;
    assert_eq!(
        reload(&host, &owner).await,
        [("edit 10".into(), Some(5_020))],
        "E2: the eleventh edit is refused"
    );
    host.shutdown().await.unwrap();
}

#[tokio::test]
async fn the_authors_delete_wins_and_is_final() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = reader(&dir).await;
    let mut events = host.ctx.journal.subscribe(None);
    let delete = ChannelRecordEntry::Delete(ChannelDelete {
        message_id: "m1".into(),
        deleted_at: 6_000,
        lamport_ts: 3,
    });

    apply(&host, &OTHER_AUTHOR, std::slice::from_ref(&delete)).await;
    assert_eq!(
        reload(&host, &owner).await.len(),
        1,
        "only the author deletes"
    );
    // The edit's seq is above the delete's: a delete still wins.
    apply(&host, &AUTHOR, &[edit("after", 8), delete]).await;
    apply(&host, &AUTHOR, &[edit("later", 9)]).await;

    assert!(
        reload(&host, &owner).await.is_empty(),
        "E3: deleted for members"
    );
    let got = channel_events(&mut events).await;
    assert!(
        matches!(&got[..], [ChannelMessageEvent::Deleted { message_id, .. }] if message_id == "m1"),
        "one delete and no edit after it: {got:?}"
    );
    host.shutdown().await.unwrap();
}
