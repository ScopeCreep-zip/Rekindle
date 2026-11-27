//! The names a tile's anchors see ([`Sibs`]) and a name's span on one axis.

use std::collections::BTreeMap;

use rekindle_types::dto::skin_tree::NodeSpan;

use super::anchor::{At, Axis};

/// The names a tile's anchors see, in its parent's frame: every tile
/// placed before it in the window outside its own branch (its siblings,
/// its ancestors' siblings), as Xfire's engine resolved a name against any
/// tile already laid in the window. Each is its box where that is
/// expressible in the frame, else the edges that are: Space Cowboy's
/// `ChatEdit` `Y="ChatArea.BOTTOM+2"` (`chatwindow.xml:37`) under
/// `ListViewBG2`, which sits on `ChatArea.BOTTOM+5` and so is fixed from
/// the window's bottom; there `ChatArea`'s bottom is a point, its top
/// (82 px from the window's top) is not.
#[derive(Clone, Default)]
pub(super) struct Sibs {
    pub(super) spans: BTreeMap<String, (NodeSpan, NodeSpan)>,
    /// Per axis (x, y), the near and far edge, when known in the frame.
    pub(super) edges: BTreeMap<String, [[Option<At>; 2]; 2]>,
    /// A sibling laid on one axis (x, y) and not yet the other (SKC3,
    /// [`super::place_order`]): what that axis's anchors see of it.
    half: BTreeMap<String, [Option<NodeSpan>; 2]>,
}

impl Sibs {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// `name` laid at `xy` in this frame; it shadows an outer tile of the
    /// same name.
    pub(super) fn insert(&mut self, name: String, xy: (NodeSpan, NodeSpan)) {
        self.edges.remove(&name);
        self.half.remove(&name);
        self.spans.insert(name, xy);
    }

    /// `name` laid on axis `i` (0 x, 1 y) only; [`Self::insert`] lays it
    /// whole once both are known.
    pub(super) fn insert_axis(&mut self, name: String, i: usize, span: NodeSpan) {
        self.half.entry(name).or_default()[i] = Some(span);
    }
}

impl FromIterator<(String, (NodeSpan, NodeSpan))> for Sibs {
    fn from_iter<I: IntoIterator<Item = (String, (NodeSpan, NodeSpan))>>(it: I) -> Self {
        Self {
            spans: it.into_iter().collect(),
            ..Self::default()
        }
    }
}

/// `name`'s span on `axis`: a sibling laid whole, else one laid on that
/// axis alone.
pub(super) fn span_of(sibs: &Sibs, name: &str, axis: &Axis) -> Option<NodeSpan> {
    let key = name.to_ascii_lowercase();
    let i = usize::from(axis.pos != "X");
    sibs.spans
        .get(&key)
        .map(|s| if i == 0 { s.0.clone() } else { s.1.clone() })
        .or_else(|| sibs.half.get(&key).and_then(|h| h[i].clone()))
}
