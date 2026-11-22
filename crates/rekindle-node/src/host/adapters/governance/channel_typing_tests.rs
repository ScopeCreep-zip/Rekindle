//! Channel typing in the Host (plan E3.4 S9): per-edit reports are not
//! per-edit sends, a start lapses with no stop frame, a re-send keeps it,
//! a message ends it, and our own message ends our session.

use std::time::{Duration, Instant};

use rekindle_events::typing::TypingSessions;
use rekindle_transport::operations::friend::FriendHost;
use rekindle_transport::operations::typing;
use rekindle_types::subscription_events::TypingContext;

use super::{context, decide, Draft, CHANNEL_TYPING_TTL_MS};
use crate::host::adapters::friends::HostFriends;
use crate::host::adapters::governance::HostGovernance;
use crate::host::direct_tests::ann;

const COMMUNITY: &str = "community";
const CHANNEL: &str = "general";
const BOB: &str = "bob";

fn channel() -> TypingContext {
    context(COMMUNITY, CHANNEL)
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
    typing::channel_frame(&friends, channel(), BOB, 60_000);
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
    typing::channel_frame(&friends, channel(), BOB, CHANNEL_TYPING_TTL_MS);
    tokio::time::sleep(Duration::from_secs(3)).await;
    typing::channel_frame(&friends, channel(), BOB, CHANNEL_TYPING_TTL_MS);
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
    typing::channel_frame(&friends, channel(), BOB, CHANNEL_TYPING_TTL_MS);
    typing::stopped(&friends, &channel(), BOB);
    assert!(stopped_within(&mut events, Duration::from_millis(500))
        .await
        .is_some());
    typing::channel_frame(&friends, channel(), "carol", CHANNEL_TYPING_TTL_MS);
    typing::channel_frame(&friends, channel(), "carol", 0);
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
    HostGovernance::new(host.ctx.clone()).end_channel_typing(COMMUNITY, CHANNEL);
    assert!(
        !friends.stores().typing_out.lock().end(&channel()),
        "the session ended with our message"
    );
}
