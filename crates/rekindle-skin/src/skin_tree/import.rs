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
use rekindle_types::dto::skin_tree::{Anchor, NodeArt, NodeRole, SkinNode, SkinTree, Span};

use super::roles::{role, Facts};
use crate::import::tree::Node;
use crate::import::xfire_xml::title::{component_image, reference, Decode};
use crate::import::ImportError;
use crate::shape;

/// The package images the tree adds, by path.
type Files = BTreeMap<String, Vec<u8>>;

fn bad(detail: impl Into<String>) -> ImportError {
    ImportError::Format {
        file: "skin tree".to_owned(),
        detail: detail.into(),
    }
}

/// One axis's attribute names.
struct Axis {
    pos: &'static str,
    just: &'static str,
    resize: &'static str,
    indent: [&'static str; 2],
    outdent: [&'static str; 2],
    far: &'static str,
    near: &'static str,
    size: &'static str,
    end_just: &'static str,
}

const X: Axis = Axis {
    pos: "X",
    just: "JustX",
    resize: "ResizeX",
    indent: ["IndentLeft", "IndentRight"],
    outdent: ["OutdentLeft", "OutdentRight"],
    far: "RIGHT",
    near: "LEFT",
    size: "WIDTH",
    end_just: "RIGHT",
};

const Y: Axis = Axis {
    pos: "Y",
    just: "JustY",
    resize: "ResizeY",
    indent: ["IndentTop", "IndentBottom"],
    outdent: ["OutdentTop", "OutdentBottom"],
    far: "BOTTOM",
    near: "TOP",
    size: "HEIGHT",
    end_just: "BOTTOM",
};

/// An edge's position: from the parent's start edge or from its end edge.
enum At {
    Start(i32),
    End(i32),
}

type Sibs = BTreeMap<String, (Span, Span)>;

fn span_of(sibs: &Sibs, name: &str, axis: &Axis) -> Option<Span> {
    sibs.get(&name.to_ascii_lowercase())
        .map(|s| if axis.pos == "X" { s.0 } else { s.1 })
}

/// A sum of signed terms, each a number or a sibling's `width`/`height`.
fn expr(v: Option<&str>, sibs: &Sibs, axis: &Axis) -> Result<i32, ImportError> {
    let Some(v) = v else { return Ok(0) };
    let mut total = 0i32;
    let mut rest = v.trim();
    let mut sign = 1;
    while !rest.is_empty() {
        let cut = rest[1..].find(['+', '-']).map_or(rest.len(), |i| i + 1);
        let (term, tail) = rest.split_at(cut);
        let term = term.trim();
        let (s, term) = match term.strip_prefix('-') {
            Some(t) => (-sign, t.trim()),
            None => (sign, term.trim_start_matches('+').trim()),
        };
        let value = match term.parse::<i32>() {
            Ok(n) => n,
            Err(_) => {
                let (name, field) = term
                    .split_once('.')
                    .ok_or_else(|| bad(format!("{v}: not a number")))?;
                if !field.eq_ignore_ascii_case(axis.size) {
                    return Err(bad(format!("{v}: {field} is not this axis's size")));
                }
                match span_of(sibs, name, axis) {
                    Some(Span::Fixed { size, .. }) => i32::from(size),
                    _ => return Err(bad(format!("{v}: {name} has no fixed size"))),
                }
            }
        };
        total += s * value;
        sign = 1;
        rest = tail.trim();
    }
    Ok(total)
}

/// `v` as a sibling's edge ± n, when it is one.
fn edge(v: &str, sibs: &Sibs, axis: &Axis) -> Result<Option<At>, ImportError> {
    let Some((name, e, n)) = reference(v) else {
        return Ok(None);
    };
    if e.eq_ignore_ascii_case(axis.size) {
        return Ok(None);
    }
    let far = e.eq_ignore_ascii_case(axis.far);
    if !far && !e.eq_ignore_ascii_case(axis.near) {
        return Err(bad(format!("{v}: {e} is not an edge of this axis")));
    }
    let s = span_of(sibs, name, axis)
        .ok_or_else(|| bad(format!("{v}: no sibling {name} before it")))?;
    Ok(Some(match (s, far) {
        (
            Span::Fixed {
                at: Anchor::Start(o),
                size,
            },
            true,
        ) => At::Start(i32::from(o) + i32::from(size) + n),
        (
            Span::Fixed {
                at: Anchor::Start(o),
                ..
            },
            false,
        ) => At::Start(i32::from(o) + n),
        (
            Span::Fixed {
                at: Anchor::End(o), ..
            },
            true,
        ) => At::End(i32::from(o) - n),
        (
            Span::Fixed {
                at: Anchor::End(o),
                size,
            },
            false,
        ) => At::End(i32::from(o) + i32::from(size) - n),
        (Span::Stretch { end, .. }, true) => At::End(i32::from(end) - n),
        (Span::Stretch { start, .. }, false) => At::Start(i32::from(start) + n),
        (
            Span::Fixed {
                at: Anchor::Centre(_),
                ..
            },
            _,
        ) => return Err(bad(format!("{v}: {name} is centred"))),
    }))
}

fn i16_of(v: i32) -> Result<i16, ImportError> {
    i16::try_from(v).map_err(|_| bad(format!("{v} px is out of range")))
}

fn axis(tile: &Node, a: &Axis, size: u16, sibs: &Sibs) -> Result<Span, ImportError> {
    let just = tile.attr_ci(a.just).map(str::trim).unwrap_or("");
    let at_end = just.eq_ignore_ascii_case(a.end_just);
    let centred = just.eq_ignore_ascii_case("CENTER") || just.eq_ignore_ascii_case("CENTRE");
    let raw = tile.attr_ci(a.pos);
    let pos = match raw {
        Some(v) => match edge(v, sibs, a)? {
            Some(at) => at,
            None => At::Start(expr(Some(v), sibs, a)?),
        },
        None => At::Start(0),
    };
    let ind = |k| expr(tile.attr_ci(k), sibs, a);
    let (ind_s, ind_e) = (ind(a.indent[0])?, ind(a.indent[1])?);
    let (out_s, out_e) = (ind(a.outdent[0])?, ind(a.outdent[1])?);
    if tile.attr_ci(a.resize).is_some_and(|r| r.trim() == "100") {
        let (start, end) = match (pos, at_end) {
            (At::Start(p), true) => (ind_s - out_s, p + ind_e - out_e),
            (At::Start(p), false) => (p + ind_s - out_s, ind_e - out_e),
            (At::End(_), _) => return Err(bad(format!("{}: a stretch from a far edge", a.pos))),
        };
        return Ok(Span::Stretch {
            start: i16_of(start)?,
            end: i16_of(end)?,
        });
    }
    let s = i32::from(size);
    let at = match (pos, at_end, centred) {
        (At::Start(p), _, true) => Anchor::Centre(i16_of(p)?),
        (At::Start(p), true, _) => Anchor::End(i16_of(p)?),
        (At::Start(p), false, _) if p < 0 => Anchor::End(i16_of(-p - s)?),
        (At::Start(p), false, _) => Anchor::Start(i16_of(p)?),
        (At::End(e), false, _) => Anchor::End(i16_of(e - s)?),
        (At::End(_), true, _) => return Err(bad(format!("{}: a far edge from a far edge", a.pos))),
    };
    Ok(Span::Fixed { at, size })
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

    fn walk(
        &mut self,
        node: &Node,
        parent: Option<NodeRole>,
    ) -> Result<Vec<SkinNode>, ImportError> {
        let mut sibs = Sibs::new();
        let mut out = Vec::new();
        for tile in node.children_named("Tile") {
            let component = tile.attr_ci("Component");
            let id = tile
                .attr_ci("Name")
                .or(component)
                .ok_or_else(|| bad("a tile with neither a name nor a component"))?
                .trim()
                .to_owned();
            let img = self.image(tile, "Normal")?;
            let size = |v: u32| u16::try_from(v).map_err(|_| bad(format!("{id}: art too large")));
            let (w, h) = match &img {
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
            let class = component
                .and_then(|c| self.by_name.get(&c.to_ascii_lowercase()))
                .and_then(|c| c.attr_ci("Class"));
            let facts = Facts {
                has_art: img.is_some(),
                class,
                children: &children,
            };
            let r = role(&id, component, parent, &facts).ok_or_else(|| {
                bad(format!(
                    "{id}: component {} has no role",
                    component.unwrap_or("-")
                ))
            })?;
            let x = axis(tile, &X, w, &sibs).map_err(|e| bad(format!("{id}: {e}")))?;
            let y = axis(tile, &Y, h, &sibs).map_err(|e| bad(format!("{id}: {e}")))?;
            sibs.insert(id.to_ascii_lowercase(), (x, y));
            let art = match (&img, r.interactive()) {
                (Some(i), true) => self.art(&id, tile, i)?,
                _ => None,
            };
            let z = tile.attr_ci("Z").map_or(Ok(0), |z| {
                z.trim()
                    .parse::<i16>()
                    .map_err(|_| bad(format!("{id}: Z={z}")))
            })?;
            let shows_with = match r {
                NodeRole::View(v) | NodeRole::ButtonRow(v) => Some(v),
                _ => None,
            };
            let kids = self.walk(tile, Some(r))?;
            out.push(SkinNode {
                id,
                role: r,
                x,
                y,
                z,
                art,
                text: None,
                shows_with,
                children: kids,
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
        root: walk.walk(layout, None)?,
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
