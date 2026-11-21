//! A skin's window as the pack's own component tree (plan SK2, design
//! `sk2/design.md` §5): every Xfire pack describes the whole buddy list as
//! named components (`MainTitleBar`, `MenuBar`, `UserNameField`,
//! `TabbedControl`, `BaseBoard`…) placed by symbolic anchors; the
//! importers resolve the anchors and the Host validates the tree and
//! emits one CSS rule per node. The window kind, minimum size, outsets
//! and frame tiles stay in [`super::skin_layout::WindowLayout`] (SK0), the
//! tree's one carrier, and the pack id in the served
//! [`super::skin_library::SkinFrames`]: one source for each.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::skin::RegionState;
pub use super::skin_title::{TitleSlices as NodeSlices, TitleText as NodeText};
pub use super::skin_tree_role::{ActionKey, Edge, LampKey, MenuKey, NodeRole, ViewKey};

/// Where a fixed-size node sits along one axis of its parent's box, px.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "offset")]
pub enum Anchor {
    /// Its near side this far from the parent's start edge.
    Start(i16),
    /// Its far side this far before the parent's end edge.
    End(i16),
    /// Centred, then moved by this much.
    Centre(i16),
}

/// One axis of a node's box, parent-relative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", deny_unknown_fields)]
pub enum NodeSpan {
    /// Its own size (its art's), placed once.
    Fixed { at: Anchor, size: u16 },
    /// Stretched between two insets from the parent's edges (Xfire's
    /// `ResizeX="100"` with `Indent*`; negative for an `Outdent*`).
    Stretch { start: i16, end: i16 },
}

impl NodeSpan {
    /// The span's start and length inside a parent `len` px long.
    #[must_use]
    pub fn place(self, len: i32) -> (i32, i32) {
        match self {
            Self::Fixed { at, size } => {
                let size = i32::from(size);
                let start = match at {
                    Anchor::Start(o) => i32::from(o),
                    Anchor::End(o) => len - i32::from(o) - size,
                    Anchor::Centre(o) => (len - size) / 2 + i32::from(o),
                };
                (start, size)
            }
            Self::Stretch { start, end } => {
                let s = i32::from(start);
                (s, len - s - i32::from(end))
            }
        }
    }
}

/// A node's art: its normal image (or 3-slice), drawn at its own size and
/// never stretched, and its other states' images of the same size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeArt {
    pub image: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub states: BTreeMap<RegionState, String>,
}

/// One component of the pack's tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkinNode {
    /// The pack's tile name, unique in the tree.
    pub id: String,
    pub role: NodeRole,
    pub x: NodeSpan,
    pub y: NodeSpan,
    /// The pack's `Z`.
    #[serde(default)]
    pub z: i16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art: Option<NodeArt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<NodeText>,
    /// The tab whose view shows this node (a view's button row).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shows_with: Option<ViewKey>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<SkinNode>,
}

/// The window's component tree: the root's children, laid in the whole
/// window (the frame rectangle grown by the layout's outsets).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkinTree {
    pub root: Vec<SkinNode>,
}

impl SkinTree {
    /// Every node, depth first, with its box in a window `w × h`.
    #[must_use]
    pub fn boxes(&self, w: i32, h: i32) -> Vec<(&SkinNode, [i32; 4])> {
        fn walk<'a>(node: &'a SkinNode, parent: [i32; 4], out: &mut Vec<(&'a SkinNode, [i32; 4])>) {
            let (left, wide) = node.x.place(parent[2]);
            let (top, tall) = node.y.place(parent[3]);
            let area = [parent[0] + left, parent[1] + top, wide, tall];
            out.push((node, area));
            for child in &node.children {
                walk(child, area, out);
            }
        }
        let mut out = Vec::new();
        for n in &self.root {
            walk(n, [0, 0, w, h], &mut out);
        }
        out
    }
}
