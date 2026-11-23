//! The words of a menu title (plan SK4b, the contract with the shell): the
//! Host serves every `MenuTitle` node's label, so the shell draws only what
//! the Host names (lead decision 7: the file menu is "Rekindle").
//!
//! The pack's word for a menu is either baked into art (Symbiosis's 47 x 20
//! `node-xfiremenu-topleft.png` reads "Xfire", Alloy's `node-filemenu.png`
//! "XFIRE", XGlow's "File", Total Chaos's "XFIRE" in its `MenuBg` under the
//! title) or drawn by the engine from its strings (Xfire Classic, Combat,
//! StealthX, ...: the title has no art). The rule, from the one table of
//! title words ([`EngineMenu::title`]):
//!
//! - the art bakes a word and the Host renames the menu: `{text, cover}`,
//!   the cover the title's usable box (plan SK5, [`usable`]): the word's
//!   box grown to the art's edge or its drawn frame, so the shell fills it
//!   with the art's ground and draws Rekindle's name in all of it (SK4b's
//!   cover was the word's box alone, 22 px on Symbiosis, and "Rekindle"
//!   showed "Re..."). Served at the window's minimum: Symbiosis
//!   `[0, 0, 45, 20]` (no frame: the whole 45 x 20 title), Alloy
//!   `[2, 0, 44, 15]` (the transparent columns beside its "E" read as
//!   frame in the rows the "E" fills), Total Chaos `[15, 0, 47, 18]` (its gauge art left
//!   of x 15 and right of x 61 is frame), XGlow `[1, 0, 21, 22]` (its rim
//!   at x 0 and x 22-23, rows 5-6), where the title is icon-shaped and
//!   carries the app mark glyph instead of the word (see [`title_label`]);
//! - the art bakes a word the Host keeps (Tools, Help): no label;
//! - no word in the art: `{text}`, no cover.
//!
//! The word's box (the part of the cover that must hide) starts as [`super::cover::icon_box`] on the art under the
//! title's box (what stands off each row's ground, the frame dropped), then
//! grows (see [`word_cover`]): that measure takes each row's ground from
//! its outer quarters, so a word reaching into a flank is read as ground
//! there (Alloy's "E", x 37-45 of its 48 px art, fills the right quarter of
//! its rows and the measure stops at x 36). The word's ink is the
//! commonest colour of what the measure found; the box grows a column or
//! row at a time while the next one holds ink, or a visible pixel off its
//! row's ground beside ink (the anti-aliased edge: Total Chaos's dim "X"
//! foot). Measured on the four art-titled packs: Symbiosis "Xfire"
//! `[12, 4, 22, 10]`, Alloy "XFIRE" `[2, 2, 44, 11]`, Total Chaos "XFIRE"
//! `[15, 3, 46, 9]` (its word x 15-49; the box keeps the right gauge bar
//! the first measure joined to the "E"), XGlow "File" and its reflection
//! `[1, 2, 21, 17]` (the reflection mirrors "File" under the word, so it
//! is hidden with it: a reflection of "File" under "Rekindle" would show
//! the old word). SK3's worded-tab band (the box less fixed fractions) is
//! not used: it guesses a framed tab's band, and a menu title has no
//! frame. The art under the title is the title's own whole picture, a part's,
//! or, when it has none, the nearest enclosing node's whole picture cropped
//! to the title's box (Total Chaos's titles lie over their words in
//! `MenuBg`).

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{Insets, SkinError, SkinRule, WindowLayout};
use rekindle_types::dto::skin_action::{EngineMenu, SkinAction};
use rekindle_types::dto::skin_tree::{NodeGlyph, NodeLabel, NodeRole, SkinNode};

use super::cover::{differing, edge_connected, icon_box, seen, ICON_TOLERANCE};
use crate::package::Package;

/// A whole picture laid in the window: its image and where its top-left
/// sits, px.
#[derive(Clone)]
struct Pic {
    image: String,
    owner: String,
    at: [i32; 2],
}

/// The node's own whole picture: its one normal piece, at least as large as
/// its box `[x, y, w, h]` (a tile that repeats, Onyx's 4 px hover ground, is
/// not one).
fn whole(n: &SkinNode, b: [i32; 4]) -> Option<Pic> {
    let art = n.art.as_ref()?;
    if art.normal.len() != 1 {
        return None;
    }
    let piece = art.normal.values().next()?;
    (i32::from(piece.width) >= b[2] && i32::from(piece.height) >= b[3]).then(|| Pic {
        image: piece.image.clone(),
        owner: n.id.clone(),
        at: [b[0], b[1]],
    })
}

