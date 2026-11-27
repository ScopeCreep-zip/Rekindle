use super::{group_call_refused, GroupCallRoster, GroupDmRosterMember};

fn held(keys: &[&str]) -> GroupCallRoster {
    GroupCallRoster::Held(
        keys.iter()
            .map(|&key| GroupDmRosterMember {
                identity: key.into(),
                profile_record_key: String::new(),
                display_name: key.into(),
            })
            .collect(),
    )
}

#[test]
fn each_cause_names_itself_and_a_member_with_company_may_call() {
    let refused = |r: &GroupCallRoster| group_call_refused(r, "me");
    assert_eq!(
        refused(&GroupCallRoster::Invited).as_deref(),
        Some("accept the invite first")
    );
    assert_eq!(
        refused(&GroupCallRoster::Unknown).as_deref(),
        Some("the group is not in your list")
    );
    assert_eq!(
        refused(&held(&["a", "b"])).as_deref(),
        Some("you are not in the group")
    );
    assert_eq!(
        refused(&held(&["me"])).as_deref(),
        Some("no one else is in the group to call")
    );
    assert_eq!(refused(&held(&["me", "a"])), None);
}
