//! The local activity feed through the Host (plan E7.15, IV2; Infoview
//! design §2.2–§3; lead decision (51)): one worker folds the Host's own
//! published journal events (a friend's presence, name, bio and new
//! friendship; our favourite added and community joined), compares each
//! friend's state with the last it saw of them
//! (`rekindle_presence::activity`), stores the entries
//! (`rekindle_db::repo::activity`) and pushes them as
//! `SystemEvent::ActivityAppended`; `ActivityList` pages them. No producer
//! calls the feed. Names are the Host's at read time, never the entry's.
//!
//! Only data a friend already sent us feeds it (design §3.1): a friend not
//! sharing their game sends none, so no game entry is made. Off
//! (`keep_activity_log`), nothing is stored.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use rekindle_db::repo::activity::{self as repo, StoredActivity};
use rekindle_presence::activity::{friend_entries, FriendActivitySnapshot};
use rekindle_types::activity::{ActivityEntry, ActivityListItem};
use rekindle_types::friend::{FriendState, FriendshipState};
use rekindle_types::subscription_events::{
    FriendEvent, MembershipEvent, PresenceEvent, StreamFrame, SubscriptionEvent, SystemEvent,
};

use crate::daemon::dispatch::DaemonContext;

/// One change of a friend: their projection row before (`None`: not a
/// friend before) and after, and their bio (before, after) when the change
/// is the bio's; otherwise the stored bio is both.
pub(crate) struct FriendChange {
    pub prev: Option<FriendState>,
    pub next: FriendState,
    pub bio: Option<(Option<String>, Option<String>)>,
}

fn snapshot(
    friend: &FriendState,
    bio: Option<String>,
    played_games: Option<BTreeSet<u32>>,
) -> FriendActivitySnapshot {
    FriendActivitySnapshot {
        public_key: friend.public_key.clone(),
        accepted: friend.friendship_state == FriendshipState::Accepted,
        display_name: friend.display_name.clone(),
        status_message: friend.status_message.clone(),
        bio,
        game: friend.game_info.clone(),
        played_games,
    }
}

/// Store the entries `change` makes while the log is on, and push them.
pub(crate) async fn ingest(ctx: &DaemonContext, change: FriendChange) {
    let (Ok(owner), Ok(db)) = (ctx.require_owner(), ctx.db.current()) else {
        return;
    };
    let at = ctx.clock.now_ms_i64();
    let stored = db
        .call(move |conn| {
            if !rekindle_db::repo::preferences::load(conn, &owner)?.keep_activity_log {
                return Ok(Vec::new());
            }
            let key = change.next.public_key.clone();
            let (prev_bio, next_bio) = if let Some(pair) = change.bio {
                pair
            } else {
                let bio = rekindle_db::repo::bio::friend(conn, &owner, &key)?;
                (bio.clone(), bio)
            };
            let played = rekindle_db::repo::friend_game_totals::game_ids(conn, &owner, &key)?;
            let prev = change
                .prev
                .as_ref()
                .map(|f| snapshot(f, prev_bio, played.clone()));
            let next = snapshot(&change.next, next_bio, played);
            let entries = friend_entries(prev.as_ref(), &next, at);
            repo::insert_returning(conn, &owner, &entries, at)
        })
        .await;
    match stored {
        Ok(rows) => publish(ctx, rows),
        Err(e) => tracing::warn!(error = %e, "activity entries not stored"),
    }
}

/// Store our own `entry` (a favourite added, a community joined) while the
/// log is on, and push it.
async fn own(ctx: &DaemonContext, entry: ActivityEntry) {
    let (Ok(owner), Ok(db)) = (ctx.require_owner(), ctx.db.current()) else {
        return;
    };
    let at = ctx.clock.now_ms_i64();
    let stored = db
        .call(move |conn| {
            if !rekindle_db::repo::preferences::load(conn, &owner)?.keep_activity_log {
                return Ok(Vec::new());
            }
            repo::insert_returning(conn, &owner, &[entry], at)
        })
        .await;
    match stored {
        Ok(rows) => publish(ctx, rows),
        Err(e) => tracing::warn!(error = %e, "own activity entry not stored"),
    }
}

fn publish(ctx: &DaemonContext, rows: Vec<StoredActivity>) {
    if rows.is_empty() {
        return;
    }
    let entries = items(ctx, rows);
    ctx.publish_event(SubscriptionEvent::System(SystemEvent::ActivityAppended {
        entries,
    }));
}

/// `rows` with each friend's names as the Host holds them now.
pub(crate) fn items(ctx: &DaemonContext, rows: Vec<StoredActivity>) -> Vec<ActivityListItem> {
    let session = ctx.session.read();
    rows.into_iter()
        .map(|row| {
            let friend = row
                .entry
                .kind
                .friend()
                .and_then(|key| session.as_ref().and_then(|s| s.friends.get(key)));
            ActivityListItem {
                id: row.id,
                at: row.entry.at,
                display_name: friend.map(|f| f.display_name.clone()),
                nickname: friend.and_then(|f| f.nickname.clone()),
                identity_pin_mismatch: friend.is_some_and(|f| f.identity_pin_mismatch),
                kind: row.entry.kind,
            }
        })
        .collect()
}

/// What the worker last saw of each friend: their projection row and bio.
type Seen = HashMap<String, (FriendState, Option<String>)>;

