//! Game totals shared with friends (plan E7.2): opt-in and off by default
//! (`Preferences.share_game_totals`, set only by the trust-changing
//! `SetShareGameTotals`). While on, our totals go to each accepted friend
//! as ratchet-sealed `GameTotals` (0x0172), on change, to a newly accepted
//! friend, and to every friend when sharing is turned on. Never in the
//! profile record: its key travels in every invite and friend code, so
//! anyone holding one would read it. A received frame replaces what we
//! held from that friend (`friend_game_totals`, removed with the friend).

use rekindle_codec::capnp_codec::direct::{DirectBody, GameTotal, GameTotals, MAX_GAME_TOTALS};
use rekindle_db::repo::game_sessions;
use rekindle_types::friend::FriendshipState;

use super::{state, FriendHost};
use crate::operations::dm;

/// Our totals as the frame states them, when sharing is on; `None` while
/// it is off or unreadable (nothing is sent).
async fn ours_if_shared(host: &dyn FriendHost) -> Option<DirectBody> {
    let owner = state::owner(host).ok()?;
    let read = state::db_call!(host, "read the game totals to share", move |conn| {
        let prefs = rekindle_db::repo::preferences::load(conn, &owner)?;
        if !prefs.share_game_totals {
            return Ok(None);
        }
        game_sessions::totals(conn, &owner).map(Some)
    })
    .await;
    let totals = match read {
        Ok(t) => t?,
        Err(e) => {
            tracing::debug!(error = %e, "game totals unread: not shared");
            return None;
        }
    };
    Some(DirectBody::GameTotals(GameTotals {
        totals: totals
            .into_iter()
            .take(MAX_GAME_TOTALS)
            .map(|t| GameTotal {
                game_id: t.game_id,
                total_secs: t.total_secs,
                last_played: t.last_played,
            })
            .collect(),
    }))
}

async fn enqueue(host: &dyn FriendHost, peer: &str, body: &DirectBody) {
    if let Err(e) = dm::handshake::enqueue(host, peer, body).await {
        tracing::warn!(to = %peer, error = %e, "GameTotals not committed to the outbox");
    }
}

/// Send our totals to `peer`, just accepted, when sharing is on.
pub(crate) async fn send(host: &dyn FriendHost, peer: &str) {
    if let Some(body) = ours_if_shared(host).await {
        enqueue(host, peer, &body).await;
    }
}

/// Send our totals to every accepted friend, when sharing is on: a game
/// session ended, or sharing was just turned on. How many it reached.
pub async fn send_to_friends(host: &dyn FriendHost) -> usize {
    let Some(body) = ours_if_shared(host).await else {
        return 0;
    };
    let friends: Vec<String> = state::friends(host)
        .into_iter()
        .filter(|f| f.friendship_state == FriendshipState::Accepted)
        .map(|f| f.public_key)
        .collect();
    for friend in &friends {
        enqueue(host, friend, &body).await;
    }
    friends.len()
}

/// A friend's `GameTotals`: kept only from an accepted friend, replacing
/// what we held from them. Whether it stored.
pub(crate) async fn on_game_totals(host: &dyn FriendHost, sender: &str, body: &GameTotals) -> bool {
    if state::friend(host, sender).map(|f| f.friendship_state) != Some(FriendshipState::Accepted) {
        tracing::debug!(from = %sender, "GameTotals from a non-friend dropped");
        return true;
    }
    let Ok(owner) = state::owner(host) else {
        return false;
    };
    let peer = sender.to_string();
    let totals: Vec<game_sessions::GameTotal> = body
        .totals
        .iter()
        .map(|t| game_sessions::GameTotal {
            game_id: t.game_id,
            total_secs: t.total_secs,
            last_played: t.last_played,
        })
        .collect();
    let stored = state::db_call!(host, "store a friend's game totals", move |conn| {
        rekindle_db::repo::friend_game_totals::replace(conn, &owner, &peer, &totals)
    })
    .await;
    match stored {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(from = %sender, error = %e, "friend game totals not stored");
            false
        }
    }
}
