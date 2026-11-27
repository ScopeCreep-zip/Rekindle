//! A sibling set laid in dependency order (SKC3; ruling 3 of
//! `design-skin-catalogue-2026-11-26.md`: "anchored before placed: resolve
//! anchors by dependency order rather than file order, where the pack's
//! graph is acyclic"). Each axis is its own graph: GlazeX's chat
//! `UserNameField` sits on `Y=CloseBox.TOP` while `CloseBox` sits on
//! `X=MinimizeBox.RIGHT` and `MinimizeBox` on `X=UserNameField.RIGHT`
//! (`GlazeXChatSkin.ini:118-136`), a cycle of tiles but none on an axis.
//! A real cycle on one axis stays the first unresolved anchor's error.

use rekindle_types::dto::skin_tree::NodeSpan;

use super::anchor::{axis, Sibs, Sizes, X, Y};
use super::import::bad;
use crate::import::tree::Node;
use crate::import::ImportError;

/// A tile to lay: its id, node, art size, size limits and whether it is
/// a slot.
pub(super) struct Lay<'a> {
    pub(super) id: &'a str,
    pub(super) tile: &'a Node,
    pub(super) wh: (u16, u16),
    pub(super) max: (Option<u16>, Option<u16>),
    pub(super) slot: bool,
}

/// Each tile's (x, y) spans, every axis laid as soon as its anchors
/// resolve, in as many passes as they need; each axis laid is seen by the
/// anchors on that axis, each tile laid whole by both. `root`: the tiles
/// are the window's own layout.
///
/// # Errors
/// The first unresolved anchor's, once a pass lays nothing.
pub(super) fn place(
    tiles: &[Lay<'_>],
    names: &mut Sibs,
    sizes: &Sizes,
    root: bool,
) -> Result<Vec<(NodeSpan, NodeSpan)>, ImportError> {
    let mut laid: Vec<[Option<NodeSpan>; 2]> = vec![[None, None]; tiles.len()];
    loop {
        let mut progress = false;
        let mut first_err = None;
        for (t, l) in tiles.iter().zip(&mut laid) {
            if l.iter().all(Option::is_some) {
                continue;
            }
            for (i, ax) in [&X, &Y].into_iter().enumerate() {
                if l[i].is_some() {
                    continue;
                }
                let (size, max) = if i == 0 {
                    (t.wh.0, t.max.0)
                } else {
                    (t.wh.1, t.max.1)
                };
                match axis(t.tile, ax, size, max, (t.slot, root), (&*names, sizes)) {
                    Ok(s) => {
                        names.insert_axis(t.id.to_ascii_lowercase(), i, s.clone());
                        l[i] = Some(s);
                        progress = true;
                    }
                    Err(e) => {
                        first_err.get_or_insert_with(|| bad(format!("{}: {e}", t.id)));
                    }
                }
            }
            if let [Some(x), Some(y)] = &l {
                names.insert(t.id.to_ascii_lowercase(), (x.clone(), y.clone()));
            }
        }
        match first_err {
            None => break,
            Some(e) if !progress => return Err(e),
            Some(_) => {}
        }
    }
    laid.into_iter()
        .zip(tiles)
        .map(|(l, t)| match l {
            [Some(x), Some(y)] => Ok((x, y)),
            _ => Err(bad(format!("{}: not placed", t.id))),
        })
        .collect()
}
