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
use super::skin_action::NodeTrigger;
use super::skin_layout::SkinWindow;
pub use super::skin_title::{TitleSlices as NodeSlices, TitleText as NodeText};
pub use super::skin_tree_role::{Edge, LampKey, NodeRole, ViewKey};

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

/// The Host's words over a node it placed (rule N4, plan SK2-N4): the
/// cloned Communities tab's art is another tab's, label and all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeLabel {
    pub text: String,
    /// `[x, y, width, height]` of the node's box, px from its top-left,
    /// holding the copied art's own word (xslimfire's and Total Chaos's
    /// tabs are one whole picture, "Files" drawn in): the shell fills it
    /// with the pack's ground before the text. `None` when the art draws
    /// no word.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<[u16; 4]>,
    /// A built-in glyph the shell draws in place of `text` when the copied
    /// art is an icon button (Symbiosis's 46 x 48 `ButtonVideosTab`): drawn
    /// in the node's box in the strip's icon colour, `text` its accessible
    /// name and tooltip. `cover`, when served, is the box the glyph
    /// replaces (the copied icon, or a renamed icon title's usable box,
    /// plan SK5). `None` for a worded tab or title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph: Option<NodeGlyph>,
}

/// The part of a text field's box its pack's interactive siblings cover
/// (plan SK5): px from the field's left and right edges, measured in the
/// tree laid at the window's served minimum (Symbiosis's notifications
/// gear over the end of its name field). The shell lays the field's text
/// and its ellipsis between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeInset {
    pub left: u16,
    pub right: u16,
}

/// The shell's built-in glyphs a [`NodeLabel`] may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeGlyph {
    Communities,
    /// The app mark (plan SK5): a renamed menu title whose node is
    /// icon-shaped (XGlow's 24 x 22 file title), where no legible
    /// "Rekindle" fits.
    Rekindle,
}

/// How a node's served target meets WCAG 2.5.8 (plan SK7): `Size`, its
/// target box is at least 24 x 24 px; `Spacing`, a neighbour left no room
/// for 24 px, but the 24 px circle centred on its box meets no other target
/// and no other undersized target's circle (the criterion's spacing
/// exception); `Short`, neither holds (the pack lays its neighbours too
/// close): a pack-layout finding, served with the box the node could grow
/// to. `Frame`: a window-drag area ([`SkinNode::drag_area`]), served its
/// pointer area with the holes cut over the controls it meets but no 2.5.8
/// rule (plan SK11 lead decision): the criterion's targets are controls,
/// and a title bar or a window's ground is no control (no OS sets a
/// minimum for it). A resize grip is a control and takes the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetRule {
    Size,
    Spacing,
    Short,
    Frame,
}

/// The target box the Host serves a small interactive node (plan SK7, WCAG
/// 2.5.8): its box grown about its art to 24 x 24 px (clamped to the
/// window, stopped at its neighbours' targets), its art drawn at its own
/// size inside. Measured at the window's served minimum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NodeTarget {
    /// The target box `[x, y, w, h]`, `x` and `y` from the art box's
    /// top-left (never positive).
    #[serde(rename = "box")]
    pub area: [i32; 4],
    /// The art's rect `[x, y, w, h]` inside the target box.
    pub art: [i32; 4],
    pub rule: TargetRule,
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
    /// Words the Host draws over the node (rule N4's Communities tab,
    /// whose art is a clone of another tab's): never a pack's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<NodeLabel>,
    /// How far the pack's interactive siblings reach into a text field
    /// (plan SK5): the shell lays the field's text inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inset: Option<NodeInset>,
    /// The Host's target box for a small interactive node (plan SK7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<NodeTarget>,
    /// The tab whose view shows this node (a view's button row).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shows_with: Option<ViewKey>,
    /// The pack's `ControlGroupId` (`xfirecomponents.xml:1172`, `:1983`):
    /// the tab controls of one group switch together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_group: Option<u16>,
    /// The pack's `(event, action)` pairs (plan SK2-A, decision 36): the
    /// tile's own, then its component's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<NodeTrigger>,
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
    /// The pack's `ControlGroupId` (`xfirecomponents.xml:1172`, `:1983`):
    /// the tab controls of one group switch together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_group: Option<u16>,
    /// The Host's words over the node (rule N4), drawn by the shell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<NodeLabel>,
    /// How far the pack's interactive siblings reach into a text field
    /// (plan SK5): the shell lays the field's text inside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inset: Option<NodeInset>,
    /// The Host's target box for a small interactive node (plan SK7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<NodeTarget>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<NodeTrigger>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ShellNode>,
}

