//! A policy set is the policy read back, through the governance merge.

use rekindle_governance::fixture::{at, merge_entries, Stamped};
use rekindle_types::id::PseudonymKey;

use super::*;

fn carded(game: Option<&str>) -> CardText {
    CardText {
        game: game.map(|name| GameInfo {
            game_name: name.into(),
            game_id: Some("4100".into()),
            elapsed_seconds: None,
            server_address: None,
        }),
        ..CardText::default()
    }
}

/// A card whose game started reaches the member list as that game, a
/// stopped one as `Idle`, and a re-read of the same game as nothing
/// (plan E3.4 S6: the card replaced the gossip `PresenceUpdate`).
#[test]
fn a_card_game_move_is_one_member_observation() {
    let playing = game_moved(Some(&carded(None)), &carded(Some("Quake")))
        .expect("a started game is observed");
    assert_eq!(playing.status, None, "the beat owns the status");
    assert!(matches!(
        playing.game,
        Some(GameActivity::Playing { ref game_name, game_id: Some(4100), .. })
            if game_name == "Quake"
    ));
    let stopped = game_moved(Some(&carded(Some("Quake"))), &carded(None))
        .expect("a stopped game is observed");
    assert_eq!(stopped.game, Some(GameActivity::Idle));
    assert_eq!(
        game_moved(Some(&carded(Some("Quake"))), &carded(Some("Quake"))),
        None
    );
    assert_eq!(game_moved(None, &carded(None)), None);
}

/// A member whose card names nobody renders as its short pseudonym
/// fingerprint, stated by the Host, never empty and never another name
/// (plan E3.4 S6); a card name renders as itself.
#[test]
fn an_unnamed_member_renders_as_its_short_fingerprint() {
    let me = PseudonymKey([0xab; 32]);
    assert_eq!(shown_name(None, &me), "abab abab");
    assert_eq!(shown_name(Some(String::new()), &me), "abab abab");
    assert_eq!(shown_name(Some("Kit".into()), &me), "Kit");
}

fn edit() -> PolicyEdit {
    PolicyEdit {
        policy_text: Some("Be kind".into()),
        max_joins_per_interval: 5,
        join_interval_seconds: 120,
    }
}

fn meta(lamport: u64) -> Stamped {
    at(
        lamport,
        GovernanceEntry::CommunityMeta {
            name: Some("C".into()),
            description: None,
            icon_hash: None,
            banner_hash: None,
        },
    )
}

#[test]
fn the_defaults_are_read_before_any_policy() {
    let view = policy_view(None);
    assert_eq!(view.policy_text, None);
    assert_eq!(
        view.max_joins_per_interval,
        CommunityPolicyState::DEFAULT_MAX_JOINS_PER_INTERVAL
    );
    assert_eq!(
        view.join_interval_seconds,
        CommunityPolicyState::DEFAULT_JOIN_INTERVAL_SECONDS
    );
}

#[test]
fn a_policy_set_is_read_back() {
    let creator = PseudonymKey([1; 32]);
    let state = merge_entries(
        &creator,
        &[(creator.clone(), vec![meta(1), at(2, policy_entry(edit()))])],
    );
    let view = policy_view(state.community_policy);
    assert_eq!(view.policy_text.as_deref(), Some("Be kind"));
    assert_eq!(view.max_joins_per_interval, 5);
    assert_eq!(view.join_interval_seconds, 120);
}

#[test]
fn a_policy_without_manage_community_is_not_read() {
    let creator = PseudonymKey([1; 32]);
    let member = PseudonymKey([2; 32]);
    let state = merge_entries(
        &creator,
        &[
            (creator.clone(), vec![meta(1)]),
            (member, vec![at(2, policy_entry(edit()))]),
        ],
    );
    assert_eq!(policy_view(state.community_policy).policy_text, None);
}