/// The box of `node` in its parent's box `parent`.
pub(super) fn place(node: &SkinNode, parent: [i32; 4]) -> [i32; 4] {
    let (left, wide) = node.x.place(parent[2]);
    let (top, high) = node.y.place(parent[3]);
    [parent[0] + left, parent[1] + top, wide, high]
}

/// The node's or a descendant's whole picture.
fn own_or_part(n: &SkinNode, b: [i32; 4]) -> Option<Pic> {
    whole(n, b).or_else(|| n.children.iter().find_map(|c| own_or_part(c, place(c, b))))
}

/// The engine menu the title opens: its own trigger or a part's (Xfire
/// Classic's `XfireMenuText` holds the `FileMenu` part that runs it).
fn menu_of(n: &SkinNode) -> Option<EngineMenu> {
    n.triggers
        .iter()
        .find_map(|t| match t.run {
            SkinAction::Menu(m) => Some(m),
            _ => None,
        })
        .or_else(|| n.children.iter().find_map(menu_of))
}

/// The box, in the title's box `title`, of the word in `pic` under it.
fn word_box(
    pic: &Pic,
    title: [i32; 4],
    package: &Package<'_>,
) -> Result<Option<[u16; 4]>, SkinError> {
    let bytes = package.get(&pic.image).ok_or_else(|| {
        SkinError::new(
            pic.image.as_str(),
            SkinRule::MissingEntry {
                field: format!("{}.art.normal", pic.owner),
            },
        )
    })?;
    let frames = crate::regions::frames(&pic.image, bytes)?;
    let Some(image) = frames.first() else {
        return Ok(None);
    };
    let (dx, dy) = (title[0] - pic.at[0], title[1] - pic.at[1]);
    let size = |v: u32| i32::try_from(v).unwrap_or(0);
    let across = title[2].min(size(image.width()) - dx);
    let down = title[3].min(size(image.height()) - dy);
    let (Ok(left), Ok(top), Ok(cols), Ok(rows)) = (
        u32::try_from(dx),
        u32::try_from(dy),
        u32::try_from(across),
        u32::try_from(down),
    ) else {
        return Ok(None);
    };
    if cols == 0 || rows == 0 {
        return Ok(None);
    }
    let under = RgbaImage::from_fn(cols, rows, |col, row| {
        *image.get_pixel(left + col, top + row)
    });
    Ok(word_cover(&under).map(|word| usable(&under, word)))
}

/// The title's usable box (plan SK5): the word's box `word` grown a column
/// or row at a time, up to the art's edge, while the next one across the
/// box holds no frame pixel (the differing pixels edge-connected, as
/// [`super::cover::icon_box`] drops them), so "Rekindle" gets all the room
/// the old word's rim leaves and the frame stays drawn.
fn usable(image: &RgbaImage, word: [u16; 4]) -> [u16; 4] {
    let (Ok(cols), Ok(rows)) = (
        usize::try_from(image.width()),
        usize::try_from(image.height()),
    ) else {
        return word;
    };
    let frame = edge_connected(&differing(image, cols), cols, rows);
    let [left, top, wide, high] = word.map(usize::from);
    let mut e = [left, top, left + wide - 1, top + high - 1];
    let column = |col: usize, e: [usize; 4]| (e[1]..=e[3]).all(|row| !frame[row * cols + col]);
    let line = |row: usize, e: [usize; 4]| (e[0]..=e[2]).all(|col| !frame[row * cols + col]);
    loop {
        let before = e;
        if e[0] > 0 && column(e[0] - 1, e) {
            e[0] -= 1;
        }
        if e[2] + 1 < cols && column(e[2] + 1, e) {
            e[2] += 1;
        }
        if e[1] > 0 && line(e[1] - 1, e) {
            e[1] -= 1;
        }
        if e[3] + 1 < rows && line(e[3] + 1, e) {
            e[3] += 1;
        }
        if e == before {
            break;
        }
    }
    let side = |v: usize| u16::try_from(v).unwrap_or(u16::MAX);
    [
        side(e[0]),
        side(e[1]),
        side(e[2] - e[0] + 1),
        side(e[3] - e[1] + 1),
    ]
}

