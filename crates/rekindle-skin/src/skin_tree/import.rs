//! The pack's component tree from its layout file (plan SK2, design §5
//! "Import"): the XML era's `<Tile>` tree, or the INI era's sections read
//! as the same tiles (`xfire_ini::title::tree`), each tile a
//! [`SkinNode`] whose symbolic anchors resolve parent-relative:
//! `X="XfireLogo.RIGHT+6"` (a sibling's edge ± n), `IndentRight="13+14"`
//! (a sum), `IndentBottom="BaseBoard.height+9"` (a sibling's size),
//! `JustX/JustY` (`RIGHT`/`BOTTOM` from the far edge, `CENTER`),
//! `ResizeX/Y="100"` with `Indent*`/`Outdent*` (stretched), and an INI
//! negative offset (from the far edge: XGlow's `XfireLogo` `X=-80`).
//! The size of a fixed tile is its component's normal art (SK1-S1's
//! [`component_pieces`], its piece set's natural size, spec A6). An anchor that does not resolve refuses the pack
//! (rule 2); a component the table lacks refuses it (rule 1) unless it is
//! classless art (Decor). The views node's box is the layout's content
//! rectangle.

use std::collections::{BTreeMap, BTreeSet};

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{Insets, RegionState, SkinWindow, TileAxis, WindowLayout};

pub use super::fit::views_content;
use rekindle_types::dto::skin_action::NodeTrigger;
use rekindle_types::dto::skin_tree::{
    NodeArt, NodePiece, NodePieces, NodeRole, NodeSpan, PieceSlot, SkinNode, SkinTree,
};

use super::anchor::{axis, client_box, client_inset, outer_for, to_tile, Sibs, Sizes, X, Y};
use super::clip::{clip, Art, Images};
use super::fit::fit;
use super::outdent;
use super::passes::{by_view, pages, qualify, rows_by_view, tab_views};
use super::rail;
use super::roles::{role, Facts};
use super::triggers::{self, Pack};
use crate::import::tree::Node;
use crate::import::xfire_xml::title::{component_pieces, Decode};
use crate::import::ImportError;
use crate::shape;

/// The package images the tree adds, by path.
type Files = BTreeMap<String, Vec<u8>>;

pub(super) fn bad(detail: impl Into<String>) -> ImportError {
    ImportError::Format {
        file: "skin tree".to_owned(),
        detail: detail.into(),
    }
}

/// A component's `MinWidth`/`MinHeight`/`MaxWidth`/`MaxHeight`.
fn limit_of(component: Option<&Node>, key: &str) -> Option<u16> {
    component?.attr_ci(key)?.trim().parse().ok()
}

/// How far a component's `DrawOnTopmost`/`DrawOnBottommost` (spec A9;
/// `Xfire.exe` UTF-16 strings at 0x2b75a4, 0x2b75c0) lifts its tile out of
/// its siblings' `Z` order: past every pack `Z` (they run -4..100), so
/// Classic's `TheX` (`xfirecomponents.xml:147`, `DrawOnTopmost="true"`)
/// draws above the siblings laid after it.
const TOPMOST: i16 = 1000;

/// The layer a tile's `component` puts it in (spec A9): [`TOPMOST`] above
/// its siblings, its negation below them, else 0.
fn layer_of(component: Option<&Node>) -> i16 {
    let on = |k: &str| {
        component
            .and_then(|c| c.attr_ci(k))
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
    };
    if on("DrawOnTopmost") {
        TOPMOST
    } else if on("DrawOnBottommost") {
        -TOPMOST
    } else {
        0
    }
}

/// A state's pieces as decoded, in the pack's order.
type Pieces = Vec<(PieceSlot, RgbaImage)>;

/// The natural size of decoded `pieces` (spec A6 `ToContent`); `None`
/// for no pieces.
fn natural_of(pieces: &Pieces) -> Option<(u32, u32)> {
    if pieces.is_empty() {
        return None;
    }
    let set: NodePieces = pieces
        .iter()
        .map(|(s, i)| {
            let side = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
            (
                *s,
                NodePiece {
                    image: String::new(),
                    width: side(i.width()),
                    height: side(i.height()),
                },
            )
        })
        .collect();
    let (w, h) = super::pieces::natural(&set);
    Some((u32::try_from(w).unwrap_or(0), u32::try_from(h).unwrap_or(0)))
}

