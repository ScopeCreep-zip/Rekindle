//! The beat view in the Host (plan E3.4 S3), on both modes: each
//! community's [`BeatView`](rekindle_presence::community::BeatView) in its
//! `CommunityState`, fed by the ring's beat changes and `HeadAdvanced`
//! hints for subkey 1 (`governance::watch`, `governance::head_advanced`),
//! and expired locally by the 15-minute rule ([`run_expiry`]). What a member
//! shows is published as `PresenceEvent::CommunityMemberChanged`, the
//! event the member list already renders, so a member whose beats stop
//! turns offline on every reader without a new write.

use std::sync::Weak;
use std::time::Duration;

use rekindle_codec::member_record::{BeatState, PresenceBeat};
use rekindle_presence::community::Observed;
use rekindle_types::id::PseudonymKey;
use rekindle_types::subscription_events::{PresenceEvent, PresenceSnapshot, SubscriptionEvent};
use tokio_util::sync::CancellationToken;

use crate::daemon::dispatch::DaemonContext;

/// The longest the expiry waits between looks: a member first seen while
/// it sleeps expires at most this late (well inside the beat period).
pub(crate) const EXPIRY_CHECK: Duration = Duration::from_secs(30);

/// Take `author`'s verified `beat` in `community` into the view, delivered
/// at `arrived_at_ms` (`None` for a cold read), and publish what it shows
/// when that changed.
pub(crate) fn observe(
    ctx: &DaemonContext,
    community: &str,
    author: &PseudonymKey,
    beat: &PresenceBeat,
    arrived_at_ms: Option<u64>,
) -> Observed {
    let member = author.to_hex();
    let now = ctx.clock.now_ms();
    let observed = {
        let mut map = ctx.communities.map().write();
        let Some(state) = map.get_mut(community) else {
            return Observed::default();
        };
        state.beat_view.observe(&member, beat, arrived_at_ms, now)
    };
    if let Some(shown) = observed.shown {
        publish(ctx, community, member, shown);
    }
    observed
}

/// What `member` (pseudonym hex) shows in `community`'s beat view now.
pub(crate) fn shown(ctx: &DaemonContext, community: &str, member: &str) -> BeatState {
    ctx.communities
        .map()
        .read()
        .get(community)
        .map_or(BeatState::Offline, |c| c.beat_view.shown(member))
}

/// How many members of `community` its beats show present now (plan
/// E7.15 IV4, the Infoview's online count).
pub(crate) fn present_count(ctx: &DaemonContext, community: &str) -> usize {
    ctx.communities
        .map()
        .read()
        .get(community)
        .map_or(0, |c| c.beat_view.present_count())
}

/// Whether `member` (pseudonym hex) may be present in `community`: no beat
/// of theirs reached us yet, or it shows them present
/// ([`rekindle_presence::community::BeatView::may_be_present`]).
pub(crate) fn may_be_present(ctx: &DaemonContext, community: &str, member: &str) -> bool {
    ctx.communities
        .map()
        .read()
        .get(community)
        .is_some_and(|c| c.beat_view.may_be_present(member))
}

/// Turn every member whose beat went stale offline, in every community;
/// the earliest next expiry.
fn expire_all(ctx: &DaemonContext) -> Option<u64> {
    let now = ctx.clock.now_ms();
    let mut turned = Vec::new();
    let mut next: Option<u64> = None;
    {
        let mut map = ctx.communities.map().write();
        for (community, state) in map.iter_mut() {
            for member in state.beat_view.expire(now) {
                turned.push((community.clone(), member));
            }
            if let Some(at) = state.beat_view.next_expiry() {
                next = Some(next.map_or(at, |n| n.min(at)));
            }
        }
    }
    for (community, member) in turned {
        publish(ctx, &community, member, BeatState::Offline);
    }
    next
}

/// Expire stale members until `stop` (the unlock's scope): wakes at the
/// earliest expiry, or after [`EXPIRY_CHECK`] when that is later. A local
/// timer over the view, never a read of a record (D4).
pub(crate) async fn run_expiry(ctx: Weak<DaemonContext>, stop: CancellationToken) {
    loop {
        let wait = {
            let Some(ctx) = ctx.upgrade() else {
                return;
            };
            let now = ctx.clock.now_ms();
            expire_all(&ctx).map_or(EXPIRY_CHECK, |at| {
                EXPIRY_CHECK.min(Duration::from_millis(at.saturating_sub(now)))
            })
        };
        tokio::select! {
            biased;
            () = stop.cancelled() => return,
            () = tokio::time::sleep(wait) => {}
        }
    }
}

fn publish(ctx: &DaemonContext, community: &str, member: String, shown: BeatState) {
    ctx.publish_event(SubscriptionEvent::Presence(
        PresenceEvent::CommunityMemberChanged {
            community: community.to_owned(),
            pseudonym: member,
            // Status only: a beat says nothing of the member's game.
            snapshot: PresenceSnapshot::status(shown_status(shown)),
        },
    ));
}

/// The member list's status for what a beat shows.
pub(crate) fn shown_status(shown: BeatState) -> &'static str {
    match shown {
        BeatState::Online => "online",
        BeatState::Away => "away",
        BeatState::Busy => "busy",
        BeatState::Offline | BeatState::Departed => "offline",
    }
}
