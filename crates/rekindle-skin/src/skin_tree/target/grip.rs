//! A resize grip's and a drag area's pointer area (plan SK11): the frame
//! band ([`band`]), the holes the press controls shown with it cut from it
//! ([`grips`]), and the right-click system menu a grip takes from the frame
//! strip it lies over ([`Grip::takes`]).

use rekindle_types::dto::skin_action::{EngineMenu, NodeTrigger, SkinAction, SkinEvent};
use rekindle_types::dto::skin_tree::{Edge, SkinNode, SkinTree, TargetRule};

use super::super::validate::{overlap, text_sized, Apart, HIT};
use super::{far, listens, meet, minus, small, targets, Laid};

/// The part of a listening node at `art` (served `area`) a grip's band
/// must leave to it: a small node's whole target box (SC 2.5.8 measures
/// it), a node already 24 x 24 the 24 x 24 square about its centre (where
/// a press lands, plan SK9), so a long edge loses only that notch.
pub(in crate::skin_tree) fn press_box(art: [i32; 4], area: [i32; 4]) -> [i32; 4] {
    if small(art) {
        return area;
    }
    let (cx, cy) = (art[0] + art[2] / 2, art[1] + art[3] / 2);
    let half = HIT / 2;
    meet([cx - half, cy - half, HIT, HIT], art).unwrap_or(art)
}

/// Whether `g` less `holes` still holds a 24 x 24 square.
fn free_square(g: [i32; 4], holes: &[[i32; 4]]) -> bool {
    let mut xs = vec![g[0], g[0] + g[2] - HIT];
    let mut ys = vec![g[1], g[1] + g[3] - HIT];
    for h in holes {
        xs.extend([h[0] + h[2], h[0] - HIT]);
        ys.extend([h[1] + h[3], h[1] - HIT]);
    }
    xs.iter().any(|&x| {
        ys.iter().any(|&y| {
            let s = [x, y, HIT, HIT];
            meet(g, s) == Some(s) && holes.iter().all(|h| !overlap(s, *h))
        })
    })
}

/// A resize grip's pointer area (plan SK11): its frame band, never its art
/// box (Onyx's `TopLeft` corner art, 83 x 128, lies over its Friends tab).
/// On each axis the grip resizes, the band is 24 px in from the art's side
/// on the window's frame (the side it drags), clamped to the art when the
/// art is thicker and grown inward when thinner: a corner, frame or inner
/// (`BRGripper`), is the 24 x 24 at the corner it resizes; an edge is
/// 24 px thick along its length. On an axis it does not resize, the band
/// runs the art's length, grown to 24 px about its centre. The band then
/// lies in the window.
#[must_use]
pub fn band(art: [i32; 4], edge: Edge, (win_w, win_h): (i32, i32)) -> [i32; 4] {
    let (low_x, high_x, low_y, high_y) = match edge {
        Edge::Top => (false, false, true, false),
        Edge::Bottom => (false, false, false, true),
        Edge::Left => (true, false, false, false),
        Edge::Right => (false, true, false, false),
        Edge::TopLeft => (true, false, true, false),
        Edge::TopRight => (false, true, true, false),
        Edge::BottomLeft => (true, false, false, true),
        Edge::BottomRight => (false, true, false, true),
    };
    let axis = |at: i32, len: i32, win: i32, low: bool, high: bool| {
        let (start, full) = if low {
            (at, HIT)
        } else if high {
            (at + len - HIT, HIT)
        } else {
            let full = len.max(HIT);
            (at - (full - len) / 2, full)
        };
        (start.min(win - full).max(0), full)
    };
    let (left, width) = axis(art[0], art[2], win_w, low_x, high_x);
    let (top, height) = axis(art[1], art[3], win_h, low_y, high_y);
    [left, top, width, height]
}

/// A resize grip's target ([`grips`]): its frame band ([`band`]), and the
/// holes cut from it (window coordinates, disjoint).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grip {
    pub laid: Laid,
    pub holes: Vec<[i32; 4]>,
    /// The triggers it takes from the system-menu strips its band lies
    /// over ([`sys_strip`]), served on the grip ([`super::nodes`]).
    pub takes: Vec<NodeTrigger>,
}