/// A window kind's shell outline: the served tree's nodes, roles and
/// nesting, for the window whose `data-skin-window` names `window`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SkinShell {
    pub window: SkinWindow,
    /// The popup this outline is and where it sits, else the window's own
    /// tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub popup: Option<ShellPopup>,
    pub root: Vec<ShellNode>,
    /// Rekindle's items for each engine menu the pack's buttons open
    /// (design §2.6), so the shell draws a pressed menu from the same
    /// answer that laid its button.
    pub menus: super::skin_action::MenuRegistry,
    /// The window's served minimum (logical px), the frame's outsets
    /// included: what the desktop shell's fit holds the window to (plan
    /// SK4); a popup's is its frame's at the launching window's minimum.
    pub min_width: u16,
    pub min_height: u16,
    /// The `skin://` href of every image this tree's own rules draw that
    /// `SkinApplied::images` (the shared sheet's) does not list, so only a
    /// window binding this shell decodes them before it shows the sheet
    /// (plan SK4).
    pub images: Vec<String>,
}

/// A popup outline's name (its key in the window's `popups`) and its
/// placement beside the launching window, served on the shell so a
/// frontend places the popup from the shell alone (plan SK2-P1c): the
/// window's [`super::skin_layout::PopupLayout`] less its tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShellPopup {
    pub name: String,
    pub anchor: super::skin_layout::PopupAnchor,
    pub width: u16,
    pub min_width: u16,
    pub min_height: u16,
    pub y: NodeSpan,
}

impl SkinNode {
    /// The node's pressing triggers and its parts' (a control's label
    /// tile holding the part that runs it, StealthX's `XfireMenuText` >
    /// `FileMenu`).
    pub fn presses(&self) -> impl Iterator<Item = &NodeTrigger> {
        self.triggers
            .iter()
            .chain(
                self.children
                    .iter()
                    .filter(|c| c.role == NodeRole::Part)
                    .flat_map(|c| c.triggers.iter()),
            )
            .filter(|t| t.presses())
    }

    /// Whether the node takes input ([`NodeRole::interactive`] over its
    /// own and its parts' triggers).
    #[must_use]
    pub fn interactive(&self) -> bool {
        let presses: Vec<NodeTrigger> = self.presses().cloned().collect();
        self.role.interactive(&presses)
    }

    /// The edge a pointer trigger resizes the window from (a pack's
    /// `Resize<Edge>` on a grip strip).
    #[must_use]
    pub fn resize_edge(&self) -> Option<Edge> {
        self.triggers.iter().find_map(|t| match t.run {
            super::skin_action::SkinAction::Resize(e) if t.on.pointer() => Some(e),
            _ => None,
        })
    }

    /// Whether the node is a window drag area: it takes the pointer, and
    /// only for the window's frame gestures
    /// ([`NodeTrigger::frame_gesture`]: move, double-click maximize), with
    /// no resize (a resize grip, [`Self::resize_edge`]). Onyx's `TopLeft`
    /// (`leftDown` drags, `doubleClick` maximizes) is one.
    #[must_use]
    pub fn drag_area(&self) -> bool {
        let mut pointer = self.triggers.iter().filter(|t| t.on.pointer()).peekable();
        pointer.peek().is_some()
            && pointer.all(NodeTrigger::frame_gesture)
            && self.resize_edge().is_none()
    }

    /// Whether the node is a frame node: a resize grip or a drag area,
    /// whose pointer area never takes a press over a press control
    /// (plan SK11, the skin's `target::grips`).
    #[must_use]
    pub fn frame_node(&self) -> bool {
        self.resize_edge().is_some() || self.drag_area()
    }

    fn shell(&self) -> ShellNode {
        ShellNode {
            id: self.id.clone(),
            role: self.role,
            shows_with: self.shows_with,
            control_group: self.control_group,
            label: self.label.clone(),
            inset: self.inset,
            target: self.target,
            triggers: self.triggers.clone(),
            children: self.children.iter().map(Self::shell).collect(),
        }
    }
}

impl SkinTree {
    /// The tree's outline for `window`'s shell: ids, roles and nesting,
    /// with the window's served minimum `min` (width, height) and the
    /// tree's own `images`.
    #[must_use]
    pub fn shell(&self, window: SkinWindow, min: (u16, u16), images: Vec<String>) -> SkinShell {
        SkinShell {
            window,
            popup: None,
            root: self.root.iter().map(SkinNode::shell).collect(),
            menus: super::skin_action::MenuRegistry::rekindle(),
            min_width: min.0,
            min_height: min.1,
            images,
        }
    }
}