/// Within [`ICON_TOLERANCE`] of `ink` on every channel.
fn inked(colour: [u8; 4], ink: [u8; 4]) -> bool {
    colour[3] != 0
        && colour
            .iter()
            .zip(ink)
            .all(|(a, b)| a.abs_diff(b) <= ICON_TOLERANCE)
}

/// The commonest colour of the measured word pixels inside `edges`
/// (`[left, top, right, bottom]`, inclusive).
fn ink_of(pixels: &[[u8; 4]], word: &[bool], cols: usize, edges: [usize; 4]) -> Option<[u8; 4]> {
    let mut counts: std::collections::HashMap<[u8; 4], usize> = std::collections::HashMap::new();
    for row in edges[1]..=edges[3] {
        for at in row * cols + edges[0]..=row * cols + edges[2] {
            if word[at] {
                *counts.entry(pixels[at]).or_default() += 1;
            }
        }
    }
    counts
        .into_iter()
        .max_by_key(|&(colour, count)| (count, colour))
        .map(|(colour, _)| colour)
}

/// Widens `edges` a column or row at a time while the next one, across the
/// box, holds a word pixel.
fn grow(word: &[bool], cols: usize, rows: usize, edges: &mut [usize; 4]) {
    let column = |col: usize, e: [usize; 4]| (e[1]..=e[3]).any(|row| word[row * cols + col]);
    let line = |row: usize, e: [usize; 4]| (e[0]..=e[2]).any(|col| word[row * cols + col]);
    loop {
        let before = *edges;
        if edges[0] > 0 && column(edges[0] - 1, *edges) {
            edges[0] -= 1;
        }
        if edges[2] + 1 < cols && column(edges[2] + 1, *edges) {
            edges[2] += 1;
        }
        if edges[1] > 0 && line(edges[1] - 1, *edges) {
            edges[1] -= 1;
        }
        if edges[3] + 1 < rows && line(edges[3] + 1, *edges) {
            edges[3] += 1;
        }
        if *edges == before {
            return;
        }
    }
}

/// The box of the word baked into `image` (see the module doc): the icon
/// box, grown while the next column or row holds the word's ink or its
/// anti-aliased edge.
fn word_cover(image: &RgbaImage) -> Option<[u16; 4]> {
    let [left, top, wide, high] = icon_box(image)?.map(usize::from);
    let cols = usize::try_from(image.width()).ok()?;
    let rows = usize::try_from(image.height()).ok()?;
    let pixels: Vec<[u8; 4]> = image
        .rows()
        .flat_map(|line| line.map(|pixel| seen(pixel.0)))
        .collect();
    let differs = differing(image, cols);
    let frame = edge_connected(&differs, cols, rows);
    let measured: Vec<bool> = differs
        .iter()
        .zip(&frame)
        .map(|(off, edge)| *off && !*edge)
        .collect();
    let mut edges = [left, top, left + wide - 1, top + high - 1];
    let ink = ink_of(&pixels, &measured, cols, edges)?;
    let like: Vec<bool> = pixels.iter().map(|&p| inked(p, ink)).collect();
    let beside_ink = |at: usize| {
        let (col, row) = (at % cols, at / cols);
        (row.saturating_sub(1)..=(row + 1).min(rows - 1)).any(|near_row| {
            (col.saturating_sub(1)..=(col + 1).min(cols - 1))
                .any(|near_col| like[near_row * cols + near_col])
        })
    };
    let word: Vec<bool> = (0..pixels.len())
        .map(|at| like[at] || (pixels[at][3] != 0 && differs[at] && beside_ink(at)))
        .collect();
    grow(&word, cols, rows, &mut edges);
    let side = |v: usize| u16::try_from(v).ok();
    Some([
        side(edges[0])?,
        side(edges[1])?,
        side(edges[2] - edges[0] + 1)?,
        side(edges[3] - edges[1] + 1)?,
    ])
}

