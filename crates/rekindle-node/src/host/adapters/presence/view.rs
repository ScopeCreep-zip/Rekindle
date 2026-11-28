//! The beat view in the Host (plan E3.4 S3), on both modes: each
//! community's [`BeatView`](rekindle_presence::community::BeatView) in its
//! `CommunityState`, fed by the ring's beat changes and `HeadAdvanced`
//! hints for subkey 1 (`governance::watch`, `governance::head_advanced`),
//! and expired locally by the 15-minute rule ([`run_expiry`]). What a member
//! shows is published as `PresenceEvent::CommunityMemberChanged`, the
//! event the member list already renders, so a member whose beats stop
//! turns offline on every reader without a new write.
//!
//! Each change of what the beats show, and each expiry, hands the shell's
//! voice roster every member's voice claim ([`reconcile_voice`], plan E3.4
//! S8-pre); a beat's sealed extras are opened with our current community
//! key ([`take_extras`]).

use std::sync::Weak;
use std::time::Duration;

use rekindle_codec::member_record::{BeatState, PresenceBeat, RecordScope};
use rekindle_presence::community::Observed;
use rekindle_secrets::keys::MediaEncryptionKey;
use rekindle_types::channel_keys::KeyScope;
use rekindle_types::id::PseudonymKey;
use rekindle_types::presence::{SessionLocation, SessionStatus, ShareScope};
use rekindle_types::subscription_events::{PresenceEvent, PresenceSnapshot, SubscriptionEvent};
use tokio_util::sync::CancellationToken;

use crate::daemon::dispatch::DaemonContext;
use crate::host::community::state::CommunityState;

/// The longest the expiry waits between looks: a member first seen while
/// it sleeps expires at most this late (well inside the beat period).
pub(crate) const EXPIRY_CHECK: Duration = Duration::from_secs(30);

/// Take `author`'s verified `beat` in `community` into the view, delivered
/// at `arrived_at_ms` (`None` for a cold read), and publish what it shows
/// when that changed. Our own beat (read back from a relayed hint) is never
/// taken in: our status is our own presence's (`host::own_presence`), which
/// our row and the buddy list both show, while our beat lags an idle change
/// by design (plan E3.4 "Constant rate"); a view entry for us would draw our
/// own row from that lagging copy (finding PR1).
pub(crate) fn observe(
    ctx: &DaemonContext,
    at: RecordScope<'_>,
    author: &PseudonymKey,
    beat: &PresenceBeat,
    arrived_at_ms: Option<u64>,
) -> Observed {
    let community = at.community_id;
    let member = author.to_hex();
    let key = community_key(ctx, community);
    let now = ctx.clock.now_ms();
    let (observed, shown) = {
        let mut map = ctx.communities.map().write();
        let Some(state) = map.get_mut(community) else {
            return Observed::default();
        };
        if state.my_pseudonym_key.as_deref() == Some(member.as_str()) {
            return Observed::default();
        }
        let before = location(state, &member, at, key.as_ref());
        let observed = state.beat_view.observe(&member, beat, arrived_at_ms, now);
        let moved = before != location(state, &member, at, key.as_ref());
        let shown = observed
            .shown
            .or_else(|| moved.then(|| state.beat_view.shown(&member)));
        (observed, shown)
    };
    if let Some(shown) = shown {
        publish(ctx, community, member, shown);
    }
    if observed.shown.is_some() || observed.voice_changed {
        reconcile_voice(ctx, community);
    }
    observed
}

/// Refresh the community roster when our own reciprocity changes.
pub(crate) fn location_policy_changed(ctx: &DaemonContext, community: &str) {
    let me = ctx
        .communities
        .map()
        .read()
        .get(community)
        .and_then(|state| state.my_pseudonym_key.clone());
    if let Some(me) = me {
        publish(ctx, community, me, own_shown(ctx).into());
    }
}

/// Hand our voice roster every member's voice claim in `community` as its
/// beats show it now (plan E3.4 S8-pre: the beat ring's `voiceChannel`
/// view feeds the reconcile).
pub(crate) fn reconcile_voice(ctx: &DaemonContext, community: &str) {
    let session = super::session_presence(ctx);
    let rows = (ctx.communities.map().read())
        .get(community)
        .map(|c| c.beat_view.voice_rows());
    if let Some(rows) = rows {
        session.reconcile_voice_roster(community, rows);
    }
}

/// What `member` (pseudonym hex) shows in `community`'s beat view now.
pub(crate) fn shown(ctx: &DaemonContext, community: &str, member: &str) -> BeatState {
    ctx.communities
        .map()
        .read()
        .get(community)
        .map_or(BeatState::Offline, |c| c.beat_view.shown(member))
}

/// Read the peer's status and reciprocated location from the same beat.
pub(crate) fn shown_location(
    ctx: &DaemonContext,
    community: &str,
    member: &str,
    record: Option<&str>,
) -> (BeatState, Option<SessionLocation>) {
    let key = community_key(ctx, community);
    let map = ctx.communities.map().read();
    let Some(state) = map.get(community) else {
        return (BeatState::Offline, None);
    };
    let shown = state.beat_view.shown(member);
    let location = record.and_then(|member_record| {
        let at = RecordScope {
            community_id: community,
            member_record,
        };
        location(state, member, at, key.as_ref())
    });
    (shown, location)
}

fn community_key(ctx: &DaemonContext, community: &str) -> Option<MediaEncryptionKey> {
    ctx.mek_cache
        .read()
        .current(community, KeyScope::Community)
        .cloned()
}

fn location(
    state: &CommunityState,
    member: &str,
    at: RecordScope<'_>,
    key: Option<&MediaEncryptionKey>,
) -> Option<SessionLocation> {
    if state.presence_policy.share_location != ShareScope::Members {
        return None;
    }
    let sealed = state.beat_view.sealed_extras(member)?;
    rekindle_presence::community::open_extras(sealed, key, at, &state.presence_policy)?.location
}

/// The status our own row shows (finding PR1, plan E3.4 S3): our own
/// presence's (`host::own_presence`), never our lagging beat, with a hidden
/// status ([`rekindle_presence::hides_presence`]: Offline or Invisible)
/// folded to Offline as the wire folds it
/// ([`SessionStatus::as_wire_str`]), so our row never shows us present
/// while we hide. The member list draws our row from it and
/// [`online_count`] counts us by it, so the two never disagree.
pub(crate) fn own_shown(ctx: &DaemonContext) -> SessionStatus {
    let status = ctx.own_presence.status();
    if rekindle_presence::hides_presence(status) {
        SessionStatus::Offline
    } else {
        status
    }
}

/// How many members of `community` show present now (plan E7.15 IV4, the
/// Infoview's online count; finding OC1): those its beats show present,
/// which never hold us ([`observe`]), plus us when we hold a seat and our
/// own row shows us present ([`own_shown`]), as Discord and Slack count
/// yourself online.
pub(crate) fn online_count(ctx: &DaemonContext, community: &str) -> usize {
    let me_present = own_shown(ctx) != SessionStatus::Offline;
    ctx.communities.map().read().get(community).map_or(0, |c| {
        c.beat_view.present_count() + usize::from(me_present && c.my_pseudonym_key.is_some())
    })
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
    let mut communities = std::collections::BTreeSet::new();
    for (community, member) in turned {
        publish(ctx, &community, member, BeatState::Offline);
        communities.insert(community);
    }
    for community in communities {
        reconcile_voice(ctx, &community);
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
