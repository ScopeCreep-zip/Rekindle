use rekindle_governance::op::OpStamp;
use rekindle_governance::state::{GovernanceState, MemberState, RoleState};
use rekindle_types::id::{PseudonymKey, RoleId};

use crate::host::community::state::{display_role_name, CommunityState};

fn member(gov: &mut GovernanceState, b: u8) {
    gov.members.insert(
        PseudonymKey([b; 32]),
        MemberState {
            member_record: format!("VLD0:{b}"),
            member_record_owner: "owner".into(),
            joined: OpStamp::MIN,
        },
    );
}

/// The left, the removed and the banned have departed; a member, a
/// re-joined one and an author never seen have not (register FX1).
#[test]
fn departed_holds_the_left_the_removed_and_the_banned() {
    let mut gov = GovernanceState::default();
    for b in 1..=5 {
        gov.op_log_records
            .insert(PseudonymKey([b; 32]), format!("VLD0:{b}"));
    }
    member(&mut gov, 1);
    gov.removed.insert(PseudonymKey([3; 32]), OpStamp::MIN);
    gov.removed.insert(PseudonymKey([5; 32]), OpStamp::MIN);
    member(&mut gov, 5);
    gov.bans.insert(PseudonymKey([4; 32]));
    member(&mut gov, 4);
    let departed = super::departed(&gov);
    let hex = |b: u8| hex::encode([b; 32]);
    assert!(!departed.contains(&hex(1)), "a member");
    assert!(departed.contains(&hex(2)), "left");
    assert!(departed.contains(&hex(3)), "removed");
    assert!(departed.contains(&hex(4)), "banned");
    assert!(!departed.contains(&hex(5)), "re-joined");
    assert!(!departed.contains(&hex(9)), "never joined");
}

fn role(gov: &mut GovernanceState, id: u8, name: &str, position: u32) {
    gov.roles.insert(
        RoleId([id; 16]),
        RoleState {
            name: name.into(),
            permissions: 0,
            position,
            color: 0,
            hoist: false,
            mentionable: false,
            self_assignable: false,
            exclusion_group: None,
            stamp: OpStamp::MIN,
        },
    );
}

/// The Owner role's position is `u32::MAX` (`origin.rs`): the projection
/// keeps it above every role, so the owner outranks a member holding
/// any other role, or none (plan E7.13, walk 19 c: it was cast to −1).
#[test]
fn the_owner_role_stays_above_every_role() {
    let mut gov = GovernanceState::default();
    role(&mut gov, 0, "@everyone", 0);
    role(&mut gov, 1, "Mods", 5);
    role(&mut gov, 2, "Top", 1 << 31);
    role(&mut gov, 0xFF, "Owner", u32::MAX);
    let mut cs = CommunityState::default();
    super::project_roles_and_categories(&mut cs, &gov);
    let owner = cs.roles.last().expect("roles projected");
    assert_eq!(owner.name, "Owner", "ascending order ends with the Owner");
    for other in cs.roles.iter().filter(|r| r.name != "Owner") {
        assert!(
            other.position < owner.position,
            "{} ranks below",
            other.name
        );
    }
    let ids: Vec<RoleId> = cs.roles.iter().map(|r| r.id).collect();
    assert_eq!(display_role_name(&ids, &cs.roles), "Owner");
}