/// The worker: fold the Host's journal until shutdown. Each session's
/// `seen` is rebuilt from the database at its first event (the journal's
/// `InvalidSession` opens it, before the session is installed).
pub(crate) async fn run_worker(ctx: Arc<DaemonContext>) {
    let mut stream = ctx.journal.subscribe(None);
    let mut seen: Option<Seen> = None;
    loop {
        let frame = tokio::select! {
            () = ctx.shutdown.requested() => return,
            frame = stream.next() => frame,
        };
        match frame {
            Some(StreamFrame::Event(s)) => {
                if seen.is_none() {
                    seen = stored_friends(&ctx).await;
                }
                if let Some(seen) = seen.as_mut() {
                    fold(&ctx, seen, &s.event).await;
                }
            }
            Some(StreamFrame::InvalidSession) => seen = None,
            Some(_) => {}
            None => return,
        }
    }
}

/// The session's friends as stored, with their bios; `None` while no
/// session is installed.
async fn stored_friends(ctx: &DaemonContext) -> Option<Seen> {
    let (Ok(owner), Ok(db)) = (ctx.require_owner(), ctx.db.current()) else {
        return None;
    };
    let read = db
        .call(move |conn| {
            let mut seen = Seen::new();
            for row in rekindle_db::repo::friends::load_all(conn, &owner)? {
                let bio = rekindle_db::repo::bio::friend(conn, &owner, &row.public_key)?;
                let friend = super::session::friend_state(row);
                seen.insert(friend.public_key.clone(), (friend, bio));
            }
            Ok(seen)
        })
        .await;
    match read {
        Ok(seen) => Some(seen),
        Err(e) => {
            tracing::warn!(error = %e, "activity feed: friends not read");
            Some(Seen::new())
        }
    }
}

/// One stored friend and their bio, for a friend `seen` does not hold.
async fn stored_friend(ctx: &DaemonContext, peer: &str) -> Option<(FriendState, Option<String>)> {
    let (Ok(owner), Ok(db)) = (ctx.require_owner(), ctx.db.current()) else {
        return None;
    };
    let peer = peer.to_owned();
    db.call(move |conn| {
        let Some(row) = rekindle_db::repo::friends::load_all(conn, &owner)?
            .into_iter()
            .find(|r| r.public_key == peer)
        else {
            return Ok(None);
        };
        let bio = rekindle_db::repo::bio::friend(conn, &owner, &peer)?;
        Ok(Some((super::session::friend_state(row), bio)))
    })
    .await
    .ok()
    .flatten()
}

/// How a published event touches a friend.
enum Touch<'a> {
    /// Their presence or name: the row changed.
    Row(&'a str),
    /// Their bio, stored before the event was published.
    Bio(&'a str),
    /// A new friendship: no state before.
    Accepted(&'a str),
    /// No longer a friend: forget them.
    Gone(&'a str),
}

fn touch(event: &SubscriptionEvent) -> Option<Touch<'_>> {
    Some(match event {
        SubscriptionEvent::Presence(PresenceEvent::FriendChanged { peer_key, .. })
        | SubscriptionEvent::Friend(FriendEvent::DisplayNameChanged { peer_key, .. }) => {
            Touch::Row(peer_key)
        }
        SubscriptionEvent::Friend(FriendEvent::BioChanged { peer_key, .. }) => Touch::Bio(peer_key),
        SubscriptionEvent::Friend(FriendEvent::Accepted { peer_key, .. }) => {
            Touch::Accepted(peer_key)
        }
        SubscriptionEvent::Friend(
            FriendEvent::Removed { peer_key }
            | FriendEvent::BlockChanged {
                peer_key,
                blocked: true,
            },
        ) => Touch::Gone(peer_key),
        _ => return None,
    })
}

/// Fold one published event into the feed.
async fn fold(ctx: &DaemonContext, seen: &mut Seen, event: &SubscriptionEvent) {
    let now = ctx.clock.now_ms_i64();
    match event {
        SubscriptionEvent::Membership(MembershipEvent::SelfJoined { community, .. }) => {
            return own(
                ctx,
                rekindle_presence::activity::joined_community(community, now),
            )
            .await;
        }
        SubscriptionEvent::System(SystemEvent::FavouriteServerAdded {
            game_id,
            server,
            label,
        }) => {
            let entry = rekindle_presence::activity::favourite_added(*game_id, server, label, now);
            return own(ctx, entry).await;
        }
        _ => {}
    }
    let Some(touch) = touch(event) else {
        return;
    };
    let (peer, prev) = match touch {
        Touch::Gone(peer) => {
            seen.remove(peer);
            return;
        }
        Touch::Accepted(peer) => (peer, None),
        Touch::Row(peer) | Touch::Bio(peer) => match seen.get(peer).cloned() {
            Some(prev) => (peer, Some(prev)),
            None => (peer, stored_friend(ctx, peer).await),
        },
    };
    let next = ctx
        .session
        .read()
        .as_ref()
        .and_then(|s| s.friends.get(peer).cloned());
    let Some(next) = next else {
        seen.remove(peer);
        return;
    };
    let prev_bio = prev.as_ref().and_then(|(_, bio)| bio.clone());
    let next_bio = if matches!(touch, Touch::Bio(_)) || prev.is_none() {
        stored_bio(ctx, peer).await
    } else {
        prev_bio.clone()
    };
    seen.insert(peer.to_owned(), (next.clone(), next_bio.clone()));
    let change = FriendChange {
        prev: prev.map(|(friend, _)| friend),
        next,
        bio: Some((prev_bio, next_bio)),
    };
    ingest(ctx, change).await;
}

async fn stored_bio(ctx: &DaemonContext, peer: &str) -> Option<String> {
    let (Ok(owner), Ok(db)) = (ctx.require_owner(), ctx.db.current()) else {
        return None;
    };
    let peer = peer.to_owned();
    db.call(move |conn| rekindle_db::repo::bio::friend(conn, &owner, &peer))
        .await
        .ok()
        .flatten()
}