/// A tile read but not yet placed.
struct Read<'n> {
    tile: &'n Node,
    id: String,
    role: NodeRole,
    /// The component's normal piece set (spec A7).
    pieces: Pieces,
    wh: (u16, u16),
    max: (Option<u16>, Option<u16>),
    /// The component's layer ([`layer_of`]).
    layer: i16,
    /// The component's `ControlGroupId` (INI tab controls that switch
    /// together).
    group: Option<String>,
    /// The tile's then its component's triggers (decision 36).
    triggers: Vec<NodeTrigger>,
}

struct Walk<'a> {
    window: SkinWindow,
    by_name: BTreeMap<String, &'a Node>,
    piece: Decode<'a>,
    files: Files,
    /// The images `files` were encoded from, by path (lead decision 15
    /// crops them).
    images: Images,
    /// Each tab control's path of child indices and its `ControlGroupId`.
    groups: Vec<(Vec<usize>, String)>,
    /// The pack's `<Popups>` tile names, lowercased.
    popups: BTreeSet<String>,
    /// Every control group id the pack's components declare.
    group_ids: BTreeSet<u16>,
}

impl Walk<'_> {
    fn pieces(&self, tile: &Node, state: &str) -> Result<Pieces, ImportError> {
        component_pieces(&self.by_name, self.piece, tile, state)
    }

    fn encode(&mut self, piece: &str, img: &RgbaImage) -> Result<Option<String>, ImportError> {
        let axes = (TileAxis::Start { offset: 0 }, TileAxis::Start { offset: 0 });
        Ok(
            shape::tile(self.window, piece, img, axes, true)?.map(|(t, png)| {
                self.files.insert(t.image.clone(), png);
                self.images.insert(t.image.clone(), img.clone());
                t.image
            }),
        )
    }

    /// Plan SK2 lead decision 8: an art-less tile with no declared size
    /// takes, on each axis, the largest art of its children stretched on
    /// that axis (XGlow's `[Add]` `Stretch=ToContent` slot holding
    /// `AddFriend` `ResizeX=100`, `buttons/add.gif` 54×18).
    fn content_size(&self, tile: &Node) -> Result<(u16, u16), ImportError> {
        let mut wh = (0u16, 0u16);
        for c in tile.children_named("Tile") {
            let normal = natural_of(&self.pieces(c, "Normal")?);
            let img = match normal {
                Some(n) => Some(n),
                None => natural_of(&self.pieces(c, "Hover")?),
            };
            let (cw, ch) = if let Some((w, h)) = img {
                (
                    u16::try_from(w).unwrap_or(u16::MAX),
                    u16::try_from(h).unwrap_or(u16::MAX),
                )
            } else {
                self.content_size(c)?
            };
            let stretched = |k: &str| c.attr_ci(k).is_some_and(|r| r.trim() == "100");
            if stretched("ResizeX") {
                wh.0 = wh.0.max(cw);
            }
            if stretched("ResizeY") {
                wh.1 = wh.1.max(ch);
            }
        }
        Ok(wh)
    }

    /// `pieces` encoded as package images named after `name` (a lone
    /// `middle` keeps the bare name), with their natural sizes.
    fn encode_set(&mut self, name: &str, pieces: &Pieces) -> Result<NodePieces, ImportError> {
        let mut set = NodePieces::new();
        for (slot, img) in pieces {
            let file = if *slot == PieceSlot::Middle && pieces.len() == 1 {
                name.to_owned()
            } else {
                format!("{name}-{}", slot.key())
            };
            if let Some(image) = self.encode(&file, img)? {
                let side = |v: u32| u16::try_from(v).unwrap_or(u16::MAX);
                set.insert(
                    *slot,
                    NodePiece {
                        image,
                        width: side(img.width()),
                        height: side(img.height()),
                    },
                );
            }
        }
        Ok(set)
    }

    /// A node's normal, hover, down and disabled piece sets (spec A10:
    /// each state is its own set drawn at the box, whatever its pieces'
    /// sizes).
    fn art(
        &mut self,
        id: &str,
        tile: &Node,
        pieces: &Pieces,
    ) -> Result<Option<NodeArt>, ImportError> {
        let slug: String = id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect();
        let normal = self.encode_set(&format!("node-{slug}"), pieces)?;
        if normal.is_empty() {
            return Ok(None);
        }
        let mut states = BTreeMap::new();
        for (state, set) in [
            (RegionState::Hover, "Hover"),
            (RegionState::Down, "Down"),
            (RegionState::Disabled, "Disabled"),
        ] {
            let other = self.pieces(tile, set)?;
            let encoded = self.encode_set(&format!("node-{slug}--{}", state.key()), &other)?;
            if !encoded.is_empty() {
                states.insert(state, encoded);
            }
        }
        Ok(Some(NodeArt { normal, states }))
    }

    /// One tile's id, role and art, before it is placed.
    fn read<'n>(&self, tile: &'n Node, parent: Option<NodeRole>) -> Result<Read<'n>, ImportError> {
        let component = tile.attr_ci("Component");
        let id = tile
            .attr_ci("Name")
            .or(component)
            .ok_or_else(|| bad("a tile with neither a name nor a component"))?
            .trim()
            .to_owned();
        let pieces = self.pieces(tile, "Normal")?;
        let comp = component.and_then(|c| self.by_name.get(&c.to_ascii_lowercase()).copied());
        let size = |v: u32| u16::try_from(v).map_err(|_| bad(format!("{id}: art too large")));
        // A tile with no art is its component's declared size: Classic's
        // `ResizeUp` `MinHeight="5"` (`xfirecomponents.xml:121`), Symbiosis's
        // grippers `MinWidth="10" MinHeight="10"` (`Components/MainWindow.xml:62`).
        let wh = match natural_of(&pieces) {
            Some((w, h)) => (size(w)?, size(h)?),
            // Art only on hover: the button's normal look is baked in the
            // art under it, and its hover image is its size (Total Chaos's
            // `TCLink`, `components.xml:507-519`).
            None => {
                if let Some((w, h)) = natural_of(&self.pieces(tile, "Hover")?) {
                    (size(w)?, size(h)?)
                } else {
                    let declared = (
                        limit_of(comp, "MinWidth").unwrap_or(0),
                        limit_of(comp, "MinHeight").unwrap_or(0),
                    );
                    if declared.0 == 0 || declared.1 == 0 {
                        let content = self.content_size(tile)?;
                        (
                            if declared.0 == 0 {
                                content.0
                            } else {
                                declared.0
                            },
                            if declared.1 == 0 {
                                content.1
                            } else {
                                declared.1
                            },
                        )
                    } else {
                        declared
                    }
                }
            }
        };
        // Spec A6, row 6: the component's `MinWidth`/`MinHeight` raise a
        // fixed tile above its art (Classic's `MainTitleBar` `MinHeight="51"`
        // over 47 px art, `xfirecomponents.xml:65-69`: 51 tall, so
        // `GripperTopLeft` `Y="MainTitleBar.Bottom-15"` lands at y 36).
        let wh = (
            wh.0.max(limit_of(comp, "MinWidth").unwrap_or(0)),
            wh.1.max(limit_of(comp, "MinHeight").unwrap_or(0)),
        );
        let children: Vec<String> = tile
            .children_named("Tile")
            .filter_map(|t| {
                t.attr_ci("Name")
                    .or(t.attr_ci("Component"))
                    .map(str::to_owned)
            })
            .collect();
        let facts = Facts {
            has_art: !pieces.is_empty(),
            class: comp.and_then(|c| c.attr_ci("Class")),
            children: &children,
        };
        let role = role(&id, component, parent, &facts).ok_or_else(|| {
            bad(format!(
                "{id}: component {} has no role",
                component.unwrap_or("-")
            ))
        })?;
        let max = (limit_of(comp, "MaxWidth"), limit_of(comp, "MaxHeight"));
        let group = comp
            .and_then(|c| c.attr_ci("ControlGroupId"))
            .or(tile.attr_ci("ControlGroupId"))
            .map(|g| g.trim().to_owned());
        let pack = Pack {
            popups: &self.popups,
            groups: &self.group_ids,
        };
        let triggers = triggers::read(tile, comp, &id, &pack)?;
        Ok(Read {
            tile,
            id,
            role,
            pieces,
            wh,
            max,
            layer: layer_of(comp),
            group,
            triggers,
        })
    }

    /// `node`'s tiles: placed in as many passes as their references need
    /// (Symbiosis's `ResizeTop` names `ResizeGripperTR`, the next tile),
    /// each against its siblings and `outer`, the names its parent sees.
    fn walk(
        &mut self,
        node: &Node,
        parent: Option<NodeRole>,
        outer: &Sibs,
        path: &[usize],
    ) -> Result<Vec<SkinNode>, ImportError> {
        let tiles = node
            .children_named("Tile")
            .map(|t| self.read(t, parent))
            .collect::<Result<Vec<_>, _>>()?;
        let mut names = outer.clone();
        let sizes: Sizes = tiles
            .iter()
            .map(|t| (t.id.to_ascii_lowercase(), t.wh))
            .collect();
        let mut placed: Vec<Option<(NodeSpan, NodeSpan)>> = vec![None; tiles.len()];
        loop {
            let mut progress = false;
            let mut first_err = None;
            for (i, t) in tiles.iter().enumerate() {
                if placed[i].is_some() {
                    continue;
                }
                let refs = (&names, &sizes);
                let xy = axis(t.tile, &X, t.wh.0, t.max.0, refs)
                    .and_then(|x| Ok((x, axis(t.tile, &Y, t.wh.1, t.max.1, refs)?)));
                match xy {
                    Ok(xy) => {
                        names.insert(t.id.to_ascii_lowercase(), xy);
                        placed[i] = Some(xy);
                        progress = true;
                    }
                    Err(e) => {
                        first_err.get_or_insert_with(|| bad(format!("{}: {e}", t.id)));
                    }
                }
            }
            match first_err {
                None => break,
                Some(e) if !progress => return Err(e),
                Some(_) => {}
            }
        }
        let mut out = Vec::new();
        for (i, (t, xy)) in tiles.into_iter().zip(placed).enumerate() {
            let mut own = path.to_vec();
            own.push(i);
            if let Some(g) = &t.group {
                self.groups.push((own.clone(), g.clone()));
            }
            let (x, y) = xy.ok_or_else(|| bad(format!("{}: not placed", t.id)))?;
            // Every tile's art is the node's (SK0: the art is the window's
            // shape; plan SK2-S1r): a decor band, logo or ad ground draws and
            // joins the outline like a button does.
            let art = if t.pieces.is_empty() {
                None
            } else {
                self.art(&t.id, t.tile, &t.pieces)?
            };
            let z = t
                .tile
                .attr_ci("Z")
                .map_or(Ok(0), |z| {
                    z.trim()
                        .parse::<i16>()
                        .map_err(|_| bad(format!("{}: Z={z}", t.id)))
                })?
                .checked_add(t.layer)
                .ok_or_else(|| bad(format!("{}: Z out of range", t.id)))?;
            // A knob rotation shows only while its tab is selected (Combat's
            // `Knob01`-`Knob03`, one place for three tabs, `combat.ini`
            // `[TabControl]`): one shows at a time, like the views.
            let shows_with = match t.role {
                NodeRole::View(v) | NodeRole::ButtonRow(v) | NodeRole::Knob(v) => Some(v),
                _ => None,
            };
            let inset = client_inset(t.tile, (&names, &sizes))?;
            let (cx, cy) = (client_box(x, inset[0])?, client_box(y, inset[1])?);
            let inner = outer_for(&names, cx, cy);
            let children = to_tile(
                self.walk(t.tile, Some(t.role), &inner, &own)?,
                (x, y),
                inset,
            )?;
            out.push(SkinNode {
                id: t.id,
                role: t.role,
                x,
                y,
                z,
                art,
                text: None,
                shows_with,
                triggers: t.triggers,
                children,
            });
        }
        Ok(out)
    }
}

