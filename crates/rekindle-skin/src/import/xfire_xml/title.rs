//! The buddy list's title strip as the pack lays it (plan SK1-S1, lead
//! decision (1); research `sk1r/research.md` §1a): the title buttons, the
//! logo and the window-name field, each at its window position resolved
//! through its parent tiles. Symbiosis `MainWindow.xml:47-58` lays
//! `InfoBar` (`Y="5" ResizeX="100" IndentLeft="5" IndentRight="18"`, art
//! 46 px) holding `XfireLogo` (`X="4" Y="5"`) and `ButtonMinimize` /
//! `ButtonMaximize` / `ButtonClose` (`JustX="Right" X="49"/"25"/"5"
//! Y="3"`), so the close button's right edge is 18 + 5 = 23 px from the
//! window's and its top 5 + 3 = 8 px. Onyx lays its buttons in
//! `TopButtonBar` (`JustX="RIGHT" X="7" Y="5"`) and its name field
//! `Title` at `X="Logo.RIGHT+2"`: a sibling's `RIGHT`, `TOP` and `BOTTOM`
//! (± n) resolve when the sibling's edge does.
//!
//! A part whose position does not resolve (a centred tile, an edge of a
//! tile of unknown size) is left out, and the strip is laid only when its
//! close button resolves (the helper refuses a strip without one). Each
//! button carries its component's Normal image and its Hover and Down
//! images (a state of another size is dropped, as [`super::controls`]
//! does), drawn at their own size.

use std::collections::BTreeMap;

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{RegionState, SkinWindow, TileAxis, WindowLayout};
use rekindle_types::dto::skin_title::{TitleLayout, TitlePart, TitleRole, TitleText};

use super::super::tree::Node;
use super::super::ImportError;
use super::controls::button_slot;
use super::descendants;
use super::tiles::normal_set;
use crate::shape;

const LOGOS: [&str; 3] = ["XfireLogo", "Logo", "LogoX"];
const NAMES: [&str; 3] = ["Title", "UserNameField", "UserName"];

/// A tile's resolved box in window px: `left`/`right` the distance of its
/// left/right edge from the window's left/right edge.
#[derive(Debug, Clone, Copy, Default)]
struct Placed {
    left: Option<i32>,
    right: Option<i32>,
    top: Option<i32>,
    w: Option<i32>,
    h: Option<i32>,
}

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn int(v: Option<&str>) -> Option<i32> {
    v.map_or(Some(0), |v| v.trim().parse().ok())
}

/// `Sibling.EDGE(+|-)n`: the sibling, the edge and n.
fn reference(v: &str) -> Option<(&str, &str, i32)> {
    let (name, rest) = v.trim().split_once('.')?;
    let cut = rest.find(['+', '-']).unwrap_or(rest.len());
    let (edge, n) = rest.split_at(cut);
    let n = match n.chars().next() {
        None => 0,
        Some('-') => -n[1..].trim().parse::<i32>().ok()?,
        Some(_) => n[1..].trim().parse().ok()?,
    };
    Some((name, edge, n))
}

/// `v` as a reference to an already placed sibling.
fn sibling<'v>(v: &'v str, sibs: &BTreeMap<String, Placed>) -> Option<(Placed, &'v str, i32)> {
    let (s, e, n) = reference(v)?;
    Some((*sibs.get(&s.to_ascii_lowercase())?, e, n))
}

fn place(
    tile: &Node,
    parent: Placed,
    sibs: &BTreeMap<String, Placed>,
    size: Option<(i32, i32)>,
) -> Placed {
    let mut out = Placed {
        w: size.map(|wh| wh.0),
        h: size.map(|wh| wh.1),
        ..Placed::default()
    };
    let at_x = tile.attr_ci("X");
    let sib = |v| sibling(v, sibs);
    if tile.attr_ci("ResizeX") == Some("100") {
        out.left = parent
            .left
            .zip(int(tile.attr_ci("IndentLeft")))
            .map(|(base, off)| base + off);
        out.right = parent
            .right
            .zip(int(tile.attr_ci("IndentRight")))
            .map(|(base, off)| base + off);
        out.w = None;
    } else if tile.attr_ci("JustX").is_some_and(|just| eq(just, "RIGHT")) {
        out.right = parent.right.zip(int(at_x)).map(|(base, off)| base + off);
    } else if tile.attr_ci("JustX").is_some_and(|just| !eq(just, "LEFT")) {
        // Centred: its edges depend on the window's width.
        out.left = None;
    } else if let Some((s, _, n)) = at_x.and_then(sib).filter(|(_, e, _)| eq(e, "RIGHT")) {
        match (s.left.zip(s.w), s.right, out.w) {
            (Some((left, width)), _, _) => out.left = Some(left + width + n),
            (None, Some(right), Some(width)) => out.right = Some(right - n - width),
            _ => {}
        }
    } else {
        out.left = parent.left.zip(int(at_x)).map(|(base, off)| base + off);
    }
    let at_y = tile.attr_ci("Y");
    out.top = match at_y.and_then(sib) {
        Some((s, edge, n)) if eq(edge, "TOP") => s.top.map(|top| top + n),
        Some((s, edge, n)) if eq(edge, "BOTTOM") => {
            s.top.zip(s.h).map(|(top, height)| top + height + n)
        }
        Some(_) => None,
        None if tile.attr_ci("JustY").is_some_and(|just| !eq(just, "TOP")) => None,
        None => parent.top.zip(int(at_y)).map(|(base, off)| base + off),
    };
    out
}