/// The Host's label for a menu title opening `menu` whose art's word, if
/// any, lies at `word`. A renamed title whose node is icon-shaped (`icon`:
/// one whole picture, its box at most twice as wide as tall, SK3's test
/// for an icon tab in [`super::place`]) gets the app mark glyph over the
/// cover, its words the accessible name (plan SK5, lead decision: XGlow's
/// 24 x 22 file title leaves 21 px, where no legible "Rekindle" fits).
fn title_label(menu: EngineMenu, word: Option<[u16; 4]>, icon: bool) -> Option<NodeLabel> {
    let words = menu.title();
    let label = |cover| NodeLabel {
        text: words.ours.to_owned(),
        cover,
        glyph: (icon && word.is_some()).then_some(NodeGlyph::Rekindle),
    };
    match word {
        Some(cover) => words.renamed().then(|| label(Some(cover))),
        None => Some(label(None)),
    }
}

fn walk(
    nodes: &mut [SkinNode],
    parent: [i32; 4],
    under: Option<&Pic>,
    package: &Package<'_>,
) -> Result<(), SkinError> {
    for n in nodes {
        let b = place(n, parent);
        if n.role == NodeRole::MenuTitle {
            if let Some(menu) = menu_of(n) {
                let own = own_or_part(n, b);
                let word = match own.as_ref().or(under) {
                    Some(pic) => word_box(pic, b, package)?,
                    None => None,
                };
                let icon = own.is_some() && b[2] <= 2 * b[3];
                n.label = title_label(menu, word, icon);
            }
        }
        let own = whole(n, b);
        walk(&mut n.children, b, own.as_ref().or(under), package)?;
    }
    Ok(())
}

/// Labels every tab the pack draws as text (plan SK6): a `Tab` node with no
/// art of its own is a text component, its word drawn by the engine from
/// the pack's strings over its button part (Classic's `TabNormalText`,
/// `Class="Text"` 40 x 15, `<Text>FriendsTitle</Text>`,
/// `xfirecomponents.xml:1071-1079`, laid over `TabNormalButton`,
/// `xfiremainwindow.xml:73-75`). Those strings are resource ids of the
/// Xfire binary (`strings.xml:21`), so the words are the Host's names for
/// the views ([`ViewKey::title`](rekindle_types::dto::skin_tree_role::ViewKey::title)): `{text}`, no cover, no glyph. A tab
/// whose art draws its word (Alloy's, Symbiosis's icons) and the Host's
/// own Communities tab, labelled already (rule N4), keep theirs.
fn tab_words(nodes: &mut [SkinNode]) {
    for n in nodes {
        if let NodeRole::Tab(view) = n.role {
            if n.art.is_none() && n.label.is_none() {
                n.label = Some(NodeLabel {
                    text: view.title().to_owned(),
                    cover: None,
                    glyph: None,
                });
            }
        }
        tab_words(&mut n.children);
    }
}

fn label_tree(
    root: &mut [SkinNode],
    outsets: Insets,
    min: [u16; 2],
    package: &Package<'_>,
) -> Result<(), SkinError> {
    walk(root, laid_window(outsets, min), None, package)
}

/// The pack's window `[x, y, w, h]` in a tree laid at the minimum `min`
/// with the frame's `outsets` around it.
pub(super) fn laid_window(outsets: Insets, min: [u16; 2]) -> [i32; 4] {
    let w = i32::from(min[0]) + i32::from(outsets.left) + i32::from(outsets.right);
    let h = i32::from(min[1]) + i32::from(outsets.top) + i32::from(outsets.bottom);
    let (left, top) = (i32::from(outsets.left), i32::from(outsets.top));
    [
        left,
        top,
        w - left - i32::from(outsets.right),
        h - top - i32::from(outsets.bottom),
    ]
}

/// Labels every menu title in `layout`'s tree and popups (see the module
/// doc), laid at the window's minimum size, and every tab the pack
/// draws as text ([`tab_words`]).
///
/// # Errors
/// A title's picture missing from the package or not decoding.
pub fn menus(layout: &mut WindowLayout, package: &Package<'_>) -> Result<(), SkinError> {
    let min = [layout.min_width, layout.min_height];
    if let Some(tree) = layout.tree.as_mut() {
        label_tree(&mut tree.root, tree.outsets, min, package)?;
        tab_words(&mut tree.root);
    }
    for popup in layout.popups.values_mut() {
        let min = [popup.min_width, popup.min_height];
        label_tree(&mut popup.tree.root, popup.tree.outsets, min, package)?;
        tab_words(&mut popup.tree.root);
    }
    Ok(())
}
