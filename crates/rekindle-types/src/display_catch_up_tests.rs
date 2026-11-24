use super::*;
use crate::subscription_events::GovernanceEvent;

fn view() -> CatchUpView {
    CatchUpView {
        catching_up: true,
        pending: vec![PendingGovOp {
            op_id: "h1".into(),
            kind: PendingGovKind::Kick,
            target: PendingGovTarget {
                member: Some("ab".into()),
                ..PendingGovTarget::default()
            },
            label: "Kick Bea".into(),
            state: PendingGovState::CatchingUp,
        }],
    }
}

/// The frontend contract (plan E3.2 S7d-keys.8, the shell's DTO): camelCase
/// keys and values, the full list.
#[test]
fn the_catch_up_event_is_camel_case_on_the_wire() {
    let view = view();
    let event = GovernanceEvent::CatchUp {
        community: "c1".into(),
        catching_up: view.catching_up,
        pending: view.pending,
    };
    let golden = serde_json::json!({"catchUp": {
        "community": "c1",
        "catchingUp": true,
        "pending": [{
            "opId": "h1",
            "kind": "kick",
            "target": {"member": "ab", "channel": null, "role": null},
            "label": "Kick Bea",
            "state": "catchingUp",
        }],
    }});
    assert_eq!(serde_json::to_value(&event).unwrap(), golden);
    let clear = GovernanceEvent::CatchUp {
        community: "c1".into(),
        catching_up: false,
        pending: vec![],
    };
    assert_eq!(
        serde_json::to_value(&clear).unwrap(),
        serde_json::json!({"catchUp": {"community": "c1", "catchingUp": false, "pending": []}})
    );
}

/// The same view on re-read rides `CommunityDetail.catch_up`, and both
/// shapes round-trip through postcard (the daemon IPC).
#[test]
fn the_catch_up_view_round_trips() {
    let view = view();
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["pending"][0]["state"], "catchingUp");
    assert_eq!(json["catchingUp"], true);
    let bytes = postcard::to_allocvec(&view).unwrap();
    assert_eq!(postcard::from_bytes::<CatchUpView>(&bytes).unwrap(), view);
    assert_eq!(
        serde_json::to_value(PendingGovState::Signing).unwrap(),
        "signing"
    );
    assert_eq!(
        serde_json::to_value(PendingGovKind::RolesChanged).unwrap(),
        "rolesChanged"
    );
}

#[test]
fn a_held_kick_names_its_member_in_its_label() {
    let target = crate::id::PseudonymKey([0xcd; 32]);
    let entry = GovernanceEntry::UnbanEntry {
        target: target.clone(),
    };
    let op = PendingGovOp::held("held-0".into(), &entry);
    assert_eq!(op.kind, PendingGovKind::Unban);
    assert_eq!(op.target.member, Some(target.to_hex()));
    assert_eq!(op.label, format!("Unban {}", &target.to_hex()[..8]));
    assert_eq!(op.state, PendingGovState::CatchingUp);
}
