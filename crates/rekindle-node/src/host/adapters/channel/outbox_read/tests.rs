//! A joiner's outbox entry before its first community key (finding CB1):
//! the sink holds it ([`Held::Pending`], the cursor stays) instead of
//! failing the pass, so a trigger that lands during the read (the key's
//! install catch-up) reads again; once the key is held the re-read
//! applies it.

use rekindle_codec::community::channel_record::{ChannelMessage, ChannelRecordEntry};
use rekindle_codec::member_record::{ChainPosition, OutboxEntry};
use rekindle_db::repo::messages::channel as rows;
use rekindle_records::chain::ChainEntry;
use rekindle_secrets::channel_body::{channel_tag, encrypt_channel_body, BodyPosition};
use rekindle_secrets::ed25519_dalek::SigningKey;
use rekindle_sync::stream::{Held, StreamSink};
use rekindle_types::channel_keys::KeyScope;

use rekindle_types::id::PseudonymKey;

use super::{outbox_stream, Before, OutboxRead, OutboxSink, Standing};
use crate::host::adapters::channel::HostChannel;
use crate::host::runtime::tests::channel_change_tests::{EDIT_CHANNEL, TEXT_KEY};
use crate::host::runtime::tests::channel_gap_tests::with_channel;
use crate::host::runtime::tests::{create_ann, offline_host, seed_registry_community};

/// The author's first outbox entry: its post "hello" in `EDIT_CHANNEL`,
/// sealed under generation 1 of the community key and signed at chain
/// seq 1.
fn first_entry(signing: &SigningKey) -> ChainEntry {
    sealed_entry(signing, 1, &TEXT_KEY)
}

/// The author's first outbox entry, its body "hello" sealed with `key`;
/// the entry and its body both name `generation`.
fn sealed_entry(signing: &SigningKey, generation: u64, key: &[u8; 32]) -> ChainEntry {
    split_entry(signing, generation, generation, key)
}

/// The author's first outbox entry in `EDIT_CHANNEL`: the entry names
/// `entry_generation` (its tag is always under `TEXT_KEY`), its body
/// "hello" is sealed with `key` and names `body_generation`.
fn split_entry(
    signing: &SigningKey,
    entry_generation: u64,
    body_generation: u64,
    key: &[u8; 32],
) -> ChainEntry {
    let author = signing.verifying_key().to_bytes();
    let at = BodyPosition {
        anchor: "c1",
        channel_id: EDIT_CHANNEL,
        author: &author,
        stream_seq: 1,
    };
    let entry = OutboxEntry {
        prev: None,
        channel_tag: channel_tag(&TEXT_KEY, EDIT_CHANNEL),
        stream_seq: 1,
        generation: entry_generation,
        committed_at: 1_000,
        entry: ChannelRecordEntry::Message(ChannelMessage {
            sequence: 1,
            sender_pseudonym: hex::encode(author),
            ciphertext: encrypt_channel_body(key, at, b"hello").unwrap(),
            mek_generation: body_generation,
            timestamp: 1_000,
            reply_to: None,
            revives: None,
            lamport_ts: 1,
            message_id: Some("m1".into()),
            attachment: None,
            flags: 0,
            mentioned_pseudonyms: Vec::new(),
            mentioned_roles: Vec::new(),
        }),
    };
    let at = ChainPosition {
        anchor: "c1",
        chain_seq: 1,
    };
    ChainEntry {
        seq: 1,
        ciphertext: entry.encode(at, signing).unwrap(),
    }
}

