//! Group lobbies on the receive path (plan E7.2 G6, SK2-G §5) over a real
//! unlocked identity: the same lobby frame read twice folds once and is
//! published once; a writer off the roster has its lobby frame dropped at
//! receive, so it never reaches `group_lobbies` or the event stream.

use rekindle_codec::capnp_codec::direct::{DirectBody, GroupId, LobbyOpen};
use rekindle_dm::sender_key;
use rekindle_dm::{GroupDmDeps, GroupSenderKey};
use rekindle_records::chain::ChainEntry;
use rekindle_sync::stream::StreamSink;
use rekindle_transport::operations::group_dm::content::{inner, stream_name};
use rekindle_transport::operations::group_dm::lobby;
use rekindle_transport::operations::group_dm::receive::GroupSink;
use rekindle_utils::OsRandom;

use super::adapters::friends::HostFriends;
use super::adapters::group_dm::HostGroupDm;
use super::direct_tests::ann;
use super::group_dm_tests::member;
use super::Host;

const GID: GroupId = [7; 16];

fn bob() -> String {
    "b".repeat(64)
}

fn eve() -> String {
    "e".repeat(64)
}

/// Ann's group with Bob and Carol; `writer`'s first key held.
async fn group(host: &HostFriends, owner: [u8; 32], writer: &str) -> GroupSenderKey {
    let deps = HostGroupDm::open(host).unwrap();
    let others = vec![member(bob(), 'b'), member("c".repeat(64), 'c')];
    let own = member(hex::encode(owner), 'a');
    let chain = format!("VLD0:{}", "Q".repeat(43));
    rekindle_dm::create(
        &deps,
        GID,
        "raid",
        own,
        others,
        &chain,
        &crate::host::group_dm_tests::rendezvous_mint(),
    )
    .await
    .unwrap();
    let key = sender_key::mint_first(&OsRandom);
    deps.put_sender_key(&GID, writer, &key).await.unwrap();
    key
}

/// `writer`'s chain entry `seq` carrying a `LobbyOpen` of one lobby.
fn open_entry(key: &GroupSenderKey, writer: &str, seq: u64) -> ChainEntry {
    let body = DirectBody::LobbyOpen(LobbyOpen {
        lobby_id: [0xab; 16],
        game_id: 4181,
    });
    let content = sender_key::seal(key, GID, writer, &inner(&body, 1_000 + seq, seq)).unwrap();
    ChainEntry {
        seq,
        ciphertext: DirectBody::GroupContent(content).encode(),
    }
}

/// The `GroupDmEvent::Lobby` frames the journal published.
async fn lobby_events(events: &mut rekindle_events::journal::EventStream) -> usize {
    let mut n = 0;
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), events.next()).await
    {
        let text = format!("{frame:?}");
        if text.contains("GroupDm") && text.contains("Lobby {") {
            n += 1;
        }
    }
    n
}

async fn setup(dir: &tempfile::TempDir, writer: &str) -> (std::sync::Arc<Host>, GroupSenderKey) {
    let (host, owner) = ann(dir).await;
    let key = group(&HostFriends::of(&host.ctx), owner, writer).await;
    (host, key)
}

#[tokio::test]
async fn the_same_lobby_frame_folded_twice_changes_nothing_the_second_time() {
    let dir = tempfile::tempdir().unwrap();
    let (host, key) = setup(&dir, &bob()).await;
    let friends = HostFriends::of(&host.ctx);
    let mut events = host.ctx.journal.subscribe(None);
    let (sink, stream) = (
        GroupSink::new(&friends, GID),
        stream_name(&hex::encode(GID)),
    );
    let gid = hex::encode(GID);
    sink.apply(&stream, &bob(), &open_entry(&key, &bob(), 1))
        .await
        .unwrap();
    let first = lobby::get(&friends, &gid).await.unwrap();
    assert_eq!(first.as_ref().map(|l| l.host_key.clone()), Some(bob()));
    // The same open read again (a replay under a later entry) folds to the
    // same row and publishes nothing.
    sink.apply(&stream, &bob(), &open_entry(&key, &bob(), 2))
        .await
        .unwrap();
    assert_eq!(lobby::get(&friends, &gid).await.unwrap(), first);
    assert_eq!(lobby_events(&mut events).await, 1, "published once");
}

#[tokio::test]
async fn a_lobby_frame_from_a_writer_off_the_roster_is_dropped_at_receive() {
    let dir = tempfile::tempdir().unwrap();
    let (host, key) = setup(&dir, &eve()).await;
    let friends = HostFriends::of(&host.ctx);
    let mut events = host.ctx.journal.subscribe(None);
    let (sink, stream) = (
        GroupSink::new(&friends, GID),
        stream_name(&hex::encode(GID)),
    );
    sink.apply(&stream, &eve(), &open_entry(&key, &eve(), 1))
        .await
        .unwrap();
    assert_eq!(lobby::get(&friends, &hex::encode(GID)).await.unwrap(), None);
    assert_eq!(lobby_events(&mut events).await, 0, "nothing published");
}
