use super::*;

const FEED_AT: i64 = 1_700_000_000_000;

fn snap() -> FriendActivitySnapshot {
    FriendActivitySnapshot {
        public_key: "bob".into(),
        accepted: true,
        display_name: "Bob".into(),
        status_message: None,
        bio: None,
        game: None,
        played_games: Some(BTreeSet::from([4181])),
    }
}

fn game(game_id: u32, elapsed_seconds: u32, server: Option<&str>) -> GameInfoState {
    GameInfoState {
        game_id,
        game_name: format!("game {game_id}"),
        server_info: None,
        elapsed_seconds,
        server_address: server.map(str::to_string),
        icon_key: None,
        joinable: server.is_some(),
        joinable_reason: server
            .is_none()
            .then_some(rekindle_types::friend::NotJoinableReason::NoServer),
    }
}

fn kinds(
    prev: Option<&FriendActivitySnapshot>,
    next: &FriendActivitySnapshot,
) -> Vec<ActivityKind> {
    let entries = friend_entries(prev, next, FEED_AT);
    assert!(entries.iter().all(|e| e.at == FEED_AT));
    entries.into_iter().map(|e| e.kind).collect()
}

fn friend() -> String {
    "bob".into()
}

/// A new friendship (no snapshot before, or a pending one) is one
/// `NowFriends`, whatever they were already doing.
#[test]
fn becoming_friends_is_now_friends_only() {
    let next = FriendActivitySnapshot {
        game: Some(game(4181, 5, None)),
        status_message: Some("hi".into()),
        ..snap()
    };
    let now = vec![ActivityKind::NowFriends { friend: friend() }];
    assert_eq!(kinds(None, &next), now);
    let pending = FriendActivitySnapshot {
        accepted: false,
        ..snap()
    };
    assert_eq!(kinds(Some(&pending), &next), now);
}

/// A pending friend produces nothing; an unchanged one produces nothing.
#[test]
fn a_pending_or_unchanged_friend_is_no_news() {
    let pending = FriendActivitySnapshot {
        accepted: false,
        game: Some(game(4181, 5, None)),
        ..snap()
    };
    assert!(kinds(None, &pending).is_empty());
    assert!(kinds(Some(&snap()), &snap()).is_empty());
}

/// Starting a game they have in their shared totals: `StartedGame` with
/// the server they shared, and no first time.
#[test]
fn starting_a_known_game_is_started_game_with_its_server() {
    let next = FriendActivitySnapshot {
        game: Some(game(4181, 0, Some("203.0.113.7:27015"))),
        ..snap()
    };
    assert_eq!(
        kinds(Some(&snap()), &next),
        [ActivityKind::StartedGame {
            friend: friend(),
            game_id: 4181,
            server: Some("203.0.113.7:27015".into()),
        }]
    );
}

/// Stopping is `StoppedGame` with the time the last snapshot showed;
/// switching games is a stop and a start.
#[test]
fn stopping_and_switching_games() {
    let playing = FriendActivitySnapshot {
        game: Some(game(4181, 90, None)),
        ..snap()
    };
    let stopped = ActivityKind::StoppedGame {
        friend: friend(),
        game_id: 4181,
        seconds: 90,
    };
    assert_eq!(
        kinds(Some(&playing), &snap()),
        std::slice::from_ref(&stopped)
    );
    let other = FriendActivitySnapshot {
        game: Some(game(4181, 120, None)),
        ..snap()
    };
    assert!(
        kinds(Some(&playing), &other).is_empty(),
        "still the same game"
    );
    let switched = FriendActivitySnapshot {
        played_games: Some(BTreeSet::from([4181, 12])),
        game: Some(game(12, 0, None)),
        ..snap()
    };
    let switched_prev = FriendActivitySnapshot {
        played_games: Some(BTreeSet::from([4181, 12])),
        ..playing
    };
    assert_eq!(
        kinds(Some(&switched_prev), &switched),
        [
            stopped,
            ActivityKind::StartedGame {
                friend: friend(),
                game_id: 12,
                server: None,
            },
        ]
    );
}

