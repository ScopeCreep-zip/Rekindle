//! The moderation refusals (plan E3.1 F2): what readers would drop is
//! refused before it is written.

use rekindle_governance::state::GovernanceState;
use rekindle_types::id::PseudonymKey;

use super::{permitted, Action, ModerationRefusal};

fn key(byte: u8) -> PseudonymKey {
    PseudonymKey([byte; 32])
}

/// A community whose creator is `key(1)`; `key(2)` and `key(3)` are
/// members holding no role.
fn community() -> GovernanceState {
    GovernanceState {
        creator: Some(key(1)),
        ..GovernanceState::default()
    }
}

#[test]
fn a_member_without_the_permission_is_refused() {
    let state = community();
    for action in [
        Action::Ban,
        Action::Unban,
        Action::Timeout(60),
        Action::RemoveTimeout,
        Action::Remove,
    ] {
        let refused = permitted(&state, &key(2), &key(3), action, 1_000).unwrap_err();
        assert!(
            matches!(refused, ModerationRefusal::NotPermitted(_)),
            "{action:?}: {refused}"
        );
        assert_eq!(refused.code(), 403);
    }
}

#[test]
fn the_creator_may_moderate_a_member() {
    let state = community();
    for action in [
        Action::Ban,
        Action::Unban,
        Action::Timeout(60),
        Action::RemoveTimeout,
        Action::Remove,
    ] {
        assert!(
            permitted(&state, &key(1), &key(2), action, 1_000).is_ok(),
            "{action:?}"
        );
    }
}

#[test]
fn nobody_bans_times_out_or_removes_the_creator() {
    let state = community();
    for action in [Action::Ban, Action::Timeout(60), Action::Remove] {
        assert!(
            permitted(&state, &key(2), &key(1), action, 1_000).is_err(),
            "{action:?}"
        );
    }
}
