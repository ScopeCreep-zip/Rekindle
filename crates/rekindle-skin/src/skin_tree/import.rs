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
//! [`component_image`]). An anchor that does not resolve refuses the pack
//! (rule 2); a component the table lacks refuses it (rule 1) unless it is
//! classless art (Decor). The views node's box is the layout's content
//! rectangle.

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{Insets, RegionState, SkinWindow, TileAxis, WindowLayout};
use rekindle_types::dto::skin_tree::{NodeArt, NodeRole, NodeSpan, SkinNode, SkinTree};

use super::anchor::{axis, outer_for, Sibs, Sizes, X, Y};
use super::passes::{by_view, pages, qualify, rows_by_view, tab_views};
use super::roles::{role, Facts};
use crate::import::tree::Node;
use crate::import::xfire_xml::title::{component_image, Decode};
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

/// A tile read but not yet placed.
struct Read<'n> {
    tile: &'n Node,
    id: String,
    role: NodeRole,
    img: Option<RgbaImage>,
    wh: (u16, u16),
    max: (Option<u16>, Option<u16>),
    /// The component's `ControlGroupId` (INI tab controls that switch
    /// together).
    group: Option<String>,
}

struct Walk<'a> {
    window: SkinWindow,
    by_name: BTreeMap<String, &'a Node>,
    piece: Decode<'a>,
    files: Files,
    /// Each tab control's path of child indices and its `ControlGroupId`.
    groups: Vec<(Vec<usize>, String)>,
}

