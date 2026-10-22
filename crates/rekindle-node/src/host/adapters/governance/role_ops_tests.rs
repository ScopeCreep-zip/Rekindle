//! The self-assign rules and the exclusion-group slug (plan E3.1 F1).

use rekindle_governance::state::{GovernanceState, RoleState};
use rekindle_governance_runtime::roles::ExclusionGroupEdit;
use rekindle_types::id::RoleId;

use super::{normalize_edit, normalize_exclusion_group, self_assignable, RoleRefusal};

fn governance_with(role_id: u32, self_assignable: bool) -> GovernanceState {
    let mut governance = GovernanceState::default();
    governance.roles.insert(
        RoleId::from_legacy_u32(role_id),
        RoleState {
            name: "gamers".into(),
            permissions: 0,
            position: 1,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable,
            exclusion_group: None,
            lamport: 1,
        },
    );
    governance
}

#[test]
fn a_self_assignable_role_may_be_taken() {
    assert!(self_assignable(&governance_with(7, true), 7).is_ok());
}

#[test]
fn a_role_not_marked_self_assignable_is_refused() {
    let refused = self_assignable(&governance_with(7, false), 7).unwrap_err();
    assert!(matches!(refused, RoleRefusal::NotSelfAssignable(7)));
    assert_eq!(refused.code(), 403);
}

#[test]
fn an_unknown_role_is_refused() {
    let refused = self_assignable(&governance_with(7, true), 8).unwrap_err();
    assert!(matches!(refused, RoleRefusal::UnknownRole(8)));
}

#[test]
fn the_slug_is_lowercased_and_a_blank_one_is_none() {
    assert_eq!(
        normalize_exclusion_group(Some(" Colour-1 ")).unwrap(),
        Some("colour-1".into())
    );
    assert_eq!(normalize_exclusion_group(Some("  ")).unwrap(), None);
    assert!(matches!(
        normalize_edit(ExclusionGroupEdit::Set(" ".into())).unwrap(),
        ExclusionGroupEdit::Clear
    ));
}

#[test]
fn a_malformed_slug_is_refused() {
    assert!(normalize_exclusion_group(Some("a b")).is_err());
    assert!(normalize_exclusion_group(Some(&"x".repeat(33))).is_err());
}
