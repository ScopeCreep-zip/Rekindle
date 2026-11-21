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

use super::skin::{Insets, RegionState};
use super::skin_layout::SkinWindow;
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
    /// Centred in its parent's box widened by `before` px at its start and
    /// `after` px at its end, then moved by `by` (plan SK2 lead decision
    /// 32): the frame the pack centred it in. A parent an `Outdent*` cut
    /// keeps its uncut length here (Total Chaos's `BottomCenter` tab,
    /// centred in the 471 px `BottomBar` that lost its first column); a
    /// client box or a capped stretch is a narrower frame (negative).
    /// Xfire floors: the start is ⌊(len + before + after − size) / 2⌋ +
    /// `by` − `before`, the one rule the shape and the stylesheet place by.
    Centre { by: i16, before: i16, after: i16 },
}

impl Anchor {
    /// Centred in the parent's own box, then moved by `by`.
    #[must_use]
    pub const fn centre(by: i16) -> Self {
        Self::Centre {
            by,
            before: 0,
            after: 0,
        }
    }
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
                    Anchor::Centre { by, before, after } => {
                        let (b, a) = (i32::from(before), i32::from(after));
                        (len + b + a - size).div_euclid(2) + i32::from(by) - b
                    }
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

/// Where one piece of a component's image set draws in its tile's box
/// (Xfire's `Image Type=`, spec A7; the engine's table, `Xfire.exe` UTF-16
/// strings at 0x2b4234-0x2b42bc): the corners once at their natural size,
/// `top`/`bottom` repeated across between the corner columns, `left`/`right`
/// repeated down between the corner rows, `middle` repeated both ways in
/// the rest. A type outside the table (Total Chaos's `center`) is no piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PieceSlot {
    TopLeft,
    Top,
    TopRight,
    Left,
    Middle,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl PieceSlot {
    /// Every slot, in drawing-table order.
    pub const ALL: [Self; 9] = [
        Self::TopLeft,
        Self::Top,
        Self::TopRight,
        Self::Left,
        Self::Middle,
        Self::Right,
        Self::BottomLeft,
        Self::Bottom,
        Self::BottomRight,
    ];

    /// The slot an Xfire `Image Type=` names, case-insensitively.
    #[must_use]
    pub fn of_type(t: &str) -> Option<Self> {
        let t = t.trim();
        Self::ALL
            .into_iter()
            .find(|s| s.key().eq_ignore_ascii_case(t))
    }

    /// The pack's (lower-case) type word.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::TopLeft => "topleft",
            Self::Top => "top",
            Self::TopRight => "topright",
            Self::Left => "left",
            Self::Middle => "middle",
            Self::Right => "right",
            Self::BottomLeft => "bottomleft",
            Self::Bottom => "bottom",
            Self::BottomRight => "bottomright",
        }
    }
}

/// One piece: its package image at its natural size, px.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodePiece {
    pub image: String,
    pub width: u16,
    pub height: u16,
}

/// One state's piece set (spec A7, A10): each state is a whole set drawn at
/// the tile's box size, so its pieces need not match the normal set's.
pub type NodePieces = BTreeMap<PieceSlot, NodePiece>;

/// A node's art: its normal piece set and its other states' sets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeArt {
    pub normal: NodePieces,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub states: BTreeMap<RegionState, NodePieces>,
}

impl NodeArt {
    /// Art of one `middle` piece `width × height` and no other state.
    #[must_use]
    pub fn middle(image: String, width: u16, height: u16) -> Self {
        Self {
            normal: NodePieces::from([(
                PieceSlot::Middle,
                NodePiece {
                    image,
                    width,
                    height,
                },
            )]),
            states: BTreeMap::new(),
        }
    }

    /// Every package image the art names, normal set first.
    pub fn images(&self) -> impl Iterator<Item = &String> {
        self.normal
            .values()
            .chain(self.states.values().flat_map(BTreeMap::values))
            .map(|p| &p.image)
    }
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

/// The window's component tree: the root's children, laid in the pack's
/// window: the whole window less `outsets`, the overhang of the nodes the
/// pack lays outside its own window (Total Chaos's `MainTitleBar`
/// `OutdentLeft="1"`), which the layout's outsets include.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkinTree {
    pub outsets: Insets,
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
        let over = self.outsets;
        let (left, top) = (i32::from(over.left), i32::from(over.top));
        let pack = [
            left,
            top,
            w - left - i32::from(over.right),
            h - top - i32::from(over.bottom),
        ];
        for n in &self.root {
            walk(n, pack, &mut out);
        }
        out
    }
}

/// One node of a window's shell outline (plan SK2-S2): what the webview
/// binds into each `div[data-skin-node]`. It carries no box: every
/// position comes from the Host's emitted rules (design §5), so the shell
/// only nests the divs in the pack's document order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellNode {
    pub id: String,
    pub role: NodeRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shows_with: Option<ViewKey>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ShellNode>,
}

/// A window kind's shell outline: the served tree's nodes, roles and
/// nesting, for the window whose `data-skin-window` names `window`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkinShell {
    pub window: SkinWindow,
    pub root: Vec<ShellNode>,
}

impl SkinNode {
    fn shell(&self) -> ShellNode {
        ShellNode {
            id: self.id.clone(),
            role: self.role,
            shows_with: self.shows_with,
            children: self.children.iter().map(Self::shell).collect(),
        }
    }
}

impl SkinTree {
    /// The tree's outline for `window`'s shell: ids, roles and nesting.
    #[must_use]
    pub fn shell(&self, window: SkinWindow) -> SkinShell {
        SkinShell {
            window,
            root: self.root.iter().map(SkinNode::shell).collect(),
        }
    }
}