/// A node shown with a frame node that may cut it: the node, its target
/// box (or art box), its press box ([`press_box`]) and its art box.
type Other<'a> = (&'a SkinNode, [i32; 4], [i32; 4], [i32; 4]);

/// Whether `n` is a frame strip whose only gesture is the right-click
/// system menu (Symbiosis's `ChatTopRow`, `OnRightClick ContextMenuSys`,
/// `Components/ChatWindow.xml:12-15`): the non-client area of a Windows
/// window, where a right-click on any part of the frame, borders included,
/// opens the system menu. A grip over it takes that trigger ([`grips`]).
fn sys_strip(n: &SkinNode) -> bool {
    !listens(n)
        && !n.triggers.is_empty()
        && n.triggers.iter().all(|t| {
            t.on == SkinEvent::RightClick
                && matches!(
                    t.run,
                    SkinAction::Menu(EngineMenu::Sys) | SkinAction::ContextMenu(EngineMenu::Sys)
                )
        })
}

/// The target of every resize grip of `tree` in `window` (plan SK11, lead
/// decision: a frameless window resizes only by dragging its frame, so a
/// grip is a pointer target like any control). Its pointer area is its
/// frame band ([`band`]: a corner 24 x 24, an edge 24 px thick along its
/// whole length), never its art box: the art draws and takes no press.
/// A frame node never takes a press over a press control, whatever the
/// pack's order (lead decision, title-bar semantics): where the band meets
/// the press box ([`press_box`]) of a listening node shown with it, that
/// stretch is cut out, as a hole the stylesheet clips from the band
/// ([`super::css`]), over the grip's art too (the art takes no press). A
/// grip never cuts another grip: a corner stacks above the edges
/// ([`super::validate::stack_z`]).
///
/// A drag area ([`SkinNode::drag_area`]: only move and double-click
/// maximize, Onyx's `TopLeft` over its Friends tab) is a frame node like a
/// grip, as a title bar is in every OS (Tauri's `data-tauri-drag-region`
/// takes only the presses no control takes): it never takes a press over
/// a press control, whatever the pack's order. Its pointer area is its art
/// box in the window, cut over the press box of every listening node and
/// the band of every grip it would lie above; the art still draws in the
/// pack's order. A drag area with no hole keeps its own box and is not
/// listed. A frame node never cuts another's area for a drag area (a grip
/// or drag area is no press control to a grip; a drag area yields to a
/// grip).
///
/// A grip's rule (WCAG 2.5.8: a grip is a control): `Size` while 24 x 24
/// of the area is left, else `Spacing` when the 24 px circle on it meets
/// no listening node's box, else `Short` (a control the pack lays over its
/// band takes that stretch). A drag area's rule is `Frame` (lead decision:
/// a window-drag area is no control, so 2.5.8 sets it no size or spacing,
/// and it is never a short-list finding).
///
/// A grip whose band meets a system-menu strip ([`sys_strip`], lead
/// ruling, as in Windows) is not cut by it: the grip resizes on a left
/// drag and takes the strip's right-click system menu itself
/// ([`Grip::takes`]), so the grip keeps its band and the strip's gesture
/// is answered over the whole frame. A strip with any other trigger cuts
/// the grip as any control does.
#[must_use]
pub fn grips(tree: &SkinTree, window: (i32, i32), laid: &[Laid]) -> Vec<Grip> {
    let apart = Apart::new(&tree.root);
    let r2 = i64::from(HIT) * i64::from(HIT);
    let boxes = tree.boxes(window.0, window.1);
    let others: Vec<Other> = boxes
        .iter()
        .filter(|(n, b)| {
            !n.frame_node()
                && b[2] > 0
                && b[3] > 0
                && (listens(n) || n.triggers.iter().any(|t| t.on.pointer()))
        })
        .map(|(n, b)| {
            let area = laid.iter().find(|l| l.id == n.id).map_or(*b, |l| l.area);
            (*n, area, press_box(*b, area), *b)
        })
        .collect();
    let frames: Vec<(&SkinNode, [i32; 4], [i32; 4], bool)> = boxes
        .iter()
        .filter(|(n, b)| n.frame_node() && !text_sized(n) && b[2] > 0 && b[3] > 0)
        .filter_map(|(n, art)| {
            let g = match n.resize_edge() {
                Some(edge) => band(*art, edge, window),
                None => meet(*art, [0, 0, window.0, window.1])?,
            };
            Some((*n, *art, g, n.resize_edge().is_some()))
        })
        .collect();
    frames
        .iter()
        .filter_map(|&(n, art, g, grip)| {
            let with: Vec<&Other> = others
                .iter()
                .filter(|(o, ..)| !apart.apart(&n.id, &o.id))
                .collect();
            let (taken, with): (Vec<&Other>, Vec<&Other>) = with
                .into_iter()
                .partition(|(o, .., art)| grip && sys_strip(o) && overlap(g, *art));
            let mut takes: Vec<NodeTrigger> = Vec::new();
            for t in taken.iter().flat_map(|(o, ..)| &o.triggers) {
                if !takes.contains(t) {
                    takes.push(t.clone());
                }
            }
            let shown: Vec<[i32; 4]> = with.iter().map(|(_, o, ..)| *o).collect();
            let mut holes: Vec<[i32; 4]> = Vec::new();
            // Every press control shown with it is cut, whatever the
            // pack's order: the served stack nests each node's z inside
            // its parent's (CSS stacking contexts), so a control's
            // effective z ([`Stack::yields`]) does not lift it over a
            // frame node; Onyx's `TabNormalButton` (inside `TabNormalText`)
            // lies under `TopLeft` (`Z=2`) though its own `Z` is higher. A
            // hole over a control already above is a no-op.
            let mut cuts: Vec<[i32; 4]> = with.iter().map(|(_, _, press, _)| *press).collect();
            if !grip {
                cuts.extend(
                    frames
                        .iter()
                        .filter(|(m, .., is_grip)| *is_grip && !apart.apart(&n.id, &m.id))
                        .map(|(_, _, b, _)| *b),
                );
            }
            for press in cuts {
                let Some(m) = meet(g, press) else { continue };
                let mut parts = vec![m];
                for h in &holes {
                    parts = parts.into_iter().flat_map(|p| minus(p, *h)).collect();
                }
                holes.extend(parts);
            }
            if !grip && holes.is_empty() {
                return None;
            }
            let rule = if !grip {
                TargetRule::Frame
            } else if free_square(g, &holes) {
                TargetRule::Size
            } else if shown.iter().all(|o| far(g, *o) >= r2) {
                TargetRule::Spacing
            } else {
                TargetRule::Short
            };
            Some(Grip {
                laid: Laid {
                    id: n.id.clone(),
                    art,
                    area: g,
                    rule,
                },
                holes,
                takes,
            })
        })
        .collect()
}

/// The [`Laid`] of every grip ([`grips`]).
#[must_use]
pub fn grip_targets(tree: &SkinTree, window: (i32, i32), laid: &[Laid]) -> Vec<Laid> {
    grips(tree, window, laid)
        .into_iter()
        .map(|g| g.laid)
        .collect()
}

/// Each grip of `tree` in `window` with holes: its box, and the holes
/// relative to it (what the stylesheet clips, [`super::css`]).
#[must_use]
pub fn grip_holes(tree: &SkinTree, window: (i32, i32)) -> Vec<(String, [i32; 4], Vec<[i32; 4]>)> {
    let laid = targets(tree, window);
    grips(tree, window, &laid)
        .into_iter()
        .filter(|g| !g.holes.is_empty())
        .map(|g| {
            let a = g.laid.area;
            let rel = g
                .holes
                .iter()
                .map(|h| [h[0] - a[0], h[1] - a[1], h[2], h[3]])
                .collect();
            (g.laid.id, a, rel)
        })
        .collect()
}
