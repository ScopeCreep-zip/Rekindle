//! Channel typing in the Host (plan E3.4 S9): per-edit reports are not
//! per-edit sends, a start lapses with no stop frame, a re-send keeps it,
//! a message ends it, and our own message ends our session.

use std::time::{Duration, Instant};

use rekindle_events::typing::TypingSessions;
use rekindle_transport::operations::friend::FriendHost;
use rekindle_transport::operations::typing;
use rekindle_types::subscription_events::TypingContext;

use rekindle_secrets::channel_body::channel_tag;
use rekindle_types::channel_keys::KeyScope;

use super::{context, decide, Draft, CHANNEL_TYPING_TTL_MS};
use crate::host::adapters::friends::HostFriends;
use crate::host::adapters::governance::HostGovernance;
use crate::host::direct_tests::ann;
use crate::host::runtime::tests::channel_change_tests::{EDIT_CHANNEL, TEXT_KEY};
use crate::host::runtime::tests::channel_gap_tests::with_channel;
use crate::host::runtime::tests::{create_ann, offline_host, seed_registry_community};

const TYPING_COMMUNITY: &str = "community";
const TYPING_CHANNEL: &str = "general";
const TYPIST_BOB: &str = "bob";

fn channel() -> TypingContext {
    context(TYPING_COMMUNITY, TYPING_CHANNEL)
}

/// Wait up to `limit` for a `Stopped` frame in `stream`; when it came.
async fn stopped_within(
    stream: &mut rekindle_events::EventStream,
    limit: Duration,
) -> Option<Instant> {
    let deadline = Instant::now() + limit;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match tokio::time::timeout(left, stream.next()).await {
            Ok(Some(frame)) if format!("{frame:?}").contains("Stopped") => {
                return Some(Instant::now())
            }
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return None,
        }
    }
    None
}

#[test]
fn ten_reports_within_three_seconds_are_one_send() {
    let mut sessions = TypingSessions::default();
    let sends = (0..10u64)
        .map(|i| decide(&mut sessions, channel(), true, i * 299))
        .filter(|draft| *draft != Draft::Quiet)
        .count();
    assert_eq!(sends, 1, "per-edit reports must not be per-edit sends");
    assert_eq!(
        decide(&mut sessions, channel(), true, 3_000),
        Draft::Refresh
    );
    assert_eq!(
        decide(&mut sessions, channel(), false, 3_100),
        Draft::Stopped
    );
    assert_eq!(decide(&mut sessions, channel(), false, 3_200), Draft::Quiet);
}

/// A start whose stop never comes is lapsed by the Host, capped at 10 s
/// whatever the sender's ttl.
#[tokio::test]
async fn a_typist_with_no_stop_frame_expires_within_ten_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _) = ann(&dir).await;
    let mut events = host.ctx.journal.subscribe(None);
    let friends = HostFriends::of(&host.ctx);
    let start = Instant::now();
    typing::channel_frame(&friends, channel(), TYPIST_BOB, "Bob".into(), 60_000);
    let ended = stopped_within(&mut events, Duration::from_secs(12)).await;
    let ended = ended.expect("the Host lapses the start without a stop frame");
    assert!(ended.duration_since(start) <= Duration::from_millis(10_500));
}

/// A re-send every 3 s keeps the typist past the 6 s ttl of the first start.
#[tokio::test]
async fn a_resend_keeps_the_typist_past_six_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _) = ann(&dir).await;
    let mut events = host.ctx.journal.subscribe(None);
    let friends = HostFriends::of(&host.ctx);
    typing::channel_frame(
        &friends,
        channel(),
        TYPIST_BOB,
        "Bob".into(),
        CHANNEL_TYPING_TTL_MS,
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    typing::channel_frame(
        &friends,
        channel(),
        TYPIST_BOB,
        "Bob".into(),
        CHANNEL_TYPING_TTL_MS,
    );
    assert!(
        stopped_within(&mut events, Duration::from_millis(4_500))
            .await
            .is_none(),
        "the re-sent start must outlive the first one's 6 s"
    );
    assert!(stopped_within(&mut events, Duration::from_secs(3))
        .await
        .is_some());
}

/// The typist's message (the ingest's hook) ends their typing at once; a
/// `ttlMs = 0` frame is their stop.
#[tokio::test]
async fn a_message_or_a_stop_frame_ends_typing() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _) = ann(&dir).await;
    let mut events = host.ctx.journal.subscribe(None);
    let friends = HostFriends::of(&host.ctx);
    typing::channel_frame(
        &friends,
        channel(),
        TYPIST_BOB,
        "Bob".into(),
        CHANNEL_TYPING_TTL_MS,
    );
    typing::stopped(&friends, &channel(), TYPIST_BOB);
    assert!(stopped_within(&mut events, Duration::from_millis(500))
        .await
        .is_some());
    typing::channel_frame(
        &friends,
        channel(),
        "carol",
        "Carol".into(),
        CHANNEL_TYPING_TTL_MS,
    );
    typing::channel_frame(&friends, channel(), "carol", "Carol".into(), 0);
    assert!(stopped_within(&mut events, Duration::from_millis(500))
        .await
        .is_some());
}

/// Our message ends our typing session with no report from the window.
#[tokio::test]
async fn our_message_ends_our_typing_session() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _) = ann(&dir).await;
    let friends = HostFriends::of(&host.ctx);
    let now = friends.clock().now_ms();
    let draft = decide(
        &mut friends.stores().typing_out.lock(),
        channel(),
        true,
        now,
    );
    assert_eq!(draft, Draft::Started);
    HostGovernance::new(host.ctx.clone()).end_channel_typing(TYPING_COMMUNITY, TYPING_CHANNEL);
    assert!(
        !friends.stores().typing_out.lock().end(&channel()),
        "the session ended with our message"
    );
}

