//! The Host's one way into its event journal for live events (plan D3.2;
//! E7.15 IV3, decision 51). Every emitter (the friend presence adapter,
//! our own presence, the transport's forwarded gossip, the GUI's
//! emitters through `Host::publish`) journals here, so what only the Host
//! knows about an event is stated once, at this point, and never in each
//! builder: today that is whether a game in a presence observation can be
//! joined, from the Host's own launch check (`launcher::not_joinable`),
//! the same check the friend list's `GameInfoState` carries.

use rekindle_events::SessionJournal;
use rekindle_types::friend::NotJoinableReason;
use rekindle_types::subscription_events::{Cursor, GameActivity, SubscriptionEvent};

/// Journal `event` for every subscriber, stated first.
pub(crate) fn publish(journal: &SessionJournal, event: SubscriptionEvent) -> Cursor {
    journal.publish(stated(event))
}

/// Why `game_id` on `server` cannot be joined here, or `None` when "Join
/// Game" can start it. An unknown game has nothing to launch.
pub(crate) fn not_joinable(
    game_id: Option<u32>,
    server: Option<&str>,
) -> Option<NotJoinableReason> {
    let db = rekindle_game_detect::GameDatabase::shared();
    match game_id {
        Some(id) => rekindle_game_detect::launcher::not_joinable(db, id, server),
        None if server.is_none() => Some(NotJoinableReason::NoServer),
        None => Some(NotJoinableReason::CannotLaunch),
    }
}

/// `event` with the Host's `joinable` stated on any game it observes.
fn stated(mut event: SubscriptionEvent) -> SubscriptionEvent {
    let SubscriptionEvent::Presence(presence) = &mut event else {
        return event;
    };
    if let Some(GameActivity::Playing {
        game_id,
        server_address,
        joinable,
        joinable_reason,
        ..
    }) = presence.snapshot_mut().and_then(|s| s.game.as_mut())
    {
        *joinable_reason = not_joinable(*game_id, server_address.as_deref());
        *joinable = joinable_reason.is_none();
    }
    event
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rekindle_types::subscription_events::{PresenceEvent, PresenceSnapshot, StreamFrame};

    use super::*;

    /// A friend's live game change, as the presence adapter builds it:
    /// `joinable` left unstated.
    fn friend_playing(game_id: u32, game_name: &str, server: &str) -> SubscriptionEvent {
        SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key: "friend".into(),
            snapshot: PresenceSnapshot::game(GameActivity::from_parts(
                Some(game_name.into()),
                Some(game_id),
                Some(1),
                Some(server.into()),
                None,
            )),
        })
    }

    /// The game of the event journaled right after `from`.
    fn journaled_after(journal: &SessionJournal, from: Cursor) -> GameActivity {
        let frames = journal.replay(from);
        let Some(StreamFrame::Event(sequenced)) = frames.first() else {
            panic!("the journal kept the event: {frames:?}");
        };
        let SubscriptionEvent::Presence(event) = &sequenced.event else {
            panic!("a presence event was journaled");
        };
        event
            .snapshot()
            .and_then(|s| s.game.clone())
            .expect("a game")
    }

    /// TF2 (4100) on a server joins over Steam; the friend then moves to
    /// Minecraft (4300), which has no way to start on a server: the
    /// journaled event flips `joinable` with the game.
    #[test]
    fn a_live_game_change_flips_joinable_on_the_journaled_event() {
        let journal = SessionJournal::new(16, 16, Arc::new(rekindle_utils::random::OsRandom));
        let start = journal.publish(SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key: "friend".into(),
            snapshot: PresenceSnapshot::status("online"),
        }));
        let launchable = publish(
            &journal,
            friend_playing(4100, "Team Fortress 2", "1.2.3.4:27015"),
        );
        publish(&journal, friend_playing(4300, "Minecraft", "1.2.3.4:25565"));

        assert!(
            matches!(
                journaled_after(&journal, start),
                GameActivity::Playing {
                    joinable: true,
                    joinable_reason: None,
                    ..
                }
            ),
            "TF2 on a server is joinable"
        );
        assert!(
            matches!(
                journaled_after(&journal, launchable),
                GameActivity::Playing {
                    joinable: false,
                    joinable_reason: Some(NotJoinableReason::CannotLaunch),
                    ..
                }
            ),
            "Minecraft cannot be started on a server, and the live event says so"
        );
    }

    /// A community member's game goes through the same entry: Minecraft
    /// on a server is journaled as `cannotLaunch` for the member profile's
    /// Join Game.
    #[test]
    fn a_member_in_minecraft_on_a_server_is_cannot_launch() {
        let journal = SessionJournal::new(16, 16, Arc::new(rekindle_utils::random::OsRandom));
        let start = journal.publish(SubscriptionEvent::Presence(PresenceEvent::FriendChanged {
            peer_key: "friend".into(),
            snapshot: PresenceSnapshot::status("online"),
        }));
        publish(
            &journal,
            SubscriptionEvent::Presence(PresenceEvent::CommunityMemberChanged {
                community: "community".into(),
                pseudonym: "member".into(),
                snapshot: PresenceSnapshot::game(GameActivity::from_parts(
                    Some("Minecraft".into()),
                    Some(4300),
                    Some(1),
                    Some("1.2.3.4:25565".into()),
                    None,
                )),
            }),
        );
        assert!(
            matches!(
                journaled_after(&journal, start),
                GameActivity::Playing {
                    joinable: false,
                    joinable_reason: Some(NotJoinableReason::CannotLaunch),
                    ..
                }
            ),
            "the member's live game states why Join Game cannot start it"
        );
    }
}
