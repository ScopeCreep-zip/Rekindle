//! A grip's edge comes from a closed table (plan SK2-A4): Symbiosis's
//! `MainResizeGripperTL`/`TR`/`BL`/`L`/`R` resize from their own corner,
//! and a grip whose name names no known edge is an import error unless
//! the pack wires its own trigger (XGlow's `[Gripper]`, Combat's
//! `[ResizeUlbr]`, both `Action.lbtndown=ResizeBR`).

use rekindle_types::dto::skin_action::{NodeTrigger, SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{Edge, NodeRole};

use super::roles::{check_grip, role, Facts, Role};

fn implied(name: &str) -> Option<Role> {
    let facts = Facts {
        has_art: true,
        class: Some("Tile"),
        children: &[],
    };
    role(name, Some(name), None, &facts)
}

fn resizes(edge: Edge) -> Role {
    Role::Acts(
        NodeRole::Button,
        Some(NodeTrigger {
            on: SkinEvent::LeftDown,
            run: SkinAction::Resize(edge),
        }),
    )
}

#[test]
fn symbiosis_grips_resize_from_their_own_edge() {
    for (name, edge) in [
        ("MainResizeGripperTL", Edge::TopLeft),
        ("MainResizeGripperTR", Edge::TopRight),
        ("MainResizeGripperBL", Edge::BottomLeft),
        ("MainResizeGripperL", Edge::Left),
        ("MainResizeGripperR", Edge::Right),
        ("MainResizeGripperBR", Edge::BottomRight),
        ("MainResizeGripperTop", Edge::Top),
        ("GripperTopRight", Edge::TopRight),
        ("GripperBottomRight", Edge::BottomRight),
        ("ResizeUp", Edge::Top),
    ] {
        assert_eq!(implied(name), Some(resizes(edge)), "{name}");
        assert_eq!(check_grip(name, Some(name), &[]), Ok(()), "{name}");
    }
}

#[test]
fn an_unknown_edge_is_refused_unless_the_pack_wires_the_grip() {
    let refused = check_grip("ResizeUlbr", Some("ResizeUlbr"), &[]);
    assert_eq!(
        refused,
        Err("ResizeUlbr: grip resizeulbr: unknown edge".to_owned())
    );
    let wired = [NodeTrigger {
        on: SkinEvent::LeftDown,
        run: SkinAction::Resize(Edge::BottomRight),
    }];
    assert_eq!(check_grip("ResizeUlbr", Some("ResizeUlbr"), &wired), Ok(()));
    // Never a guessed corner: the name implies no trigger at all.
    assert_eq!(implied("Gripper"), Some(Role::Acts(NodeRole::Button, None)));
}

/// SK2-P1f: the info popup's `InfoGripperTR`/`BR` are corner art, not
/// grips: every pack draws them with no trigger and no `Cursor` (Classic's
/// `InfoInfoGripperTR`, `xfirecomponents.xml:1962-1977`; Combat's
/// `CombatInfoSkin.ini:111-121`), while the popup's resize is its own wired
/// `InfoResizeRight` (`ResizeR`).
#[test]
fn info_popup_grippers_are_corner_art() {
    for (name, component) in [
        ("InfoGripperTR", "InfoInfoGripperTR"),
        ("InfoGripperBR", "InfoInfoGripperBR"),
        ("InfoGripperTR", "InfoGripperTR"),
        ("InfoGripperBR", "InfoGripperBR"),
    ] {
        let facts = Facts {
            has_art: true,
            class: Some("Tile"),
            children: &[],
        };
        assert_eq!(
            role(name, Some(component), None, &facts),
            Some(Role::Shows(NodeRole::Decor)),
            "{name} {component}"
        );
        assert_eq!(check_grip(name, Some(component), &[]), Ok(()), "{name}");
    }
    assert_eq!(implied("InfoInfoResizeRight"), Some(resizes(Edge::Right)));
}