#[tokio::test]
async fn an_entry_before_the_first_key_is_held_and_a_re_read_applies_it() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    (host.ctx.communities).set_governance_state("c1", with_channel());
    // The chain's first entry links to nothing, so the port is not read.
    let (sim, mut nodes) = rekindle_netsim::Netsim::nodes(1, 89);
    let _pump = sim.start();
    let port = nodes.remove(0).ports.records.start_session().unwrap();
    let signing = SigningKey::from_bytes(&[6; 32]);
    let channel = HostChannel::new(&host.ctx);
    let sink = OutboxSink {
        channel: &channel,
        community_id: "c1",
        author: signing.verifying_key().to_bytes(),
        member_record: "VLD0:member",
        backfill: true,
        standing: Standing::Member,
        port: &*port,
        last: parking_lot::Mutex::new(None),
    };
    let stream = outbox_stream("c1");
    let writer = format!(
        "{}:VLD0:member",
        hex::encode(signing.verifying_key().to_bytes())
    );
    let entry = first_entry(&signing);

    let held = sink.apply(&stream, &writer, &entry).await;
    assert!(
        matches!(held, Ok(Held::Pending)),
        "waiting for its key is a hold, not a failed pass: {held:?}"
    );
    assert_eq!(sink.cursor(&stream, &writer).await.unwrap(), 0);

    let cache = crate::daemon::mek_rotation::MekCacheAdapter::new(std::sync::Arc::clone(
        &host.ctx.mek_cache,
    ));
    let key = rekindle_transport::crypto::mek::Mek::from_bytes(TEXT_KEY, 1);
    rekindle_mek_rotation::ChannelMekCache::insert(&cache, "c1", KeyScope::Community, key);

    let held = sink.apply(&stream, &writer, &entry).await;
    assert!(matches!(held, Ok(Held::Stored)), "{held:?}");
    assert_eq!(sink.cursor(&stream, &writer).await.unwrap(), 1);
    let bodies: Vec<String> = (host.ctx.db.current().unwrap())
        .call(move |conn| rows::page(conn, &owner, EDIT_CHANNEL, None, 10))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.body)
        .collect();
    assert_eq!(bodies, ["hello"], "the re-read stored the held post");
    host.shutdown().await.unwrap();
}

/// The race the sink's standing leaves open: the author's leave merges
/// after the read took it as a member and before the D26 gate judges the
/// post, so the gate drops it as from a non-member. The entry is held (the
/// cursor stays and it is read again under the departed standing), never
/// passed; under a departed standing the same drop is final.
#[tokio::test]
async fn a_post_whose_leave_merged_during_its_judgement_is_held_not_passed() {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let signing = SigningKey::from_bytes(&[6; 32]);
    let author = signing.verifying_key().to_bytes();
    // Departed: its member record known, its membership gone, no departure
    // head of this outbox.
    let mut state = with_channel();
    (state.op_log_records).insert(PseudonymKey(author), "VLD0:member".into());
    (host.ctx.communities).set_governance_state("c1", state);
    let cache = crate::daemon::mek_rotation::MekCacheAdapter::new(std::sync::Arc::clone(
        &host.ctx.mek_cache,
    ));
    let key = rekindle_transport::crypto::mek::Mek::from_bytes(TEXT_KEY, 1);
    rekindle_mek_rotation::ChannelMekCache::insert(&cache, "c1", KeyScope::Community, key);
    let channel = HostChannel::new(&host.ctx);
    let entry = first_entry(&signing);
    let read = |departed| OutboxRead {
        community_id: "c1",
        author: &author,
        member_record: "VLD0:member",
        backfill: true,
        departed,
        departure_head: None,
        before: Before::Known(None),
    };

    let held = channel.apply_outbox_entry(&read(false), &entry).await;
    assert!(
        matches!(held, Ok(Held::Pending)),
        "judged under a member standing its leave overtook: held: {held:?}"
    );
    let held = channel.apply_outbox_entry(&read(true), &entry).await;
    assert!(
        matches!(held, Ok(Held::Stored)),
        "judged departed with no head keeping it: dropped and passed: {held:?}"
    );
    let rows = (host.ctx.db.current().unwrap())
        .call(move |conn| rows::page(conn, &owner, EDIT_CHANNEL, None, 10))
        .await
        .unwrap();
    assert!(rows.is_empty(), "neither judgement stored the post");
    host.shutdown().await.unwrap();
}

/// A second community key, generation 2's.
const NEXT_KEY: [u8; 32] = [7; 32];

fn install(host: &crate::host::Host, key: [u8; 32], generation: u64) {
    let cache = crate::daemon::mek_rotation::MekCacheAdapter::new(std::sync::Arc::clone(
        &host.ctx.mek_cache,
    ));
    let key = rekindle_transport::crypto::mek::Mek::from_bytes(key, generation);
    rekindle_mek_rotation::ChannelMekCache::insert(&cache, "c1", KeyScope::Community, key);
}

/// A reader of `c1` whose merged state converged on generation
/// `converged`, holding the community keys `held`.
async fn keyed_reader(
    dir: &tempfile::TempDir,
    converged: u64,
    held: &[([u8; 32], u64)],
) -> (std::sync::Arc<crate::host::Host>, String) {
    let host = offline_host(dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let mut state = with_channel();
    state.mek_generation = converged;
    (host.ctx.communities).set_governance_state("c1", state);
    for (key, generation) in held {
        install(&host, *key, *generation);
    }
    (host, owner)
}

fn member_read(author: &[u8; 32]) -> OutboxRead<'_> {
    OutboxRead {
        community_id: "c1",
        author,
        member_record: "VLD0:member",
        backfill: true,
        departed: false,
        departure_head: None,
        before: Before::Known(None),
    }
}