/// A game absent from the totals they shared before is their first time;
/// with no shared totals there is nothing to compare, so no first time.
#[test]
fn first_played_is_against_the_shared_totals() {
    let next = FriendActivitySnapshot {
        game: Some(game(12, 0, None)),
        played_games: Some(BTreeSet::from([4181, 12])),
        ..snap()
    };
    let started = ActivityKind::StartedGame {
        friend: friend(),
        game_id: 12,
        server: None,
    };
    assert_eq!(
        kinds(Some(&snap()), &next),
        [
            started.clone(),
            ActivityKind::FirstPlayed {
                friend: friend(),
                game_id: 12,
            },
        ]
    );
    let no_totals = FriendActivitySnapshot {
        played_games: None,
        ..snap()
    };
    let next_no_totals = FriendActivitySnapshot {
        played_games: None,
        ..next
    };
    assert_eq!(kinds(Some(&no_totals), &next_no_totals), [started]);
}

/// A friend with game sharing off sends no current game, so no game entry
/// of any kind, even with totals shared and a new game in them.
#[test]
fn no_game_entry_when_game_sharing_is_off() {
    let next = FriendActivitySnapshot {
        played_games: Some(BTreeSet::from([4181, 12])),
        ..snap()
    };
    assert!(kinds(Some(&snap()), &next).is_empty());
}

/// A changed published name is `RenamedSelf` from → to; an empty one is
/// not news.
#[test]
fn a_new_display_name_is_renamed_self() {
    let next = FriendActivitySnapshot {
        display_name: "Robert".into(),
        ..snap()
    };
    assert_eq!(
        kinds(Some(&snap()), &next),
        [ActivityKind::RenamedSelf {
            friend: friend(),
            from: "Bob".into(),
            to: "Robert".into(),
        }]
    );
    let empty = FriendActivitySnapshot {
        display_name: String::new(),
        ..snap()
    };
    assert!(kinds(Some(&snap()), &empty).is_empty());
}

/// A new status message is `StatusMessage` with its text; clearing it is
/// not news.
#[test]
fn a_new_status_message_is_news_and_clearing_is_not() {
    let next = FriendActivitySnapshot {
        status_message: Some("raiding".into()),
        ..snap()
    };
    assert_eq!(
        kinds(Some(&snap()), &next),
        [ActivityKind::StatusMessage {
            friend: friend(),
            text: "raiding".into(),
        }]
    );
    assert!(kinds(Some(&next), &snap()).is_empty());
    let blank = FriendActivitySnapshot {
        status_message: Some(String::new()),
        ..snap()
    };
    assert!(kinds(Some(&next), &blank).is_empty());
}

/// A changed bio is `BioChanged`; clearing it is not news.
#[test]
fn a_changed_bio_is_bio_changed() {
    let next = FriendActivitySnapshot {
        bio: Some("I play".into()),
        ..snap()
    };
    assert_eq!(
        kinds(Some(&snap()), &next),
        [ActivityKind::BioChanged { friend: friend() }]
    );
    assert!(kinds(Some(&next), &snap()).is_empty());
}

/// Our own events: a favourite added and a community joined, about no
/// friend.
#[test]
fn own_events_name_no_friend() {
    let fav = favourite_added(4181, "203.0.113.7:27015", "LAN", FEED_AT);
    assert_eq!(
        fav,
        ActivityEntry {
            at: FEED_AT,
            kind: ActivityKind::FavouriteAdded {
                game_id: 4181,
                address: "203.0.113.7:27015".into(),
                label: "LAN".into(),
            },
        }
    );
    let joined = joined_community("c1", FEED_AT);
    assert_eq!(
        joined.kind,
        ActivityKind::JoinedCommunity {
            community: "c1".into()
        }
    );
    assert_eq!(fav.kind.friend(), None);
    assert_eq!(joined.kind.friend(), None);
}
