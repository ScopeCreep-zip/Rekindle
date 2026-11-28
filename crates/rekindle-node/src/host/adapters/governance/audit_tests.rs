//! The audit log's rows and the notification default (plan E3.1 F4).

use rekindle_governance::fixture::{at, stamp, Stamped};
use rekindle_governance::state::{GovernanceState, NotificationDefaultState};
use rekindle_types::dto::community::CommunityAuditEntry;
use rekindle_types::governance::GovernanceEntry;
use rekindle_types::id::{ChannelId, PseudonymKey, RoleId};

use super::{audit_page, notification_default, AuditNames, MAX_AUDIT_PAGE};

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
                    role_id: RoleId([7; 16]),
                },
            ),
            at(
                5,
                GovernanceEntry::BanEntry {
                    target: key(3),
                    reason: Some("spam".into()),
                    target_seq_frontier: 0,
                },
            ),
        ],
    )]
}

/// `key(2)` is a member whose card names it "Bea"; `key(1)`, the author,
/// has no card; `key(3)`, banned, is no member.
fn with_names(
    page: impl FnOnce(&AuditNames<'_>) -> Vec<CommunityAuditEntry>,
) -> Vec<CommunityAuditEntry> {
    let card_name = |p: &PseudonymKey| (*p == key(2)).then(|| "Bea".to_owned());
    page(&AuditNames {
        members: [key(1), key(2)].iter().map(|k| hex::encode(k.0)).collect(),
        card_name: &card_name,
    })
}

fn page(before: Option<u64>, limit: u32) -> Vec<CommunityAuditEntry> {
    with_names(|names| audit_page(authored(), before, limit, names))
}

/// Each row names its author and member target as the member list does
/// (plan E3.4 S6): an author with no card is its short fingerprint, never
/// empty or a raw key; a member target is its card's name; a person who is
/// no member is its short fingerprint.
#[test]
fn an_author_without_a_card_is_named_by_its_fingerprint() {
    let rows = page(None, 50);
    let fingerprint = rekindle_crypto::identity::short_key_fingerprint(&[1; 32]);
    assert!(rows.iter().all(|r| r.actor_name == fingerprint), "{rows:?}");
    assert_eq!(rows[0].action, "member_banned");
    assert_eq!(rows[1].target_name.as_deref(), Some("Bea"));
}

/// An entry never loses who it was about (plan E3.4 S6): a banned member,
/// no longer counted, is named by its short fingerprint, the name the
/// member list gives a key without a card; a channel is no person, so its
/// row names no target.
#[test]
fn a_banned_target_is_its_fingerprint_and_a_channel_is_unnamed() {
    let rows = page(None, 50);
    assert_eq!(rows[0].action, "member_banned");
    let banned = rekindle_crypto::identity::short_key_fingerprint(&[3; 32]);
    assert_eq!(rows[0].target_name.as_deref(), Some(banned.as_str()));
    let archived = vec![at(
        6,
        GovernanceEntry::ChannelArchived {
            channel_id: ChannelId([9; 16]),
        },
    )];
    let rows = with_names(|names| audit_page(vec![(key(1), archived)], None, 50, names));
    assert_eq!(rows[0].action, "channel_archived");
    assert_eq!(
        rows[0].target.as_deref(),
        Some(hex::encode([9u8; 16]).as_str())
    );
    assert_eq!(rows[0].target_name, None, "a channel is no person");
}

#[test]
fn a_ban_and_a_role_grant_are_rows_newest_first() {
    let rows = page(None, 50);
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
    let older = page(Some(5), 50);
    assert_eq!(older.len(), 1);
    assert_eq!(older[0].action, "role_assigned");
    assert_eq!(page(None, 0).len(), 1, "a page holds a row");
    let many: Vec<Stamped> = (0..u64::from(MAX_AUDIT_PAGE) + 10)
        .map(|lamport| at(lamport, GovernanceEntry::UnbanEntry { target: key(3) }))
        .collect();
    assert_eq!(
        with_names(|names| audit_page(vec![(key(1), many)], None, u32::MAX, names)).len(),
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