fn role(tile: &Node) -> Option<TitleRole> {
    let names = [tile.attr_ci("Name"), tile.attr_ci("Component")];
    names
        .into_iter()
        .flatten()
        .find_map(|n| match button_slot(n) {
            Some("titlebar.btn.minimize") => Some(TitleRole::Minimize),
            Some("titlebar.btn.maximize") => Some(TitleRole::Maximize),
            Some("titlebar.btn.close") => Some(TitleRole::Close),
            _ if LOGOS.iter().any(|l| eq(l, n)) => Some(TitleRole::Logo),
            _ if NAMES.iter().any(|l| eq(l, n)) => Some(TitleRole::Name),
            _ => None,
        })
}

/// The first `<Text>` face under `component` or a child tile's component.
fn face(
    tile: &Node,
    component: Option<&Node>,
    by_name: &BTreeMap<String, &Node>,
) -> Option<TitleText> {
    let own = component.and_then(|c| c.children_named("Text").next());
    let child = || {
        tile.children_named("Tile").find_map(|t| {
            by_name
                .get(&t.attr_ci("Component")?.to_ascii_lowercase())?
                .children_named("Text")
                .next()
        })
    };
    let t = own.or_else(child)?;
    let font: String = t
        .attr_ci("Font")?
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '-')
        .collect();
    Some(TitleText {
        font: font.trim().to_owned(),
        size: t.attr_ci("Size")?.trim().parse().ok()?,
        bold: t.attr_ci("Bold").is_some_and(|b| eq(b, "true")),
    })
}

/// The package images a part adds, by path.
type Files = Vec<(String, Vec<u8>)>;

type Decode<'a> = &'a dyn Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError>;

struct Walk<'a> {
    window: SkinWindow,
    by_name: BTreeMap<String, &'a Node>,
    piece: Decode<'a>,
    parts: Vec<(TitlePart, Files)>,
    bars: Vec<i32>,
}

