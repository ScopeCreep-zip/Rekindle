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
//!   the cover the word's box, so the shell fills it with the art's ground
//!   and draws Rekindle's name there;
//! - the art bakes a word the Host keeps (Tools, Help): no label;
//! - no word in the art: `{text}`, no cover.
//!
//! The word's box is [`super::cover::icon_box`] on the art under the title's
//! box: a menu title's word sits on a flat ground away from the art's edges,
//! exactly the shape that measure reads (on Symbiosis's art,
//! `[11, 5, 23, 10]`). SK3's worded-tab band (the box less fixed fractions)
//! is not used: it guesses a framed tab's band, and a menu title has no
//! frame, so the band would cover most of the art and miss nothing only by
//! luck. The art under the title is the title's own whole picture, a part's,
//! or, when it has none, the nearest enclosing node's whole picture cropped
//! to the title's box (Total Chaos's titles lie over their words in
//! `MenuBg`).

use rekindle_image::RgbaImage;
use rekindle_types::dto::skin::{Insets, SkinError, SkinRule, WindowLayout};
use rekindle_types::dto::skin_action::{EngineMenu, SkinAction};
use rekindle_types::dto::skin_tree::{NodeLabel, NodeRole, SkinNode};

use super::cover::icon_box;
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

/// The box of `n` in its parent's box `p`.
fn place(n: &SkinNode, p: [i32; 4]) -> [i32; 4] {
    let (x, w) = n.x.place(p[2]);
    let (y, h) = n.y.place(p[3]);
    [p[0] + x, p[1] + y, w, h]
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

/// The box, in the title's box `b`, of the word in `pic` under it.
fn word_box(pic: &Pic, b: [i32; 4], package: &Package<'_>) -> Result<Option<[u16; 4]>, SkinError> {
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
    let (dx, dy) = (b[0] - pic.at[0], b[1] - pic.at[1]);
    let size = |v: u32| i32::try_from(v).unwrap_or(0);
    let across = b[2].min(size(image.width()) - dx);
    let down = b[3].min(size(image.height()) - dy);
    let (Ok(x0), Ok(y0), Ok(w), Ok(h)) = (
        u32::try_from(dx),
        u32::try_from(dy),
        u32::try_from(across),
        u32::try_from(down),
    ) else {
        return Ok(None);
    };
    if w == 0 || h == 0 {
        return Ok(None);
    }
    let under = RgbaImage::from_fn(w, h, |x, y| *image.get_pixel(x0 + x, y0 + y));
    Ok(icon_box(&under))
}

/// The Host's label for a menu title opening `menu` whose art's word, if
/// any, lies at `word`.
fn title_label(menu: EngineMenu, word: Option<[u16; 4]>) -> Option<NodeLabel> {
    let words = menu.title();
    let label = |cover| NodeLabel {
        text: words.ours.to_owned(),
        cover,
        glyph: None,
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
                let word = match own_or_part(n, b).as_ref().or(under) {
                    Some(pic) => word_box(pic, b, package)?,
                    None => None,
                };
                n.label = title_label(menu, word);
            }
        }
        let own = whole(n, b);
        walk(&mut n.children, b, own.as_ref().or(under), package)?;
    }
    Ok(())
}

fn label_tree(
    root: &mut [SkinNode],
    outsets: Insets,
    min: [u16; 2],
    package: &Package<'_>,
) -> Result<(), SkinError> {
    let w = i32::from(min[0]) + i32::from(outsets.left) + i32::from(outsets.right);
    let h = i32::from(min[1]) + i32::from(outsets.top) + i32::from(outsets.bottom);
    let (left, top) = (i32::from(outsets.left), i32::from(outsets.top));
    let window = [
        left,
        top,
        w - left - i32::from(outsets.right),
        h - top - i32::from(outsets.bottom),
    ];
    walk(root, window, None, package)
}

/// Labels every menu title in `layout`'s tree and popups (see the module
/// doc), laid at the window's minimum size.
///
/// # Errors
/// A title's picture missing from the package or not decoding.
pub fn menus(layout: &mut WindowLayout, package: &Package<'_>) -> Result<(), SkinError> {
    let min = [layout.min_width, layout.min_height];
    if let Some(tree) = layout.tree.as_mut() {
        label_tree(&mut tree.root, tree.outsets, min, package)?;
    }
    for popup in layout.popups.values_mut() {
        let min = [popup.min_width, popup.min_height];
        label_tree(&mut popup.tree.root, popup.tree.outsets, min, package)?;
    }
    Ok(())
}