/// A typist whose key is a real member key, signed for `seed`.
fn typist_key(seed: u8) -> String {
    let signing = rekindle_secrets::ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
    hex::encode(signing.verifying_key().to_bytes())
}

/// A Host holding community `c1` with `EDIT_CHANNEL` under generation 1
/// of the community text key `TEXT_KEY`, and its journal.
async fn seeded() -> (
    tempfile::TempDir,
    std::sync::Arc<crate::host::Host>,
    rekindle_events::EventStream,
) {
    let dir = tempfile::tempdir().unwrap();
    let host = offline_host(&dir).await;
    let owner = create_ann(&host).await.public_key;
    seed_registry_community(&host, &owner).await;
    (host.ctx.communities).set_governance_state("c1", with_channel());
    let cache = crate::daemon::mek_rotation::MekCacheAdapter::new(std::sync::Arc::clone(
        &host.ctx.mek_cache,
    ));
    let key = rekindle_transport::crypto::mek::Mek::from_bytes(TEXT_KEY, 1);
    rekindle_mek_rotation::ChannelMekCache::insert(&cache, "c1", KeyScope::Community, key);
    let events = host.ctx.journal.subscribe(None);
    (dir, host, events)
}

/// The next typing event in `stream` within `limit`, as debug text.
async fn next_typing(stream: &mut rekindle_events::EventStream, limit: Duration) -> Option<String> {
    let deadline = Instant::now() + limit;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match tokio::time::timeout(left, stream.next()).await {
            Ok(Some(frame)) => {
                let text = format!("{frame:?}");
                if text.contains("Typing(") {
                    return Some(text);
                }
            }
            Ok(None) | Err(_) => return None,
        }
    }
    None
}

/// A gossip typing frame through the ingest's hook names the channel
/// behind its tag and the typist by the Host's name; its stop
/// (`ttlMs = 0`) ends it.
#[tokio::test]
async fn a_gossip_typing_frame_starts_and_its_stop_frame_stops() {
    let (_dir, host, mut events) = seeded().await;
    let gov = HostGovernance::new(host.ctx.clone());
    let bob = typist_key(9);
    let tag = channel_tag(&TEXT_KEY, EDIT_CHANNEL);
    gov.on_typing_frame("c1", &bob, &bob, &tag, CHANNEL_TYPING_TTL_MS);
    let started = next_typing(&mut events, Duration::from_millis(500)).await;
    let started = started.expect("the frame becomes a Started");
    assert!(started.contains("Started"), "{started}");
    assert!(started.contains(EDIT_CHANNEL), "{started}");
    gov.on_typing_frame("c1", &bob, &bob, &tag, 0);
    let stopped = next_typing(&mut events, Duration::from_millis(500)).await;
    assert!(stopped.is_some_and(|s| s.contains("Stopped")));
}

/// A frame naming a typist other than its signer is dropped.
#[tokio::test]
async fn a_frame_naming_someone_but_its_signer_is_dropped() {
    let (_dir, host, mut events) = seeded().await;
    let gov = HostGovernance::new(host.ctx.clone());
    let tag = channel_tag(&TEXT_KEY, EDIT_CHANNEL);
    gov.on_typing_frame(
        "c1",
        &typist_key(9),
        &typist_key(10),
        &tag,
        CHANNEL_TYPING_TTL_MS,
    );
    assert_eq!(
        next_typing(&mut events, Duration::from_millis(500)).await,
        None
    );
}

/// A frame under a key we do not hold as current (rotation) names no
/// channel: dropped silently, no event and no error.
#[tokio::test]
async fn a_frame_under_a_key_we_do_not_hold_is_dropped_silently() {
    let (_dir, host, mut events) = seeded().await;
    let gov = HostGovernance::new(host.ctx.clone());
    let bob = typist_key(9);
    let stale = channel_tag(&[8; 32], EDIT_CHANNEL);
    gov.on_typing_frame("c1", &bob, &bob, &stale, CHANNEL_TYPING_TTL_MS);
    assert_eq!(
        next_typing(&mut events, Duration::from_millis(500)).await,
        None
    );
}

/// An unlisted typist is named by their short fingerprint, never their
/// raw key; a verified card's name wins.
#[tokio::test]
async fn an_unlisted_typist_is_named_by_the_fingerprint() {
    let (_dir, host, mut events) = seeded().await;
    let gov = HostGovernance::new(host.ctx.clone());
    let bob = typist_key(9);
    let fingerprint = crate::host::adapters::governance::community_profile::shown_name(
        None,
        &rekindle_types::id::PseudonymKey::from_hex_lossy(&bob),
    );
    assert!(!fingerprint.is_empty() && fingerprint != bob);
    assert_eq!(gov.typist_name("c1", &bob), fingerprint);
    let tag = channel_tag(&TEXT_KEY, EDIT_CHANNEL);
    gov.on_typing_frame("c1", &bob, &bob, &tag, CHANNEL_TYPING_TTL_MS);
    let started = next_typing(&mut events, Duration::from_millis(500))
        .await
        .unwrap();
    assert!(
        started.contains(&format!("name: {fingerprint:?}")),
        "{started}"
    );
    let carol = typist_key(11);
    let noted = (host.ctx.communities).note_member_card(
        "c1",
        &rekindle_types::id::PseudonymKey::from_hex_lossy(&carol),
        rekindle_governance_runtime::CardText {
            display_name: Some("Carol".into()),
            custom_status: None,
        },
    );
    assert!(noted);
    assert_eq!(gov.typist_name("c1", &carol), "Carol");
}
