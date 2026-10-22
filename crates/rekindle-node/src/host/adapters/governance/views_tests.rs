//! A community's window view (plan E3.1 D1b).

use super::{community_view, role_view};
use crate::host::community::{CommunityState, RoleDefinition};

fn owner_role() -> RoleDefinition {
    RoleDefinition {
        id: 0,
        name: "Owner".into(),
        color: 0x00ff_aa00,
        permissions: u64::MAX >> 1,
        position: 100,
        hoist: true,
        mentionable: false,
        self_assignable: false,
        exclusion_group: None,
    }
}

#[test]
fn role_permissions_ride_as_decimal_text() {
    let view = role_view(&owner_role());
    assert_eq!(view.permissions, (u64::MAX >> 1).to_string());
    let json = serde_json::to_value(&view).unwrap();
    assert!(json["permissions"].is_string(), "{json}");
    assert!(json.get("selfAssignable").is_some(), "camelCase: {json}");
}

#[test]
fn the_view_names_our_role_and_carries_our_standing() {
    let mut state = CommunityState::new("gk");
    state.name = "Raiders".into();
    state.roles = vec![owner_role()];
    state.my_role_ids = vec![0];
    state.my_pseudonym_key = Some("ab".repeat(32));
    state.mek_generation = 3;
    let view = community_view(&state);
    assert_eq!(view.name, "Raiders");
    assert_eq!(view.my_role.as_deref(), Some("Owner"));
    assert_eq!(view.my_role_ids, vec![0]);
    assert_eq!(view.mek_generation, 3);
    assert_eq!(view.roles.len(), 1);
    let json = serde_json::to_value(&view).unwrap();
    assert!(json.get("myPseudonymKey").is_some(), "{json}");
    assert!(
        json.get("myBio").is_none(),
        "unset profile fields are omitted"
    );
}
