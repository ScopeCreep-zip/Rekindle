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

/// A component's `MaxWidth`/`MaxHeight`.
fn max_of(component: Option<&Node>, key: &str) -> Option<u16> {
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
}

struct Walk<'a> {
    window: SkinWindow,
    by_name: BTreeMap<String, &'a Node>,
    piece: Decode<'a>,
    files: Files,
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
        let size = |v: u32| u16::try_from(v).map_err(|_| bad(format!("{id}: art too large")));
        let wh = match &img {
            Some(i) => (size(i.width())?, size(i.height())?),
            None => (0, 0),
        };
        let children: Vec<String> = tile
            .children_named("Tile")
            .filter_map(|t| {
                t.attr_ci("Name")
                    .or(t.attr_ci("Component"))
                    .map(str::to_owned)
            })
            .collect();
        let comp = component.and_then(|c| self.by_name.get(&c.to_ascii_lowercase()).copied());
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
        let max = (max_of(comp, "MaxWidth"), max_of(comp, "MaxHeight"));
        Ok(Read {
            tile,
            id,
            role,
            img,
            wh,
            max,
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
        for (t, xy) in tiles.into_iter().zip(placed) {
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
            let shows_with = match t.role {
                NodeRole::View(v) | NodeRole::ButtonRow(v) => Some(v),
                _ => None,
            };
            let inner = outer_for(&names, x, y);
            let children = self.walk(t.tile, Some(t.role), &inner)?;
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
    let mut walk = Walk {
        window,
        by_name,
        piece: &piece,
        files: Files::new(),
    };
    let tree = SkinTree {
        root: walk.walk(layout, None, &Sibs::new())?,
    };
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