/// `layout` with the side rail its components launch laid as its last
/// top-level tile (StealthX's `MenuButton` `<Popup SaveState="1">
/// MainMenuPopup</Popup>`, `components.xml:549-562`; the popup tile
/// `MainMenuPopup` `Component="MainMenuWindow"` `JustX="LEFT" X="0"
/// IndentTop="29" IndentBottom="28"`, `popups.xml:2`, is laid against the
/// window it launches from, resized as its window component says
/// (`MainMenuWindow` `BindY="true" ResizeY="100"`, `components.xml:815`:
/// the rail runs from 29 px below the window's top to 28 px above its
/// bottom, as the original shows). Other popups (the InfoView) are not
/// read here.
fn with_rail(layout: &Node, components: &[Node], by_name: &BTreeMap<String, &Node>) -> Node {
    // Only the popups this window's own tiles launch: the chat window's
    // `MenuButton` launches its own rail (`popups.xml:112-140`).
    let launched: Vec<String> = crate::import::xfire_xml::descendants(layout, "Tile")
        .into_iter()
        .filter_map(|t| t.attr_ci("Component"))
        .filter_map(|c| by_name.get(&c.to_ascii_lowercase()))
        .flat_map(|c| c.children_named("Popup"))
        .map(|p| p.text.trim().to_ascii_lowercase())
        .collect();
    let rail = |t: &&Node| {
        let none = Facts {
            has_art: false,
            class: None,
            children: &[],
        };
        let name = t.attr_ci("Name").unwrap_or_default();
        launched.contains(&name.to_ascii_lowercase())
            && role(name, t.attr_ci("Component"), None, &none) == Some(NodeRole::SideRail)
    };
    let mut laid = layout.clone();
    laid.children.extend(
        components
            .iter()
            .filter(|n| n.name.eq_ignore_ascii_case("Popups"))
            .flat_map(|p| p.children_named("Tile"))
            .filter(rail)
            .map(|t| {
                let mut t = t.clone();
                let window = t
                    .attr_ci("Component")
                    .and_then(|c| by_name.get(&c.to_ascii_lowercase()));
                for k in ["ResizeX", "ResizeY"] {
                    let declared = window.and_then(|w| w.attr_ci(k)).map(str::to_owned);
                    if let (None, Some(v)) = (t.attr_ci(k), declared) {
                        t.attrs.insert(k.to_owned(), v);
                    }
                }
                t
            }),
    );
    laid
}

