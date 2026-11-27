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

#[test]
fn community_voice_events_use_the_member_list_name() {
    use rekindle_types::subscription_events::VoiceParticipant;

    let communities = CommunityStore::new(Arc::default());
    let scope = VoiceScope::Community {
        community: "c".into(),
        channel: "voice".into(),
    };
    let named = PseudonymKey([0x23; 32]);
    assert!(communities.note_member_card(
        "c",
        &named,
        rekindle_governance_runtime::CardText {
            display_name: Some("River".into()),
            custom_status: None,
            game: None,
        },
    ));
    let named_key = "23".repeat(32);
    let raw_key = "ab".repeat(32);
    let join = SubscriptionEvent::Voice(VoiceEvent::Joined {
        scope: scope.clone(),
        pseudonym: named_key.clone(),
        display_name: Some(named_key.clone()),
        route_blob: None,
    });
    assert!(matches!(
        with_voice_names(&communities, join),
        SubscriptionEvent::Voice(VoiceEvent::Joined { display_name: Some(name), .. })
            if name == "River"
    ));
    let roster = SubscriptionEvent::Voice(VoiceEvent::RosterUpdated {
        scope,
        participants: vec![
            VoiceParticipant {
                pseudonym_key: named_key,
                display_name: Some("Wire nickname".into()),
            },
            VoiceParticipant {
                pseudonym_key: raw_key.clone(),
                display_name: Some(raw_key),
            },
        ],
    });
    let SubscriptionEvent::Voice(VoiceEvent::RosterUpdated { participants, .. }) =
        with_voice_names(&communities, roster)
    else {
        panic!("a voice roster");
    };
    assert_eq!(participants[0].display_name.as_deref(), Some("River"));
    assert_eq!(participants[1].display_name.as_deref(), Some("abab abab"));
}