impl Walk<'_> {
    fn image(&self, tile: &Node, state: &str) -> Result<Option<RgbaImage>, ImportError> {
        match component_image(&self.by_name, self.piece, tile, state) {
            Err(ImportError::MissingFile { .. } | ImportError::UndefinedBitmap { .. }) => Ok(None),
            other => other,
        }
    }

    fn encode(&mut self, piece: &str, img: &RgbaImage) -> Result<Option<String>, ImportError> {
        let axes = (TileAxis::Start { offset: 0 }, TileAxis::Start { offset: 0 });
        Ok(
            shape::tile(self.window, piece, img, axes, true)?.map(|(t, png)| {
                self.files.insert(t.image.clone(), png);
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
            let normal = self.image(c, "Normal")?;
            let img = if normal.is_some() {
                normal
            } else {
                self.image(c, "Hover")?
            };
            let (cw, ch) = if let Some(i) = img {
                (
                    u16::try_from(i.width()).unwrap_or(u16::MAX),
                    u16::try_from(i.height()).unwrap_or(u16::MAX),
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

    /// An interactive node's normal, hover and down art at one size.
    fn art(
        &mut self,
        id: &str,
        tile: &Node,
        img: &RgbaImage,
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
        let Some(image) = self.encode(&format!("node-{slug}"), img)? else {
            return Ok(None);
        };
        let mut states = BTreeMap::new();
        for (state, set) in [
            (RegionState::Hover, "Hover"),
            (RegionState::Down, "Down"),
            (RegionState::Disabled, "Disabled"),
        ] {
            let Some(other) = self.image(tile, set)? else {
                continue;
            };
            if other.dimensions() == img.dimensions() {
                if let Some(path) = self.encode(&format!("node-{slug}--{}", state.key()), &other)? {
                    states.insert(state, path);
                }
            }
        }
        Ok(Some(NodeArt { image, states }))
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
        let img = self.image(tile, "Normal")?;
        let comp = component.and_then(|c| self.by_name.get(&c.to_ascii_lowercase()).copied());
        let size = |v: u32| u16::try_from(v).map_err(|_| bad(format!("{id}: art too large")));
        // A tile with no art is its component's declared size: Classic's
        // `ResizeUp` `MinHeight="5"` (`xfirecomponents.xml:121`), Symbiosis's
        // grippers `MinWidth="10" MinHeight="10"` (`Components/MainWindow.xml:62`).
        let wh = match &img {
            Some(i) => (size(i.width())?, size(i.height())?),
            // Art only on hover: the button's normal look is baked in the
            // art under it, and its hover image is its size (Total Chaos's
            // `TCLink`, `components.xml:507-519`).
            None => {
                if let Some(h) = self.image(tile, "Hover")? {
                    (size(h.width())?, size(h.height())?)
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
        let children: Vec<String> = tile
            .children_named("Tile")
            .filter_map(|t| {
                t.attr_ci("Name")
                    .or(t.attr_ci("Component"))
                    .map(str::to_owned)
            })
            .collect();
        let facts = Facts {
            has_art: img.is_some(),
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
        Ok(Read {
            tile,
            id,
            role,
            img,
            wh,
            max,
            group,
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
            let art = match (&t.img, t.role.interactive()) {
                (Some(i), true) => self.art(&t.id, t.tile, i)?,
                _ => None,
            };
            let z = t.tile.attr_ci("Z").map_or(Ok(0), |z| {
                z.trim()
                    .parse::<i16>()
                    .map_err(|_| bad(format!("{}: Z={z}", t.id)))
            })?;
            // A knob rotation shows only while its tab is selected (Combat's
            // `Knob01`-`Knob03`, one place for three tabs, `combat.ini`
            // `[TabControl]`): one shows at a time, like the views.
            let shows_with = match t.role {
                NodeRole::View(v) | NodeRole::ButtonRow(v) | NodeRole::Knob(v) => Some(v),
                _ => None,
            };
            let inner = outer_for(&names, x, y);
            let children = self.walk(t.tile, Some(t.role), &inner, &own)?;
            out.push(SkinNode {
                id: t.id,
                role: t.role,
                x,
                y,
                z,
                art,
                text: None,
                shows_with,
                children,
            });
        }
        Ok(out)
    }
}

/// The largest window the fit is searched in, px.
const MOST: i32 = 4096;

/// Whether every node of `tree` laid in a `w × h` window has a box (no
/// negative length) and, when it has a size on an axis, lies inside.
fn fits(tree: &SkinTree, w: i32, h: i32, x_axis: bool) -> bool {
    tree.boxes(w, h).iter().all(|(_, b)| {
        let (at, len, win) = if x_axis {
            (b[0], b[2], w)
        } else {
            (b[1], b[3], h)
        };
        len >= 0 && (len == 0 || (at >= 0 && at + len <= win))
    })
}

/// The pack's overhang: how far its nodes reach outside its own window
/// (laid at the largest size, where only offsets lay a node outside).
fn overhang(tree: &SkinTree) -> Insets {
    let boxes = tree.boxes(MOST, MOST);
    let reach = |f: &dyn Fn(&[i32; 4]) -> i32| {
        boxes
            .iter()
            .filter(|(_, b)| b[2] > 0 && b[3] > 0)
            .map(|(_, b)| f(b))
            .max()
            .map_or(0, |v| u16::try_from(v.max(0)).unwrap_or(u16::MAX))
    };
    Insets {
        left: reach(&|b| -b[0]),
        top: reach(&|b| -b[1]),
        right: reach(&|b| b[0] + b[2] - MOST),
        bottom: reach(&|b| b[1] + b[3] - MOST),
    }
}

/// Sizes `built`'s window to the tree (plan SK2 decision 5): the tree's
/// overhang joins the layout's outsets, and the smallest window is the
/// pack's declared `MinWidth`/`MinHeight` (Combat `combat.ini:3-4`, Total
/// Chaos `components.xml:4-5`), grown until every node lies inside it.
fn fit(
    tree: &mut SkinTree,
    pack_min: (u16, u16),
    layout: &mut WindowLayout,
) -> Result<(), ImportError> {
    let over = overhang(tree);
    tree.outsets = over;
    let o = &mut layout.outsets;
    o.left = o.left.saturating_add(over.left);
    o.top = o.top.saturating_add(over.top);
    o.right = o.right.saturating_add(over.right);
    o.bottom = o.bottom.saturating_add(over.bottom);
    let (ow, oh) = (
        i32::from(o.left) + i32::from(o.right),
        i32::from(o.top) + i32::from(o.bottom),
    );
    let (tw, th) = (
        i32::from(over.left) + i32::from(over.right),
        i32::from(over.top) + i32::from(over.bottom),
    );
    let w0 = (i32::from(layout.min_width) + ow).max(i32::from(pack_min.0) + tw);
    let h0 = (i32::from(layout.min_height) + oh).max(i32::from(pack_min.1) + th);
    let w = (w0..=MOST)
        .find(|&w| fits(tree, w, MOST, true))
        .ok_or_else(|| bad("the tree fits no window width"))?;
    let h = (h0..=MOST)
        .find(|&h| fits(tree, w, h, false))
        .ok_or_else(|| bad("the tree fits no window height"))?;
    let frame = |v: i32| u16::try_from(v).map_err(|_| bad("window too large"));
    layout.min_width = frame(w - ow)?;
    layout.min_height = frame(h - oh)?;
    Ok(())
}

/// The views node's box as insets from the frame rectangle: the same at
/// the smallest window and one 100 px larger each way, else `None`.
#[must_use]
pub fn views_content(tree: &SkinTree, layout: &WindowLayout) -> Option<Insets> {
    let o = layout.outsets;
    let w = i32::from(layout.min_width) + i32::from(o.left) + i32::from(o.right);
    let h = i32::from(layout.min_height) + i32::from(o.top) + i32::from(o.bottom);
    let at = |w: i32, h: i32| {
        tree.boxes(w, h)
            .into_iter()
            .find(|(n, _)| n.role == NodeRole::Views)
            .map(|(_, b)| [b[0], b[1], w - b[0] - b[2], h - b[1] - b[3]])
    };
    let a = at(w, h)?;
    (at(w + 100, h + 100)? == a).then_some(())?;
    let side = |v: i32, out: u16| u16::try_from(v - i32::from(out)).ok();
    Some(Insets {
        left: side(a[0], o.left)?,
        top: side(a[1], o.top)?,
        right: side(a[2], o.right)?,
        bottom: side(a[3], o.bottom)?,
    })
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
/// `built`, and makes its views node's box the content rectangle.
///
/// # Errors
/// A tile the role table lacks, an anchor that does not resolve, a views
/// node that is not stretched on both axes, or what `piece` returns.
pub fn add(
    window: SkinWindow,
    layout: &Node,
    components: &[Node],
    piece: impl Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError>,
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
    let mut walk = Walk {
        window,
        by_name,
        piece: &piece,
        files: Files::new(),
        groups: Vec::new(),
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
    let window = layout.attr_ci("Component").unwrap_or("MainWindow");
    let declared = walk.by_name.get(&window.to_ascii_lowercase()).copied();
    let pack_min = (
        limit_of(declared, "MinWidth").unwrap_or(0),
        limit_of(declared, "MinHeight").unwrap_or(0),
    );
    fit(&mut tree, pack_min, &mut built.0)?;
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
