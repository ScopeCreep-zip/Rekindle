//! The boxes no served text may run over (plan SK12), moved out of
//! [`super`] whole: what each obstacle is, how a content text's line meets
//! one (plan SK15b-2), and which obstacles stop a given node.

use rekindle_types::dto::skin_tree::{NodeRole, SkinNode, SkinTree, ViewKey};

use super::super::inset::is_text;
use super::super::titles::{laid_window, place};
use super::{draws_text, extent, reach, targets, Targets};

/// A box no text may run over (plan SK12), laid on the pack's boxes: a
/// blocking node's press area ([`super::reach`] over [`super::extent`]), another text's
/// box (its rows, at least one), or the views (the app's content, the
/// buddy list and its search field). The served text stops at the near
/// edge of any of them on its line, wherever it lies in the tree, except
/// its own ancestors and descendants.
pub(in crate::skin_tree) struct Obstacle {
    pub(in crate::skin_tree) id: String,
    pub(in crate::skin_tree) up: Vec<String>,
    pub(super) shows: Option<ViewKey>,
    /// A blocking node, which [`super::usable`] already counts among its
    /// siblings.
    pub(super) blocker: bool,
    pub(in crate::skin_tree) b: [i32; 4],
    /// The node's own box ([`super::extent`] for a blocker), before [`super::reach`]
    /// grows it over its target (see [`on_line`]).
    pub(super) art: [i32; 4],
}

impl Obstacle {
    /// This stop's box as it stops a content text's line ([`on_line`]).
    pub(in crate::skin_tree) fn on_content_line(&self) -> [i32; 4] {
        on_line(self.b, self.art, true)
    }
}

/// The box `b` of a stop whose own box is `art`, as it stops a text that
/// `keeps_line` or not. A content text's line is the shell's line box at
/// the pack's point (plan SK15b-2, [`super::content`]): a press area's
/// target padding above or below its art does not cut that line (XGlow's
/// `Status` art starts 3 px under the name's line, `Y="UserName.BOTTOM+2"`
/// then `+1`, `Xglow.ini:195-217`; its 24 px target reaches 2 px into it),
/// while its padding on the line's own axis still stops the words (plan
/// SK12: Onyx's `ToolsMenu` target 1 px left of its art).
pub(super) fn on_line(b: [i32; 4], art: [i32; 4], keeps_line: bool) -> [i32; 4] {
    if keeps_line {
        [b[0], art[1], b[2], art[3]]
    } else {
        b
    }
}

/// Every obstacle of `tree` laid in `window` (see [`Obstacle`]), on the
/// boxes as laid: what a content text's row may not reach (plan SK15b-2,
/// [`super::content`]).
pub(in crate::skin_tree) fn obstacles_of(tree: &SkinTree, window: (i32, i32)) -> Vec<Obstacle> {
    let targets = targets(tree, window);
    let mut all = Vec::new();
    let at = laid_window(tree.outsets, window);
    obstacles(&tree.root, at, &[], (&targets, None), &mut all);
    all
}

pub(super) fn obstacles(
    nodes: &[SkinNode],
    parent: [i32; 4],
    up: &[String],
    (targets, view): (&Targets, Option<ViewKey>),
    out: &mut Vec<Obstacle>,
) {
    for n in nodes {
        let b = place(n, parent);
        // A node shows with its own view or its nearest ancestor's (Simple-X's
        // `BigBrowseFileText` under the Files row's `SearchFiles`).
        let shows = n.shows_with.or(view);
        let blocker = n.interactive() && !is_text(n.role) && n.resize_edge().is_none();
        let own = if blocker {
            Some(extent(n, b, targets))
        } else if draws_text(n) {
            Some([b[0], b[1], b[2], b[3].max(1)])
        } else {
            (n.role == NodeRole::Views).then_some(b)
        };
        if let Some(art) = own {
            let reached = if n.role == NodeRole::Views {
                art
            } else {
                reach(n, art, targets)
            };
            out.push(Obstacle {
                id: n.id.clone(),
                up: up.to_vec(),
                shows,
                blocker,
                b: reached,
                art,
            });
        }
        let mut own = up.to_vec();
        own.push(n.id.clone());
        obstacles(&n.children, b, &own, (targets, shows), out);
    }
}

/// The obstacles that stop `n`'s text (see [`Obstacle`]): not `n`, its
/// ancestors (`up`) or descendants, not another view's, and not a
/// blocking sibling (counted by [`super::usable`] with its span).
/// With `siblings`, the blocking siblings count too (the plan SK13 cut).
pub(super) fn stops(
    n: &SkinNode,
    up: &[String],
    all: &[Obstacle],
    siblings: bool,
) -> Vec<[i32; 4]> {
    let mine = all
        .iter()
        .find(|o| o.id == n.id && o.up == up)
        .map_or(n.shows_with, |o| o.shows);
    all.iter()
        .filter(|o| {
            let apart = o.shows.is_some() && mine.is_some() && o.shows != mine;
            let kin = o.id == n.id || o.up.contains(&n.id) || up.contains(&o.id);
            let sibling_blocker = !siblings && o.blocker && o.up == up;
            !(kin || apart || sibling_blocker)
        })
        .map(|o| on_line(o.b, o.art, n.fit.is_some()))
        .collect()
}