/// Adds the component tree `layout` (the window's layout file) lays to
/// `built`, and makes its views node's box the content rectangle; `rows`
/// are the tiles `built`'s frame is laid from (a row stack's, else none:
/// the frame is the window component's own box).
///
/// # Errors
/// A tile the role table lacks, an anchor that does not resolve, a views
/// node that is not stretched on both axes, or what `piece` returns.
pub fn add(
    window: SkinWindow,
    layout: &Node,
    components: &[Node],
    piece: impl Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError>,
    rows: &[String],
    built: &mut (WindowLayout, BTreeMap<String, Vec<u8>>),
) -> Result<(), ImportError> {
    let mut by_name = BTreeMap::new();
    for c in components
        .iter()
        .flat_map(|n| crate::import::xfire_xml::descendants(n, "Component"))
    {
        if let Some(name) = c.attr_ci("Name") {
            by_name.entry(name.to_ascii_lowercase()).or_insert(c);
        }
    }
    let laid = with_rail(layout, components, &by_name);
    let popups = components
        .iter()
        .filter(|n| n.name.eq_ignore_ascii_case("Popups"))
        .flat_map(|p| p.children_named("Tile"))
        .filter_map(|t| t.attr_ci("Name"))
        .map(|n| n.trim().to_ascii_lowercase())
        .collect();
    let group_ids = by_name
        .values()
        .flat_map(|c| [c.attr_ci("GroupId"), c.attr_ci("ControlGroupId")])
        .flatten()
        .filter_map(|g| g.trim().parse().ok())
        .collect();
    let mut walk = Walk {
        window,
        by_name,
        piece: &piece,
        files: Files::new(),
        images: Images::new(),
        groups: Vec::new(),
        popups,
        group_ids,
    };
    let mut tree = SkinTree {
        outsets: Insets {
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
        },
        root: walk.walk(&laid, None, &Sibs::new(), &[])?,
    };
    tab_views(&mut tree.root);
    pages(&mut tree.root, &walk.groups);
    by_view(&mut tree.root);
    rows_by_view(&mut tree.root)?;
    qualify(&mut tree.root);
    let mut art = Art {
        window,
        images: &mut walk.images,
        files: &mut walk.files,
    };
    outdent::clip_window(&mut tree.root, &mut art)?;
    clip(&mut tree.root, (None, None), &mut art)?;
    let window = layout.attr_ci("Component").unwrap_or("MainWindow");
    let declared = walk.by_name.get(&window.to_ascii_lowercase()).copied();
    let pack_min = (
        limit_of(declared, "MinWidth").unwrap_or(0),
        limit_of(declared, "MinHeight").unwrap_or(0),
    );
    fit(&mut tree, pack_min, rows, &mut built.0)?;
    rail::lay(walk.window, &tree, (&laid, &walk.by_name), &piece, built)?;
    if tree
        .boxes(0, 0)
        .iter()
        .any(|(n, _)| n.role == NodeRole::Views)
    {
        built.0.content = views_content(&tree, &built.0).ok_or_else(|| {
            bad("the views node is not a box inset from the frame on all four sides")
        })?;
    }
    built.1.extend(walk.files);
    built.0.tree = Some(tree);
    Ok(())
}
