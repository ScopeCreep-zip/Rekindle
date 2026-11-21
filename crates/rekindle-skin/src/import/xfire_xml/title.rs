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
//! Classic v3 (`xfiremainwindow.xml:2-12`) lays `MainTitleBar`
//! `IndentLeft="14" IndentRight="13+14"` (an indent may be a sum), its
//! `UserNameField` `X="XfireLogo.RIGHT+6" ResizeX="100" IndentRight="48"`
//! (a stretching tile starts at a sibling's edge) and `CloseBox` at
//! `UserNameField.RIGHT+1` (the edge of a stretching sibling is its right
//! indent). A stretching tile's `X` adds to its `IndentLeft`, or, with
//! `JustX="Right"` (Space Cowboy's `MainTitleBar` `X="85"`), to its
//! `IndentRight`.
//!
//! A left-placed tile inside a right-placed parent of known width (Alloy's
//! `[TopButtonBar]` `JustX=RIGHT X=7`, its `CloseBox` at `X=37`) is placed
//! from the parent's right edge.
//!
//! The name field takes its text colour from the pack's colour group
//! (`Color="UserNameCG"`), its face from its own `<Text>` or its child
//! text tile's (whose `X` / `IndentRight` are the text's inset), and its
//! component's `Left` / `Middle` / `Right` images as its 3-slice art.
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
use rekindle_types::dto::skin_title::{TitleLayout, TitlePart, TitleRole, TitleSlices, TitleText};
use rekindle_types::dto::skin_tree::PieceSlot;

use super::super::tree::Node;
use super::super::ImportError;
use super::controls::button_slot;
use super::descendants;
use super::tiles::normal_set;
use super::ColourRow;
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

/// A numeric attribute: absent is 0; `13+14` is a sum (Classic v3).
fn int(v: Option<&str>) -> Option<i32> {
    v.map_or(Some(0), |v| {
        v.split('+').map(|t| t.trim().parse::<i32>().ok()).sum()
    })
}

fn add_opt(a: Option<i32>, b: Option<i32>) -> Option<i32> {
    a.zip(b).map(|(a, b)| a + b)
}

/// `Sibling.EDGE(+|-)n`: the sibling, the edge and n.
pub(crate) fn reference(v: &str) -> Option<(&str, &str, i32)> {
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
    let right_just = tile.attr_ci("JustX").is_some_and(|just| eq(just, "RIGHT"));
    if tile.attr_ci("ResizeX") == Some("100") {
        let start = match at_x.and_then(sib).filter(|(_, e, _)| eq(e, "RIGHT")) {
            Some((s, _, n)) => s.left.zip(s.w).map(|(left, width)| left + width + n),
            None if right_just => parent.left,
            None => add_opt(parent.left, int(at_x)),
        };
        let end = if right_just {
            add_opt(parent.right, int(at_x))
        } else {
            parent.right
        };
        out.left = add_opt(start, int(tile.attr_ci("IndentLeft")));
        out.right = add_opt(end, int(tile.attr_ci("IndentRight")));
        out.w = None;
    } else if right_just {
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
        out.left = add_opt(parent.left, int(at_x));
        if out.left.is_none() {
            // A left-placed tile in a right-placed parent of known width
            // (Alloy's `TopButtonBar` `JustX=RIGHT`, its buttons at
            // `X=1/19/37`): its right edge from the parent's.
            out.right = match (parent.right, parent.w, int(at_x), out.w) {
                (Some(right), Some(pw), Some(x), Some(w)) => Some(right + pw - x - w),
                _ => None,
            };
        }
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
    colours: &[ColourRow],
) -> Option<TitleText> {
    let own = component
        .and_then(|c| c.children_named("Text").next())
        .map(|t| (t, [0, 0]));
    let child = || {
        tile.children_named("Tile").find_map(|t| {
            let text = by_name
                .get(&t.attr_ci("Component")?.to_ascii_lowercase())?
                .children_named("Text")
                .next()?;
            let inset = |a| int(t.attr_ci(a)).and_then(|v| u16::try_from(v).ok());
            Some((text, [inset("X")?, inset("IndentRight")?]))
        })
    };
    let (t, inset) = own.or_else(child)?;
    let colour = t
        .attr_ci("Color")
        .and_then(|c| colours.iter().find(|r| eq(&r.name, c.trim())))
        .and_then(|r| {
            let [red, green, blue] = r.value.map(|v| u8::try_from(v).ok());
            Some(format!("#{:02x}{:02x}{:02x}", red?, green?, blue?))
        });
    let font: String = t
        .attr_ci("Font")?
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == ' ' || *c == '-')
        .collect();
    Some(TitleText {
        font: font.trim().to_owned(),
        size: t.attr_ci("Size")?.trim().parse().ok()?,
        bold: t.attr_ci("Bold").is_some_and(|b| eq(b, "true")),
        colour,
        inset,
    })
}

/// The package images a part adds, by path.
type Files = Vec<(String, Vec<u8>)>;

pub(crate) type Decode<'a> = &'a dyn Fn(&str, Option<&str>) -> Result<RgbaImage, ImportError>;

struct Walk<'a> {
    window: SkinWindow,
    by_name: BTreeMap<String, &'a Node>,
    piece: Decode<'a>,
    colours: &'a [ColourRow],
    parts: Vec<(TitlePart, Files)>,
    bars: Vec<i32>,
}

