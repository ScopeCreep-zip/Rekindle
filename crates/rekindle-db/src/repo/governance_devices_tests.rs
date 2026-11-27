use super::*;
use crate::repo::fixture::{self, OWNER};

fn row(member: &str, key: &str, incarnation: u64, state: DeviceState) -> DeviceRow {
    DeviceRow {
        member: member.into(),
        device_key: key.into(),
        incarnation,
        device_record: format!("VLD0:{key}"),
        state,
        fork_evidence: None,
    }
}

#[test]
fn the_set_is_replaced_whole_and_lists_in_cap_order() {
    let conn = fixture::conn();
    fixture::community(&conn, "c1");
    fixture::community(&conn, "c2");
    let readmitted = DeviceRow {
        fork_evidence: Some([5; 32]),
        ..row("p1", "d3", 2, DeviceState::Effective)
    };
    replace(
        &conn,
        OWNER,
        "c1",
        &[
            readmitted.clone(),
            row("p1", "d1", 0, DeviceState::Retired),
            row("p2", "d2", 0, DeviceState::Pending),
        ],
    )
    .unwrap();
    replace(
        &conn,
        OWNER,
        "c2",
        &[row("p1", "e1", 0, DeviceState::Inert)],
    )
    .unwrap();
    assert_eq!(
        list(&conn, OWNER, "c1", Some("p1")).unwrap(),
        vec![row("p1", "d1", 0, DeviceState::Retired), readmitted]
    );
    replace(
        &conn,
        OWNER,
        "c1",
        &[row("p2", "d2", 0, DeviceState::Effective)],
    )
    .unwrap();
    assert_eq!(
        list(&conn, OWNER, "c1", None).unwrap(),
        vec![row("p2", "d2", 0, DeviceState::Effective)]
    );
    assert_eq!(list(&conn, OWNER, "c2", None).unwrap().len(), 1);
}
