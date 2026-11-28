//! Our shared game on our card (`M`/0), plan E3.4 S6: the field-fate table
//! puts `game_info` on the card, and the card replaced the gossip
//! `PresenceUpdate` that carried it.
//!
//! **What is written.** The game [`OwnPresence`](crate::host::own_presence)
//! shows, passed through the community's sharing consent with
//! [`rekindle_presence::community::apply_sharing_policy`] (architecture
//! §10 / §16.3, default-deny): a game is the session's `activity`, so a
//! community whose policy does not share activity, or a status that hides
//! presence (Invisible, Offline, or `share_online` off), gets no game. The
//! card carries no elapsed time: it changes every second, and a card is
//! re-read by every reader only when its seq advances (plan E3.4 "Card"),
//! so only a start, a stop or a server change rewrites it.
//!
//! **Coalescing.** Every own-presence change wakes the writer
//! (`OwnPresence::card_game`); it waits [`CARD_GAME_SETTLE`] for the burst
//! to settle, then rewrites the card only in the communities whose card
//! cache shows another game. Each card write is a Veilid `set_value`
//! fan-out followed by a forced read on every member whose ring watches
//! the record (a ValueChange carries only its first subkey's value,
//! veilid-core 0.5.8 `veilid_state.rs:334-336`); a game launcher flips
//! through several states at start, and one write covers them.

use std::sync::Weak;
use std::time::Duration;

use rekindle_types::dto::presence::OwnPresenceView;
use rekindle_types::presence::{GameInfo, MemberSession, PresenceSharingPolicy};
use tokio_util::sync::CancellationToken;

use super::super::governance::HostGovernance;
use crate::daemon::dispatch::DaemonContext;

/// How long the writer lets a burst of own-presence changes settle before
/// it rewrites a card.
pub(crate) const CARD_GAME_SETTLE: Duration = Duration::from_secs(5);

/// The game our card says in a community with `policy`, while we show
/// `own`: `None` when we play none, the policy withholds activity, or the
/// status hides presence.
#[must_use]
pub(crate) fn shared_game(
    own: &OwnPresenceView,
    policy: &PresenceSharingPolicy,
) -> Option<GameInfo> {
    let game = own.game.as_ref()?;
    let session = rekindle_presence::community::apply_sharing_policy(
        MemberSession {
            status: own.status,
            location: None,
            activity: Some(game.game_name.clone()),
            last_active: 0,
        },
        policy,
    );
    if rekindle_presence::hides_presence(session.status) {
        return None;
    }
    session.activity.map(|game_name| GameInfo {
        game_name,
        game_id: (game.game_id != 0).then(|| game.game_id.to_string()),
        elapsed_seconds: None,
        server_address: game.server_address.clone(),
    })
}

/// Rewrite our card's game wherever it moved, once per settled burst of
/// own-presence changes, until `stop` (the unlock's scope).
pub(crate) async fn run(ctx: Weak<DaemonContext>, stop: CancellationToken) {
    loop {
        let Some(wake) = ctx.upgrade().map(|c| c.own_presence.card_game.clone()) else {
            return;
        };
        tokio::select! {
            biased;
            () = stop.cancelled() => return,
            () = wake.notified() => {}
        }
        tokio::select! {
            biased;
            () = stop.cancelled() => return,
            () = tokio::time::sleep(CARD_GAME_SETTLE) => {}
        }
        let Some(ctx) = ctx.upgrade() else {
            return;
        };
        HostGovernance::new(ctx).rewrite_moved_card_games().await;
    }
}

#[cfg(test)]
#[path = "card_game_tests.rs"]
mod tests;
