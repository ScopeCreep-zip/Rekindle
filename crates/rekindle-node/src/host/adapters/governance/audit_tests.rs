//! The audit log's rows and the notification default (plan E3.1 F4).

use rekindle_governance::fixture::{at, stamp, Stamped};
use rekindle_governance::state::{GovernanceState, NotificationDefaultState};
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{PseudonymKey, RoleId};

use super::{audit_page, notification_default, MAX_AUDIT_PAGE};

fn key(byte: u8) -> PseudonymKey {
    PseudonymKey([byte; 32])
}

/// The creator `key(1)` grants a role to `key(2)` at Lamport 4 and bans
/// `key(3)` at 5, exactly the entries the role and ban writes make.
fn authored() -> Vec<(PseudonymKey, Vec<Stamped>)> {
    vec![(
        key(1),
        vec![
            at(
                4,
                GovernanceEntry::RoleAssignment {
                    target: key(2),
                    role_id: RoleId::from_legacy_u32(7),
                },
            ),
            at(
                5,
                GovernanceEntry::BanEntry {
                    target: key(3),
                    reason: Some("spam".into()),
                },
            ),
        ],
    )]
}

#[test]
fn a_ban_and_a_role_grant_are_rows_newest_first() {
    let rows = audit_page(authored(), None, 50);
    let actions: Vec<&str> = rows.iter().map(|r| r.action.as_str()).collect();
    assert_eq!(actions, ["member_banned", "role_assigned"]);
    let ban = &rows[0];
    assert_eq!(ban.actor_pseudonym, hex::encode([1u8; 32]));
    assert_eq!(ban.target.as_deref(), Some(hex::encode([3u8; 32]).as_str()));
    assert_eq!(ban.details.as_deref(), Some("spam"));
    assert_eq!(ban.timestamp, 5);
    assert_eq!(
        rows[1].target.as_deref(),
        Some(hex::encode([2u8; 32]).as_str())
    );
}

#[test]
fn a_page_is_older_than_its_cursor_and_bounded() {
    let older = audit_page(authored(), Some(5), 50);
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].action, "role_assigned");
    assert_eq!(
        audit_page(authored(), None, 0).len(),
        1,
        "a page holds a row"
    );
    let many: Vec<Stamped> = (0..u64::from(MAX_AUDIT_PAGE) + 10)
        .map(|lamport| at(lamport, GovernanceEntry::UnbanEntry { target: key(3) }))
        .collect();
    assert_eq!(
        audit_page(vec![(key(1), many)], None, u32::MAX).len(),
        usize::try_from(MAX_AUDIT_PAGE).unwrap()
    );
}

#[test]
fn the_notification_default_is_the_merged_level() {
    let mut state = GovernanceState::default();
    assert_eq!(notification_default(&state), None);
    state.notification_default = Some(NotificationDefaultState {
        level: "mentions".into(),
        stamp: stamp(3),
    });
    assert_eq!(notification_default(&state).as_deref(), Some("mentions"));
    state.notification_default = Some(NotificationDefaultState {
        level: "loud".into(),
        stamp: stamp(4),
    });
    assert_eq!(
        notification_default(&state),
        None,
        "readers drop other levels"
    );
}
