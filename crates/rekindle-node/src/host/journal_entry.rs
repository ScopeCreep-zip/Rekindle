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
use rekindle_types::id::PseudonymKey;
use rekindle_types::key_format::pseudonym_hex;
use rekindle_types::subscription_events::{
    Cursor, GameActivity, SubscriptionEvent, VoiceEvent, VoiceScope,
};

use super::adapters::governance::community_profile::resolved_member_name;
use super::community::CommunityStore;

/// Journal `event` for every subscriber, stated first.
pub(crate) fn publish(journal: &SessionJournal, event: SubscriptionEvent) -> Cursor {
    journal.publish(stated(event))
}

/// State the member-list name on the voice roster and its join events.
pub(crate) fn with_voice_names(
    communities: &CommunityStore,
    mut event: SubscriptionEvent,
) -> SubscriptionEvent {
    let SubscriptionEvent::Voice(voice) = &mut event else {
        return event;
    };
    let name = |scope: &VoiceScope, key: &str| {
        let community = scope.community()?;
        let pseudonym = PseudonymKey(pseudonym_hex(key).ok()?.to_bytes());
        Some(resolved_member_name(communities, community, &pseudonym))
    };
    match voice {
        VoiceEvent::Joined {
            scope: scope @ VoiceScope::Community { .. },
            pseudonym,
            display_name,
            ..
        } => {
            *display_name = name(scope, pseudonym);
        }
        VoiceEvent::RosterUpdated {
            scope: scope @ VoiceScope::Community { .. },
            participants,
        } => {
            for participant in participants {
                participant.display_name = name(scope, &participant.pseudonym_key);
            }
        }
        _ => {}
    }
    event
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
#[path = "journal_entry_tests.rs"]
mod tests;