impl Walk<'_> {
    fn image(&self, tile: &Node) -> Result<Option<RgbaImage>, ImportError> {
        self.image_of(tile, "Normal")
    }

    /// `tile`'s component's `state` set image (its `Middle`, else its first).
    fn image_of(&self, tile: &Node, state: &str) -> Result<Option<RgbaImage>, ImportError> {
        let component = tile
            .attr_ci("Component")
            .and_then(|c| self.by_name.get(&c.to_ascii_lowercase()));
        let set = component.and_then(|c| {
            if eq(state, "Normal") {
                normal_set(c)
            } else {
                c.children_named("Images")
                    .find(|i| i.attr_ci("State").is_some_and(|s| eq(s, state)))
            }
        });
        let Some(set) = set else {
            return Ok(None);
        };
        let img = set
            .children_named("Image")
            .find(|i| i.attr_ci("Type").is_some_and(|t| eq(t, "Middle")))
            .or_else(|| set.children_named("Image").next());
        img.map(|i| (self.piece)(&i.text, set.attr_ci("Color")))
            .transpose()
    }

    /// `img` as the package image `piece`; `None` when wholly transparent.
    fn encode(
        &self,
        piece: &str,
        img: &RgbaImage,
    ) -> Result<Option<(String, Vec<u8>)>, ImportError> {
        let axes = (TileAxis::Start { offset: 0 }, TileAxis::Start { offset: 0 });
        Ok(shape::tile(self.window, piece, img, axes, true)?.map(|(t, png)| (t.image, png)))
    }

    fn part(
        &mut self,
        tile: &Node,
        role: TitleRole,
        p: Placed,
        img: Option<&RgbaImage>,
    ) -> Result<(), ImportError> {
        let Some(top) = p.top.and_then(|t| u16::try_from(t).ok()) else {
            return Ok(());
        };
        let u = |v: i32| u16::try_from(v).ok();
        let x = match (p.left.and_then(u), p.right.and_then(u), role) {
            (Some(start), Some(end), TitleRole::Name) => TileAxis::Repeat { start, end },
            (Some(offset), None, _) => TileAxis::Start { offset },
            (None, Some(offset), _) => TileAxis::End { offset },
            _ => return Ok(()),
        };
        let (mut width, mut height) = (p.w.and_then(u).unwrap_or(0), p.h.and_then(u).unwrap_or(0));
        let (mut image, mut files, mut text) = (None, Files::new(), None);
        let mut states = BTreeMap::new();
        if role != TitleRole::Name {
            {
                let Some(img) = img else { return Ok(()) };
                let Some((path, bytes)) = self.encode(&format!("title-{}", role.key()), img)?
                else {
                    return Ok(());
                };
                image = Some(path.clone());
                files.push((path, bytes));
                for (state, set) in [(RegionState::Hover, "Hover"), (RegionState::Down, "Down")] {
                    let Some(other) = self.image_of(tile, set)? else {
                        continue;
                    };
                    let name = format!("title-{}--{}", role.key(), state.key());
                    if other.dimensions() == img.dimensions() && role.is_button() {
                        if let Some((path, bytes)) = self.encode(&name, &other)? {
                            states.insert(state, path.clone());
                            files.push((path, bytes));
                        }
                    }
                }
            }
        }
        if role == TitleRole::Name {
            {
                let component = tile
                    .attr_ci("Component")
                    .and_then(|c| self.by_name.get(&c.to_ascii_lowercase()).copied());
                let Some(f) = face(tile, component, &self.by_name) else {
                    return Ok(());
                };
                if matches!(x, TileAxis::Repeat { .. }) {
                    width = 0;
                }
                if height == 0 {
                    height = u16::from(f.size) + 4;
                }
                text = Some(f);
            }
        }
        if self.parts.iter().any(|(q, _)| q.role == role) {
            return Ok(());
        }
        self.parts.push((
            TitlePart {
                role,
                x,
                y: top,
                width,
                height,
                image,
                states,
                text,
            },
            files,
        ));
        Ok(())
    }

    fn walk(&mut self, node: &Node, parent: Placed) -> Result<(), ImportError> {
        let mut sibs: BTreeMap<String, Placed> = BTreeMap::new();
        for tile in node.children_named("Tile") {
            let img = self.image(tile)?;
            let size = img.as_ref().map(|i| {
                (
                    i32::try_from(i.width()).unwrap_or(i32::MAX),
                    i32::try_from(i.height()).unwrap_or(i32::MAX),
                )
            });
            let p = place(tile, parent, &sibs, size);
            if let Some(r) = role(tile) {
                self.part(tile, r, p, img.as_ref())?;
                if r.is_button() {
                    self.bars
                        .extend(parent.top.zip(parent.h).map(|(t, h)| t + h));
                }
            }
            if let Some(n) = tile.attr_ci("Name") {
                sibs.insert(n.to_ascii_lowercase(), p);
            }
            let mut inner = p;
            if inner.h.is_none() {
                inner.h = self
                    .image(tile)?
                    .map(|i| i32::try_from(i.height()).unwrap_or(i32::MAX));
            }
            self.walk(tile, inner)?;
        }
        Ok(())
    }
}

/// Adds the strip `layout` (the window's layout file) lays to `built`.
///
/// # Errors
/// What `piece` returns, or a logo that does not encode.
pub fn add(
    window: SkinWindow,
    layout: &Node,
    components: &[Node],
    piece: impl Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError>,
    built: &mut (WindowLayout, BTreeMap<String, Vec<u8>>),
) -> Result<(), ImportError> {
    let mut by_name = BTreeMap::new();
    for c in components.iter().flat_map(|n| descendants(n, "Component")) {
        if let Some(name) = c.attr_ci("Name") {
            by_name.entry(name.to_ascii_lowercase()).or_insert(c);
        }
    }
    let root = Placed {
        left: Some(0),
        right: Some(0),
        top: Some(0),
        ..Placed::default()
    };
    let mut walk = Walk {
        window,
        by_name,
        piece: &piece,
        parts: Vec::new(),
        bars: Vec::new(),
    };
    walk.walk(layout, root)?;
    let buttons_bottom = walk
        .parts
        .iter()
        .filter(|(p, _)| p.role.is_button())
        .map(|(p, _)| i32::from(p.y) + i32::from(p.height))
        .max();
    let Some(strip) = buttons_bottom.map(|b| walk.bars.iter().copied().fold(b, i32::max)) else {
        return Ok(());
    };
    // A logo or name field below the strip (Symbiosis's user name under the
    // menu) is no part of it.
    walk.parts.retain(|(p, _)| i32::from(p.y) < strip);
    if !walk.parts.iter().any(|(p, _)| p.role == TitleRole::Close) {
        return Ok(());
    }
    let bottom = walk
        .parts
        .iter()
        .map(|(p, _)| i32::from(p.y) + i32::from(p.height))
        .fold(strip, i32::max);
    let Ok(bottom) = u16::try_from(bottom) else {
        return Ok(());
    };
    let mut parts = Vec::new();
    for (p, files) in walk.parts {
        built.1.extend(files);
        parts.push(p);
    }
    built.0.title = Some(TitleLayout { bottom, parts });
    Ok(())
}