impl Walk<'_> {
    fn image(&self, tile: &Node) -> Result<Option<RgbaImage>, ImportError> {
        self.image_of(tile, "Normal")
    }

    /// `tile`'s component's `state` set image (its `Middle`, else its first).
    fn image_of(&self, tile: &Node, state: &str) -> Result<Option<RgbaImage>, ImportError> {
        component_image(&self.by_name, self.piece, tile, state)
    }

    /// The name field's 3-slice art: its component's normal `Left`,
    /// `Middle` and `Right` images, one height, each encoded.
    fn slices(&self, tile: &Node) -> Result<Option<(TitleSlices, u16, Files)>, ImportError> {
        let set = tile
            .attr_ci("Component")
            .and_then(|c| self.by_name.get(&c.to_ascii_lowercase()))
            .and_then(|c| normal_set(c));
        let Some(set) = set else { return Ok(None) };
        let mut out = Vec::new();
        for ty in ["Left", "Middle", "Right"] {
            let Some(i) = set
                .children_named("Image")
                .find(|i| i.attr_ci("Type").is_some_and(|t| eq(t, ty)))
            else {
                return Ok(None);
            };
            let img = (self.piece)(&i.text, set.attr_ci("Color"))?;
            out.push(img);
        }
        let height = out[0].height();
        if out.iter().any(|i| i.height() != height) {
            return Ok(None);
        }
        let mut files = Files::new();
        let mut enc = |img: &RgbaImage, ty: &str| -> Result<Option<String>, ImportError> {
            let Some((path, bytes)) = self.encode(&format!("title-name--{ty}"), img)? else {
                return Ok(None);
            };
            files.push((path.clone(), bytes));
            Ok(Some(path))
        };
        let (Some(left), Some(middle), Some(right)) = (
            enc(&out[0], "left")?,
            enc(&out[1], "middle")?,
            enc(&out[2], "right")?,
        ) else {
            return Ok(None);
        };
        let Ok(height) = u16::try_from(height) else {
            return Ok(None);
        };
        Ok(Some((
            TitleSlices {
                left,
                middle,
                right,
            },
            height,
            files,
        )))
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
        let mut slices = None;
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
                let Some(f) = face(tile, component, &self.by_name, self.colours) else {
                    return Ok(());
                };
                if let Some((art, h, art_files)) = self.slices(tile)? {
                    height = h;
                    slices = Some(art);
                    files.extend(art_files);
                }
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
                slices,
            },
            files,
        ));
        Ok(())
    }

    fn walk(&mut self, node: &Node, parent: Placed) -> Result<(), ImportError> {
        let mut sibs: BTreeMap<String, Placed> = BTreeMap::new();
        for tile in node.children_named("Tile") {
            // A tile whose art the tree lacks (Classic's `%partnergraphic%`
            // ad, Space Cowboy's undefined `MENUBARNORMAL`) has no known
            // size: its edges do not resolve (SK0: an undefined bitmap lays
            // nothing).
            let img = match self.image(tile) {
                Err(ImportError::MissingFile { .. } | ImportError::UndefinedBitmap { .. }) => None,
                other => other?,
            };
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
                inner.h = img
                    .as_ref()
                    .map(|i| i32::try_from(i.height()).unwrap_or(i32::MAX));
            }
            self.walk(tile, inner)?;
        }
        Ok(())
    }
}

/// `tile`'s component's `state` set image (its `Middle`, else its first);
/// `by_name` holds the components by lower-case name.
///
/// # Errors
/// What `piece` returns.
pub(crate) fn component_image(
    by_name: &BTreeMap<String, &Node>,
    piece: Decode<'_>,
    tile: &Node,
    state: &str,
) -> Result<Option<RgbaImage>, ImportError> {
    let component = tile
        .attr_ci("Component")
        .and_then(|c| by_name.get(&c.to_ascii_lowercase()));
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
    img.map(|i| piece(&i.text, set.attr_ci("Color")))
        .transpose()
}

/// `tile`'s component's `state` set as its frame pieces (spec A7: the
/// engine resolves each piece type per state, `Xfire.exe` 0x2bbb18 "No
/// images found for type %ws of state %ws of %ws"): each `Image Type=` the
/// piece table names, the first of a type; a type outside the table
/// (Total Chaos's `center`) and a piece whose bitmap is not in the pack
/// (spec A8, SK0: an undefined bitmap lays nothing) draw nothing.
///
/// # Errors
/// What `piece` returns for a bitmap that is there but does not decode.
pub(crate) fn component_pieces(
    by_name: &BTreeMap<String, &Node>,
    piece: Decode<'_>,
    tile: &Node,
    state: &str,
) -> Result<Vec<(PieceSlot, RgbaImage)>, ImportError> {
    let component = tile
        .attr_ci("Component")
        .and_then(|c| by_name.get(&c.to_ascii_lowercase()));
    let set = component.and_then(|c| {
        if eq(state, "Normal") {
            normal_set(c)
        } else {
            c.children_named("Images")
                .find(|i| i.attr_ci("State").is_some_and(|s| eq(s, state)))
        }
    });
    let mut out: Vec<(PieceSlot, RgbaImage)> = Vec::new();
    for i in set.iter().flat_map(|s| s.children_named("Image")) {
        let Some(slot) = i.attr_ci("Type").and_then(PieceSlot::of_type) else {
            continue;
        };
        if out.iter().any(|(s, _)| *s == slot) {
            continue;
        }
        match piece(&i.text, set.and_then(|s| s.attr_ci("Color"))) {
            Ok(img) => out.push((slot, img)),
            Err(ImportError::MissingFile { .. } | ImportError::UndefinedBitmap { .. }) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(out)
}

/// Adds the strip `layout` (the window's layout file) lays to `built`.
///
/// # Errors
/// What `piece` returns, or a logo that does not encode.
pub fn add(
    window: SkinWindow,
    layout: &Node,
    components: &[Node],
    colours: &[ColourRow],
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
        colours,
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

#[cfg(test)]
#[path = "title_tests.rs"]
mod tests;
