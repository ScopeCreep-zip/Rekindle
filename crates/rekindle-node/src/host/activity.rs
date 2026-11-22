//! The local activity feed through the Host (plan E7.15, IV2; Infoview
//! design §2.2–§3): the friend ingest compares a friend's stored state
//! before and after each change it applies (`rekindle_presence::activity`)
//! and stores the entries (`rekindle_db::repo::activity`), then pushes them
//! as `SystemEvent::ActivityAppended`; `ActivityList` pages them. Names are
//! the Host's at read time, never the entry's.
//!
//! Only data a friend already sent us feeds it (design §3.1): a friend not
//! sharing their game sends none, so no game entry is made. Off
//! (`keep_activity_log`), nothing is stored.

use std::collections::BTreeSet;

use rekindle_db::repo::activity::{self as repo, StoredActivity};
use rekindle_presence::activity::{friend_entries, FriendActivitySnapshot};
use rekindle_types::activity::{ActivityEntry, ActivityListItem};
use rekindle_types::friend::{FriendState, FriendshipState};
use rekindle_types::subscription_events::{SubscriptionEvent, SystemEvent};

use super::adapters::friends::HostFriends;
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

impl HostFriends {
    /// Feed `change` to the activity log off the caller's path.
    pub(crate) fn note_friend_change(&self, change: FriendChange) {
        let host = self.clone();
        rekindle_transport::operations::friend::FriendHost::scope(self).spawn_or_drop(
            "activity feed",
            async move { ingest(host.ctx(), change).await },
        );
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
            let (prev_bio, next_bio) = match change.bio {
                Some(pair) => pair,
                None => {
                    let bio = rekindle_db::repo::bio::friend(conn, &owner, &key)?;
                    (bio.clone(), bio)
                }
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
pub(crate) async fn own(ctx: &DaemonContext, entry: ActivityEntry) {
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