async fn stored(host: &crate::host::Host, owner: &str) -> Vec<rows::StoredChannelMessage> {
    let owner = owner.to_owned();
    (host.ctx.db.current().unwrap())
        .call(move |conn| rows::page(conn, &owner, EDIT_CHANNEL, None, 10))
        .await
        .unwrap()
}

/// Plan KP: an entry sealed under generation 2, which the state converged
/// on and we are owed, is held (the cursor stays) while we hold only
/// generation 1; once generation 2 installs, the re-read the install
/// starts delivers it with its text.
#[tokio::test]
async fn an_entry_whose_key_is_owed_is_held_then_delivered_with_its_body() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = keyed_reader(&dir, 2, &[(TEXT_KEY, 1)]).await;
    let signing = SigningKey::from_bytes(&[6; 32]);
    let author = signing.verifying_key().to_bytes();
    let channel = HostChannel::new(&host.ctx);
    let entry = sealed_entry(&signing, 2, &NEXT_KEY);

    let held = channel
        .apply_outbox_entry(&member_read(&author), &entry)
        .await;
    assert!(
        matches!(held, Ok(Held::Pending)),
        "an entry of a generation we are owed is held: {held:?}"
    );
    assert!(stored(&host, &owner).await.is_empty(), "nothing stored yet");

    install(&host, NEXT_KEY, 2);
    let held = channel
        .apply_outbox_entry(&member_read(&author), &entry)
        .await;
    assert!(matches!(held, Ok(Held::Stored)), "{held:?}");
    let rows = stored(&host, &owner).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].body, "hello", "delivered with its text");
    assert_eq!(rows[0].undecryptable_reason, None);
    host.shutdown().await.unwrap();
}

/// Plan KP: an entry tagged under generation 1, which we hold, so its
/// channel is found, whose body names generation 2, which the state
/// converged on and we are owed: held at the body (the cursor stays),
/// nothing stored; once generation 2 installs, the re-read delivers it
/// with its text.
#[tokio::test]
async fn an_entry_whose_body_key_is_owed_is_held_then_delivered() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = keyed_reader(&dir, 2, &[(TEXT_KEY, 1)]).await;
    let signing = SigningKey::from_bytes(&[6; 32]);
    let author = signing.verifying_key().to_bytes();
    let channel = HostChannel::new(&host.ctx);
    let entry = split_entry(&signing, 1, 2, &NEXT_KEY);

    let held = channel
        .apply_outbox_entry(&member_read(&author), &entry)
        .await;
    assert!(
        matches!(held, Ok(Held::Pending)),
        "a body of a generation we are owed is held: {held:?}"
    );
    assert!(stored(&host, &owner).await.is_empty(), "nothing stored yet");

    install(&host, NEXT_KEY, 2);
    let held = channel
        .apply_outbox_entry(&member_read(&author), &entry)
        .await;
    assert!(matches!(held, Ok(Held::Stored)), "{held:?}");
    let rows = stored(&host, &owner).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].body, "hello", "delivered with its text");
    assert_eq!(rows[0].undecryptable_reason, None);
    host.shutdown().await.unwrap();
}

/// Plan KP: an entry naming generation 1, which we hold, whose body does
/// not open under it is damaged: stored without its body, reason
/// `damaged`, and published so; the cursor passes it.
#[tokio::test]
async fn an_entry_whose_held_key_does_not_open_it_is_stored_damaged() {
    use rekindle_types::dto::channel_message::UndecryptableReason;
    use rekindle_types::subscription_events::{
        ChannelMessageEvent, StreamFrame, SubscriptionEvent,
    };

    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = keyed_reader(&dir, 1, &[(TEXT_KEY, 1)]).await;
    let signing = SigningKey::from_bytes(&[6; 32]);
    let author = signing.verifying_key().to_bytes();
    let channel = HostChannel::new(&host.ctx);
    let mut events = host.ctx.journal.subscribe(None);
    let entry = sealed_entry(&signing, 1, &[9; 32]);

    let held = channel
        .apply_outbox_entry(&member_read(&author), &entry)
        .await;
    assert!(matches!(held, Ok(Held::Stored)), "{held:?}");
    let rows = stored(&host, &owner).await;
    assert_eq!(rows.len(), 1, "stored visibly, never skipped: {rows:?}");
    assert_eq!(rows[0].body, "");
    assert_eq!(rows[0].undecryptable_reason.as_deref(), Some("damaged"));

    let mut published = Vec::new();
    while let Ok(Some(frame)) =
        tokio::time::timeout(std::time::Duration::from_millis(50), events.next()).await
    {
        if let StreamFrame::Event(s) = frame {
            if let SubscriptionEvent::ChannelMessage(ChannelMessageEvent::New {
                body,
                undecryptable,
                ..
            }) = s.event
            {
                published.push((body, undecryptable.map(|u| u.reason)));
            }
        }
    }
    assert_eq!(
        published,
        [(None, Some(UndecryptableReason::Damaged))],
        "published without its body, with why"
    );
    host.shutdown().await.unwrap();
}

