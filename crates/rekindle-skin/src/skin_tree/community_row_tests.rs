//! Plan E7.7: every built-in serves the Communities view's own button row,
//! cloned from the pack's row, with Open, Leave and New as Host actions;
//! Open and Leave follow the Host's `can`/`cannot` (disabled, never
//! hidden, while no community is selected).

use rekindle_types::dto::skin_action::{
    AppSignal, Capability, NodeState, NodeTrigger, SkinAction, SkinEvent,
};
use rekindle_types::dto::skin_tree::{NodeRole, SkinNode};
use rekindle_types::dto::skin_tree_role::ViewKey;

use super::community_row::find;
use super::golden_tests::main_layout;

const BUILTINS: [&str; 11] = [
    "symbiosis",
    "xfire-classic",
    "combat",
    "alloy",
    "onyx",
    "simple-x",
    "spacecowboy",
    "stealthx",
    "total-chaos",
    "xglow",
    "xslimfire",
];

fn buttons<'a>(nodes: &'a [SkinNode], out: &mut Vec<&'a SkinNode>) {
    for n in nodes {
        if n.role == NodeRole::Button {
            out.push(n);
        }
        buttons(&n.children, out);
    }
}

fn labels<'a>(n: &'a SkinNode, out: &mut Vec<&'a str>) {
    if let Some(l) = &n.label {
        out.push(&l.text);
    }
    for c in &n.children {
        labels(c, out);
    }
}

fn disables(b: &SkinNode, cap: Capability) -> bool {
    b.triggers.contains(&NodeTrigger {
        on: SkinEvent::App(AppSignal::Cannot(cap)),
        run: SkinAction::SetState(NodeState::Disabled),
    }) && b.triggers.contains(&NodeTrigger {
        on: SkinEvent::App(AppSignal::Can(cap)),
        run: SkinAction::SetState(NodeState::Normal),
    })
}

#[test]
fn every_builtin_serves_a_communities_row_with_open_leave_new() {
    let row_role = NodeRole::ButtonRow(ViewKey::Communities);
    for id in BUILTINS {
        let main = main_layout(id);
        let roots = std::iter::once(main.tree.as_ref().unwrap().root.as_slice())
            .chain(main.popups.values().map(|p| p.tree.root.as_slice()));
        let rows: Vec<&SkinNode> = roots
            .filter_map(|r| find(r, &|n| n.role == row_role))
            .collect();
        assert_eq!(rows.len(), 1, "{id}: one ButtonRow(Communities)");
        let mut found = Vec::new();
        buttons(&rows[0].children, &mut found);
        let actions: Vec<Vec<&SkinAction>> = found
            .iter()
            .map(|b| {
                b.triggers
                    .iter()
                    .filter(|t| t.presses())
                    .map(|t| &t.run)
                    .filter(|r| !matches!(r, SkinAction::SetState(_) | SkinAction::PlaySound(_)))
                    .collect()
            })
            .collect();
        assert_eq!(
            actions,
            [
                vec![&SkinAction::OpenCommunity],
                vec![&SkinAction::LeaveCommunity],
                vec![&SkinAction::StartCommunity],
            ],
            "{id}: Open, Leave, New"
        );
        assert!(disables(found[0], Capability::OpenCommunity), "{id}: Open");
        assert!(
            disables(found[1], Capability::LeaveCommunity),
            "{id}: Leave"
        );
        assert!(
            found[2]
                .triggers
                .iter()
                .all(|t| !matches!(t.on, SkinEvent::App(_))),
            "{id}: New is always enabled"
        );
        for (b, words) in found.iter().zip(["Open", "Leave", "New"]) {
            let mut texts = Vec::new();
            labels(b, &mut texts);
            assert!(texts.iter().all(|t| *t == words), "{id}: {texts:?}");
        }
        assert!(
            find(&rows[0].children, &|n| n.role == NodeRole::RoomName).is_none(),
            "{id}: no room-name field"
        );
    }
}

/// The row's slot `words` in built-in `id`, and the button `donor` there
/// (`None` for a plain slot).
fn slot_and_donor(id: &str, words: &str, donor: Option<&str>) -> (SkinNode, Option<SkinNode>) {
    let main = main_layout(id);
    let roots: Vec<&[SkinNode]> = std::iter::once(main.tree.as_ref().unwrap().root.as_slice())
        .chain(main.popups.values().map(|p| p.tree.root.as_slice()))
        .collect();
    let slot_id = format!("Communities{words}");
    let slot = roots
        .iter()
        .find_map(|r| find(r, &|n| n.id == slot_id))
        .cloned()
        .expect("the slot");
    let donor = donor.map(|d| {
        roots
            .iter()
            .find_map(|r| find(r, &|n| n.id == d))
            .cloned()
            .expect("the donor")
    });
    (slot, donor)
}

/// Lead ruling 2026-11-24: each button's art by its meaning, from the
/// pack's own buttons; a pack with none serves a plain worded button.
#[test]
fn each_button_takes_the_packs_art_for_its_meaning() {
    for (id, words, donor) in [
        ("symbiosis", "Open", Some("ChatRoomEnter")),
        ("symbiosis", "Leave", Some("ChatRoomDelete")),
        ("symbiosis", "New", Some("ChatRoomCreate")),
        ("combat", "Leave", Some("StopButton")),
        ("combat", "New", Some("AddServerButton")),
        ("xfire-classic", "Open", Some("ChatButton")),
        ("xfire-classic", "Leave", None),
    ] {
        let (slot, donor) = slot_and_donor(id, words, donor);
        if let Some(d) = donor {
            assert_eq!(slot.art, d.art, "{id} {words}");
        } else {
            assert!(slot.art.is_none(), "{id} {words}: plain");
            let text = &slot.children[0];
            assert_eq!(text.label.as_ref().map(|l| l.text.as_str()), Some(words));
        }
    }
}