/// Plan KP: an entry from before our first key (generation 1, our first
/// held is 2) is history we never had: skipped, not held, not stored.
#[tokio::test]
async fn an_entry_from_before_our_first_key_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let (host, owner) = keyed_reader(&dir, 2, &[(NEXT_KEY, 2)]).await;
    let signing = SigningKey::from_bytes(&[6; 32]);
    let author = signing.verifying_key().to_bytes();
    let channel = HostChannel::new(&host.ctx);
    let entry = sealed_entry(&signing, 1, &TEXT_KEY);

    let held = channel
        .apply_outbox_entry(&member_read(&author), &entry)
        .await;
    assert!(
        matches!(held, Ok(Held::Stored)),
        "passed, never held for a key we will not get: {held:?}"
    );
    assert!(stored(&host, &owner).await.is_empty(), "and not stored");
    host.shutdown().await.unwrap();
}

/// Live finding CH1: the author creates a channel and posts in it at once,
/// and our diff of its outbox runs before the channel's `ChannelCreated`
/// merges here. The entry maps to no channel while we hold its key, so it
/// passes and the cursor moves past it; the merge that creates the channel
/// forgets the writer's cursor, so the re-diff stores the post.
#[tokio::test]
async fn a_post_read_before_its_channel_merged_is_shown_once_the_channel_merges() {
    use rekindle_governance_runtime::deps::GovernanceRuntimeDeps as _;
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    let mut before = with_channel();
    before.channels.clear();
    (host.ctx.communities).set_governance_state("c1", before);
    let cache = crate::daemon::mek_rotation::MekCacheAdapter::new(std::sync::Arc::clone(
        &host.ctx.mek_cache,
    ));
    let key = rekindle_transport::crypto::mek::Mek::from_bytes(TEXT_KEY, 1);
    rekindle_mek_rotation::ChannelMekCache::insert(&cache, "c1", KeyScope::Community, key);
    let (sim, mut nodes) = rekindle_netsim::Netsim::nodes(1, 89);
    let _pump = sim.start();
    let port = nodes.remove(0).ports.records.start_session().unwrap();
    let signing = SigningKey::from_bytes(&[6; 32]);
    let channel = HostChannel::new(&host.ctx);
    let sink = OutboxSink {
        channel: &channel,
        community_id: "c1",
        author: signing.verifying_key().to_bytes(),
        member_record: "VLD0:member",
        backfill: true,
        standing: Standing::Member,
        port: &*port,
        last: parking_lot::Mutex::new(None),
    };
    let stream = outbox_stream("c1");
    let writer = format!(
        "{}:VLD0:member",
        hex::encode(signing.verifying_key().to_bytes())
    );
    let entry = first_entry(&signing);
    let db = host.ctx.db.current().unwrap();
    let page = || {
        let (owner, db) = (owner.clone(), db.clone());
        async move {
            db.call(move |conn| rows::page(conn, &owner, EDIT_CHANNEL, None, 10))
                .await
                .unwrap()
                .into_iter()
                .map(|row| row.body)
                .collect::<Vec<String>>()
        }
    };

    let held = sink.apply(&stream, &writer, &entry).await;
    assert!(matches!(held, Ok(Held::Stored)), "{held:?}");
    assert_eq!(sink.cursor(&stream, &writer).await.unwrap(), 1);
    assert!(page().await.is_empty(), "no channel to show it in yet");

    host.governance().set_governance_state("c1", with_channel());
    let mut cursor = 1;
    for _ in 0..100 {
        cursor = sink.cursor(&stream, &writer).await.unwrap();
        if cursor == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(
        cursor, 0,
        "the channel's creation forgets the writer's cursor"
    );

    let held = sink.apply(&stream, &writer, &entry).await;
    assert!(matches!(held, Ok(Held::Stored)), "{held:?}");
    assert_eq!(page().await, ["hello"], "the re-diff shows the post");
    host.shutdown().await.unwrap();
}
